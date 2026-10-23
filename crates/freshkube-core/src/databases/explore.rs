//! What the database explorer needs from the cluster (#948): the pod to
//! forward to, and the login to connect with.
//!
//! This is the one place the Databases feature reads a Secret. It does so
//! only when the user connects an explorer tab, once per connection, and
//! hands the value to the caller in a [`Login`], which is memory only, wiped
//! when dropped and printed as `<redacted>`. Nothing here logs it, stores it
//! or returns it in an error. The superuser Secret is never read: the choice
//! is the reader Secret the team names, then the operator's app user.
use super::{Database, Engine, Fact, Role};
use crate::failure::Failure;
use k8s_openapi::api::core::v1::{Pod, Secret};
use kube::Api;
use std::fmt;
use zeroize::Zeroizing;

/// The port a PostgreSQL pod serves.
pub const POSTGRES_PORT: u16 = 5432;

/// The pod a forward reaches.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PodChoice {
    pub name: String,
    pub uid: String,
    /// False when no replica was ready and the primary was used instead.
    pub replica: bool,
}

/// Why the explorer can't start, each a state of the connect step.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ExploreError {
    /// Only PostgreSQL can be explored in this slice.
    Unsupported(Engine),
    /// No instance is ready to forward to.
    NoReadyPod,
    /// Neither the reader Secret nor the app user's exists.
    NoSecret,
    /// The Secret exists but lacks a username or password.
    SecretIncomplete(String),
    Read(Failure),
}

/// Picks the pod: a ready replica, else the ready primary, by name.
pub fn choose_pod(database: &Database) -> Result<(String, bool), ExploreError> {
    if database.engine != Engine::CloudNativePg {
        return Err(ExploreError::Unsupported(database.engine));
    }
    let ready = |role: Role| {
        let mut names: Vec<&str> = database
            .instances
            .iter()
            .filter(|i| i.ready == Fact::Known(true) && i.role == Fact::Known(role))
            .map(|i| i.name.as_str())
            .collect();
        names.sort_unstable();
        names.first().map(|name| name.to_string())
    };
    if let Some(name) = ready(Role::Replica) {
        return Ok((name, true));
    }
    ready(Role::Primary)
        .map(|name| (name, false))
        .ok_or(ExploreError::NoReadyPod)
}

/// The pod to forward to, with the UID that keeps the forward on that pod.
pub async fn pick_pod(
    client: &kube::Client,
    database: &Database,
) -> Result<PodChoice, ExploreError> {
    let (name, replica) = choose_pod(database)?;
    let pods: Api<Pod> = Api::namespaced(client.clone(), &database.identity.namespace);
    let meta = pods
        .get_metadata(&name)
        .await
        .map_err(|e| ExploreError::Read(Failure::from_kube(e)))?;
    Ok(PodChoice {
        name,
        uid: meta.metadata.uid.unwrap_or_default(),
        replica,
    })
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LoginKind {
    /// The Secret the team names for read-only access.
    Reader,
    /// The operator's application user: its role can write, its session can't.
    AppUser,
}

impl LoginKind {
    pub fn label(self) -> &'static str {
        match self {
            Self::Reader => "reader Secret",
            Self::AppUser => "application user",
        }
    }
}

/// A login read from a Secret. No `Display`, and `Debug` hides the password.
pub struct Login {
    pub kind: LoginKind,
    /// The Secret's name, which is not secret.
    pub secret: String,
    pub user: String,
    pub database: String,
    password: Zeroizing<String>,
}

impl Login {
    pub fn password(&self) -> &str {
        &self.password
    }
}

impl fmt::Debug for Login {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Login")
            .field("kind", &self.kind)
            .field("secret", &self.secret)
            .field("user", &self.user)
            .field("database", &self.database)
            .field("password", &"<redacted>")
            .finish()
    }
}

/// The Secret CloudNativePG keeps for a cluster's application user.
pub fn app_secret_name(cluster: &str) -> String {
    format!("{cluster}-app")
}

/// Reads the login once: the reader Secret if one is named and exists, else
/// the app user's. `reader` is the team's name for it (E4 supplies it).
pub async fn read_login(
    client: &kube::Client,
    database: &Database,
    reader: Option<&str>,
) -> Result<Login, ExploreError> {
    let secrets: Api<Secret> = Api::namespaced(client.clone(), &database.identity.namespace);
    let app = app_secret_name(&database.identity.name);
    let candidates = reader
        .map(|name| (LoginKind::Reader, name.to_owned()))
        .into_iter()
        .chain(std::iter::once((LoginKind::AppUser, app)));
    for (kind, name) in candidates {
        let found = secrets
            .get_opt(&name)
            .await
            .map_err(|e| ExploreError::Read(Failure::from_kube(e)))?;
        if let Some(secret) = found {
            return login_of(kind, name, secret);
        }
    }
    Err(ExploreError::NoSecret)
}

