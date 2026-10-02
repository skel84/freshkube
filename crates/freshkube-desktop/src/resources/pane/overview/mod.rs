//! The Overview tab: what every kind has in common, conditions, and a
//! Secret's keys with their values hidden until revealed.

use freshkube_core::resources::{Condition, SecretValue};
use gpui_kit::assets::IconName;
use gpui_kit::component::{
    Sizable,
    button::{Button, ButtonVariants},
    h_flex, v_flex,
};
use gpui_kit::prelude::*;
use gpui_kit::*;

use super::{DetailPane, local_time};
use crate::palette::palette;
use crate::resources::detail::{Detail, DocumentView, Reveal};
use crate::screens::field;
use crate::ui::{self, MONO_FONT, Tone, dp};

const LABELS_SHOWN: usize = 12;
const ANNOTATIONS_SHOWN: usize = 6;
/// Even "Show all" draws at most this many labels or annotations; the YAML
/// has every one.
const MAX_SHOWN: usize = 200;
const ANNOTATION_CHARS: usize = 240;
/// Revealed values are cut to this many characters when drawn; Copy takes
/// the whole value.
const VALUE_CHARS: usize = 4_000;

struct ConditionLine {
    kind: SharedString,
    status: SharedString,
    tone: Tone,
    detail: Option<SharedString>,
    changed: Option<SharedString>,
}

impl ConditionLine {
    fn new(condition: &Condition) -> Self {
        let detail = [condition.reason.as_str(), condition.message.as_str()]
            .into_iter()
            .filter(|text| !text.is_empty())
            .collect::<Vec<_>>()
            .join(" · ");
        Self {
            kind: condition.kind.clone().into(),
            status: condition.status.clone().into(),
            tone: condition_tone(&condition.kind, &condition.status),
            detail: (!detail.is_empty()).then(|| detail.into()),
            changed: condition
                .changed
                .map(|time| format!("since {}", local_time(time)).into()),
        }
    }
}

/// How a condition reads. Most are good when `True`; a few report a problem
/// when `True`. Types this doesn't know stay neutral rather than guess.
fn condition_tone(kind: &str, status: &str) -> Tone {
    const GOOD_WHEN_TRUE: [&str; 9] = [
        "Ready",
        "Available",
        "Initialized",
        "ContainersReady",
        "PodScheduled",
        "PodReadyToStartContainers",
        "Progressing",
        "Complete",
        "Established",
    ];
    const BAD_WHEN_TRUE: [&str; 6] = [
        "MemoryPressure",
        "DiskPressure",
        "PIDPressure",
        "NetworkUnavailable",
        "ReplicaFailure",
        "Failed",
    ];
    let good_when_true = if GOOD_WHEN_TRUE.contains(&kind) {
        true
    } else if BAD_WHEN_TRUE.contains(&kind) {
        false
    } else {
        return if status == "Unknown" {
            Tone::Unknown
        } else {
            Tone::Outline
        };
    };
    match status {
        "True" | "False" if (status == "True") == good_when_true => Tone::Good,
        "True" | "False" => Tone::Crit,
        "Unknown" => Tone::Unknown,
        _ => Tone::Outline,
    }
}

struct SecretLine {
    name: String,
    size: SharedString,
}

/// The overview's text, derived once per document read, never while drawing.
pub(super) struct Summary {
    kind: SharedString,
    created: Option<SharedString>,
    deleting: Option<SharedString>,
    generation: Option<SharedString>,
    owners: Vec<SharedString>,
    conditions: Vec<ConditionLine>,
    labels: Vec<SharedString>,
    label_count: usize,
    annotations: Vec<(SharedString, SharedString)>,
    annotation_count: usize,
    finalizers: Vec<SharedString>,
    /// A Secret's type and keys with their sizes.
    secret: Option<(SharedString, Vec<SecretLine>)>,
}

impl Summary {
    pub(super) fn new(view: &DocumentView) -> Self {
        let document = &view.document;
        let overview = &document.overview;
        Self {
            kind: format!("{} · {}", document.kind, document.api_version).into(),
            created: overview.created.map(|time| local_time(time).into()),
            deleting: overview.deleting.map(|time| local_time(time).into()),
            generation: overview.generation.map(|generation| {
                match overview.observed_generation {
                    Some(observed) if observed < generation => {
                        format!("{generation}; its controller has seen {observed}")
                    }
                    Some(_) => format!("{generation}, seen by its controller"),
                    None => generation.to_string(),
                }
                .into()
            }),
            owners: overview
                .owners
                .iter()
                .map(|owner| {
                    let controller = if owner.controller {
                        " (controller)"
                    } else {
                        ""
                    };
                    format!("{} {}{controller}", owner.kind, owner.name).into()
                })
                .collect(),
            conditions: overview.conditions.iter().map(ConditionLine::new).collect(),
            labels: overview
                .labels
                .iter()
                .take(MAX_SHOWN)
                .map(|(key, value)| format!("{key}={value}").into())
                .collect(),
            label_count: overview.labels.len(),
            annotations: overview
                .annotations
                .iter()
                .take(MAX_SHOWN)
                .map(|(key, value)| (key.clone().into(), shorten(value, ANNOTATION_CHARS).into()))
                .collect(),
            annotation_count: overview.annotations.len(),
            finalizers: overview
                .finalizers
                .iter()
                .map(|finalizer| finalizer.clone().into())
                .collect(),
            secret: overview.secret.as_ref().map(|secret| {
                (
                    secret.secret_type.clone().into(),
                    secret
                        .keys
                        .iter()
                        .map(|key| SecretLine {
                            name: key.name.clone(),
                            size: byte_size(key.bytes).into(),
                        })
                        .collect(),
                )
            }),
        }
    }
}

fn byte_size(bytes: usize) -> String {
    match bytes {
        1 => "1 byte".into(),
        0..1_024 => format!("{bytes} bytes"),
        _ => format!("{:.1} KiB", bytes as f64 / 1_024.),
    }
}

/// `text` on one line, cut to `max` characters.
fn shorten(text: &str, max: usize) -> String {
    let line = text.trim_end().replace('\n', " ↵ ");
    match line.char_indices().nth(max) {
        Some((cut, _)) => format!("{}…", &line[..cut]),
        None => line,
    }
}

mod view;

#[cfg(test)]
mod tests {
    use super::{Tone, condition_tone, shorten};

    #[test]
    fn conditions_read_good_or_bad_only_when_their_meaning_is_known() {
        assert_eq!(condition_tone("Ready", "True"), Tone::Good);
        assert_eq!(condition_tone("Ready", "False"), Tone::Crit);
        assert_eq!(condition_tone("MemoryPressure", "False"), Tone::Good);
        assert_eq!(condition_tone("MemoryPressure", "True"), Tone::Crit);
        assert_eq!(condition_tone("Ready", "Unknown"), Tone::Unknown);
        assert_eq!(condition_tone("SomethingCustom", "True"), Tone::Outline);
        assert_eq!(condition_tone("SomethingCustom", "Unknown"), Tone::Unknown);
    }

    #[test]
    fn long_values_are_shortened_onto_one_line() {
        assert_eq!(shorten("a\nb\n", 10), "a ↵ b");
        assert_eq!(shorten("ééééé", 3), "ééé…");
        assert_eq!(shorten("abc", 3), "abc");
    }
}
