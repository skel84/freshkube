//! Workloads: Kubernetes workload health for the whole cluster, like the
//! TUI's workload view. Namespaces come first with problems on top; each one
//! expands to its deployments, statefulsets, daemonsets and the pods that need
//! attention. Read-only: nothing here changes the cluster.
use std::collections::HashSet;
use std::sync::Arc;

use chrono::{DateTime, Utc};
use freshkube_core::constants::HIGH_RESTART_THRESHOLD;
use freshkube_core::pluralize;
use freshkube_core::workloads::{
    HealthState, NamespaceSummary, PodInfo, PodIssue, WorkloadCollectionOutcome, WorkloadInfo,
    WorkloadKind, WorkloadSnapshot, WorkloadSource, WorkloadSourceError,
};
use gpui_kit::assets::IconName;
use gpui_kit::component::{
    Icon, Selectable, Sizable,
    button::{Button, ButtonVariants},
    h_flex,
    input::{Input, InputEvent, InputState},
    v_flex,
};
use gpui_kit::prelude::*;
use gpui_kit::*;
use tokio::runtime::Handle;

use super::{
    Loader, Scope, ScreenEvent, ScreenPanel, ScreenSource, failure_banner, field, gate, mono,
    partial_notice, refresh_control, retry_button, segment,
};
use crate::palette::palette;
use crate::ui::{self, MONO_FONT, Tone, dp};
use freshkube_ui::inspector::{self, InspectorSplit};
use freshkube_ui::page::{self, PageHeader};
use freshkube_ui::status::Segment;
use freshkube_ui::table::{self, DataTable, TableState};
use source::Derived;

const CONTEXT: &str = "TalosWorkloads";
const PAGE_ROWS: isize = 20;
/// The page header's id prefix.
const PREFIX: &str = "workloads";

actions!(
    talos_workloads,
    [
        NextItem,
        PreviousItem,
        FirstItem,
        LastItem,
        NextPage,
        PreviousPage,
        ToggleExpanded,
        ToggleUnhealthy,
        FocusFilter,
        ClearFilter
    ]
);

/// What a refresh loads: the snapshot plus any resource lists that failed.
#[derive(Clone, Debug)]
pub(crate) struct WorkloadData {
    snapshot: WorkloadSnapshot,
    unavailable: Vec<WorkloadSourceError>,
    missing_notice: Vec<String>,
}

impl WorkloadData {
    fn missing(&self, source: WorkloadSource) -> bool {
        self.unavailable.iter().any(|error| error.source == source)
    }
}

/// Identifies a row across refreshes and filter changes.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(crate) enum ItemKey {
    Namespace(String),
    Workload {
        namespace: String,
        name: String,
        kind: WorkloadKind,
    },
    Pod {
        namespace: String,
        name: String,
    },
}

/// A visible row, as indexes into the snapshot.
#[derive(Clone, Copy, Debug)]
pub(crate) enum RowRef {
    Namespace(usize),
    Workload(usize, usize),
    Pod(usize, usize),
}

impl RowRef {
    fn key(self, snapshot: &WorkloadSnapshot) -> ItemKey {
        match self {
            RowRef::Namespace(ns) => ItemKey::Namespace(snapshot.namespaces[ns].name.clone()),
            RowRef::Workload(ns, ix) => {
                let workload = &snapshot.namespaces[ns].workloads[ix];
                ItemKey::Workload {
                    namespace: workload.namespace.clone(),
                    name: workload.name.clone(),
                    kind: workload.kind,
                }
            }
            RowRef::Pod(ns, ix) => {
                let pod = &snapshot.namespaces[ns].problem_pods[ix];
                ItemKey::Pod {
                    namespace: pod.namespace.clone(),
                    name: pod.name.clone(),
                }
            }
        }
    }
}

/// Text for one row, whatever its kind.
pub(crate) struct RowView {
    health: HealthState,
    tone: Tone,
    nested: bool,
    chevron: Option<IconName>,
    name: String,
    kind: &'static str,
    ready: String,
    issue: String,
}

fn issues_in(namespace: &NamespaceSummary) -> usize {
    namespace.problem_pods.len()
        + namespace
            .workloads
            .iter()
            .filter(|workload| workload.health != HealthState::Healthy)
            .count()
}

fn health_label(health: HealthState) -> &'static str {
    match health {
        HealthState::Failing => "Failing",
        HealthState::Degraded => "Degraded",
        HealthState::Pending => "Pending",
        HealthState::Healthy => "Healthy",
    }
}

fn health_tone(health: HealthState) -> Tone {
    match health {
        HealthState::Failing => Tone::Crit,
        HealthState::Degraded => Tone::Warn,
        HealthState::Pending => Tone::Unknown,
        HealthState::Healthy => Tone::Good,
    }
}

