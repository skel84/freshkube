//! The runner's side of a silence or an Expire (#884, W2): preflight (the
//! level, the access review per HTTP method, the Service the person saw, the
//! preview Alertmanager computes), the check again just before the send, and
//! the read-back that says whether it took effect. The steps of an object
//! write that have no meaning here (UID and resourceVersion of the silence,
//! a server-side dry run) are replaced as `docs/WRITE_MODEL.md` says.
use std::time::Duration;

use chrono::{DateTime, Utc};
use serde_json::Value;
use tokio::time::{Instant, sleep};

use super::change::{ExpireChange, Scope, SilenceChange, questions};
use super::silence::{Class, Draft, MIN_DURATION, Mode, classify};
use super::writer::{AlertWriter, Muted, Preview, ReadBack, WriteError, same_matchers};
use crate::monitoring::alerts::SilenceState;
use crate::write::{
    Access, ActionDescription, Block, Change, ChangeKind, ChangedWhat, FieldChange, Inverse,
    NotSent, ObjectChange, ObjectDiff, Observation, Observed, Prepared, Protection, RunId, Runner,
    Tier, Verdict, Watching, ask_access, requirement_for,
};

/// What the person was shown, kept to compare with what is true at send.
#[derive(Debug)]
pub(in crate::write) struct AlertSeen {
    preview: Option<Preview>,
    class: Option<Class>,
}

fn same_alerts(a: &Preview, b: &Preview) -> bool {
    // A count past the bound can't be listed, so it is compared as a count.
    a.muted == b.muted && (!a.muted.is_exact() || a.fingerprints == b.fingerprints)
}

fn is_alert(change: &Change) -> bool {
    matches!(change, Change::Silence(_) | Change::Expire(_))
}

/// Whether an action is a silence or an Expire, which have their own path.
pub(in crate::write) fn is_alert_action(action: &ActionDescription) -> bool {
    action.changes.iter().any(|c| is_alert(&c.change))
}

/// The sentence the audit keeps: matcher names, the `alertname` value and
/// counts. No other matcher value and no comment.
fn silence_sentence(change: &SilenceChange, class: &Class, muted: Muted) -> String {
    let name = change
        .draft
        .matchers
        .iter()
        .find(|m| m.name == "alertname" && m.equal && !m.regex)
        .map(|m| m.value.chars().take(100).collect::<String>())
        .unwrap_or_else(|| "(no alertname)".into());
    let count = match muted {
        Muted::Exactly(n) => n.to_string(),
        Muted::AtLeast(n) => format!("more than {n}"),
    };
    format!(
        "Silence alertname={name}: {} matchers, {} h, {}, mutes {count} now",
        change.draft.matchers.len(),
        change.draft.duration.as_secs() / 3600,
        if class.is_narrow() { "narrow" } else { "broad" },
    )
}

fn lines_of(preview: &Preview) -> Vec<String> {
    let mut lines: Vec<String> = preview
        .alerts
        .iter()
        .map(|alert| match &alert.cluster {
            Some(cluster) => format!("{} ({cluster})", alert.name),
            None => alert.name.clone(),
        })
        .collect();
    if preview.other_clusters > 0 {
        lines.push(format!(
            "{} of them belong to other clusters",
            preview.other_clusters
        ));
    }
    lines
}

impl AlertSeen {
    /// For an Expire, which compares the silence it read, not a preview.
    pub(in crate::write) fn expire() -> Self {
        Self {
            preview: None,
            class: None,
        }
    }
}

impl Runner {
    /// The scope the runner resolved; a page can't set it. Without one no
    /// silence is sent.
    pub fn with_alert_scope(mut self, scope: Scope) -> Self {
        self.alert_scope = Some(scope);
        self
    }

    fn refuse(object: &str, message: impl Into<String>) -> Verdict {
        Verdict::Blocked(Block::Refused {
            object: object.to_owned(),
            message: message.into(),
        })
    }

