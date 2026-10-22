//! The runner: confirmation met, audit `sending`, send, observe, audit the
//! result. Plain Rust with no UI types. The front end gives it a client, a
//! prepared action and what the person did, and reads the events and the
//! result. Submitting is one-way: a [`Prepared`] is consumed, and after a
//! request that got no answer the result is "unknown" and nothing is resent.

use std::{io, sync::Mutex, time::Duration};

use super::{
    LevelCell, Phase, Protection,
    action::{ActionDescription, Inverse, ObjectChange, Target},
    audit::{AuditObject, AuditRequest, AuditStore, Record, RunId, SCHEMA, scrub},
    outcome::{Accepted, Observed, Watching, observe},
    preflight::{Prepared, Who},
    requirement::{Confirmation, UNDO_WINDOW},
    send::{ChangedWhat, Conflict, Refusal, send},
    undo::UndoOffer,
};

/// Sends prepared actions to one cluster.
pub struct Runner {
    pub(super) client: kube::Client,
    pub(super) audit: AuditStore,
    pub(super) who: Who,
    pub(super) watching: Watching,
    /// The cluster's level as it is now. A write reads it again just before
    /// it is sent.
    pub(super) level: LevelCell,
}

/// Something the front end can draw while a write runs.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Event {
    /// The write waits this long and can still be cancelled.
    CancelWindow(Duration),
    /// The `sending` line is written and requests begin.
    Sending,
    /// The API accepted this object's part (index into the action's changes).
    Accepted(usize),
    /// Looking for the cluster to reach the new state.
    Observing { waited: Duration, limit: Duration },
}

/// Why nothing was sent.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum NotSent {
    /// The level refuses writes.
    ReadOnly(&'static str),
    /// What the person did does not meet the level's requirement.
    Unconfirmed,
    /// The audit log can't be written, so nothing is sent.
    AuditUnavailable(String),
    /// The level got stricter after the action was prepared: nothing was
    /// sent, and the person is asked again under the new level.
    LevelChanged(Protection),
    /// Exec and port forwarding keep their own owners; they never go through
    /// this path.
    Session,
}

/// What happened to one object.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Step {
    /// An earlier object failed, so this one was not sent.
    NotAttempted,
    /// The API refused it.
    Refused {
        code: u16,
        message: String,
    },
    Denied(String),
    /// Server-side apply conflict. No force exists.
    Conflict(Conflict),
    Changed(ChangedWhat),
    /// No answer: whether the cluster took it is not known.
    Unknown(String),
    Accepted {
        accepted: Accepted,
        observed: Observed,
    },
}

