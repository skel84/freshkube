//! What a dialog shows, decided here from the tier, the level and what
//! preflight found, so a page only draws it. Also the words a finished write
//! is reported in: accepted, observed, not observed and unknown are told
//! apart, and only the first is ever success.

use std::time::Duration;

use super::{
    action::ActionDescription,
    outcome::Observed,
    run::Runner,
    send::{Refusal, get},
};
use super::{
    audit::scrub,
    preflight::{Block, ObjectDiff, Prepared, Verdict},
    requirement::Requirement,
    run::{Finished, Headline, NotSent, Step},
    send::{ChangedWhat, Conflict},
};

/// The request the dialog asks the person to confirm.
#[derive(Clone, Debug, PartialEq)]
pub struct Ask {
    /// The dialog's title: `Scale Deployment/web`.
    pub title: String,
    pub sentence: String,
    /// The confirm button's label.
    pub verb: String,
    /// What to type before the confirm enables, if anything: the object's
    /// name, or the entry's id for several. Pasting is allowed.
    pub type_name: Option<String>,
    /// For a spec change: the dry run's diff.
    pub diff: Vec<ObjectDiff>,
    /// For a destructive action: everything it reaches.
    pub impact: Vec<String>,
    /// For a destructive action: the confirm turns into a Cancel with a
    /// countdown of this length.
    pub cancel_window: Option<Duration>,
    /// The confirm is drawn as dangerous.
    pub danger: bool,
}

/// What the dialog is, from the person's side.
#[derive(Clone, Debug, PartialEq)]
pub enum Dialog {
    /// The level refuses it. The control is disabled with this reason;
    /// nothing is asked and nothing is sent.
    Disabled { reason: String },
    /// Ask first.
    Ask(Ask),
    /// Send at once; the toast says nothing was asked and why.
    SendWithoutAsking { notice: &'static str },
    /// Preflight refused it: access, the dry run, or the cluster not
    /// answering. Only Close.
    Refused { text: String },
    /// The object is not as it was when opened. Only "Review the new
    /// version" (which runs the dry run again), never "send anyway". When it
    /// is gone, or is a different object, there is nothing to review.
    Changed { text: String, can_review: bool },
    /// Another field manager owns what the write sets. No force option; the
    /// rest of the write was not sent.
    Conflict {
        managers: Vec<String>,
        fields: Vec<String>,
        text: String,
    },
}

/// The toast's wording when nothing was asked because the cluster is open.
pub const OPEN_NOTICE: &str = "Sent without asking because this cluster is open.";

pub const REVIEW_LABEL: &str = "Review the new version";

fn ask(prepared: &Prepared, entry: &str) -> Ask {
    let requirement: &Requirement = prepared.requirement();
    let objects = prepared.impact();
    let action = prepared.description();
    Ask {
        title: format!("{} {}", action.verb, objects.join(", ")),
        sentence: action.sentence.clone(),
        verb: action.verb.clone(),
        type_name: requirement.type_name.then(|| prepared.expected_name(entry)),
        diff: if requirement.diff {
            prepared.diffs().to_vec()
        } else {
            vec![]
        },
        impact: if requirement.impact { objects } else { vec![] },
        cancel_window: requirement.cancel_window,
        danger: requirement.cancel_window.is_some(),
    }
}

fn conflict_text(object: &str, conflict: &Conflict) -> String {
    let managers = if conflict.managers.is_empty() {
        "another field manager".to_owned()
    } else {
        conflict.managers.join(", ")
    };
    let fields = if conflict.fields.is_empty() {
        "fields this write sets".to_owned()
    } else {
        conflict.fields.join(", ")
    };
    format!(
        "{object}: {managers} owns {fields}. The rest of the write was not sent. Nothing is forced; change the write or ask the owner."
    )
}

impl Dialog {
    /// What the person sees for this preflight verdict under the entry
    /// `entry`.
    pub fn for_verdict(verdict: &Verdict, entry: &str) -> Dialog {
        match verdict {
            Verdict::Ready(prepared) if prepared.requirement().confirm => {
                Dialog::Ask(ask(prepared, entry))
            }
            Verdict::Ready(_) => Dialog::SendWithoutAsking {
                notice: OPEN_NOTICE,
            },
            Verdict::Blocked(Block::ReadOnly(reason)) => Dialog::Disabled {
                reason: (*reason).to_owned(),
            },
            Verdict::Blocked(Block::Changed { what, object }) => Dialog::Changed {
                text: format!("{object}: {}", what.text()),
                can_review: *what == ChangedWhat::Modified,
            },
            Verdict::Blocked(Block::Conflict { object, conflict }) => Dialog::Conflict {
                managers: conflict.managers.clone(),
                fields: conflict.fields.clone(),
                text: conflict_text(object, conflict),
            },
            Verdict::Blocked(block) => Dialog::Refused { text: block.text() },
        }
    }
}

impl Finished {
    /// Whether the report stays until the person closes it. Only an observed
    /// success may go away by itself.
    pub fn sticky(&self) -> bool {
        self.headline() != Headline::Done
    }

