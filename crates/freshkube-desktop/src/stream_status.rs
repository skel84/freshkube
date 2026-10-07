//! What a stream's controls say about it: the pod and workload logs and the
//! shell tabs share it.

use crate::ui;
use gpui_kit::SharedString;

/// What the controls of a pod's log, a workload's log and a shell tab say
/// about their stream, derived when it changes.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Status {
    pub(crate) tone: ui::Tone,
    pub(crate) tag: SharedString,
    pub(crate) text: SharedString,
    /// Both, for assistive technology and tests.
    pub(crate) label: SharedString,
}

impl Default for Status {
    fn default() -> Self {
        Self {
            tone: ui::Tone::Unknown,
            tag: Default::default(),
            text: Default::default(),
            label: Default::default(),
        }
    }
}

impl Status {
    /// The status for a tag and the words beside it, labelled "tag: text".
    pub(crate) fn new(tone: ui::Tone, tag: &str, text: &str) -> Self {
        let label = match (tag, text) {
            ("", text) => text.to_owned(),
            (tag, "") => tag.to_owned(),
            (tag, text) => format!("{tag}: {text}"),
        };
        Self {
            tone,
            tag: tag.to_owned().into(),
            text: text.to_owned().into(),
            label: label.into(),
        }
    }
}