    /// The access review per method and the Service as the person saw it.
    async fn gate(&self, change: &ObjectChange) -> Result<(), Block> {
        let object = change.target.label();
        for question in questions(&change.change) {
            match ask_access(&self.client, &question).await {
                Access::Allowed => {}
                Access::Denied { reason } => {
                    return Err(Block::AccessDenied {
                        object,
                        verb: question.verb,
                        reason,
                    });
                }
                Access::Unknown { message } => {
                    return Err(Block::AccessUnknown { object, message });
                }
            }
        }
        for depends in &change.depends {
            self.check_depends(depends).await?;
        }
        Ok(())
    }

    /// An Expire's gate, and the silence read back: it must still be the
    /// one the person saw. Returns what it mutes now, for the dialog.
    pub(in crate::write) async fn check_expire(
        &self,
        change: &ObjectChange,
        expire: &ExpireChange,
    ) -> Result<Vec<String>, Block> {
        self.gate(change).await?;
        let object = change.target.label();
        let writer = AlertWriter::new(self.client.clone(), expire.service.clone());
        match writer.read_back(&expire.id).await {
            Err(error) => {
                return Err(Block::NoAnswer {
                    object,
                    message: error.words(),
                });
            }
            Ok(ReadBack::NotFound) => {
                return Err(Block::Changed {
                    object,
                    what: ChangedWhat::Gone,
                });
            }
            Ok(ReadBack::Found(silence)) => {
                let live = silence.state != SilenceState::Expired;
                if !live
                    || silence.ends_at != expire.ends_at
                    || !same_matchers(&silence.matchers, &expire.matchers)
                {
                    return Err(Block::Changed {
                        object,
                        what: ChangedWhat::Moved,
                    });
                }
            }
        }
        // What it holds back now. Informational: it can't be read for a
        // silence this app wouldn't have made, and that doesn't stop it.
        let draft = Draft {
            matchers: expire.matchers.clone(),
            comment: "expire".into(),
            duration: MIN_DURATION,
            mode: Mode::Cluster,
        };
        Ok(match writer.preview(&draft).await {
            Ok(preview) => lines_of(&preview),
            Err(_) => vec!["What it mutes now could not be read.".to_owned()],
        })
    }

