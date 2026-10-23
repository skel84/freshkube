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
use kube::{
    Api,
    api::{ApiResource, DynamicObject},
};
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
    /// The Secret or login is a superuser's; it is never offered.
    Superuser(String),
    /// The chosen pod reports no UID, so a forward could not stay on it.
    NoUid(String),
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
    let Some(uid) = meta.metadata.uid.filter(|uid| !uid.is_empty()) else {
        return Err(ExploreError::NoUid(name));
    };
    Ok(PodChoice { name, uid, replica })
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

/// The Secret CloudNativePG keeps for a cluster's superuser by default.
pub fn superuser_secret_name(cluster: &str) -> String {
    format!("{cluster}-superuser")
}

/// The Secret CloudNativePG keeps for a cluster's CA by default.
pub fn ca_secret_name(cluster: &str) -> String {
    format!("{cluster}-ca")
}

/// The Service name a server certificate covers: the read-only Service when
/// the pod is a replica, the read-write one for the primary.
pub fn server_name(database: &Database, replica: bool) -> String {
    format!(
        "{}-{}.{}.svc",
        database.identity.name,
        if replica { "ro" } else { "rw" },
        database.identity.namespace
    )
}

/// What the Cluster's spec names, so a renamed Secret is still recognised.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SpecNames {
    pub superuser_secret: Option<String>,
    pub server_ca_secret: Option<String>,
}

fn spec_names(object: &DynamicObject) -> SpecNames {
    let name = |path: &[&str]| {
        let mut value = object.data.get("spec")?;
        for key in path {
            value = value.get(key)?;
        }
        value.as_str().map(str::to_owned).filter(|n| !n.is_empty())
    };
    SpecNames {
        superuser_secret: name(&["superuserSecret", "name"]),
        server_ca_secret: name(&["certificates", "serverCASecret"]),
    }
}

async fn read_spec_names(client: &kube::Client, database: &Database) -> SpecNames {
    let id = &database.identity;
    let resource = ApiResource {
        group: id.group.clone(),
        version: id.version.clone(),
        api_version: format!("{}/{}", id.group, id.version),
        kind: id.kind.clone(),
        plural: id.plural.clone(),
    };
    let api: Api<DynamicObject> = Api::namespaced_with(client.clone(), &id.namespace, &resource);
    match api.get_opt(&id.name).await {
        Ok(Some(object)) => spec_names(&object),
        _ => SpecNames::default(),
    }
}

/// Refuses a Secret that is the superuser's, by the default name or the
/// one the Cluster's spec gives.
pub fn refuse_superuser_secret(
    cluster: &str,
    names: &SpecNames,
    secret: &str,
) -> Result<(), ExploreError> {
    let is_superuser = secret == superuser_secret_name(cluster)
        || names.superuser_secret.as_deref() == Some(secret);
    if is_superuser {
        Err(ExploreError::Superuser(secret.to_owned()))
    } else {
        Ok(())
    }
}