impl Step {
    /// What happened to this object, in words that never read as more than
    /// it was.
    pub fn describe(&self) -> String {
        match self {
            Step::NotAttempted => "not sent (an earlier object failed)".to_owned(),
            Step::Refused { code, message } => {
                format!("refused by the cluster ({code}): {message}")
            }
            Step::Denied(message) => format!("not allowed: {message}"),
            Step::Conflict(conflict) => format!(
                "conflict with {} on {}",
                conflict.managers.join(", "),
                conflict.fields.join(", ")
            ),
            Step::Changed(what) => what.text().to_owned(),
            Step::Unknown(_) => {
                "no answer; not known whether the cluster took it, check the object".to_owned()
            }
            Step::Accepted { observed, .. } => match observed {
                Observed::Reached => "accepted and seen to take effect".to_owned(),
                Observed::NotObserved { .. } => "accepted, not seen to take effect".to_owned(),
                Observed::Diverged { .. } => {
                    "accepted, but the object is no longer the one written".to_owned()
                }
            },
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ObjectResult {
    pub object: String,
    pub step: Step,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Status {
    NotSent(NotSent),
    Cancelled,
    Sent,
}

/// How a write ended, in the words a toast uses.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Headline {
    /// Accepted and observed. The only success.
    Done,
    /// Accepted, and the cluster did not reach the state in time.
    NotObserved,
    /// Accepted, and the object is not the one that was written.
    Diverged,
    /// No answer; not known whether the cluster took it.
    Unknown,
    /// Some objects were taken and some were not.
    Partial,
    Refused,
    Conflict,
    Changed,
    Cancelled,
    NotSent,
}

#[derive(Clone, Debug)]
pub struct Finished {
    pub run: RunId,
    pub action: String,
    pub status: Status,
    pub objects: Vec<ObjectResult>,
    /// Undo, offered for [`UNDO_WINDOW`], when every object was accepted.
    pub undo: Option<UndoOffer>,
    /// The audit log refused a line after the request went out. The write is
    /// still reported; the person is told the log is incomplete.
    pub audit_error: Option<String>,
}

impl Finished {
    pub fn headline(&self) -> Headline {
        match &self.status {
            Status::NotSent(_) => return Headline::NotSent,
            Status::Cancelled => return Headline::Cancelled,
            Status::Sent => {}
        }
        let accepted = self
            .objects
            .iter()
            .filter(|object| matches!(object.step, Step::Accepted { .. }))
            .count();
        if accepted > 0 && accepted < self.objects.len() {
            return Headline::Partial;
        }
        let first = self.objects.first().map(|object| &object.step);
        match first {
            Some(Step::Accepted { .. }) => {
                let observed = self.objects.iter().filter_map(|o| match &o.step {
                    Step::Accepted { observed, .. } => Some(observed),
                    _ => None,
                });
                let mut worst = Headline::Done;
                for observed in observed {
                    match observed {
                        Observed::Reached => {}
                        Observed::NotObserved { .. } => worst = Headline::NotObserved,
                        Observed::Diverged { .. } => return Headline::Diverged,
                    }
                }
                worst
            }
            Some(Step::Conflict(_)) => Headline::Conflict,
            Some(Step::Changed(_)) => Headline::Changed,
            Some(Step::Unknown(_)) => Headline::Unknown,
            _ => Headline::Refused,
        }
    }

    /// True only when every object was accepted and seen to reach its state.
    pub fn succeeded(&self) -> bool {
        self.headline() == Headline::Done
    }
}

impl Runner {
    pub fn new(client: kube::Client, audit: AuditStore, who: Who) -> Self {
        let level = LevelCell::new(who.protection);
        Self {
            client,
            audit,
            who,
            watching: Watching::default(),
            level,
        }
    }

    /// Follows a level that changes while the app runs, such as the one the
    /// shell keeps for the window's cluster.
    pub fn following(mut self, level: LevelCell) -> Self {
        self.level = level;
        self
    }

    /// The level an action is held to now: the strictest of the cluster's
    /// current level and its destination's.
    pub fn level_for(&self, action: &ActionDescription) -> Protection {
        action.level(self.level.get())
    }

    /// Looks for outcomes for this long and this often instead of the default.
    pub fn watching(mut self, watching: Watching) -> Self {
        self.watching = watching.clamped();
        self
    }

    pub fn who(&self) -> &Who {
        &self.who
    }

    pub(super) fn record(
        &self,
        run: &RunId,
        action: &ActionDescription,
        objects: &[&ObjectChange],
        phase: Phase,
        result: &str,
    ) -> io::Result<()> {
        self.audit.append(&Record {
            schema: SCHEMA,
            at: chrono::Utc::now(),
            run: run.as_str().to_owned(),
            entry: self.who.entry.clone(),
            context: self.who.context.clone(),
            actor: self.who.actor.clone(),
            action: action.kind.clone(),
            tier: action.tier.key().to_owned(),
            protection: action.level(self.level.get()).key().to_owned(),
            objects: objects.iter().map(|change| audit_object(change)).collect(),
            request: AuditRequest {
                sentence: scrub(&action.sentence),
            },
            phase,
            result: scrub(result),
        })
    }

    /// Sends a prepared action once the person has met its requirement.
    /// `cancel` is asked during a destructive action's cancel window and once
    /// before the first request; `events` is told what is happening.
    pub async fn execute(
        &self,
        prepared: Prepared,
        confirmation: Confirmation,
        cancel: impl Fn() -> bool + Send,
        events: impl FnMut(Event) + Send,
    ) -> Finished {
        // Behind a mutex so the future is `Send` and can run on a runtime;
        // the lock is never held across an await.
        let events = Mutex::new(events);
        let tell = |event: Event| (events.lock().unwrap_or_else(|p| p.into_inner()))(event);
        let Prepared {
            description,
            requirement,
            run,
            undo: is_undo,
            level: prepared_level,
            inverses,
            ..
        } = prepared;
        let all: Vec<&ObjectChange> = description.changes.iter().collect();
        let finished = |status: Status, objects: Vec<ObjectResult>, undo, audit_error| Finished {
            run: run.clone(),
            action: description.kind.clone(),
            status,
            objects,
            undo,
            audit_error,
        };
        if description.tier == super::Tier::Session {
            return finished(Status::NotSent(NotSent::Session), vec![], None, None);
        }
        // Read again just before the send, not as it was when the dialog opened.
        let now = self.level_for(&description);
        if let Some(reason) = now.refusal().or(requirement.blocked) {
            return finished(
                Status::NotSent(NotSent::ReadOnly(reason)),
                vec![],
                None,
                None,
            );
        }
        if now < prepared_level {
            return finished(
                Status::NotSent(NotSent::LevelChanged(now)),
                vec![],
                None,
                None,
            );
        }
        if !requirement.satisfied_by(&confirmation, &description.names(), &self.who.entry) {
            return finished(Status::NotSent(NotSent::Unconfirmed), vec![], None, None);
        }
        if let Some(window) = requirement.cancel_window {
            tell(Event::CancelWindow(window));
            let started = tokio::time::Instant::now();
            while started.elapsed() < window && !cancel() {
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
        }
        if cancel() {
            let _ = self.record(
                &run,
                &description,
                &all,
                Phase::Cancelled,
                "Cancelled before sending.",
            );
            return finished(Status::Cancelled, vec![], None, None);
        }
        let first = if is_undo {
            Phase::UndoSent
        } else {
            Phase::Sending
        };
        if let Err(error) = self.record(&run, &description, &all, first, "") {
            return finished(
                Status::NotSent(NotSent::AuditUnavailable(error.to_string())),
                vec![],
                None,
                None,
            );
        }
        tell(Event::Sending);

        let mut audit_error = None;
        let mut note = |outcome: io::Result<()>| {
            if let Err(error) = outcome {
                audit_error.get_or_insert(error.to_string());
            }
        };
        let mut results: Vec<ObjectResult> = vec![];
        let mut taken: Vec<(usize, Accepted)> = vec![];
        for (index, change) in description.changes.iter().enumerate() {
            let object = change.target.label();
            let one = [change];
            if results
                .last()
                .is_some_and(|last| !matches!(last.step, Step::Accepted { .. }))
            {
                results.push(ObjectResult {
                    object,
                    step: Step::NotAttempted,
                });
                continue;
            }
            let step = match send(&self.client, &change.target, &change.change, false).await {
                Ok(reply) => {
                    let accepted = Accepted {
                        resource_version: reply["metadata"]["resourceVersion"]
                            .as_str()
                            .map(str::to_owned),
                    };
                    note(self.record(
                        &run,
                        &description,
                        &one,
                        Phase::Accepted,
                        "The API accepted it.",
                    ));
                    tell(Event::Accepted(index));
                    taken.push((index, accepted.clone()));
                    Step::Accepted {
                        accepted,
                        observed: Observed::NotObserved {
                            waited: Duration::ZERO,
                            why: "Not looked at yet.",
                        },
                    }
                }
                Err(Refusal::Conflict(conflict)) => {
                    note(self.record(
                        &run,
                        &description,
                        &one,
                        Phase::Conflict,
                        &format!(
                            "Conflict with {}: {}",
                            conflict.managers.join(", "),
                            conflict.fields.join(", ")
                        ),
                    ));
                    Step::Conflict(conflict)
                }
                Err(Refusal::Changed(what)) => {
                    note(self.record(&run, &description, &one, Phase::Conflict, what.text()));
                    Step::Changed(what)
                }
                Err(Refusal::Denied(message)) => {
                    note(self.record(&run, &description, &one, Phase::Refused, &message));
                    Step::Denied(message)
                }
                Err(Refusal::Rejected { code, message }) => {
                    note(self.record(
                        &run,
                        &description,
                        &one,
                        Phase::Refused,
                        &format!("{code}: {message}"),
                    ));
                    Step::Refused { code, message }
                }
                Err(Refusal::NoAnswer(message)) => {
                    note(self.record(
                        &run,
                        &description,
                        &one,
                        Phase::Unknown,
                        &format!("No answer: {message}. Whether the cluster took it is not known."),
                    ));
                    Step::Unknown(message)
                }
            };
            results.push(ObjectResult { object, step });
        }

        // Look for the new state in every accepted object at once.
        let limit = self.watching.clamped().wait;
        let looks = taken.iter().map(|(index, _)| {
            let change = &description.changes[*index];
            let tell = &tell;
            async move {
                let observed = observe(
                    &self.client,
                    &change.target,
                    &change.observe,
                    self.watching,
                    |waited| {
                        tell(Event::Observing { waited, limit });
                    },
                )
                .await;
                (*index, observed)
            }
        });
        for (index, observed) in futures::future::join_all(looks).await {
            let change = &description.changes[index];
            let (phase, text) = match &observed {
                Observed::Reached => (
                    Phase::Observed,
                    "The cluster reached the new state.".to_owned(),
                ),
                Observed::NotObserved { waited, why } => (
                    Phase::NotObserved,
                    format!("{why} Waited {} s.", waited.as_secs()),
                ),
                Observed::Diverged { why } => (Phase::NotObserved, (*why).to_owned()),
            };
            note(self.record(&run, &description, &[change], phase, &text));
            if let Some(Step::Accepted { observed: slot, .. }) =
                results.get_mut(index).map(|r| &mut r.step)
            {
                *slot = observed;
            }
        }

        let undo = if requirement.undo && !is_undo {
            self.offer_undo(&description, &inverses, &results, &taken)
        } else {
            None
        };
        if let Some(offer) = &undo {
            let _ = self.record(
                &run,
                &description,
                &all,
                Phase::UndoOffered,
                &format!("Undo offered for {} s.", offer.window.as_secs()),
            );
        }
        finished(Status::Sent, results, undo, audit_error)
    }

    /// Undo is offered when every object was accepted and has a way back.
    fn offer_undo(
        &self,
        description: &ActionDescription,
        inverses: &[Option<Inverse>],
        results: &[ObjectResult],
        taken: &[(usize, Accepted)],
    ) -> Option<UndoOffer> {
        if taken.len() != description.changes.len()
            || !results
                .iter()
                .all(|r| matches!(r.step, Step::Accepted { .. }))
        {
            return None;
        }
        let mut items = vec![];
        for (index, accepted) in taken {
            let change = &description.changes[*index];
            let inverse = inverses.get(*index)?.as_ref()?;
            let target = Target {
                resource_version: accepted.resource_version.clone()?,
                ..change.target.clone()
            };
            items.push((
                ObjectChange {
                    target,
                    change: inverse.change.clone(),
                    observe: inverse.observe.clone(),
                    inverse: None,
                },
                inverse.guard.clone(),
            ));
        }
        Some(UndoOffer {
            action: description.kind.clone(),
            sentence: description.sentence.clone(),
            window: UNDO_WINDOW,
            destination: description.destination.clone(),
            items,
        })
    }
}

fn audit_object(change: &ObjectChange) -> AuditObject {
    let target = &change.target;
    AuditObject {
        group: target.group.clone(),
        resource: target.resource.clone(),
        namespace: target.namespace.clone(),
        name: target.name.clone(),
        uid: target.uid.clone(),
        fields: change.change.field_paths(),
    }
}
