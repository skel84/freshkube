//! A follow: what the user asked to be told about, on which cluster, and
//! which read it needs.

use std::fmt;
use std::time::Duration;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// The most follows at once. A 21st is refused, not queued.
pub const MAX_FOLLOWS: usize = 20;
/// How often Coroot's incidents are read.
pub const COROOT_EVERY: Duration = Duration::from_secs(60);
/// A crash-looping pod fires again only after it has been Ready this long.
pub const REARM_AFTER_READY: Duration = Duration::from_secs(5 * 60);

/// What a follow is about. Each says in words what it keeps reading
/// ([`Subject::reads`]).
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum Subject {
    /// A pod that keeps crash-looping.
    CrashLoop { namespace: String, pod: String },
    /// A Freight that reaches a Stage (production, for the change page).
    Landing {
        project: String,
        stage: String,
        freight: String,
    },
    /// A Freight whose verification in a Stage fails.
    Verification {
        project: String,
        stage: String,
        freight: String,
    },
    /// An incident that opens in Coroot, for one application or for any.
    Incident { application: Option<String> },
}

/// The read a subject needs. Follows with the same key share one read: one
/// watch per object or Kargo project, and Coroot once a minute.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum WatchKey {
    Pod { namespace: String, name: String },
    Kargo { project: String },
    Coroot,
}

impl Subject {
    pub fn watch_key(&self) -> WatchKey {
        match self {
            Subject::CrashLoop { namespace, pod } => WatchKey::Pod {
                namespace: namespace.clone(),
                name: pod.clone(),
            },
            Subject::Landing { project, .. } | Subject::Verification { project, .. } => {
                WatchKey::Kargo {
                    project: project.clone(),
                }
            }
            Subject::Incident { .. } => WatchKey::Coroot,
        }
    }

    /// What the follow is, as a row names it.
    pub fn title(&self) -> String {
        match self {
            Subject::CrashLoop { namespace, pod } => {
                format!("Pod {namespace}/{pod} keeps crash-looping")
            }
            Subject::Landing { stage, freight, .. } => {
                format!("Freight {freight} reaches {stage}")
            }
            Subject::Verification { stage, freight, .. } => {
                format!("Verification of {freight} fails in {stage}")
            }
            Subject::Incident {
                application: Some(app),
            } => format!("An incident opens in {app}"),
            Subject::Incident { application: None } => "An incident opens".to_owned(),
        }
    }

    /// What it keeps reading, said before it is turned on.
    pub fn reads(&self) -> String {
        match self.watch_key() {
            WatchKey::Pod { namespace, name } => format!("Watches pod {namespace}/{name}"),
            WatchKey::Kargo { project } => format!("Watches Kargo project {project}"),
            WatchKey::Coroot => "Reads Coroot's incidents once a minute".to_owned(),
        }
    }

    /// When it ends: a follow for one landing or one verification ends
    /// with its answer; the others run until stopped.
    pub fn ends(&self) -> &'static str {
        match self {
            Subject::Landing { .. } => "Ends when it lands",
            Subject::Verification { .. } => "Ends when it is verified or fails",
            Subject::CrashLoop { .. } | Subject::Incident { .. } => "Until you stop it",
        }
    }

    /// Whether a failure: crash loops, failed verifications and incidents
    /// come through quiet hours; a landing does not.
    pub fn is_failure(&self) -> bool {
        !matches!(self, Subject::Landing { .. })
    }
}

/// One thing followed.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Follow {
    pub id: u64,
    pub subject: Subject,
    /// The context it reads in, named by the user's choice, never the
    /// kubeconfig's current one.
    pub context: String,
    /// Whether the cluster is protected or read-only, which turns Hide
    /// details on by default for its updates.
    #[serde(default)]
    pub sensitive: bool,
    pub since: DateTime<Utc>,
}

impl fmt::Display for Follow {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} · {}", self.subject.title(), self.context)
    }
}

/// Why a follow was not added.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Refused {
    /// Already at [`MAX_FOLLOWS`].
    Full,
    /// The same subject on the same context is already followed.
    Duplicate,
    /// The shell has offered no connection for the context.
    NotOpen,
}

impl Refused {
    pub fn words(&self) -> String {
        match self {
            Refused::Full => format!("You follow {MAX_FOLLOWS} items already. Stop one first."),
            Refused::Duplicate => "You follow this already.".to_owned(),
            Refused::NotOpen => "This cluster isn't open here.".to_owned(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn follows_of_one_kargo_project_share_a_watch() {
        let landing = Subject::Landing {
            project: "shop".into(),
            stage: "prod-ams".into(),
            freight: "wonky-otter".into(),
        };
        let verification = Subject::Verification {
            project: "shop".into(),
            stage: "prod-ams".into(),
            freight: "wonky-otter".into(),
        };
        assert_eq!(landing.watch_key(), verification.watch_key());
        assert_ne!(
            landing.watch_key(),
            Subject::CrashLoop {
                namespace: "shop".into(),
                pod: "api".into()
            }
            .watch_key()
        );
    }

    #[test]
    fn only_a_landing_is_not_a_failure() {
        assert!(
            !Subject::Landing {
                project: "p".into(),
                stage: "s".into(),
                freight: "f".into()
            }
            .is_failure()
        );
        assert!(Subject::Incident { application: None }.is_failure());
    }

    #[test]
    fn a_subject_says_what_it_reads() {
        let subject = Subject::Incident {
            application: Some("checkout".into()),
        };
        assert_eq!(subject.reads(), "Reads Coroot's incidents once a minute");
        assert_eq!(subject.title(), "An incident opens in checkout");
    }
}
