//! What a read saw, turned into updates. Pure: the clock is an argument,
//! so a test steps it.
//!
//! One [`Tracker`] belongs to one follow and keeps what it needs to say
//! nothing twice:
//!
//! - **the same state never repeats**: an update is made only when the
//!   follow's state changes;
//! - **a crash loop fires once**, and again only after the pod has been
//!   Ready for [`REARM_AFTER_READY`];
//! - **an incident is told once**, by its key; the incidents open when the
//!   follow starts are its baseline and never announced;
//! - **a follow that can't be read** is [`Status::NotWatching`], told once
//!   until it reads again, and never as a failure.

use std::collections::VecDeque;

use chrono::{DateTime, Utc};

use crate::model::{Follow, REARM_AFTER_READY, Subject};

/// The most incident keys a tracker remembers.
const REMEMBERED_INCIDENTS: usize = 500;

/// What the owner's read of a pod saw.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PodSeen {
    pub crash_looping: bool,
    pub ready: bool,
    pub restarts: u32,
}

/// What a read of a Kargo project saw of one Stage.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StageSeen {
    pub name: String,
    /// The Freight the Stage runs.
    pub current_freight: Vec<String>,
    /// Each verification: the Freight it covers and its phase.
    pub verifications: Vec<(Vec<String>, Option<String>)>,
}

/// What a read of Coroot saw of one incident.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IncidentSeen {
    pub key: String,
    pub application: String,
    pub critical: bool,
    pub open: bool,
    pub description: String,
}

/// One answer of a read.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Seen {
    /// The pod, or `None` when it is no longer there.
    Pod(Option<PodSeen>),
    Stages(Vec<StageSeen>),
    Incidents(Vec<IncidentSeen>),
    /// The read can't be made: refused, unreachable, not installed.
    Unreadable(String),
}

/// Whether a follow is being read.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum Status {
    /// No answer yet.
    #[default]
    Starting,
    Watching,
    /// Why not, in words for the person.
    NotWatching(String),
}

impl Status {
    pub fn label(&self) -> &str {
        match self {
            Status::Starting => "Starting",
            Status::Watching => "Watching",
            Status::NotWatching(_) => "Not watching",
        }
    }
}

/// One thing to tell the person.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Update {
    pub follow: u64,
    /// The state it reports, such as `crash-looping`; never repeated.
    pub state: String,
    pub headline: String,
    pub detail: String,
    /// Whether it comes through quiet hours.
    pub failure: bool,
    /// Whether its cluster is protected or read-only; the owner sets it
    /// from the follow's cluster, so a change of level reaches updates
    /// still to come.
    pub sensitive: bool,
    pub at: DateTime<Utc>,
}

/// What one answer led to.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Step {
    pub updates: Vec<Update>,
    pub status: Status,
    /// Whether the follow has its answer and ends.
    pub finished: bool,
}

/// One follow's memory.
#[derive(Debug)]
pub struct Tracker {
    follow: u64,
    subject: Subject,
    last_state: Option<String>,
    /// Whether a crash loop may fire.
    armed: bool,
    ready_since: Option<DateTime<Utc>>,
    /// The incident keys told or in the baseline; `None` until the first
    /// answer.
    incidents: Option<VecDeque<String>>,
    /// Whether the current outage has been told.
    unreadable_told: bool,
}

impl Tracker {
    pub fn new(follow: &Follow) -> Self {
        Self {
            follow: follow.id,
            subject: follow.subject.clone(),
            last_state: None,
            armed: true,
            ready_since: None,
            incidents: None,
            unreadable_told: false,
        }
    }