fn login_of(kind: LoginKind, name: String, secret: Secret) -> Result<Login, ExploreError> {
    let data = secret.data.unwrap_or_default();
    let text = |key: &str| {
        data.get(key)
            .and_then(|v| String::from_utf8(v.0.clone()).ok())
            .filter(|v| !v.is_empty())
    };
    let user = text("username").or_else(|| text("user"));
    let password = text("password");
    let (Some(user), Some(password)) = (user, password) else {
        return Err(ExploreError::SecretIncomplete(name));
    };
    Ok(Login {
        kind,
        secret: name,
        user,
        database: text("dbname").unwrap_or_else(|| "app".into()),
        password: Zeroizing::new(password),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::databases::{Identity, Instance};
    use k8s_openapi::ByteString;
    use std::collections::BTreeMap;

    fn database(instances: &[(&str, Role, bool)], engine: Engine) -> Database {
        let mut db = crate::databases::example::inventory(chrono::Utc::now())
            .databases
            .items
            .remove(0);
        db.engine = engine;
        db.identity = Identity {
            group: "postgresql.cnpg.io".into(),
            version: "v1".into(),
            kind: "Cluster".into(),
            plural: "clusters".into(),
            namespace: "staging".into(),
            name: "orders-db".into(),
            uid: "u".into(),
        };
        db.instances = instances
            .iter()
            .map(|(name, role, ready)| {
                let mut i = Instance::named(*name);
                i.role = Fact::Known(*role);
                i.ready = Fact::Known(*ready);
                i
            })
            .collect();
        db
    }

    #[test]
    fn a_ready_replica_comes_first_then_the_primary() {
        let db = database(
            &[
                ("orders-db-1", Role::Primary, true),
                ("orders-db-3", Role::Replica, true),
                ("orders-db-2", Role::Replica, true),
            ],
            Engine::CloudNativePg,
        );
        assert_eq!(choose_pod(&db), Ok(("orders-db-2".into(), true)));
        let db = database(
            &[
                ("orders-db-1", Role::Primary, true),
                ("orders-db-2", Role::Replica, false),
            ],
            Engine::CloudNativePg,
        );
        assert_eq!(choose_pod(&db), Ok(("orders-db-1".into(), false)));
        let db = database(
            &[("orders-db-1", Role::Primary, false)],
            Engine::CloudNativePg,
        );
        assert_eq!(choose_pod(&db), Err(ExploreError::NoReadyPod));
    }

    #[test]
    fn mariadb_is_not_explored_yet() {
        let db = database(&[("m-0", Role::Primary, true)], Engine::MariaDb);
        assert_eq!(
            choose_pod(&db),
            Err(ExploreError::Unsupported(Engine::MariaDb))
        );
    }

    fn secret(pairs: &[(&str, &str)]) -> Secret {
        Secret {
            data: Some(
                pairs
                    .iter()
                    .map(|(k, v)| (k.to_string(), ByteString(v.as_bytes().to_vec())))
                    .collect::<BTreeMap<_, _>>(),
            ),
            ..Secret::default()
        }
    }

    #[test]
    fn a_login_reads_the_cnpg_keys_and_hides_the_password() {
        let s = secret(&[
            ("username", "app"),
            ("password", "hunter2"),
            ("dbname", "shop"),
        ]);
        let login = login_of(LoginKind::AppUser, "orders-db-app".into(), s).unwrap();
        assert_eq!(
            (login.user.as_str(), login.database.as_str()),
            ("app", "shop")
        );
        assert_eq!(login.password(), "hunter2");
        assert!(!format!("{login:?}").contains("hunter2"));
    }

    #[test]
    fn a_secret_without_a_password_is_incomplete_and_the_error_has_no_value() {
        let s = secret(&[("username", "app")]);
        let error = login_of(LoginKind::AppUser, "orders-db-app".into(), s)
            .err()
            .unwrap();
        assert_eq!(
            error,
            ExploreError::SecretIncomplete("orders-db-app".into())
        );
    }

    #[test]
    fn the_app_secret_is_never_the_superuser_secret() {
        assert_eq!(app_secret_name("orders-db"), "orders-db-app");
    }
}