/// A pod's tone: the skull when a container ran and stopped, otherwise its
/// severity's.
fn pod_tone(issue: &PodIssue) -> Tone {
    if issue.died() {
        Tone::Died
    } else {
        health_tone(issue.severity())
    }
}

fn issue_detail(issue: &PodIssue) -> String {
    match issue {
        PodIssue::HighRestarts(count) => {
            format!("{count} restarts (flagged from {HIGH_RESTART_THRESHOLD})")
        }
        PodIssue::Unknown(reason) => format!("Unclassified state: {reason}"),
        other => other.label().to_owned(),
    }
}

/// Like the TUI's age column: the largest whole unit.
fn age(created: Option<DateTime<Utc>>) -> String {
    let Some(created) = created else {
        return "unknown".into();
    };
    let elapsed = Utc::now().signed_duration_since(created);
    if elapsed.num_days() > 0 {
        format!("{}d", elapsed.num_days())
    } else if elapsed.num_hours() > 0 {
        format!("{}h", elapsed.num_hours())
    } else if elapsed.num_minutes() > 0 {
        format!("{}m", elapsed.num_minutes())
    } else {
        format!("{}s", elapsed.num_seconds().max(0))
    }
}

/// The visible rows for one data set and one set of filters. The data is
/// held, so pointer identity can't be reused by a newer set.
pub(crate) struct WorkloadsScreen {
    _runtime: Handle,
    summary_managed: bool,
    source: Option<ScreenSource>,
    loader: Loader<Arc<WorkloadData>>,
    selected: Option<ItemKey>,
    collapsed: HashSet<String>,
    only_unhealthy: bool,
    query: Entity<InputState>,
    focus: FocusHandle,
    table: TableState,
    /// The rows and columns, derived by [`Self::sync`].
    derived: Option<Derived>,
    /// The selection's details, derived by [`Self::sync`]; `None` without
    /// a selection, so no Inspector shows.
    detail: Option<detail::Detail>,
    /// The table and the Inspector, whose width is saved under `health`.
    split: InspectorSplit,
    /// The most the Name column takes in the list's width, so its glyph
    /// and name can pin when the table scrolls sideways.
    name_most: f32,
    /// The status bar's line and the loader revision it was derived at.
    status: Option<(u64, Segment)>,
    _subscription: Subscription,
    /// Caret and selection changes redraw the filter; this view is cached, so
    /// it has to hear about them.
    _query_observer: Subscription,
}

impl EventEmitter<ScreenEvent> for WorkloadsScreen {}

impl WorkloadData {
    pub(crate) fn from_outcome(outcome: &WorkloadCollectionOutcome) -> Result<Arc<Self>, String> {
        match outcome.snapshot() {
            Some(snapshot) => Ok(Arc::new(Self {
                snapshot: snapshot.clone(),
                unavailable: outcome.unavailable().to_vec(),
                missing_notice: outcome
                    .unavailable()
                    .iter()
                    .map(|error| format!("{}: {}", error.source.label(), error.message))
                    .collect(),
            })),
            None => Err(outcome
                .unavailable()
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>()
                .join("; ")),
        }
    }
}

impl ScreenPanel for WorkloadsScreen {
    fn new(runtime: Handle, window: &mut Window, cx: &mut Context<Self>) -> Self {
        cx.bind_keys([
            KeyBinding::new("down", NextItem, Some(CONTEXT)),
            KeyBinding::new("up", PreviousItem, Some(CONTEXT)),
            KeyBinding::new("home", FirstItem, Some(CONTEXT)),
            KeyBinding::new("end", LastItem, Some(CONTEXT)),
            KeyBinding::new("pagedown", NextPage, Some(CONTEXT)),
            KeyBinding::new("pageup", PreviousPage, Some(CONTEXT)),
            KeyBinding::new("enter", ToggleExpanded, Some(CONTEXT)),
            KeyBinding::new("u", ToggleUnhealthy, Some(CONTEXT)),
            KeyBinding::new("/", FocusFilter, Some(CONTEXT)),
            KeyBinding::new("escape", ClearFilter, Some(CONTEXT)),
        ]);
        let query = cx.new(|cx| InputState::new(window, cx).placeholder("Filter  /"));
        let subscription = cx.subscribe_in(&query, window, |this, _, event, window, cx| {
            match event {
                InputEvent::Change => {
                    this.sync(cx);
                    this.table.reveal(0, ScrollStrategy::Top);
                    cx.notify();
                }
                // Enter hands the keyboard back to the list.
                InputEvent::PressEnter { .. } => window.focus(&this.focus, cx),
                _ => {}
            }
        });
        Self {
            _runtime: runtime,
            summary_managed: false,
            source: None,
            loader: Loader::default(),
            selected: None,
            collapsed: HashSet::new(),
            only_unhealthy: false,
            _query_observer: cx.observe(&query, |this, _, cx| {
                this.sync(cx);
                cx.notify();
            }),
            query,
            focus: cx.focus_handle(),
            table: TableState::new("workload"),
            derived: None,
            detail: None,
            split: {
                let file = crate::navigation_file::NavigationFile::global(cx);
                InspectorSplit::new(
                    file.inspector_width("health"),
                    move |width, cx| file.set_inspector_width("health", width, cx),
                    cx,
                )
            },
            name_most: f32::INFINITY,
            status: None,
            _subscription: subscription,
        }
    }

