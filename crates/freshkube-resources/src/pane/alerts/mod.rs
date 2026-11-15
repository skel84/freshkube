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
use freshkube_core::monitoring::alerts::{Alert, AlertState, Severity, State};
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
    pub(super) state: &'static str,
    pub(super) title: SharedString,
    pub(super) since: Option<SharedString>,
    pub(super) text: Option<SharedString>,
}

pub(super) enum Body {
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
    let tone = match alert.severity {
        Severity::Critical => Tone::Crit,
        Severity::Warning => Tone::Warn,
        Severity::Info | Severity::Other => Tone::Unknown,
    };
    AlertLine {
        id: format!("alert-{}", alert.fingerprint).into(),
        tone,
        severity: alert.severity.label(),
        state: match alert.state {
            AlertState::Pending => "Pending",
            _ => "Firing",
        },
        title: alert.name.clone().into(),
        since: alert
            .since
            .map(|since| format!("since {}", local_time(since)).into()),
        text: alert
            .summary
            .clone()
            .or_else(|| alert.description.clone())
            .map(Into::into),
    }
}

impl DetailPane {
    /// The session's latest alert state, or `None` when no read runs.
    pub(crate) fn set_alerts(&mut self, state: Option<Arc<State>>, cx: &mut Context<Self>) {
        if self.alerts.state == state {
            return;
        }
        self.alerts.state = state;
        self.derive_alerts();
        if self.active && self.alerts.body.is_some() {
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
        Some(
            match named::section(state, subject, &identity.namespace, &identity.name) {
                Section::Unknown(why) => Body::Unknown(why.into()),
                Section::Quiet(text) => Body::Quiet(text.into()),
                Section::StaleQuiet(text) => Body::StaleQuiet(text.into()),
                Section::Alerts { alerts, stale } => Body::Lines {
                    hidden: alerts.len().saturating_sub(SHOWN),
                    lines: alerts.iter().take(SHOWN).map(line).collect(),
                    stale,
                },
            },
        )
    }
}