    /// The sentence a toast leads with. Words that only fit success are used
    /// for success alone.
    pub fn message(&self) -> String {
        match self.headline() {
            Headline::Done => "Done. The cluster reached the new state.".to_owned(),
            Headline::NotObserved => {
                let waited = self
                    .objects
                    .iter()
                    .find_map(|o| match &o.step {
                        Step::Accepted {
                            observed: Observed::NotObserved { waited, .. },
                            ..
                        } => Some(waited.as_secs()),
                        _ => None,
                    })
                    .unwrap_or_default();
                format!(
                    "Accepted, but not seen to take effect after {waited} s. Check the object; it may still be rolling out."
                )
            }
            Headline::Diverged => {
                "Accepted, but the object is no longer the one that was written.".to_owned()
            }
            Headline::Unknown => "No answer from the cluster. It is not known whether it accepted the write; check the object. Nothing was sent again.".to_owned(),
            Headline::Partial => {
                let parts: Vec<String> = self
                    .objects
                    .iter()
                    .map(|o| format!("{}: {}", o.object, o.step.describe()))
                    .collect();
                format!("Not every object was changed. {}.", parts.join("; "))
            }
            Headline::Refused => match self.objects.first().map(|o| &o.step) {
                Some(Step::Refused { code, message }) => format!("The cluster refused it ({code}): {message}"),
                Some(Step::Denied(message)) => format!("You are not allowed to do that: {message}"),
                _ => "The cluster refused it.".to_owned(),
            },
            Headline::Conflict => match self.objects.first().map(|o| &o.step) {
                Some(Step::Conflict(conflict)) => conflict_text(&self.objects[0].object, conflict),
                _ => "A field manager owns what the write sets. The rest was not sent.".to_owned(),
            },
            Headline::Changed => match self.objects.first().map(|o| &o.step) {
                Some(Step::Changed(what)) => format!("{} {}", self.objects[0].object, what.text()),
                _ => "The object changed. Nothing was sent.".to_owned(),
            },
            Headline::Cancelled => "Cancelled. Nothing was sent.".to_owned(),
            Headline::NotSent => match &self.status {
                super::run::Status::NotSent(NotSent::ReadOnly(reason)) => {
                    format!("{reason} Nothing was sent.")
                }
                super::run::Status::NotSent(NotSent::LevelChanged(level)) => format!(
                    "This cluster is now {}. Nothing was sent; you will be asked again.",
                    level.label()
                ),
                super::run::Status::NotSent(NotSent::Session) => {
                    "Shells and port forwards are not writes. Nothing was sent.".to_owned()
                }
                super::run::Status::NotSent(NotSent::Unconfirmed) => {
                    "It was not confirmed. Nothing was sent.".to_owned()
                }
                super::run::Status::NotSent(NotSent::AuditUnavailable(why)) => format!(
                    "The audit log can't be written, so nothing was sent: {}",
                    scrub(why)
                ),
                super::run::Status::NotSent(NotSent::Example) => {
                    "This is example data. Nothing was sent.".to_owned()
                }
                _ => "Nothing was sent.".to_owned(),
            },
        }
    }
}

impl Runner {
    /// Re-bases an action on the objects as they are now: reads each target
    /// and takes its current UID and resourceVersion, so preflight can run
    /// again and show the diff against the new version. It sends nothing and
    /// confirms nothing; the person is asked again.
    pub async fn review_new_version(
        &self,
        mut action: ActionDescription,
    ) -> Result<ActionDescription, Block> {
        for change in &mut action.changes {
            let target = &mut change.target;
            let object = target.label();
            match get(&self.client, target).await {
                Ok(current) => {
                    // A different UID is a different object: nothing to review.
                    if current["metadata"]["uid"].as_str() != Some(target.uid.as_str()) {
                        return Err(Block::Changed {
                            object,
                            what: ChangedWhat::Recreated,
                        });
                    }
                    target.resource_version = current["metadata"]["resourceVersion"]
                        .as_str()
                        .unwrap_or_default()
                        .to_owned();
                }
                Err(Refusal::Changed(what)) => return Err(Block::Changed { object, what }),
                Err(Refusal::Denied(reason)) => {
                    return Err(Block::AccessDenied {
                        object,
                        verb: "get",
                        reason,
                    });
                }
                Err(Refusal::NoAnswer(message) | Refusal::Rejected { message, .. }) => {
                    return Err(Block::NoAnswer { object, message });
                }
                Err(Refusal::Conflict(_)) => unreachable!("a read does not conflict"),
            }
        }
        Ok(action)
    }
}
