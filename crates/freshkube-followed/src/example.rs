//! Example data for `--fixture` (`FRESHKUBE_FOLLOWS`): reads that answer
//! from a script on the GPUI clock, and follows and notices seeded so a
//! smoke shows the bell and each state.
//!
//! Without a shape, a follow made in the example cluster plays a small
//! story: a pod becomes Ready and then crash-loops, a Freight lands and a
//! verification fails, a Coroot incident opens. The shapes seed the bell
//! and play nothing, so a capture is the same each time.

use std::time::Duration;

use chrono::Duration as Span;
use gpui_kit::*;

use crate::model::{MAX_FOLLOWS, Subject, WatchKey};
use crate::notice::{Notice, Quiet};
use crate::owner::{Connection, Followed};
use crate::rules::{IncidentSeen, PodSeen, Seen, StageSeen, Status, Update};

/// `FRESHKUBE_FOLLOWS`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Shape {
    /// The bell with New and Earlier, and three follows.
    Bell,
    /// One follow in each state: Starting, Watching, Not watching.
    States,
    /// Quiet hours on: a held notice and a failure that came through.
    Quiet,
    /// A protected cluster, so Hide details is on by default.
    Hidden,
    /// The most follows, so the next is refused.
    Full,
    /// Saved follows of a cluster that isn't open: Not watching.
    Unwatched,
}

impl Shape {
    pub fn parse(value: &str) -> Option<Self> {
        Some(match value {
            "bell" => Self::Bell,
            "states" => Self::States,
            "quiet" => Self::Quiet,
            "hidden" => Self::Hidden,
            "full" => Self::Full,
            "unwatched" => Self::Unwatched,
            _ => return None,
        })
    }
}

/// What the example reads answer, and the shape that was asked for.
#[derive(Clone, Debug, Default)]
pub struct Script {
    pub shape: Option<Shape>,
}

/// One answer, after a wait.
pub struct Play {
    pub after: Duration,
    pub seen: Seen,
}

impl Script {
    /// What the example read for `key` answers, for the subjects that read
    /// it. A shape plays nothing.
    pub fn plays(&self, key: &WatchKey, subjects: &[Subject]) -> Vec<Play> {
        if self.shape.is_some() {
            return Vec::new();
        }
        let play = |seconds, seen| Play {
            after: Duration::from_secs(seconds),
            seen,
        };
        match key {
            WatchKey::Pod { .. } => {
                let pod = |crash_looping, ready| {
                    Seen::Pod(Some(PodSeen {
                        crash_looping,
                        ready,
                        restarts: 4,
                    }))
                };
                vec![play(1, pod(false, true)), play(4, pod(true, false))]
            }
            WatchKey::Kargo { .. } => {
                let stages = |landed: bool, failed: bool| {
                    let mut seen: Vec<StageSeen> = Vec::new();
                    for subject in subjects {
                        let (Subject::Landing { stage, freight, .. }
                        | Subject::Verification { stage, freight, .. }) = subject
                        else {
                            continue;
                        };
                        let at = match seen.iter().position(|s| &s.name == stage) {
                            Some(at) => at,
                            None => {
                                seen.push(StageSeen {
                                    name: stage.clone(),
                                    current_freight: vec!["older-hare".to_owned()],
                                    verifications: Vec::new(),
                                });
                                seen.len() - 1
                            }
                        };
                        if landed {
                            seen[at].current_freight = vec![freight.clone()];
                        }
                        if failed {
                            seen[at]
                                .verifications
                                .push((vec![freight.clone()], Some("Failed".to_owned())));
                        }
                    }
                    Seen::Stages(seen)
                };
                vec![
                    play(1, stages(false, false)),
                    play(4, stages(true, false)),
                    play(7, stages(true, true)),
                ]
            }
            WatchKey::Coroot => {
                let incident = |key: &str| IncidentSeen {
                    key: key.to_owned(),
                    application: "checkout".to_owned(),
                    critical: true,
                    open: true,
                    description: "high latency".to_owned(),
                };
                vec![
                    play(1, Seen::Incidents(vec![incident("a")])),
                    play(5, Seen::Incidents(vec![incident("a"), incident("b")])),
                ]
            }
        }
    }
}

fn pod(namespace: &str, pod: &str) -> Subject {
    Subject::CrashLoop {
        namespace: namespace.into(),
        pod: pod.into(),
    }
}

fn landing(freight: &str) -> Subject {
    Subject::Landing {
        project: "checkout".into(),
        stage: "prod-ams".into(),
        freight: freight.into(),
    }
}

