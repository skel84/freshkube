//! The Alerts section of a pod, workload or node pane (#884): the active
//! alerts whose labels name this object, from the session's alert read. The
//! pane starts no read: the shell hands it the session's latest state
//! (`set_alerts`), and the lines are derived when that or the open object
//! changes, never in render. Core's `alerts::object` decides which alerts name
//! the object and what may be said when none do: an unread, stale, refused or
//! cut-off read is never "none".
use super::*;
use crate::ui::Tone;
use freshkube_core::monitoring::alerts::object::{self as named, Section};
use freshkube_core::monitoring::alerts::{Alert, State};
use std::sync::Arc;

#[cfg(test)]
mod tests;
mod view;

/// The most alerts the section lists; the rest are counted.
const SHOWN: usize = 8;

pub(super) struct AlertLine {
    pub(super) id: SharedString,
    pub(super) tone: Tone,
    pub(super) severity: &'static str,
    pub(super) title: SharedString,
    /// The state and since when, built with the line, not in render.
    pub(super) meta: SharedString,
    pub(super) text: Option<SharedString>,
    pub(super) aria: SharedString,
}

pub(super) enum Body {
    /// The session's first read hasn't answered.
    Reading,
    /// Nothing is known, and why.
    Unknown(SharedString),
    /// A current read, and none names this object.
    Quiet(SharedString),
    Lines {
        lines: Vec<AlertLine>,
        hidden: usize,
        /// They come from a read that couldn't be repeated.
        stale: bool,
    },
    /// An old read with none for this object: not "none".
    StaleQuiet(SharedString),
}

#[derive(Default)]
pub(super) struct AlertsView {
    state: Option<Arc<State>>,
    pub(super) body: Option<Body>,
}

fn line(alert: &Alert) -> AlertLine {
    let state = alert.state.label();
    let meta = match alert.since {
        Some(since) => format!("{state} since {}", local_time(since)),
        None => state.to_owned(),
    };
    AlertLine {
        id: format!("alert-{}", alert.fingerprint).into(),
        tone: freshkube_monitoring::alerts::severity_tone(alert.severity),
        severity: alert.severity.label(),
        aria: format!("{} alert {}: {meta}", alert.severity.label(), alert.name).into(),
        meta: meta.into(),
        title: alert.name.clone().into(),
        text: alert
            .summary
            .clone()
            .or_else(|| alert.description.clone())
            .map(Into::into),
    }
}

impl DetailPane {
    /// Whether the pane holds an Alerts section for its object, for the
    /// screen's tests.
    #[cfg(test)]
    pub(crate) fn has_alerts_section(&self) -> bool {
        self.alerts.body.is_some()
    }

    /// The session's latest alert state, or `None` when no read runs.
    pub(crate) fn set_alerts(&mut self, state: Option<Arc<State>>, cx: &mut Context<Self>) {
        if self.alerts.state == state {
            return;
        }
        let drawn = self.alerts.body.is_some();
        self.alerts.state = state;
        self.derive_alerts();
        // Going from a section to none redraws too, or the old one stays.
        if self.active && (drawn || self.alerts.body.is_some()) {
            cx.notify();
        }
    }

    /// Derives the section for the open object, from the held state.
    pub(super) fn derive_alerts(&mut self) {
        self.alerts.body = self.alerts_body();
    }

    fn alerts_body(&self) -> Option<Body> {
        let state = self.alerts.state.as_ref()?;
        let target = &self.detail.as_ref()?.target;
        let subject = named::Subject::of(&target.kind.group, &target.kind.kind)?;
        let identity = &target.identity;
        Some(body_of(state, subject, &identity.namespace, &identity.name))
    }
}

/// The section's body for one object under the session's state.
fn body_of(state: &State, subject: named::Subject<'_>, namespace: &str, name: &str) -> Body {
    match named::section(state, subject, namespace, name) {
        Section::Reading => Body::Reading,
        Section::Unknown(why) => Body::Unknown(why.into()),
        Section::Quiet(text) => Body::Quiet(text.into()),
        Section::StaleQuiet(text) => Body::StaleQuiet(text.into()),
        Section::Alerts { alerts, stale } => Body::Lines {
            hidden: alerts.len().saturating_sub(SHOWN),
            lines: alerts.iter().take(SHOWN).map(line).collect(),
            stale,
        },
    }
}

/// The Alerts section of a Node in the Nodes workspace, whose own Overview
/// isn't a detail pane: the same lines and the same words, derived once from
/// the session's state and drawn by the caller.
pub struct AlertsBlock(Body);

impl AlertsBlock {
    /// The section for the node named `node` under `state`.
    pub fn for_node(state: &State, node: &str) -> Self {
        Self(body_of(state, named::Subject::Node, "", node))
    }

    pub fn render(&self, cx: &App) -> AnyElement {
        view::section(&self.0, cx)
    }
}