    /// Preflight of a silence or an Expire, in the framework's order: the
    /// level (no request under `read-only`), the scope, the access review,
    /// the Service, then what Alertmanager would mute now.
    pub(in crate::write) async fn preflight_alertmanager(
        &self,
        mut action: ActionDescription,
    ) -> Verdict {
        let [change] = action.changes.as_mut_slice() else {
            return Verdict::Blocked(Block::Empty);
        };
        let object = change.target.label();
        // The runner sets what a page must not: the mode, the actor and, for
        // a shared store, which entries the silence can reach.
        let mut destinations = None;
        if let Change::Silence(silence) = &mut change.change {
            let Some(scope) = &self.alert_scope else {
                return Self::refuse(
                    &object,
                    "This window can't say which cluster's alerts these are, so nothing was sent.",
                );
            };
            let Some(mode) = scope.mode() else {
                return Self::refuse(
                    &object,
                    "This Alertmanager's alerts carry no cluster label, so a silence can't be limited to one cluster. Nothing was sent.",
                );
            };
            silence.draft.mode = mode;
            silence.actor = self.who.actor.clone();
            if let Scope::Central { entries, value, .. } = scope {
                let exact = silence.draft.has_exact_cluster();
                destinations = Some(
                    entries
                        .iter()
                        .filter(|(cluster, _)| !exact || cluster == value)
                        .map(|(_, destination)| destination.clone())
                        .collect(),
                );
            }
        }
        if let Some(destinations) = destinations {
            action.destinations = destinations;
        }
        // The entry's id is what a protected cluster asks to be typed.
        action.subjects = vec![self.who.entry.clone()];
        let level = self.level_for(&action);
        if let Some(reason) = level.refusal() {
            return Verdict::Blocked(Block::ReadOnly(reason));
        }
        let change = action.changes[0].clone();
        let (preview, class, lines) = match &change.change {
            Change::Silence(silence) => match self.silence_preview(&change, silence, level).await {
                Ok(seen) => seen,
                Err(block) => return Verdict::Blocked(block),
            },
            Change::Expire(expire) => match self.check_expire(&change, expire).await {
                Ok(lines) => (None, None, lines),
                Err(block) => return Verdict::Blocked(block),
            },
            _ => return Verdict::Blocked(Block::Empty),
        };
        let mut inverses = vec![None];
        if let (Change::Silence(silence), Some(class), Some(preview)) =
            (&change.change, &class, &preview)
        {
            action.tier = if class.is_narrow() {
                Tier::Reversible
            } else {
                Tier::SpecChange
            };
            action.sentence = silence_sentence(silence, class, preview.muted);
            if class.is_narrow() {
                // Its way back, Expire of the id, is known once it is accepted.
                action.changes[0].inverse = Some(crate::write::InverseFrom::after_accept());
                inverses[0] = Some(Inverse {
                    change: Change::Expire(Box::new(ExpireChange {
                        service: silence.service.clone(),
                        id: String::new(),
                        matchers: vec![],
                        ends_at: None,
                    })),
                    observe: Observation::SilenceExpired,
                    guard: Observation::Unobservable,
                });
            }
        }
        let mut shown = lines;
        if let Some(Class::Broad(reasons)) = &class {
            shown.extend(reasons.iter().map(|reason| reason.words()));
        }
        action.preview = vec![crate::write::Preview {
            title: if class.is_some() {
                "Mutes now"
            } else {
                "Will notify again"
            }
            .to_owned(),
            lines: shown,
        }];
        let requirement = requirement_for(&action, level);
        let diffs = vec![ObjectDiff {
            object,
            changes: action.changes[0]
                .change
                .field_paths()
                .into_iter()
                .map(|path| FieldChange {
                    path,
                    kind: ChangeKind::Added,
                })
                .collect(),
        }];
        Verdict::Ready(Box::new(Prepared {
            description: action,
            requirement,
            run: RunId::new(),
            diffs,
            undo: false,
            level,
            own: self.level.get(),
            inverses,
            alert_seen: Some(AlertSeen { preview, class }),
        }))
    }

    /// The preview of a silence and how it is classed. Example data has no
    /// Alertmanager: it mutes nothing.
    async fn silence_preview(
        &self,
        change: &ObjectChange,
        silence: &SilenceChange,
        level: Protection,
    ) -> Result<(Option<Preview>, Option<Class>, Vec<String>), Block> {
        let object = change.target.label();
        let preview = if self.example {
            Preview::none()
        } else {
            self.gate(change).await?;
            let writer = AlertWriter::new(self.client.clone(), silence.service.clone());
            writer
                .preview(&silence.draft)
                .await
                .map_err(|error| match error {
                    WriteError::Invalid(refusal) => Block::Refused {
                        object: object.clone(),
                        message: refusal.words(),
                    },
                    other => Block::NoAnswer {
                        object: object.clone(),
                        message: other.words(),
                    },
                })?
        };
        let class =
            classify(&silence.draft, &preview, level).map_err(|refusal| Block::Refused {
                object,
                message: refusal.words(),
            })?;
        let lines = lines_of(&preview);
        Ok((Some(preview), Some(class), lines))
    }

