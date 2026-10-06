//! Workloads: Kubernetes workload health for the whole cluster, like the
//! TUI's workload view. Namespaces come first with problems on top; each one
//! expands to its deployments, statefulsets, daemonsets and the pods that need
//! attention. Read-only: nothing here changes the cluster.
use std::collections::HashSet;
use std::sync::Arc;

use chrono::{DateTime, Utc};
use freshkube_core::constants::HIGH_RESTART_THRESHOLD;
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
    panel, partial_notice, refresh_control, retry_button, segment,
};
use crate::palette::palette;
use crate::ui::{self, MONO_FONT, Tone, dp};
use freshkube_ui::page::{self, PageHeader};
use freshkube_ui::status::Segment;
use freshkube_ui::table::{self, DataTable, TableState};
use source::Derived;

const CONTEXT: &str = "TalosWorkloads";
const PAGE_ROWS: isize = 20;
/// The page header's id prefix.
const PREFIX: &str = "workloads";
/// The details' height under the list on a narrow page.
const DETAILS_HEIGHT: f32 = 240.;

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

fn plural(count: usize, one: &str, many: &str) -> String {
    format!("{count} {}", if count == 1 { one } else { many })
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
        self.loader
            .resolve(source.target.clone(), Ok(Arc::new(example(&source))));
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
                    ready: plural(namespace.total_workloads, "workload", "workloads"),
                    issue: if issues > 0 {
                        plural(issues, "issue", "issues")
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
                    ready: plural(pod.restarts.max(0) as usize, "restart", "restarts"),
                    issue: pod.issue.label().to_owned(),
                }
            }
        }
    }

    fn step(&mut self, delta: isize, cx: &mut Context<Self>) {
        self.sync(cx);
        if let Some(key) = table::step(self, delta, cx) {
            self.selected = Some(key);
            table::reveal(self, ScrollStrategy::Nearest);
            cx.notify();
        }
    }

    fn select(&mut self, key: ItemKey, cx: &mut Context<Self>) {
        self.selected = Some(key);
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

    fn clear_filter(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.query
            .update(cx, |input, cx| input.set_value("", window, cx));
        self.only_unhealthy = false;
        self.sync(cx);
        cx.notify();
    }

    fn details(&self, cx: &mut Context<Self>) -> Div {
        let p = palette(cx);
        let hint = |text: &'static str| {
            panel(cx)
                .p_4()
                .text_color(p.muted)
                .text_size(dp(12.5))
                .child(text)
        };
        let (Some(key), Some(data)) = (self.selected.as_ref(), self.loader.data()) else {
            return hint("Select a namespace, workload or pod to see its details.");
        };
        let namespaces = &data.snapshot.namespaces;
        let gone = || hint("The selected item is no longer reported by the cluster.");
        let title = |name: String, health: HealthState, tone: Tone, cx: &App| {
            h_flex()
                .id("workload-detail-title")
                .test_support()
                .aria_label(format!("{name} · {}", health_label(health)))
                .gap_2()
                .flex_wrap()
                .child(
                    div()
                        .font_family(MONO_FONT)
                        .text_size(dp(14.))
                        .font_weight(FontWeight::SEMIBOLD)
                        .truncate()
                        .child(name),
                )
                .child(ui::tag(tone, None, health_label(health), cx))
        };
        match key {
            ItemKey::Namespace(name) => {
                let Some(namespace) = namespaces.iter().find(|ns| &ns.name == name) else {
                    return gone();
                };
                let failing = namespace
                    .problem_pods
                    .iter()
                    .filter(|pod| pod.issue.severity() == HealthState::Failing)
                    .count();
                panel(cx)
                    .p_4()
                    .gap_2p5()
                    .child(title(
                        namespace.name.clone(),
                        namespace.health,
                        health_tone(namespace.health),
                        cx,
                    ))
                    .child(field("Kind", mono("Namespace"), cx))
                    .child(field(
                        "Workloads",
                        mono(format!(
                            "{} of {} healthy",
                            namespace.healthy_workloads, namespace.total_workloads
                        )),
                        cx,
                    ))
                    .child(field(
                        "Pods needing attention",
                        mono(format!(
                            "{} ({failing} failing)",
                            namespace.problem_pods.len()
                        )),
                        cx,
                    ))
                    .child(field(
                        "Issues",
                        mono(match issues_in(namespace) {
                            0 => "none".to_owned(),
                            count => plural(count, "issue", "issues"),
                        }),
                        cx,
                    ))
            }
            ItemKey::Workload {
                namespace,
                name,
                kind,
            } => {
                let Some(workload) =
                    namespaces
                        .iter()
                        .find(|ns| &ns.name == namespace)
                        .and_then(|ns| {
                            ns.workloads
                                .iter()
                                .find(|workload| &workload.name == name && workload.kind == *kind)
                        })
                else {
                    return gone();
                };
                panel(cx)
                    .p_4()
                    .gap_2p5()
                    .child(title(
                        workload.name.clone(),
                        workload.health,
                        health_tone(workload.health),
                        cx,
                    ))
                    .child(field("Namespace", mono(workload.namespace.clone()), cx))
                    .child(field("Kind", mono(workload.kind.label()), cx))
                    .child(field(
                        "Ready / desired",
                        mono(format!("{} / {}", workload.ready, workload.desired)),
                        cx,
                    ))
                    .child(field(
                        "Issues",
                        v_flex()
                            .id("workload-issues")
                            .test_support()
                            .aria_label(if workload.issues.is_empty() {
                                "none reported".to_owned()
                            } else {
                                workload.issues.join("; ")
                            })
                            .children(if workload.issues.is_empty() {
                                vec![div().text_color(p.muted).child("none reported")]
                            } else {
                                workload
                                    .issues
                                    .iter()
                                    .map(|issue| div().child(issue.clone()))
                                    .collect()
                            }),
                        cx,
                    ))
            }
            ItemKey::Pod { namespace, name } => {
                let Some(pod) = namespaces
                    .iter()
                    .find(|ns| &ns.name == namespace)
                    .and_then(|ns| ns.problem_pods.iter().find(|pod| &pod.name == name))
                else {
                    return gone();
                };
                let known_node = pod.node.as_ref().filter(|node| {
                    self.source
                        .as_ref()
                        .is_some_and(|source| source.nodes.iter().any(|n| &n.name == *node))
                });
                panel(cx)
                    .p_4()
                    .gap_2p5()
                    .child(title(
                        pod.name.clone(),
                        pod.issue.severity(),
                        pod_tone(&pod.issue),
                        cx,
                    ))
                    .child(field("Namespace", mono(pod.namespace.clone()), cx))
                    .child(field("Kind", mono("Pod"), cx))
                    .child(field(
                        "Node",
                        h_flex()
                            .gap_2()
                            .child(match &pod.node {
                                Some(node) => mono(node.clone()),
                                None => div().text_color(p.muted).child("not scheduled"),
                            })
                            .when_some(known_node.cloned(), |this, node| {
                                this.child(
                                    Button::new("select-node")
                                        .link()
                                        .small()
                                        .label("Inspect node")
                                        .on_click(cx.listener(move |_, _, _, cx| {
                                            cx.emit(ScreenEvent::SelectNode(node.clone()))
                                        })),
                                )
                            }),
                        cx,
                    ))
                    .child(field("Phase", mono(pod.phase.clone()), cx))
                    .child(field("Issue", mono(issue_detail(&pod.issue)), cx))
                    .child(field("Restarts", mono(pod.restarts.to_string()), cx))
                    .child(field(
                        "Created",
                        mono(match pod.created_at {
                            Some(created) => format!(
                                "{} ({} ago)",
                                created.format("%Y-%m-%d %H:%M UTC"),
                                age(Some(created))
                            ),
                            None => "unknown".to_owned(),
                        }),
                        cx,
                    ))
            }
        }
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

impl WorkloadsScreen {
    fn render_header(&self, window: &mut Window, cx: &mut Context<Self>) -> Div {
        let header = PageHeader::new(PREFIX, "Workloads");
        let header = if self.loader.data().is_some() {
            let filter = div().child(
                Input::new(&self.query)
                    .id("workload-filter")
                    .aria_label("Filter workloads by namespace, name, kind, node or issue")
                    .small()
                    .h(dp(ui::CONTROL_HEIGHT))
                    .cleanable(true)
                    .prefix(Icon::new(IconName::Search).size(dp(14.))),
            );
            header
                .filter(filter)
                .foldable(self.render_unhealthy(cx), self.unhealthy_fold(cx))
        } else {
            header
        };
        let refresh = refresh_control(
            header.id("refresh"),
            "Refresh workloads",
            self.source.as_ref(),
            &self.loader,
            cx,
        );
        header.control(refresh).render(window, cx)
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

    fn render_unhealthy(&self, cx: &mut Context<Self>) -> Button {
        Button::new("only-unhealthy")
            .outline()
            .small()
            .h(dp(ui::CONTROL_HEIGHT))
            .icon(IconName::ListFilter)
            .label("Only unhealthy")
            .selected(self.only_unhealthy)
            .on_click(
                cx.listener(|view, _, _, cx| view.set_only_unhealthy(!view.only_unhealthy, cx)),
            )
    }

    /// Only unhealthy folded: a checked item.
    fn unhealthy_fold(&self, cx: &mut Context<Self>) -> page::Fold {
        let on = self.only_unhealthy;
        page::Fold::from(page::checked_item(
            "Only unhealthy",
            on,
            page::handler(cx, |view: &mut Self, _, cx| {
                view.set_only_unhealthy(!view.only_unhealthy, cx)
            }),
        ))
        .changed(on.then(|| "Only unhealthy".into()))
    }

    /// What shows in the table's place: no Kubernetes API, or `gate()`'s
    /// states.
    fn render_state(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        // No data and the load failed: the Kubernetes API isn't reachable.
        // Nothing is known, so nothing is shown as failed.
        if let (Some(_), None, false, Some(error)) = (
            self.source.as_ref(),
            self.loader.data(),
            self.loader.is_loading(),
            self.loader.error(),
        ) {
            let retry = retry_button("screen-retry", cx);
            return Some(
                ui::empty_state(
                    IconName::Unplug,
                    "Kubernetes API unavailable",
                    "Workload health comes from the Kubernetes API, which couldn't be reached. Nothing is known yet, so nothing is shown as failed. Check the kubeconfig in Settings and that the API server is up, then retry.",
                    Some(error.to_owned()),
                    vec![retry],
                    cx,
                )
                .id("k8s-unavailable")
                .test_support()
                .role(Role::Status)
                .aria_label("Kubernetes API unavailable")
                .into_any_element(),
            );
        }
        gate(
            self.source.as_ref(),
            &self.loader,
            Scope::Cluster,
            "workloads",
            cx,
        )
        .map(IntoElement::into_any_element)
    }

    /// The table edge to edge, with the selection's details beside it on a
    /// wide page and below it on a narrow one.
    fn render_split(&self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let beside = crate::screens::beside(window);
        let table = div()
            .id("workloads-table")
            .flex()
            .flex_col()
            .flex_1()
            .min_h_0()
            .child(DataTable::new().render(self, window, cx).flex_1().min_h_0())
            .into_any_element();
        let details = div()
            .id("workload-details")
            .test_support()
            .size_full()
            .overflow_y_scroll()
            .restrict_scroll_to_axis()
            .when_else(
                beside,
                |this| this.pr(dp(page::PANE_PADDING)).py(dp(page::PANE_PADDING_Y)),
                |this| this.px(dp(page::PANE_PADDING)).pb(dp(page::PANE_PADDING_Y)),
            )
            .child(self.details(cx))
            .into_any_element();
        crate::screens::split_fill("workloads-split", beside, DETAILS_HEIGHT, table, details)
    }
}

/// Builds a controller the way the collector classifies it.
fn workload(
    namespace: &str,
    name: &str,
    kind: WorkloadKind,
    ready: i32,
    desired: i32,
) -> WorkloadInfo {
    let (health, issues) = if desired == 0 || ready == desired {
        (HealthState::Healthy, Vec::new())
    } else if ready == 0 {
        (
            HealthState::Failing,
            vec![format!("No pods ready (0/{desired})")],
        )
    } else {
        (
            HealthState::Degraded,
            vec![format!("Partial: {ready}/{desired} ready")],
        )
    };
    WorkloadInfo {
        name: name.into(),
        namespace: namespace.into(),
        kind,
        ready,
        desired,
        health,
        issues,
    }
}

fn pod(
    namespace: &str,
    name: &str,
    node: Option<&str>,
    phase: &str,
    restarts: i32,
    issue: PodIssue,
    age_minutes: i64,
) -> PodInfo {
    PodInfo {
        name: name.into(),
        namespace: namespace.into(),
        node: node.map(str::to_owned),
        phase: phase.into(),
        restarts,
        issue,
        created_at: Some(Utc::now() - chrono::Duration::minutes(age_minutes)),
    }
}

fn namespace(
    name: &str,
    workloads: Vec<WorkloadInfo>,
    problem_pods: Vec<PodInfo>,
) -> NamespaceSummary {
    let worst_workload = workloads
        .iter()
        .map(|workload| workload.health)
        .min()
        .unwrap_or(HealthState::Healthy);
    let worst_pod = problem_pods
        .iter()
        .map(|pod| pod.issue.severity())
        .min()
        .unwrap_or(HealthState::Healthy);
    NamespaceSummary {
        name: name.into(),
        health: worst_workload.min(worst_pod),
        total_workloads: workloads.len(),
        healthy_workloads: workloads
            .iter()
            .filter(|workload| workload.health == HealthState::Healthy)
            .count(),
        workloads,
        problem_pods,
    }
}

/// Example workloads for `--fixture`: the usual system components plus a few
/// applications. The degraded worker has crashing and restarting pods; the
/// silent worker simply isn't counted as ready. Nothing here depends on which
/// node is the target.
fn example(source: &ScreenSource) -> WorkloadData {
    use WorkloadKind::{DaemonSet, Deployment, StatefulSet};
    let nodes = &source.nodes;
    let total = nodes.len() as i32;
    let degraded = nodes
        .iter()
        .find(|node| node.name.contains("wk-fra1-02"))
        .map(|node| node.name.as_str());
    let silent = nodes
        .iter()
        .find(|node| node.name.contains("wk-fra1-03"))
        .map(|node| node.name.as_str());
    let node_ready = total - i32::from(degraded.is_some()) - i32::from(silent.is_some());
    // Applications land on the degraded worker when there is one.
    let app_node = degraded.or_else(|| {
        nodes
            .iter()
            .find(|node| node.role != crate::presentation::Role::ControlPlane)
            .map(|node| node.name.as_str())
    });

    let kube_system = namespace(
        "kube-system",
        vec![
            workload("kube-system", "coredns", Deployment, 2, 2),
            workload("kube-system", "metrics-server", Deployment, 1, 1),
            workload("kube-system", "kube-flannel", DaemonSet, node_ready, total),
            workload("kube-system", "kube-proxy", DaemonSet, node_ready, total),
        ],
        degraded
            .map(|node| {
                vec![
                    pod(
                        "kube-system",
                        "kube-flannel-q7x4d",
                        Some(node),
                        "Running",
                        14,
                        PodIssue::CrashLoopBackOff,
                        60 * 24 * 3,
                    ),
                    pod(
                        "kube-system",
                        "kube-proxy-9tn2m",
                        Some(node),
                        "Running",
                        6,
                        PodIssue::HighRestarts(6),
                        60 * 24 * 3,
                    ),
                ]
            })
            .unwrap_or_default(),
    );
    let shop = namespace(
        "shop",
        vec![
            workload("shop", "web", Deployment, 3, 3),
            workload("shop", "api", Deployment, 2, 3),
            workload("shop", "worker", Deployment, 0, 2),
            workload("shop", "postgres", StatefulSet, 1, 1),
        ],
        vec![
            pod(
                "shop",
                "api-6d8f7c9b5-k2w9z",
                app_node,
                "Running",
                7,
                PodIssue::OOMKilled,
                95,
            ),
            pod(
                "shop",
                "worker-5c7b8d6f4-hq8r2",
                app_node,
                "Pending",
                0,
                PodIssue::ImagePullBackOff,
                12,
            ),
            pod(
                "shop",
                "worker-5c7b8d6f4-zl4vn",
                app_node,
                "Pending",
                0,
                PodIssue::ErrImagePull,
                12,
            ),
        ],
    );
    let monitoring = namespace(
        "monitoring",
        vec![
            workload("monitoring", "grafana", Deployment, 1, 1),
            workload("monitoring", "alertmanager", Deployment, 1, 1),
            workload("monitoring", "prometheus", StatefulSet, 1, 1),
            workload("monitoring", "node-exporter", DaemonSet, node_ready, total),
        ],
        Vec::new(),
    );
    let cert_manager = namespace(
        "cert-manager",
        vec![
            workload("cert-manager", "cert-manager", Deployment, 1, 1),
            workload("cert-manager", "cert-manager-cainjector", Deployment, 1, 1),
            workload("cert-manager", "cert-manager-webhook", Deployment, 1, 1),
        ],
        Vec::new(),
    );
    let mut namespaces = vec![kube_system, shop, monitoring, cert_manager];
    namespaces.sort_by(|left, right| {
        left.health
            .cmp(&right.health)
            .then_with(|| left.name.cmp(&right.name))
    });
    let count_kind = |kind: WorkloadKind| {
        namespaces
            .iter()
            .flat_map(|ns| &ns.workloads)
            .filter(|workload| workload.kind == kind)
            .count()
    };
    let healthy_pods: i32 = namespaces
        .iter()
        .flat_map(|ns| &ns.workloads)
        .map(|workload| workload.ready)
        .sum();
    let pods_with = |severity: fn(HealthState) -> bool| {
        namespaces
            .iter()
            .flat_map(|ns| &ns.problem_pods)
            .filter(|pod| severity(pod.issue.severity()))
            .count()
    };
    let snapshot = WorkloadSnapshot {
        target: source.target.context.clone(),
        total_deployments: count_kind(Deployment),
        total_statefulsets: count_kind(StatefulSet),
        total_daemonsets: count_kind(DaemonSet),
        total_pods_healthy: healthy_pods.max(0) as usize,
        total_pods_degraded: pods_with(|health| {
            matches!(health, HealthState::Degraded | HealthState::Pending)
        }),
        total_pods_failing: pods_with(|health| health == HealthState::Failing),
        namespaces,
    };
    WorkloadData {
        snapshot,
        unavailable: Vec::new(),
        missing_notice: Vec::new(),
    }
}

mod source;
#[cfg(test)]
mod tests;

impl Render for WorkloadsScreen {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        crate::desktop::probe::hit("workloads");
        self.sync(cx);
        let header = self.render_header(window, cx);
        // The table runs edge to edge under the toolbar; the banners and a
        // state in the table's place sit in an inset between them. A short
        // page scrolls its frame, so the list keeps some rows.
        let page = page::page("workloads-page")
            .overflow_y_scroll()
            .restrict_scroll_to_axis()
            .child(page::toolbar(cx).child(header));
        let page = match (self.render_state(cx), self.loader.data()) {
            (Some(state), _) => page.child(
                page::inset()
                    .id("workloads-state")
                    .test_support()
                    .child(state),
            ),
            (None, Some(data)) => {
                let banners: Vec<AnyElement> = failure_banner(&self.loader, cx)
                    .map(IntoElement::into_any_element)
                    .into_iter()
                    .chain(partial_notice(data.missing_notice.clone(), cx))
                    .collect();
                page.when(!banners.is_empty(), |page| {
                    page.child(
                        page::inset()
                            .flex()
                            .flex_col()
                            .gap(dp(page::PANE_PADDING_Y))
                            .children(banners),
                    )
                })
                .child(self.render_split(window, cx))
            }
            (None, None) => page,
        };
        // The keys live on a wrapper drawn in every state, so `/` and Escape
        // still work while the filters hide every row.
        div()
            .id("health-body")
            .test_support()
            .key_context(CONTEXT)
            .track_focus(&self.focus)
            .flex()
            .flex_col()
            .size_full()
            .min_h_0()
            .on_action(cx.listener(|view, _: &NextItem, _, cx| view.step(1, cx)))
            .on_action(cx.listener(|view, _: &PreviousItem, _, cx| view.step(-1, cx)))
            .on_action(cx.listener(|view, _: &FirstItem, _, cx| view.step(isize::MIN, cx)))
            .on_action(cx.listener(|view, _: &LastItem, _, cx| view.step(isize::MAX, cx)))
            .on_action(cx.listener(|view, _: &NextPage, _, cx| view.step(PAGE_ROWS, cx)))
            .on_action(cx.listener(|view, _: &PreviousPage, _, cx| view.step(-PAGE_ROWS, cx)))
            .on_action(cx.listener(|view, _: &ToggleExpanded, _, cx| view.toggle_expanded(cx)))
            .on_action(cx.listener(|view, _: &ToggleUnhealthy, _, cx| {
                view.set_only_unhealthy(!view.only_unhealthy, cx)
            }))
            .on_action(cx.listener(|view, _: &FocusFilter, window, cx| {
                let focus = view.query.read(cx).focus_handle(cx);
                window.focus(&focus, cx);
            }))
            .on_action(
                cx.listener(|view, _: &ClearFilter, window, cx| view.clear_filter(window, cx)),
            )
            .child(page)
    }
}
