//! An HTTPRoute's trace as rows (#949, N2a): derived once from core's
//! `RouteEntry` when an answer arrives, and drawn compact (the Routes
//! Inspector, a route's Overview) or in full (a route's "Trace" tab). The
//! words are the trace's own; nothing here reads the cluster or applies
//! anything: a proposed ReferenceGrant is text with a Copy button.
use freshkube_core::resources::traffic::routes::{
    Branch, BranchKind, BranchVerdict, Owner, ParentTrace, RouteEntry, RouteVerdict, StepVerdict,
    TraceStep,
};
use gpui_kit::prelude::*;
use gpui_kit::*;

use crate::ui::Tone;

mod view;
pub use view::{OnLink, backends, compact, full, proposals};
#[cfg(test)]
mod tests;

/// Where a link from the trace goes, through the normal object entry point.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TraceLink {
    Gateway { namespace: String, name: String },
    Service { namespace: String, name: String },
}

/// The tone a verdict is drawn in. Unknown is never green.
pub fn verdict_tone(verdict: RouteVerdict) -> Tone {
    match verdict {
        RouteVerdict::NotServing => Tone::Crit,
        RouteVerdict::Partly => Tone::Warn,
        RouteVerdict::Unknown => Tone::Unknown,
        RouteVerdict::Serving => Tone::Good,
        RouteVerdict::Mesh => Tone::Outline,
    }
}

fn step_tone(verdict: StepVerdict) -> (Tone, &'static str) {
    match verdict {
        StepVerdict::Pass => (Tone::Good, "Passes"),
        StepVerdict::Warn => (Tone::Warn, "Warning"),
        StepVerdict::Fail => (Tone::Crit, "Broken"),
        StepVerdict::Unknown => (Tone::Unknown, "Unknown"),
        StepVerdict::Neutral => (Tone::Outline, "Not judged"),
        StepVerdict::NotReached => (Tone::Waiting, "Not reached"),
    }
}

fn branch_tone(verdict: BranchVerdict) -> (Tone, &'static str) {
    match verdict {
        BranchVerdict::Serving => (Tone::Good, "Serving"),
        BranchVerdict::Broken => (Tone::Crit, "Not serving"),
        BranchVerdict::Unknown => (Tone::Unknown, "Unknown"),
        BranchVerdict::Excluded => (Tone::Outline, "No traffic"),
    }
}

pub(crate) struct StepRow {
    pub id: SharedString,
    pub name: &'static str,
    pub tone: Tone,
    pub state: &'static str,
    pub owner: Option<&'static str>,
    pub asks: SharedString,
    pub serves: SharedString,
    pub reason: SharedString,
    pub controller: Option<SharedString>,
    pub stale: bool,
}

pub(crate) struct BranchRow {
    pub id: SharedString,
    pub title: SharedString,
    pub rule: SharedString,
    pub share: Option<SharedString>,
    pub tone: Tone,
    pub state: &'static str,
    pub steps: Vec<StepRow>,
    pub service: Option<TraceLink>,
}

pub(crate) struct ParentRow {
    pub id: SharedString,
    pub title: SharedString,
    pub controller: Option<SharedString>,
    pub tone: Tone,
    pub state: &'static str,
    /// Where this parent stops, in the trace's words; none when it serves.
    pub stop: Option<SharedString>,
    pub steps: Vec<StepRow>,
    pub branches: Vec<BranchRow>,
    pub gateway: Option<TraceLink>,
}

pub(crate) struct ProposalRow {
    pub id: SharedString,
    pub title: SharedString,
    pub yaml: SharedString,
}

/// One route's trace, ready to draw.
pub struct TraceModel {
    pub(crate) id: SharedString,
    /// `host/path`, for the tab's title.
    pub label: SharedString,
    pub verdict: RouteVerdict,
    pub(crate) headline: SharedString,
    pub(crate) owner: Option<&'static str>,
    pub(crate) stale: bool,
    pub(crate) parents: Vec<ParentRow>,
    pub(crate) proposals: Vec<ProposalRow>,
}

