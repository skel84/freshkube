//! The states of the connect step that are not "connected", each with words
//! that say what happened and what to do about it.
use freshkube_core::databases::explore::ExploreError;
use freshkube_db::ConnectError;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Fail {
    /// Only PostgreSQL can be explored in this slice.
    Unsupported(String),
    NoReadyPod,
    /// Neither Secret exists; the name is the one looked for last.
    NoSecret(String),
    SecretIncomplete(String),
    /// The Secret or login is a superuser's.
    Superuser(String),
    NoUid(String),
    /// The cluster refused or failed a read before the database was reached.
    Cluster(String),
    ForwardFailed(String),
    Refused(String),
    TimedOut,
    Unreachable(String),
    NotReadOnly(String),
    /// TLS was offered and the certificate was not accepted.
    Untrusted(String),
}

impl Fail {
    pub(crate) fn of_explore(error: ExploreError, app_secret: &str) -> Self {
        match error {
            ExploreError::Unsupported(engine) => Self::Unsupported(engine.product().to_owned()),
            ExploreError::NoReadyPod => Self::NoReadyPod,
            ExploreError::NoSecret => Self::NoSecret(app_secret.to_owned()),
            ExploreError::SecretIncomplete(name) => Self::SecretIncomplete(name),
            ExploreError::Superuser(name) => Self::Superuser(name),
            ExploreError::NoUid(pod) => Self::NoUid(pod),
            ExploreError::Read(failure) => {
                Self::Cluster(format!("{}: {}", failure.kind.label(), failure.message))
            }
        }
    }

    pub(crate) fn of_connect(error: ConnectError) -> Self {
        match error {
            ConnectError::Refused(why) => Self::Refused(why),
            ConnectError::TimedOut => Self::TimedOut,
            ConnectError::Unreachable(why) => Self::Unreachable(why),
            ConnectError::NotReadOnly(why) => Self::NotReadOnly(why),
            ConnectError::Untrusted(why) => Self::Untrusted(why),
        }
    }

    /// Stable, for element ids and tests.
    pub(crate) fn slug(&self) -> &'static str {
        match self {
            Self::Unsupported(_) => "unsupported",
            Self::NoReadyPod => "no-ready-pod",
            Self::NoSecret(_) => "no-secret",
            Self::SecretIncomplete(_) => "secret-incomplete",
            Self::Superuser(_) => "superuser",
            Self::NoUid(_) => "no-uid",
            Self::Untrusted(_) => "untrusted",
            Self::Cluster(_) => "cluster",
            Self::ForwardFailed(_) => "forward-failed",
            Self::Refused(_) => "refused",
            Self::TimedOut => "timed-out",
            Self::Unreachable(_) => "unreachable",
            Self::NotReadOnly(_) => "not-read-only",
        }
    }

    pub(crate) fn title(&self) -> &'static str {
        match self {
            Self::Unsupported(_) => "Not explorable yet",
            Self::NoReadyPod => "No pod is ready",
            Self::NoSecret(_) => "No reader Secret",
            Self::SecretIncomplete(_) => "The Secret has no login",
            Self::Superuser(_) => "That is a superuser login",
            Self::NoUid(_) => "The pod can't be tied to a forward",
            Self::Untrusted(_) => "The server's certificate was not accepted",
            Self::Cluster(_) => "Couldn't read the cluster",
            Self::ForwardFailed(_) => "The forward failed",
            Self::Refused(_) => "The database refused the connection",
            Self::TimedOut => "The database did not answer in time",
            Self::Unreachable(_) => "The database could not be reached",
            Self::NotReadOnly(_) => "The session could not be made read-only",
        }
    }

    /// What happened, then what to do, as one description.
    pub(crate) fn words(&self) -> String {
        match self {
            Self::Unsupported(product) => format!(
                "{product} can't be explored in this version. PostgreSQL databases can."
            ),
            Self::NoReadyPod => "No instance of this database is ready, so there is nothing to connect to. Wait for one to become ready, then try again.".into(),
            Self::NoSecret(app) => format!(
                "Neither a reader Secret nor the application user's Secret ({app}) exists in this namespace. Ask your team for the reader Secret, or create {app}; Freshkube never offers the superuser's."
            ),
            Self::SecretIncomplete(name) => format!(
                "The Secret {name} has no username or password. Fix the Secret, then try again."
            ),
            Self::Superuser(name) => format!(
                "{name} is a superuser's login. Freshkube never connects with one; ask your team for a reader Secret."
            ),
            Self::NoUid(pod) => format!(
                "The pod {pod} reports no UID, so a forward could not stay on it. Try again once it is running."
            ),
            Self::Untrusted(why) => format!(
                "{why}. The server offers TLS but its certificate doesn't match the cluster's CA for this Service, so Freshkube didn't send a login to it."
            ),
            Self::Cluster(why) => format!("{why}. Check your access to this namespace, then try again."),
            Self::ForwardFailed(why) => format!(
                "{why}. The forward needs your permission to create pods/portforward; its state is in the status bar."
            ),
            Self::Refused(why) => format!(
                "The server said: {why}. The login in the Secret may be out of date, or this user may not reach the database."
            ),
            Self::TimedOut => "Nothing answered within the connect timeout. The forward may have lost its pod; try again.".into(),
            Self::Unreachable(why) => format!("{why}. The forward may have ended; try again."),
            Self::NotReadOnly(why) => format!(
                "{why}. Freshkube only explores through a read-only session, so it did not use this one."
            ),
        }
    }

    /// Whether trying again could help.
    pub(crate) fn retry(&self) -> bool {
        // Trying again can't change these: the engine, the setting the
        // server keeps off, or a Secret that is the wrong kind.
        !matches!(
            self,
            Self::Unsupported(_) | Self::NotReadOnly(_) | Self::Superuser(_)
        )
    }
}
