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

async fn look(
    client: &kube::Client,
    observation: &Observation,
    target: &Target,
    now: Result<Value, Refusal>,
) -> Look {
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
    if reached(client, observation, target, &object).await {
        Look::Reached
    } else {
        Look::Waiting
    }
}

/// Whether the observation holds on the object as read now.
fn reached<'a>(
    client: &'a kube::Client,
    observation: &'a Observation,
    target: &'a Target,
    object: &'a Value,
) -> std::pin::Pin<Box<dyn std::future::Future<Output = bool> + Send + 'a>> {
    Box::pin(async move {
        match observation {
            Observation::FieldEquals { pointer, value } => object.pointer(pointer) == Some(value),
            Observation::FieldEqualsOrAbsent { pointer, value } => {
                object.pointer(pointer) == Some(value) || absent_is_zero(object, pointer, value)
            }
            Observation::FieldAbsent { pointer } => {
                matches!(object.pointer(pointer), None | Some(Value::Null))
            }
            Observation::FieldMoved { pointer, from } => object
                .pointer(pointer)
                .is_some_and(|now| !now.is_null() && now != from),
            Observation::All(all) => {
                for one in all {
                    if !reached(client, one, target, object).await {
                        return false;
                    }
                }
                true
            }
            Observation::Any(any) => {
                for one in any {
                    if reached(client, one, target, object).await {
                        return true;
                    }
                }
                false
            }
            Observation::Child {
                group,
                version,
                kind,
                resource,
                annotation,
            } => {
                controlled_with(client, target, (group, version, kind, resource), annotation).await
            }
            Observation::Exists => true,
            Observation::Gone | Observation::Unobservable => false,
        }
    })
}

/// Whether the namespace holds an object of this kind that `target` controls
/// and that carries the annotation. A list that fails says no.
async fn controlled_with(
    client: &kube::Client,
    target: &Target,
    (group, version, kind, resource): (&str, &str, &str, &str),
    (key, value): &(String, String),
) -> bool {
    use kube::api::{Api, ApiResource, DynamicObject, ListParams};
    let Some(namespace) = target.namespace.as_deref() else {
        return false;
    };
    let resource = ApiResource {
        group: group.into(),
        version: version.into(),
        api_version: format!("{group}/{version}"),
        kind: kind.into(),
        plural: resource.into(),
    };
    let api: Api<DynamicObject> = Api::namespaced_with(client.clone(), namespace, &resource);
    let Ok(list) = api.list(&ListParams::default().limit(500)).await else {
        return false;
    };
    list.items.iter().any(|item| {
        let annotated = item
            .metadata
            .annotations
            .as_ref()
            .and_then(|annotations| annotations.get(key))
            == Some(value);
        let controlled = item
            .metadata
            .owner_references
            .as_ref()
            .is_some_and(|owners| {
                owners
                    .iter()
                    .any(|owner| owner.uid == target.uid && owner.controller == Some(true))
            });
        annotated && controlled
    })
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
        match look(client, observation, target, get(client, target).await).await {
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