    /// What `seen` leads to at `now`.
    pub fn observe(&mut self, seen: &Seen, now: DateTime<Utc>) -> Step {
        let mut step = Step {
            status: Status::Watching,
            ..Step::default()
        };
        match (seen, self.subject.clone()) {
            (Seen::Unreadable(why), _) => return self.unreadable(why, now),
            (Seen::Pod(None), _) => {
                return self.unreadable("The pod is no longer there.", now);
            }
            (
                Seen::Pod(Some(pod)),
                Subject::CrashLoop {
                    namespace,
                    pod: name,
                },
            ) => {
                self.crash_loop(pod, &namespace, &name, now, &mut step);
            }
            (
                Seen::Stages(stages),
                Subject::Landing {
                    project,
                    stage,
                    freight,
                },
            ) => {
                let Some(found) = stages.iter().find(|s| s.name == stage) else {
                    return self.unreadable(&format!("Stage {stage} is not in {project}."), now);
                };
                if found.current_freight.contains(&freight) {
                    self.say(
                        &mut step,
                        "landed",
                        format!("{freight} reached {stage}"),
                        format!("In Kargo project {project}."),
                        false,
                        now,
                    );
                    step.finished = true;
                }
            }
            (
                Seen::Stages(stages),
                Subject::Verification {
                    project,
                    stage,
                    freight,
                },
            ) => {
                let Some(found) = stages.iter().find(|s| s.name == stage) else {
                    return self.unreadable(&format!("Stage {stage} is not in {project}."), now);
                };
                let phase = found
                    .verifications
                    .iter()
                    .find(|(covered, _)| covered.contains(&freight))
                    .and_then(|(_, phase)| phase.as_deref())
                    .map(str::to_ascii_lowercase);
                match phase.as_deref() {
                    Some("failed" | "error" | "aborted" | "inconclusive") => {
                        let phase = phase.unwrap_or_default();
                        self.say(
                            &mut step,
                            &format!("verification-{phase}"),
                            format!("Verification of {freight} failed in {stage}"),
                            format!("Kargo project {project} says {phase}."),
                            true,
                            now,
                        );
                        step.finished = true;
                    }
                    Some("successful") => step.finished = true,
                    _ => {}
                }
            }
            (Seen::Incidents(incidents), Subject::Incident { application }) => {
                self.incidents(incidents, application.as_deref(), now, &mut step);
            }
            // An answer for another kind of read: nothing to say.
            _ => {}
        }
        self.unreadable_told = false;
        step
    }

    fn unreadable(&mut self, why: &str, now: DateTime<Utc>) -> Step {
        let mut step = Step {
            status: Status::NotWatching(why.to_owned()),
            ..Step::default()
        };
        if !self.unreadable_told {
            self.unreadable_told = true;
            step.updates.push(Update {
                follow: self.follow,
                state: "not-watching".into(),
                headline: "Not watching".into(),
                detail: format!("{} · {why}", self.subject.title()),
                failure: false,
                sensitive: false,
                at: now,
            });
        }
        step
    }

    fn crash_loop(
        &mut self,
        pod: &crate::rules::PodSeen,
        namespace: &str,
        name: &str,
        now: DateTime<Utc>,
        step: &mut Step,
    ) {
        if let Some(since) = self.ready_since
            && (now - since).to_std().is_ok_and(|d| d >= REARM_AFTER_READY)
        {
            self.armed = true;
            self.last_state = None;
        }
        self.ready_since = if pod.ready {
            self.ready_since.or(Some(now))
        } else {
            None
        };
        if pod.crash_looping && self.armed {
            self.armed = false;
            self.say(
                step,
                "crash-looping",
                format!("{name} keeps crash-looping"),
                format!("In {namespace}, restarted {} times.", pod.restarts),
                true,
                now,
            );
        }
    }

    fn incidents(
        &mut self,
        seen: &[IncidentSeen],
        application: Option<&str>,
        now: DateTime<Utc>,
        step: &mut Step,
    ) {
        let Some(known) = self.incidents.as_mut() else {
            // The first answer is the baseline: what is open now was not
            // opened after the follow.
            self.incidents = Some(seen.iter().map(|i| i.key.clone()).collect());
            return;
        };
        for incident in seen {
            if !incident.open || known.contains(&incident.key) {
                continue;
            }
            if application.is_some_and(|app| app != incident.application) {
                continue;
            }
            known.push_back(incident.key.clone());
            while known.len() > REMEMBERED_INCIDENTS {
                known.pop_front();
            }
            let severity = if incident.critical {
                "Critical"
            } else {
                "Warning"
            };
            step.updates.push(Update {
                follow: self.follow,
                state: format!("incident:{}", incident.key),
                headline: format!("Incident opened in {}", incident.application),
                detail: format!("{severity}: {}", incident.description),
                failure: true,
                sensitive: false,
                at: now,
            });
        }
    }

    /// Adds an update unless the follow is already in that state.
    fn say(
        &mut self,
        step: &mut Step,
        state: &str,
        headline: String,
        detail: String,
        failure: bool,
        now: DateTime<Utc>,
    ) {
        if self.last_state.as_deref() == Some(state) {
            return;
        }
        self.last_state = Some(state.to_owned());
        step.updates.push(Update {
            follow: self.follow,
            state: state.to_owned(),
            headline,
            detail,
            failure,
            sensitive: false,
            at: now,
        });
    }
}

#[cfg(test)]
mod tests;