    fn set_source(&mut self, source: Option<ScreenSource>, _: &mut Window, cx: &mut Context<Self>) {
        let changed = self.source.as_ref().map(|source| &source.target)
            != source.as_ref().map(|source| &source.target);
        if changed {
            // Workloads are cluster-wide, but a new target may be another
            // context, so nothing from the old one is kept.
            self.loader.reset();
            self.selected = None;
            self.collapsed.clear();
        }
        self.source = source;
        // Inspect node follows the target's nodes, so the details are
        // derived again.
        self.detail = None;
        self.sync(cx);
        cx.notify();
    }

    fn activate(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.loader.data().is_none() && !self.loader.is_loading() {
            self.refresh(window, cx);
        }
    }

    fn focus(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        window.focus(&self.focus, cx);
    }

    fn status(&mut self) -> Option<&Segment> {
        self.sync_status();
        self.status.as_ref().map(|(_, line)| line)
    }

    fn refresh(&mut self, _: &mut Window, cx: &mut Context<Self>) {
        if self.summary_managed {
            cx.emit(ScreenEvent::RefreshSummary);
            return;
        }
        let Some(source) = self.source.clone() else {
            return;
        };
        if self.loader.is_loading() {
            return;
        }
        if source.live.is_some() {
            return;
        }
        self.loader.resolve(
            source.target.clone(),
            Ok(Arc::new(example::example(&source))),
        );
        self.sync(cx);
        cx.notify();
    }
}

impl WorkloadsScreen {
    /// Apply Health data prepared by the shell's shared observation session.
    pub(crate) fn apply_summary(
        &mut self,
        context: &str,
        data: Result<Arc<WorkloadData>, String>,
        cx: &mut Context<Self>,
    ) {
        self.summary_managed = true;
        if self.source.is_none() {
            self.source = Some(ScreenSource {
                target: crate::backend::Target {
                    epoch: 0,
                    context: context.into(),
                    node: String::new(),
                    address: String::new(),
                },
                nodes: Arc::default(),
                live: None,
            });
        }
        let source = self.source.as_ref().unwrap();
        self.loader.resolve(source.target.clone(), data);
        self.sync(cx);
        cx.notify();
    }

    fn filter_text(&self, cx: &App) -> String {
        self.query.read(cx).value().trim().to_lowercase()
    }

    /// Visible rows: namespaces (problems first, as collected) followed by
    /// their workloads and problem pods unless collapsed.
    fn compute_rows(&self, data: &WorkloadData, query: &str) -> Vec<RowRef> {
        let mut rows = Vec::new();
        for (ns_ix, namespace) in data.snapshot.namespaces.iter().enumerate() {
            let ns_match = query.is_empty() || namespace.name.to_lowercase().contains(query);
            let workloads: Vec<usize> = namespace
                .workloads
                .iter()
                .enumerate()
                .filter(|(_, workload)| {
                    (!self.only_unhealthy || workload.health != HealthState::Healthy)
                        && (ns_match || workload_matches(workload, query))
                })
                .map(|(ix, _)| ix)
                .collect();
            let pods: Vec<usize> = namespace
                .problem_pods
                .iter()
                .enumerate()
                .filter(|(_, pod)| ns_match || pod_matches(pod, query))
                .map(|(ix, _)| ix)
                .collect();
            let own =
                ns_match && (!self.only_unhealthy || namespace.health != HealthState::Healthy);
            if !own && workloads.is_empty() && pods.is_empty() {
                continue;
            }
            rows.push(RowRef::Namespace(ns_ix));
            if self.collapsed.contains(&namespace.name) {
                continue;
            }
            rows.extend(workloads.into_iter().map(|ix| RowRef::Workload(ns_ix, ix)));
            rows.extend(pods.into_iter().map(|ix| RowRef::Pod(ns_ix, ix)));
        }
        rows
    }