    /// Just before the send: what the silence mutes and how it is classed,
    /// or the silence an Expire names, read again. A difference sends
    /// nothing; a read that fails sends nothing.
    pub(in crate::write) async fn recheck_alertmanager(
        &self,
        description: &ActionDescription,
        seen: &AlertSeen,
        level: Protection,
    ) -> Result<(), NotSent> {
        let moved = || {
            NotSent::Moved(
                "What this would change is no longer what you were shown. Nothing was sent."
                    .to_owned(),
            )
        };
        let change = &description.changes[0];
        match &change.change {
            Change::Silence(silence) => {
                let writer = AlertWriter::new(self.client.clone(), silence.service.clone());
                let now = writer.preview(&silence.draft).await.map_err(|error| {
                    NotSent::Unchecked(format!("{} Nothing was sent.", error.words()))
                })?;
                let class = classify(&silence.draft, &now, level)
                    .map_err(|refusal| NotSent::Unchecked(refusal.words()))?;
                let same = seen.preview.as_ref().is_some_and(|p| same_alerts(p, &now))
                    && seen.class.as_ref() == Some(&class);
                if same { Ok(()) } else { Err(moved()) }
            }
            Change::Expire(expire) => match self.check_expire(change, expire).await {
                Ok(_) => Ok(()),
                Err(Block::Changed { .. }) => Err(moved()),
                Err(other) => Err(NotSent::Unchecked(format!(
                    "{} Nothing was sent.",
                    other.text()
                ))),
            },
            _ => Ok(()),
        }
    }

    /// Looks for the silence by id until it reads as the write says, it
    /// turns out to be another, or the wait ends. A silence not found is
    /// not done.
    pub(in crate::write) async fn observe_alert(
        &self,
        change: &ObjectChange,
        reply: &Value,
        watching: Watching,
        mut progress: impl FnMut(Duration),
    ) -> Observed {
        let watching = watching.clamped();
        let (service, id, silence) = match &change.change {
            Change::Silence(c) => (
                &c.service,
                reply["silenceID"].as_str().unwrap_or_default().to_owned(),
                Some(c),
            ),
            Change::Expire(c) => (&c.service, c.id.clone(), None),
            _ => {
                return Observed::NotObserved {
                    waited: Duration::ZERO,
                    why: "This action has no outcome the app can watch.",
                };
            }
        };
        let ends_at = reply["endsAt"]
            .as_str()
            .and_then(|text| DateTime::parse_from_rfc3339(text).ok())
            .map(|time| time.with_timezone(&Utc));
        let writer = AlertWriter::new(self.client.clone(), service.clone());
        let started = Instant::now();
        loop {
            if let Ok(ReadBack::Found(found)) = writer.read_back(&id).await {
                match silence {
                    Some(c) => {
                        let ours = same_matchers(&found.matchers, &c.draft.matchers)
                            && found.comment == c.draft.comment.trim()
                            && ends_at.is_none_or(|ends| found.ends_at == Some(ends));
                        if !ours {
                            return Observed::Diverged {
                                why: "The silence is no longer the one written.",
                            };
                        }
                        if found.state == SilenceState::Expired {
                            return Observed::Diverged {
                                why: "The silence expired before it was seen to take effect.",
                            };
                        }
                        return Observed::Reached;
                    }
                    None if found.state == SilenceState::Expired => return Observed::Reached,
                    None => {}
                }
            }
            let waited = started.elapsed();
            if waited >= watching.wait {
                return Observed::NotObserved {
                    waited,
                    why: "Alertmanager did not show the silence in its new state in time.",
                };
            }
            progress(waited);
            sleep(watching.every.min(watching.wait - waited)).await;
        }
    }

    /// An Expire of the silence just created, for Undo: only after it is
    /// accepted is its id known.
    pub(in crate::write) fn expire_of(
        &self,
        silence: &SilenceChange,
        reply: &Value,
    ) -> Option<Change> {
        let id = reply["silenceID"].as_str()?.to_owned();
        let ends_at = reply["endsAt"]
            .as_str()
            .and_then(|text| DateTime::parse_from_rfc3339(text).ok())
            .map(|time| time.with_timezone(&Utc));
        Some(Change::Expire(Box::new(ExpireChange {
            service: silence.service.clone(),
            id,
            matchers: silence.draft.matchers.clone(),
            ends_at,
        })))
    }
}