impl TraceModel {
    pub fn new(entry: &RouteEntry, id: &str) -> Self {
        let trace = &entry.trace;
        let namespace = entry.facts.namespace.as_str();
        let id: SharedString = id.to_owned().into();
        let parents = trace
            .parents
            .iter()
            .enumerate()
            .map(|(ix, parent)| parent_row(&id, ix, parent, namespace))
            .collect();
        let proposals = trace
            .proposals()
            .into_iter()
            .enumerate()
            .map(|(ix, proposal)| ProposalRow {
                id: format!("{id}-grant-{ix}").into(),
                title: format!(
                    "ReferenceGrant in {} for Service {} (HTTPRoutes in {} only)",
                    proposal.namespace, proposal.service, proposal.from_namespace
                )
                .into(),
                yaml: proposal.yaml.clone().into(),
            })
            .collect();
        Self {
            label: entry.facts.label().into(),
            verdict: trace.verdict,
            headline: format!(
                "{} · {}",
                trace.verdict.label(),
                trace.summary.line()
            )
            .into(),
            owner: trace.summary.owner.map(Owner::label),
            stale: trace.stale,
            parents,
            proposals,
            id,
        }
    }

    /// The prefix of every element id this model draws.
    pub fn id(&self) -> &str {
        &self.id
    }

    /// The words of the verdict and where it stops, for a row or a tooltip.
    pub fn headline(&self) -> &str {
        &self.headline
    }

    pub fn proposals(&self) -> impl Iterator<Item = &str> {
        self.proposals.iter().map(|p| p.yaml.as_ref())
    }
}

fn step_row(prefix: &str, ix: usize, step: &TraceStep) -> StepRow {
    let (tone, state) = step_tone(step.verdict);
    StepRow {
        id: format!("{prefix}-step-{ix}").into(),
        name: step.id.label(),
        tone,
        state,
        owner: step.owner.filter(|_| tone != Tone::Good).map(Owner::label),
        asks: step.asks.clone().into(),
        serves: step.serves.clone().into(),
        reason: step.reason.clone().into(),
        controller: step.controller.clone().map(Into::into),
        stale: step.stale,
    }
}

fn branch_row(prefix: &str, ix: usize, branch: &Branch) -> BranchRow {
    let (tone, state) = branch_tone(branch.verdict);
    let id = format!("{prefix}-branch-{ix}");
    BranchRow {
        title: match branch.kind {
            BranchKind::Service => branch.label.clone().into(),
            BranchKind::Redirect => "Redirect, answered by the gateway".into(),
            BranchKind::Extension => "ExtensionRef, handled by an extension".into(),
            BranchKind::Other => format!("{}, not a Service", branch.label).into(),
            BranchKind::Empty => "Rule without a backend".into(),
        },
        rule: branch.rule.clone().into(),
        share: branch.share.map(|share| format!("{share}%").into()),
        tone,
        state,
        steps: branch
            .steps
            .iter()
            .enumerate()
            .map(|(n, step)| step_row(&id, n, step))
            .collect(),
        service: branch
            .service
            .clone()
            .map(|(namespace, name)| TraceLink::Service { namespace, name }),
        id: id.into(),
    }
}

fn parent_row(prefix: &str, ix: usize, parent: &ParentTrace, route_namespace: &str) -> ParentRow {
    let id = format!("{prefix}-parent-{ix}");
    let stop = parent
        .steps
        .iter()
        .chain(parent.branches.iter().flat_map(|b| &b.steps))
        .find(|s| matches!(s.verdict, StepVerdict::Fail | StepVerdict::Unknown))
        .map(|s| format!("stops at {} — {}", s.id.label(), s.reason).into());
    ParentRow {
        title: if parent.parent.is_gateway() {
            format!("Gateway {}", parent.label).into()
        } else {
            format!("{} {}", parent.parent.kind, parent.label).into()
        },
        controller: parent.controller.clone().map(Into::into),
        tone: verdict_tone(parent.verdict),
        state: parent.verdict.label(),
        stop,
        steps: parent
            .steps
            .iter()
            .enumerate()
            .map(|(n, step)| step_row(&id, n, step))
            .collect(),
        branches: parent
            .branches
            .iter()
            .enumerate()
            .map(|(n, branch)| branch_row(&id, n, branch))
            .collect(),
        gateway: parent.parent.is_gateway().then(|| TraceLink::Gateway {
            namespace: parent.parent.namespace_or(route_namespace).to_owned(),
            name: parent.parent.name.clone(),
        }),
        id: id.into(),
    }
}
