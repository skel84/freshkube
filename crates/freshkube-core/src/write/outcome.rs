//! Accepted is not observed. The API saying yes is one fact; the cluster
//! reaching the new state is another. An outcome never seen says so after a
//! bounded wait, and never claims success.

use std::time::Duration;

use serde_json::Value;
use tokio::time::{Instant, sleep};

use super::{
    action::{Observation, Target},
    send::{Refusal, get},
};

/// How long to look by default, and the most anyone may ask for.
pub const DEFAULT_WAIT: Duration = Duration::from_secs(30);
pub const MAX_WAIT: Duration = Duration::from_secs(120);

/// How to look: for how long, and how often.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Watching {
    pub wait: Duration,
    pub every: Duration,
}

impl Default for Watching {
    fn default() -> Self {
        Self {
            wait: DEFAULT_WAIT,
            every: Duration::from_secs(1),
        }
    }
}

impl Watching {
    pub fn clamped(self) -> Self {
        Self {
            wait: self.wait.min(MAX_WAIT),
            every: self.every.max(Duration::from_millis(10)),
        }
    }
}

/// Why an action with nothing to watch is not seen to take effect.
pub const NOT_WATCHABLE: &str = "This action has no outcome the app can watch.";

/// The API took the write.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Accepted {
    /// The object's resourceVersion after the write, if the reply gave it.
    pub resource_version: Option<String>,
}

/// Whether the cluster got there.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Observed {
    Reached,
    /// The wait ended first. Not a failure and not success.
    NotObserved {
        waited: Duration,
        why: &'static str,
    },
    /// The object is no longer the one that was written.
    Diverged {
        why: &'static str,
    },
}

impl Observed {
    pub fn is_reached(&self) -> bool {
        matches!(self, Observed::Reached)
    }
}

enum Look {
    Reached,
    Waiting,
    Diverged(&'static str),
}

fn look(observation: &Observation, target: &Target, now: Result<Value, Refusal>) -> Look {
    let object = match now {
        Ok(object) => object,
        Err(Refusal::Changed(super::send::ChangedWhat::Gone)) => {
            return match observation {
                Observation::Gone => Look::Reached,
                _ => Look::Diverged("The object was deleted."),
            };
        }
        // A failed read is a reason to look again, not an answer.
        Err(_) => return Look::Waiting,
    };
    let uid = object["metadata"]["uid"].as_str().unwrap_or_default();
    if !target.uid.is_empty() && uid != target.uid {
        return match observation {
            Observation::Gone => Look::Reached,
            _ => Look::Diverged("The object was deleted and created again."),
        };
    }
    match observation {
        Observation::FieldEquals { pointer, value } => {
            if object.pointer(pointer) == Some(value) {
                Look::Reached
            } else {
                Look::Waiting
            }
        }
        Observation::FieldEqualsOrAbsent { pointer, value } => {
            if object.pointer(pointer) == Some(value) || absent_is_zero(&object, pointer, value) {
                Look::Reached
            } else {
                Look::Waiting
            }
        }
        Observation::Exists => Look::Reached,
        Observation::Gone | Observation::Unobservable => Look::Waiting,
    }
}

/// An absent count reads as 0, as the API omits a count at zero.
pub(super) fn absent_is_zero(object: &Value, pointer: &str, value: &Value) -> bool {
    value == &Value::from(0) && matches!(object.pointer(pointer), None | Some(Value::Null))
}

/// Looks until the observation holds, the object turns out not to be the one
/// written, or the wait ends. `progress` is told how long it has looked.
pub async fn observe(
    client: &kube::Client,
    target: &Target,
    observation: &Observation,
    watching: Watching,
    mut progress: impl FnMut(Duration),
) -> Observed {
    let watching = watching.clamped();
    if matches!(observation, Observation::Unobservable) {
        return Observed::NotObserved {
            waited: Duration::ZERO,
            why: NOT_WATCHABLE,
        };
    }
    let started = Instant::now();
    loop {
        match look(observation, target, get(client, target).await) {
            Look::Reached => return Observed::Reached,
            Look::Diverged(why) => return Observed::Diverged { why },
            Look::Waiting => {}
        }
        let waited = started.elapsed();
        if waited >= watching.wait {
            return Observed::NotObserved {
                waited,
                why: "The cluster did not reach the new state in time.",
            };
        }
        progress(waited);
        sleep(watching.every.min(watching.wait - waited)).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_wait_is_bounded() {
        let clamped = Watching {
            wait: Duration::from_secs(3600),
            every: Duration::ZERO,
        }
        .clamped();
        assert_eq!(clamped.wait, MAX_WAIT);
        assert!(clamped.every > Duration::ZERO);
        assert_eq!(Watching::default().wait, Duration::from_secs(30));
    }
}