    fn describe(&self, row: RowRef, data: &WorkloadData) -> RowView {
        let namespaces = &data.snapshot.namespaces;
        match row {
            RowRef::Namespace(ns) => {
                let namespace = &namespaces[ns];
                let issues = issues_in(namespace);
                RowView {
                    health: namespace.health,
                    tone: health_tone(namespace.health),
                    nested: false,
                    chevron: Some(if self.collapsed.contains(&namespace.name) {
                        IconName::ChevronRight
                    } else {
                        IconName::ChevronDown
                    }),
                    name: namespace.name.clone(),
                    kind: "Namespace",
                    ready: pluralize(namespace.total_workloads, "workload", "workloads"),
                    issue: if issues > 0 {
                        pluralize(issues, "issue", "issues")
                    } else {
                        "healthy".into()
                    },
                }
            }
            RowRef::Workload(ns, ix) => {
                let workload = &namespaces[ns].workloads[ix];
                RowView {
                    health: workload.health,
                    tone: health_tone(workload.health),
                    nested: true,
                    chevron: None,
                    name: workload.name.clone(),
                    kind: workload.kind.label(),
                    ready: format!("{}/{} ready", workload.ready, workload.desired),
                    issue: workload.issues.join(", "),
                }
            }
            RowRef::Pod(ns, ix) => {
                let pod = &namespaces[ns].problem_pods[ix];
                RowView {
                    health: pod.issue.severity(),
                    tone: pod_tone(&pod.issue),
                    nested: true,
                    chevron: None,
                    name: pod.name.clone(),
                    kind: "Pod",
                    ready: pluralize(pod.restarts.max(0) as usize, "restart", "restarts"),
                    issue: pod.issue.label().to_owned(),
                }
            }
        }
    }

    fn step(&mut self, delta: isize, window: &Window, cx: &mut Context<Self>) {
        self.sync(cx);
        if let Some(key) = table::step(self, delta, cx) {
            let was_open = self.selected.is_some();
            self.selected = Some(key);
            self.sync(cx);
            table::reveal(self, ScrollStrategy::Nearest);
            self.reveal_if_opened(was_open, window, cx);
            cx.notify();
        }
    }

    /// Stacked, the Inspector that a selection opens shrinks the table
    /// over the selected row; bring the row back into view once the
    /// table's height settles, as Nodes does.
    fn reveal_if_opened(&self, was_open: bool, window: &Window, cx: &mut Context<Self>) {
        if !was_open
            && self.selected.is_some()
            && crate::screens::page_width(window) < inspector::SPLIT_WIDTH
        {
            // Closed again: there is nothing to reveal.
            table::reveal_when_settled(
                cx.entity().downgrade(),
                |screen| screen.selected.is_some(),
                window,
            );
        }
    }

    fn select(&mut self, key: ItemKey, cx: &mut Context<Self>) {
        self.selected = Some(key);
        self.sync(cx);
        cx.notify();
    }

    /// Like the TUI's Enter: open or close the selected item's namespace.
    fn toggle_expanded(&mut self, cx: &mut Context<Self>) {
        let namespace = match &self.selected {
            Some(ItemKey::Namespace(name)) => name.clone(),
            _ => return,
        };
        if !self.collapsed.remove(&namespace) {
            self.collapsed.insert(namespace);
        }
        self.sync(cx);
        cx.notify();
    }

    pub(crate) fn set_only_unhealthy(&mut self, on: bool, cx: &mut Context<Self>) {
        self.only_unhealthy = on;
        self.sync(cx);
        self.table.reveal(0, ScrollStrategy::Top);
        cx.notify();
    }

    /// Escape steps back: it clears the filters, then closes the
    /// Inspector by clearing the selection.
    fn escape(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.query.read(cx).value().is_empty() || self.only_unhealthy {
            self.clear_filter(window, cx);
        } else if self.selected.take().is_some() {
            self.sync(cx);
            cx.notify();
        }
    }

    fn clear_filter(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.query
            .update(cx, |input, cx| input.set_value("", window, cx));
        self.only_unhealthy = false;
        self.sync(cx);
        cx.notify();
    }

    /// Derives the status bar's line again when a new snapshot arrives:
    /// the counts don't follow the filters.
    fn sync_status(&mut self) {
        let revision = self.loader.revision();
        if self.status.as_ref().is_some_and(|(at, _)| *at == revision) {
            return;
        }
        let parts = self
            .loader
            .data()
            .map(|data| source::status_parts(data))
            .unwrap_or_default();
        let line = segment(self.source.as_ref(), &self.loader, parts);
        self.status = Some((revision, line));
    }
}

fn workload_matches(workload: &WorkloadInfo, query: &str) -> bool {
    query.is_empty()
        || format!(
            "{} {} {}",
            workload.name,
            workload.kind.label(),
            workload.issues.join(" ")
        )
        .to_lowercase()
        .contains(query)
}

fn pod_matches(pod: &PodInfo, query: &str) -> bool {
    query.is_empty()
        || format!(
            "{} {} {} {} pod",
            pod.name,
            pod.node.as_deref().unwrap_or(""),
            pod.phase,
            pod.issue.label()
        )
        .to_lowercase()
        .contains(query)
}

mod detail;
mod example;
mod source;
#[cfg(test)]
mod tests;
mod view;