impl Followed {
    /// Seeds the shape's follows and notices once, in `connection`'s
    /// context; the follows read through the example script.
    pub fn seed_example(&mut self, connection: &Connection, cx: &mut Context<Self>) {
        let Some(shape) = self.script.as_ref().and_then(|s| s.shape) else {
            return;
        };
        if !self.items.is_empty() || self.seeded {
            return;
        }
        self.seeded = true;
        let mut connection = connection.clone();
        match shape {
            Shape::Bell => {
                for subject in [
                    pod("shop", "api-7c9"),
                    landing("wonky-otter"),
                    Subject::Incident { application: None },
                ] {
                    let _ = self.follow(subject, connection.clone(), cx);
                }
                self.seed_notices(
                    &[
                        (
                            "crash-looping",
                            "api-7c9 keeps crash-looping",
                            "In shop, restarted 4 times.",
                            true,
                            2,
                        ),
                        (
                            "landed",
                            "wonky-otter reached prod-ams",
                            "In Kargo project checkout.",
                            false,
                            20,
                        ),
                        (
                            "incident:a",
                            "Incident opened in checkout",
                            "Critical: high latency",
                            true,
                            190,
                        ),
                        (
                            "landed",
                            "tidy-heron reached prod-ams",
                            "In Kargo project checkout.",
                            false,
                            400,
                        ),
                    ],
                    2,
                    cx,
                );
            }
            Shape::States => {
                for subject in [
                    pod("shop", "api-7c9"),
                    pod("shop", "worker-5d2"),
                    pod("shop", "cron-1"),
                ] {
                    let _ = self.follow(subject, connection.clone(), cx);
                }
                let states = [
                    Status::Watching,
                    Status::Starting,
                    Status::NotWatching("Coroot isn't connected.".to_owned()),
                ];
                for (item, status) in self.items.iter_mut().zip(states) {
                    item.status = status;
                }
            }
            Shape::Quiet => {
                let _ = self.follow(pod("shop", "api-7c9"), connection.clone(), cx);
                let _ = self.follow(landing("wonky-otter"), connection.clone(), cx);
                self.settings.quiet = Quiet {
                    enabled: true,
                    from: 0,
                    until: 24 * 60 - 1,
                };
                self.local_minute = std::sync::Arc::new(|| 12 * 60);
                self.seed_notices(
                    &[
                        (
                            "crash-looping",
                            "api-7c9 keeps crash-looping",
                            "In shop, restarted 4 times.",
                            true,
                            2,
                        ),
                        (
                            "landed",
                            "wonky-otter reached prod-ams",
                            "In Kargo project checkout.",
                            false,
                            20,
                        ),
                    ],
                    2,
                    cx,
                );
                if let Some(notice) = self.inbox.new.iter_mut().find(|n| !n.has_failure()) {
                    notice.held = true;
                }
            }
            Shape::Hidden => {
                // Hide details on, as a protected cluster has it by default
                // (the example cluster itself is as open as the header says):
                // the notice is told at once and says only its summary.
                self.settings.hide_details = Some(true);
                let _ = self.follow(pod("shop", "api-7c9"), connection.clone(), cx);
                let at = self.now(cx) - Span::seconds(2);
                self.show(
                    Notice {
                        id: 1,
                        at,
                        updates: vec![Update {
                            follow: 1,
                            state: "crash-looping".to_owned(),
                            headline: "api-7c9 keeps crash-looping".to_owned(),
                            detail: "In shop, restarted 4 times.".to_owned(),
                            failure: true,
                            sensitive: false,
                            at,
                        }],
                        held: false,
                    },
                    cx,
                );
            }
            Shape::Full => {
                for n in 0..MAX_FOLLOWS {
                    let _ = self.follow(
                        pod("shop", &format!("worker-{n:02}")),
                        connection.clone(),
                        cx,
                    );
                }
            }
            Shape::Unwatched => {
                // Saved follows of a cluster that isn't open here.
                connection.context = "retired-ams".to_owned();
                for subject in [pod("shop", "api-7c9"), landing("wonky-otter")] {
                    let _ = self.follow(subject, connection.clone(), cx);
                }
                self.connections.remove("retired-ams");
                self.feeds.clear();
                for item in &mut self.items {
                    item.status = Status::NotWatching(crate::owner::NOT_CONNECTED.to_owned());
                }
            }
        }
        cx.notify();
    }

    /// Puts example notices in the bell: `new` of them stay New.
    fn seed_notices(
        &mut self,
        notices: &[(&str, &str, &str, bool, i64)],
        new: usize,
        cx: &mut Context<Self>,
    ) {
        let now = self.now(cx);
        for (n, (state, headline, detail, failure, seconds_ago)) in notices.iter().enumerate().rev()
        {
            self.inbox.push(Notice {
                id: n as u64 + 1,
                at: now - Span::seconds(*seconds_ago),
                updates: vec![Update {
                    follow: 1,
                    state: (*state).to_owned(),
                    headline: (*headline).to_owned(),
                    detail: (*detail).to_owned(),
                    failure: *failure,
                    sensitive: false,
                    at: now - Span::seconds(*seconds_ago),
                }],
                held: false,
            });
        }
        // The oldest are already seen.
        while self.inbox.new.len() > new {
            if let Some(old) = self.inbox.new.pop_back() {
                self.inbox.earlier.push_front(old);
            }
        }
    }
}