/// Reads the login once: the reader Secret if one is named and exists, else
/// the app user's. `reader` is the team's name for it (E4 supplies it). A
/// Secret that is the cluster's superuser's, however it was named, and a
/// login whose user is `postgres` are refused.
pub async fn read_login(
    client: &kube::Client,
    database: &Database,
    reader: Option<&str>,
) -> Result<Login, ExploreError> {
    let cluster = &database.identity.name;
    let names = read_spec_names(client, database).await;
    if let Some(reader) = reader {
        refuse_superuser_secret(cluster, &names, reader)?;
    }
    let secrets: Api<Secret> = Api::namespaced(client.clone(), &database.identity.namespace);
    let app = app_secret_name(cluster);
    refuse_superuser_secret(cluster, &names, &app)?;
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

/// The cluster CA's certificate, when it can be read: `ca.crt` only. The
/// Secret also holds the CA's private key, which is dropped unread. `None`
/// means the connection is encrypted without being verified.
pub async fn read_ca(client: &kube::Client, database: &Database) -> Option<String> {
    let names = read_spec_names(client, database).await;
    let name = names
        .server_ca_secret
        .unwrap_or_else(|| ca_secret_name(&database.identity.name));
    let secrets: Api<Secret> = Api::namespaced(client.clone(), &database.identity.namespace);
    let secret = secrets.get_opt(&name).await.ok()??;
    let mut data = secret.data.unwrap_or_default();
    let ca = data
        .remove("ca.crt")
        .and_then(|v| String::from_utf8(v.0).ok());
    drop(data);
    ca.filter(|pem| pem.contains("BEGIN CERTIFICATE"))
}

fn login_of(kind: LoginKind, name: String, secret: Secret) -> Result<Login, ExploreError> {
    let mut data = secret.data.unwrap_or_default();
    let mut take = |key: &str| {
        data.remove(key)
            .and_then(|v| String::from_utf8(v.0).ok())
            .filter(|v| !v.is_empty())
    };
    let user = take("username").or_else(|| take("user"));
    let password = take("password");
    let database = take("dbname");
    // The Secret's other keys (connection strings that embed the password)
    // are dropped as soon as the three needed values are out.
    drop(data);
    let (Some(user), Some(password)) = (user, password) else {
        return Err(ExploreError::SecretIncomplete(name));
    };
    if user == "postgres" {
        return Err(ExploreError::Superuser(name));
    }
    Ok(Login {
        kind,
        secret: name,
        user,
        database: database.unwrap_or_else(|| "app".into()),
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

    #[test]
    fn a_reader_named_as_the_superuser_secret_is_refused_by_default_name_or_spec() {
        let none = SpecNames::default();
        assert_eq!(
            refuse_superuser_secret("orders-db", &none, "orders-db-superuser"),
            Err(ExploreError::Superuser("orders-db-superuser".into()))
        );
        let renamed = SpecNames {
            superuser_secret: Some("dba-login".into()),
            ..none.clone()
        };
        assert_eq!(
            refuse_superuser_secret("orders-db", &renamed, "dba-login"),
            Err(ExploreError::Superuser("dba-login".into()))
        );
        assert_eq!(
            refuse_superuser_secret("orders-db", &renamed, "orders-reader"),
            Ok(())
        );
    }

    #[test]
    fn a_login_whose_user_is_postgres_is_refused_whatever_the_secret_is_called() {
        let s = secret(&[("username", "postgres"), ("password", "x")]);
        assert_eq!(
            login_of(LoginKind::Reader, "orders-reader".into(), s).err(),
            Some(ExploreError::Superuser("orders-reader".into()))
        );
    }

    #[test]
    fn the_spec_names_come_from_the_cluster_object() {
        let object: DynamicObject = serde_json::from_value(serde_json::json!({
            "apiVersion": "postgresql.cnpg.io/v1", "kind": "Cluster",
            "metadata": {"name": "orders-db"},
            "spec": {"superuserSecret": {"name": "dba-login"},
                     "certificates": {"serverCASecret": "my-ca"}}
        }))
        .unwrap();
        assert_eq!(
            spec_names(&object),
            SpecNames {
                superuser_secret: Some("dba-login".into()),
                server_ca_secret: Some("my-ca".into())
            }
        );
    }

    #[tokio::test]
    async fn a_reader_input_naming_the_superuser_secret_never_reads_any_secret() {
        let (client, recorder) = crate::fake_api::client(|r| {
            r.path
                .contains("/clusters/")
                .then(|| crate::fake_api::json(404, "{}"))
        });
        let db = database(
            &[("orders-db-1", Role::Primary, true)],
            Engine::CloudNativePg,
        );
        let error = read_login(&client, &db, Some("orders-db-superuser"))
            .await
            .err();
        assert_eq!(
            error,
            Some(ExploreError::Superuser("orders-db-superuser".into()))
        );
        recorder.assert_all("only the cluster is read", |r| !r.path.contains("/secrets"));
    }

    #[test]
    fn the_server_name_is_the_service_a_replica_or_the_primary_answers() {
        let db = database(
            &[("orders-db-1", Role::Primary, true)],
            Engine::CloudNativePg,
        );
        assert_eq!(server_name(&db, true), "orders-db-ro.staging.svc");
        assert_eq!(server_name(&db, false), "orders-db-rw.staging.svc");
    }
}
