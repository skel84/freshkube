//! Workloads: Kubernetes workload health for the whole cluster, like the
//! TUI's workload view. Namespaces come first with problems on top; each one
//! expands to its deployments, statefulsets, daemonsets and the pods that need
//! attention. Read-only: nothing here changes the cluster.
use std::cell::RefCell;
use std::collections::HashSet;
use std::rc::Rc;
use std::sync::Arc;

use chrono::{DateTime, Utc};
use freshkube_core::constants::HIGH_RESTART_THRESHOLD;
use freshkube_core::workloads::{
    HealthState, NamespaceSummary, PodInfo, PodIssue, WorkloadCollectionOutcome, WorkloadInfo,
    WorkloadKind, WorkloadSnapshot, WorkloadSource, WorkloadSourceError, collect_workloads,
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
    Column, Loader, Scope, ScreenEvent, ScreenPanel, ScreenSource, cell, content_width,
    failure_banner, field, gated_page, header, mono, page_body, page_scroll, panel, partial_notice,
    retry_button, table_width,
};
use crate::palette::palette;
use crate::ui::{self, MONO_FONT, Tone, dp};

const CONTEXT: &str = "TalosWorkloads";
const ROW_HEIGHT: f32 = 28.;
const PAGE_ROWS: isize = 20;
/// The details pane sits beside the list only when the list still has this
/// much for names: enough to tell pods of one workload apart.
const NAME_BESIDE_DETAILS: f32 = 320.;
const DETAILS_WIDTH: f32 = 340.;
const GAP: f32 = 14.;
/// Below this content width the issue column is left to the details pane.
const ISSUE_COLUMN: f32 = 700.;
const LIST_MIN_HEIGHT: f32 = 200.;
const DETAILS_HEIGHT: f32 = 240.;

const STATUS: Column = Column {
    label: "Status",
    width: Some(100.),
};
const NAME: Column = Column {
    label: "Name",
    width: None,
};
const KIND: Column = Column {
    label: "Kind",
    width: Some(108.),
};
const READY: Column = Column {
    label: "Ready / restarts",
    width: Some(120.),
};
const ISSUE: Column = Column {
    label: "Issue",
    width: Some(200.),
};

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
}

impl WorkloadData {
    fn missing(&self, source: WorkloadSource) -> bool {
        self.unavailable.iter().any(|error| error.source == source)
    }
}

/// Identifies a row across refreshes and filter changes.
#[derive(Clone, Debug, PartialEq, Eq)]
enum ItemKey {
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
enum RowRef {
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
struct RowView {
    health: HealthState,
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

fn health_tone(health: HealthState) -> (Tone, IconName) {
    match health {
        HealthState::Failing => (Tone::Crit, IconName::CircleX),
        HealthState::Degraded => (Tone::Warn, IconName::CircleAlert),
        HealthState::Pending => (Tone::Unknown, IconName::Hourglass),
        HealthState::Healthy => (Tone::Good, IconName::CircleCheck),
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
struct CachedRows {
    data: Arc<WorkloadData>,
    settings: RowSettings,
    rows: Rc<Vec<RowRef>>,
}

#[derive(Clone, Debug, PartialEq)]
struct RowSettings {
    query: String,
    only_unhealthy: bool,
    collapsed: HashSet<String>,
}

pub(crate) struct WorkloadsScreen {
    runtime: Handle,
    source: Option<ScreenSource>,
    loader: Loader<Arc<WorkloadData>>,
    selected: Option<ItemKey>,
    collapsed: HashSet<String>,
    only_unhealthy: bool,
    query: Entity<InputState>,
    focus: FocusHandle,
    scroll: UniformListScrollHandle,
    rows: RefCell<Option<CachedRows>>,
    _subscription: Subscription,
    /// Caret and selection changes redraw the filter; this view is cached, so
    /// it has to hear about them.
    _query_observer: Subscription,
}

impl EventEmitter<ScreenEvent> for WorkloadsScreen {}

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
        let query = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("Filter by namespace, name, kind, node or issue")
        });
        let subscription = cx.subscribe_in(&query, window, |this, _, event, window, cx| {
            match event {
                InputEvent::Change => {
                    this.scroll.scroll_to_item(0, ScrollStrategy::Top);
                    cx.notify();
                }
                // Enter hands the keyboard back to the list.
                InputEvent::PressEnter { .. } => window.focus(&this.focus, cx),
                _ => {}
            }
        });
        Self {
            runtime,
            source: None,
            loader: Loader::default(),
            selected: None,
            collapsed: HashSet::new(),
            only_unhealthy: false,
            _query_observer: cx.observe(&query, |_, _, cx| cx.notify()),
            query,
            focus: cx.focus_handle(),
            scroll: UniformListScrollHandle::new(),
            rows: RefCell::new(None),
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

    fn refresh(&mut self, _: &mut Window, cx: &mut Context<Self>) {
        let Some(source) = self.source.clone() else {
            return;
        };
        if self.loader.is_loading() {
            return;
        }
        let Some(live) = source.live.clone() else {
            self.loader
                .resolve(source.target.clone(), Ok(Arc::new(example(&source))));
            cx.notify();
            return;
        };
        let context = source.target.context.clone();
        self.loader.load(
            source.target.clone(),
            &self.runtime,
            "workloads",
            async move {
                let client = live.kubernetes().await?;
                match collect_workloads(context, client).await {
                    WorkloadCollectionOutcome::Complete(snapshot) => Ok(Arc::new(WorkloadData {
                        snapshot,
                        unavailable: Vec::new(),
                    })),
                    WorkloadCollectionOutcome::Partial {
                        snapshot,
                        unavailable,
                    } => Ok(Arc::new(WorkloadData {
                        snapshot,
                        unavailable,
                    })),
                    WorkloadCollectionOutcome::Unavailable { errors, .. } => {
                        // The next refresh revalidates instead of reusing this client.
                        live.forget_kubernetes();
                        Err(errors
                            .iter()
                            .map(ToString::to_string)
                            .collect::<Vec<_>>()
                            .join("; "))
                    }
                }
            },
            |screen: &mut Self| &mut screen.loader,
            cx,
        );
        cx.notify();
    }
}

impl WorkloadsScreen {
    fn filter_text(&self, cx: &App) -> String {
        self.query.read(cx).value().trim().to_lowercase()
    }

    /// Visible rows: namespaces (problems first, as collected) followed by
    /// their workloads and problem pods unless collapsed.
    fn rows(&self, cx: &App) -> Rc<Vec<RowRef>> {
        let Some(data) = self.loader.data() else {
            return Rc::default();
        };
        let settings = RowSettings {
            query: self.filter_text(cx),
            only_unhealthy: self.only_unhealthy,
            collapsed: self.collapsed.clone(),
        };
        let mut cache = self.rows.borrow_mut();
        if let Some(cached) = cache
            .as_ref()
            .filter(|cached| Arc::ptr_eq(&cached.data, data) && cached.settings == settings)
        {
            return cached.rows.clone();
        }
        crate::desktop::probe::hit("workloads.rows");
        let rows = Rc::new(self.compute_rows(data, &settings.query));
        *cache = Some(CachedRows {
            data: data.clone(),
            settings,
            rows: rows.clone(),
        });
        rows
    }

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
        let rows = self.rows(cx);
        let Some(data) = self.loader.data() else {
            return;
        };
        if rows.is_empty() {
            return;
        }
        let current = rows
            .iter()
            .position(|row| Some(row.key(&data.snapshot)) == self.selected);
        let next = match current {
            Some(ix) => ix.saturating_add_signed(delta).min(rows.len() - 1),
            None if delta < 0 => rows.len() - 1,
            None => 0,
        };
        self.selected = Some(rows[next].key(&data.snapshot));
        self.scroll.scroll_to_item(next, ScrollStrategy::Nearest);
        cx.notify();
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
        cx.notify();
    }

    fn set_only_unhealthy(&mut self, on: bool, cx: &mut Context<Self>) {
        self.only_unhealthy = on;
        self.scroll.scroll_to_item(0, ScrollStrategy::Top);
        cx.notify();
    }

    fn clear_filter(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.query
            .update(cx, |input, cx| input.set_value("", window, cx));
        self.only_unhealthy = false;
        cx.notify();
    }

    fn summary(&self, data: &WorkloadData, cx: &App) -> impl IntoElement + use<> {
        let p = palette(cx);
        let snapshot = &data.snapshot;
        // A list that didn't answer is unknown, not zero.
        let count = |value: usize, source: WorkloadSource| {
            if data.missing(source) {
                "unknown".to_owned()
            } else {
                value.to_string()
            }
        };
        let item = |label: &'static str, value: String| {
            h_flex()
                .gap_1p5()
                .child(div().text_color(p.muted).child(label))
                .child(mono(value))
        };
        let pods = |health: HealthState, value: usize| {
            let (tone, icon) = health_tone(health);
            ui::tag(
                tone,
                Some(icon),
                format!(
                    "{} {}",
                    count(value, WorkloadSource::Pods),
                    health_label(health).to_lowercase()
                ),
                cx,
            )
        };
        let summary = format!(
            "{} deployments, {} statefulsets, {} daemonsets, {} healthy pods, {} degraded, {} failing",
            count(snapshot.total_deployments, WorkloadSource::Deployments),
            count(snapshot.total_statefulsets, WorkloadSource::StatefulSets),
            count(snapshot.total_daemonsets, WorkloadSource::DaemonSets),
            count(snapshot.total_pods_healthy, WorkloadSource::Pods),
            count(snapshot.total_pods_degraded, WorkloadSource::Pods),
            count(snapshot.total_pods_failing, WorkloadSource::Pods),
        );
        h_flex()
            .id("workload-summary")
            .test_support()
            .role(Role::Status)
            .aria_label(summary)
            .gap_x_5()
            .gap_y_1p5()
            .flex_wrap()
            .text_size(dp(12.5))
            .child(item(
                "Deployments",
                count(snapshot.total_deployments, WorkloadSource::Deployments),
            ))
            .child(item(
                "StatefulSets",
                count(snapshot.total_statefulsets, WorkloadSource::StatefulSets),
            ))
            .child(item(
                "DaemonSets",
                count(snapshot.total_daemonsets, WorkloadSource::DaemonSets),
            ))
            .child(
                h_flex()
                    .gap_1p5()
                    .child(div().text_color(p.muted).child("Pods"))
                    .child(pods(HealthState::Healthy, snapshot.total_pods_healthy))
                    .child(pods(HealthState::Degraded, snapshot.total_pods_degraded))
                    .child(pods(HealthState::Failing, snapshot.total_pods_failing)),
            )
    }

    fn toolbar(&self, cx: &mut Context<Self>) -> Div {
        h_flex()
            .gap_2p5()
            .flex_wrap()
            .child(
                div().flex_1().min_w(dp(180.)).max_w(dp(320.)).child(
                    Input::new(&self.query)
                        .id("workload-filter")
                        .aria_label("Filter workloads by namespace, name, kind, node or issue")
                        .small()
                        .cleanable(true)
                        .prefix(Icon::new(IconName::Search).size(dp(14.))),
                ),
            )
            .child(
                Button::new("only-unhealthy")
                    .outline()
                    .small()
                    .icon(IconName::ListFilter)
                    .label("Only unhealthy")
                    .selected(self.only_unhealthy)
                    .on_click(cx.listener(|view, _, _, cx| {
                        view.set_only_unhealthy(!view.only_unhealthy, cx)
                    })),
            )
    }

    fn render_row(
        &self,
        ix: usize,
        row: RowRef,
        data: &WorkloadData,
        show_issue: bool,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        let p = palette(cx);
        let key = row.key(&data.snapshot);
        let selected = self.selected.as_ref() == Some(&key);
        let view = self.describe(row, data);
        let is_namespace = matches!(row, RowRef::Namespace(_));
        let (tone, icon) = health_tone(view.health);
        let label = health_label(view.health);
        let aria = format!(
            "{} {} · {label} · {} · {}",
            view.kind, view.name, view.ready, view.issue
        );
        h_flex()
            .id(("workload-row", ix))
            .test_support()
            .role(Role::ListBoxOption)
            .aria_selected(selected)
            .aria_label(aria)
            .w_full()
            .h(dp(ROW_HEIGHT))
            .font_family(MONO_FONT)
            .text_size(dp(12.))
            .cursor_pointer()
            .when(selected, |this| this.bg(p.accent_soft).text_color(p.accent))
            .when(!selected, |this| this.hover(|style| style.bg(p.hover)))
            .child(
                cell(STATUS)
                    .flex()
                    .items_center()
                    .child(ui::tag(tone, Some(icon), label, cx)),
            )
            .child(
                cell(NAME)
                    .flex()
                    .items_center()
                    .gap_1p5()
                    .when(view.nested, |this| this.pl(dp(28.)))
                    .when(is_namespace, |this| this.font_weight(FontWeight::SEMIBOLD))
                    .children(view.chevron.map(|chevron| Icon::new(chevron).size(dp(13.))))
                    // Without `min_w_0` a long pod name widens the column
                    // and shifts every later cell in its row.
                    .child(div().flex_1().min_w_0().truncate().child(view.name)),
            )
            .child(cell(KIND).text_color(p.muted).child(view.kind))
            .child(cell(READY).child(view.ready))
            .when(show_issue, |this| {
                this.child(
                    cell(ISSUE)
                        .when(!selected, |this| this.text_color(p.muted))
                        .child(view.issue),
                )
            })
            .on_click(cx.listener(move |view, _, window, cx| {
                let was_selected = view.selected.as_ref() == Some(&key);
                view.select(key.clone(), cx);
                if is_namespace && was_selected {
                    view.toggle_expanded(cx);
                }
                window.focus(&view.focus, cx);
            }))
    }

    /// Width the list needs, with room for whole pod names, before the
    /// details pane may sit beside it.
    fn width_beside_details(show_issue: bool) -> f32 {
        let name = Column {
            width: Some(NAME_BESIDE_DETAILS),
            ..NAME
        };
        let mut columns = vec![STATUS, name, KIND, READY];
        if show_issue {
            columns.push(ISSUE);
        }
        table_width(&columns)
    }

    fn head(&self, show_issue: bool, cx: &App) -> Div {
        let p = palette(cx);
        let mut head = h_flex().py(dp(7.)).border_b_1().border_color(p.line);
        let mut columns = vec![STATUS, NAME, KIND, READY];
        if show_issue {
            columns.push(ISSUE);
        }
        for column in columns {
            head = head.child(cell(column).child(ui::caption(column.label, cx)));
        }
        head
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
        let title = |name: String, health: HealthState, cx: &App| {
            let (tone, icon) = health_tone(health);
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
                .child(ui::tag(tone, Some(icon), health_label(health), cx))
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
                    .child(title(namespace.name.clone(), namespace.health, cx))
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
                    .child(title(workload.name.clone(), workload.health, cx))
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
                    .child(title(pod.name.clone(), pod.issue.severity(), cx))
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

impl Render for WorkloadsScreen {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        crate::desktop::probe::hit("workloads");
        // No data and the load failed: the Kubernetes API isn't reachable.
        // Nothing is known, so nothing is shown as failed.
        if let (Some(source), None, false, Some(error)) = (
            self.source.as_ref(),
            self.loader.data(),
            self.loader.is_loading(),
            self.loader.error(),
        ) {
            let retry = retry_button("screen-retry", cx);
            let error = error.to_owned();
            let top = header("Workloads", source, Scope::Cluster, &self.loader, cx);
            return page_scroll("workloads-page")
                .child(
                    page_body().child(top).child(
                        ui::empty_state(
                            IconName::Unplug,
                            "Kubernetes API unavailable",
                            "Workload health comes from the Kubernetes API, which couldn't be reached. Nothing is known yet, so nothing is shown as failed. Check the kubeconfig in Settings and that the API server is up, then retry.",
                            Some(error),
                            vec![retry],
                            cx,
                        )
                        .id("k8s-unavailable")
                        .test_support()
                        .role(Role::Status)
                        .aria_label("Kubernetes API unavailable"),
                    ),
                )
                .into_any_element();
        }
        if let Some(page) = gated_page(
            "workloads-page",
            "Workloads",
            Scope::Cluster,
            self.source.as_ref(),
            &self.loader,
            "workloads",
            cx,
        ) {
            return page;
        }
        let (Some(source), Some(data)) = (self.source.clone(), self.loader.data()) else {
            return div().into_any_element();
        };
        let p = palette(cx);
        let rows = self.rows(cx);
        let row_count = rows.len();
        let width = content_width(window);
        let show_issue = width >= ISSUE_COLUMN;
        let wide = width >= Self::width_beside_details(show_issue) + DETAILS_WIDTH + GAP;
        let missing: Vec<String> = data
            .unavailable
            .iter()
            .map(|error| format!("{}: {}", error.source.label(), error.message))
            .collect();
        let empty = if data.snapshot.namespaces.is_empty() {
            "No workloads found in this cluster."
        } else {
            "No workloads match these filters."
        };
        let summary = self.summary(data, cx);
        let list = panel(cx)
            .flex_1()
            .min_h(dp(LIST_MIN_HEIGHT))
            .overflow_hidden()
            .child(self.head(show_issue, cx))
            .child(
                div()
                    .id("workload-list")
                    .test_support()
                    .role(Role::ListBox)
                    .aria_label(
                        "Namespaces, workloads and pods needing attention; arrows select, Enter opens or closes a namespace, U shows only unhealthy",
                    )
                    .key_context(CONTEXT)
                    .track_focus(&self.focus)
                    .on_action(cx.listener(|view, _: &NextItem, _, cx| view.step(1, cx)))
                    .on_action(cx.listener(|view, _: &PreviousItem, _, cx| view.step(-1, cx)))
                    .on_action(cx.listener(|view, _: &FirstItem, _, cx| view.step(isize::MIN, cx)))
                    .on_action(cx.listener(|view, _: &LastItem, _, cx| view.step(isize::MAX, cx)))
                    .on_action(cx.listener(|view, _: &NextPage, _, cx| view.step(PAGE_ROWS, cx)))
                    .on_action(
                        cx.listener(|view, _: &PreviousPage, _, cx| view.step(-PAGE_ROWS, cx)),
                    )
                    .on_action(
                        cx.listener(|view, _: &ToggleExpanded, _, cx| view.toggle_expanded(cx)),
                    )
                    .on_action(cx.listener(|view, _: &ToggleUnhealthy, _, cx| {
                        view.set_only_unhealthy(!view.only_unhealthy, cx)
                    }))
                    .on_action(cx.listener(|view, _: &FocusFilter, window, cx| {
                        let focus = view.query.read(cx).focus_handle(cx);
                        window.focus(&focus, cx);
                    }))
                    .on_action(cx.listener(|view, _: &ClearFilter, window, cx| {
                        view.clear_filter(window, cx)
                    }))
                    .flex_1()
                    .min_h_0()
                    .map(|this| {
                        if row_count == 0 {
                            this.child(
                                div()
                                    .px_3()
                                    .py_3p5()
                                    .text_size(dp(12.5))
                                    .text_color(p.muted)
                                    .child(empty),
                            )
                            .into_any_element()
                        } else {
                            this.child(
                                uniform_list(
                                    "workload-rows",
                                    row_count,
                                    cx.processor(move |view, range: std::ops::Range<usize>, _, cx| {
                                        let rows = view.rows(cx);
                                        let Some(data) = view.loader.data() else {
                                            return Vec::new();
                                        };
                                        range
                                            .filter_map(|ix| {
                                                rows.get(ix).map(|row| {
                                                    view.render_row(ix, *row, data, show_issue, cx)
                                                })
                                            })
                                            .collect::<Vec<_>>()
                                    }),
                                )
                                .track_scroll(&self.scroll)
                                .size_full(),
                            )
                            .into_any_element()
                        }
                    }),
            );
        let details = self.details(cx);
        // Short windows scroll the page rather than squeezing the list.
        let split = if wide {
            h_flex()
                .flex_1()
                .min_h(dp(LIST_MIN_HEIGHT))
                .items_stretch()
                .gap(dp(GAP))
                .child(v_flex().flex_1().min_w_0().min_h_0().child(list))
                .child(
                    div()
                        .id("workload-details")
                        .test_support()
                        .w(dp(DETAILS_WIDTH))
                        .flex_none()
                        .overflow_y_scroll()
                        .child(details),
                )
        } else {
            h_flex()
                .flex_1()
                .min_h(dp(LIST_MIN_HEIGHT + GAP + DETAILS_HEIGHT))
                .child(
                    v_flex().size_full().gap(dp(GAP)).child(list).child(
                        div()
                            .id("workload-details")
                            .test_support()
                            .h(dp(DETAILS_HEIGHT))
                            .flex_none()
                            .overflow_y_scroll()
                            .child(details),
                    ),
                )
        };
        v_flex()
            .id("workloads-page")
            .size_full()
            .min_h_0()
            .overflow_y_scroll()
            .px(dp(crate::desktop::PAGE_PADDING))
            .pt(dp(22.))
            .pb(dp(18.))
            .gap(dp(14.))
            .child(header(
                "Workloads",
                &source,
                Scope::Cluster,
                &self.loader,
                cx,
            ))
            .children(failure_banner(&self.loader, cx))
            .children(partial_notice(missing, cx))
            .child(summary)
            .child(self.toolbar(cx))
            .child(split)
            .into_any_element()
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
    }
}

#[cfg(test)]
mod ui_tests {
    use std::sync::Arc;

    use freshkube_core::workloads::{HealthState, WorkloadSource, WorkloadSourceError};
    use gpui_kit::component::Root;
    use gpui_kit::test::TestWindowExt;
    use gpui_kit::{AppContext, Entity, TestAppContext, WindowHandle, px, size};
    use tokio::runtime::{Builder, Runtime};

    // Not `super::*`: gpui_kit's glob would shadow the built-in `#[test]`.
    use super::{ItemKey, RowRef, ScreenPanel, ScreenSource, WorkloadData, WorkloadsScreen};
    use crate::backend::Target;
    use crate::{fixture, presentation};

    fn source(node: &str) -> ScreenSource {
        let nodes = presentation::node_summaries(&fixture::cluster("prod-fra", 1));
        let summary = nodes.iter().find(|summary| summary.name == node).unwrap();
        ScreenSource {
            target: Target {
                epoch: 1,
                context: "prod-fra".into(),
                node: summary.name.clone(),
                address: summary.address.clone(),
            },
            nodes: Arc::new(nodes),
            live: None,
        }
    }

    fn mount(
        cx: &mut TestAppContext,
        node: &str,
    ) -> (Runtime, Entity<WorkloadsScreen>, WindowHandle<Root>) {
        mount_sized(cx, node, 1100.)
    }

    fn mount_sized(
        cx: &mut TestAppContext,
        node: &str,
        width: f32,
    ) -> (Runtime, Entity<WorkloadsScreen>, WindowHandle<Root>) {
        let runtime = Builder::new_multi_thread()
            .worker_threads(1)
            .enable_all()
            .build()
            .unwrap();
        cx.update(|cx| {
            gpui_kit::init(cx);
            crate::theme::install(cx);
        });
        let source = source(node);
        let mut screen = None;
        let handle = cx.open_window(size(px(width), px(760.)), |window, cx| {
            let view = cx.new(|cx| {
                let mut view = WorkloadsScreen::new(runtime.handle().clone(), window, cx);
                view.set_source(Some(source), window, cx);
                view.activate(window, cx);
                view
            });
            screen = Some(view.clone());
            Root::new(view, window, cx)
        });
        cx.run_until_parked();
        (runtime, screen.unwrap(), handle)
    }

    fn row_key(screen: &WorkloadsScreen, ix: usize, cx: &gpui_kit::App) -> ItemKey {
        let rows = screen.rows(cx);
        rows[ix].key(&screen.loader.data().unwrap().snapshot)
    }

    #[gpui_kit::test]
    fn details_sit_beside_the_list_only_when_names_keep_their_room(cx: &mut TestAppContext) {
        for (width, beside) in [(1700., true), (1320., false)] {
            let (_runtime, _screen, handle) = mount_sized(cx, "talos-cp-fra1-01", width);
            cx.update_window(handle.into(), |_, window, cx| {
                window.render_frame(cx);
                let list = window.find("workload-list").bounds();
                let details = window.find("workload-details").bounds();
                if beside {
                    assert!(details.left() >= list.right(), "{width}: {details:?}");
                } else {
                    assert!(details.top() >= list.bottom(), "{width}: {details:?}");
                }
            })
            .unwrap();
        }
    }

    #[gpui_kit::test]
    fn keyboard_selects_rows_and_updates_details(cx: &mut TestAppContext) {
        let (_runtime, screen, handle) = mount(cx, "talos-cp-fra1-01");
        cx.update_window(handle.into(), |_, window, cx| {
            window.render_frame(cx);
            assert!(screen.read(cx).loader.data().is_some());
            window.click(("workload-row", 0usize), cx);
            assert_eq!(
                screen.read(cx).selected,
                Some(row_key(screen.read(cx), 0, cx))
            );
            window.press("down", cx);
            window.render_frame(cx);
            assert_eq!(window.find(("workload-row", 1usize)).selected(), Some(true));
            assert_eq!(
                screen.read(cx).selected,
                Some(row_key(screen.read(cx), 1, cx))
            );
            window.find("workload-detail-title");
            window.press("end", cx);
            window.render_frame(cx);
            let last = screen.read(cx).rows(cx).len() - 1;
            assert_eq!(window.find(("workload-row", last)).selected(), Some(true));
            window.press("home", cx);
            assert_eq!(
                screen.read(cx).selected,
                Some(row_key(screen.read(cx), 0, cx))
            );
        })
        .unwrap();
    }

    #[gpui_kit::test]
    fn unhealthy_filter_narrows_the_list(cx: &mut TestAppContext) {
        let (_runtime, screen, handle) = mount(cx, "talos-cp-fra1-01");
        cx.update_window(handle.into(), |_, window, cx| {
            window.render_frame(cx);
            let all = screen.read(cx).rows(cx);
            assert!(
                all.iter().any(|row| matches!(
                    row.key(&screen.read(cx).loader.data().unwrap().snapshot),
                    ItemKey::Namespace(name) if name == "cert-manager"
                )),
                "the healthy namespace is listed"
            );
            window.click("only-unhealthy", cx);
            window.render_frame(cx);
            let screen_ref = screen.read(cx);
            let narrowed = screen_ref.rows(cx);
            assert!(narrowed.len() < all.len());
            let snapshot = &screen_ref.loader.data().unwrap().snapshot;
            for row in narrowed.iter() {
                match *row {
                    RowRef::Namespace(ns) => {
                        assert_ne!(snapshot.namespaces[ns].health, HealthState::Healthy)
                    }
                    RowRef::Workload(ns, ix) => assert_ne!(
                        snapshot.namespaces[ns].workloads[ix].health,
                        HealthState::Healthy
                    ),
                    RowRef::Pod(..) => {}
                }
            }
        })
        .unwrap();
    }

    #[gpui_kit::test]
    fn enter_collapses_a_namespace(cx: &mut TestAppContext) {
        let (_runtime, screen, handle) = mount(cx, "talos-cp-fra1-01");
        cx.update_window(handle.into(), |_, window, cx| {
            window.render_frame(cx);
            let before = screen.read(cx).rows(cx).len();
            window.click(("workload-row", 0usize), cx);
            window.press("enter", cx);
            window.render_frame(cx);
            assert!(screen.read(cx).rows(cx).len() < before);
            window.press("enter", cx);
            assert_eq!(screen.read(cx).rows(cx).len(), before);
        })
        .unwrap();
    }

    #[gpui_kit::test]
    fn example_reflects_the_degraded_worker(cx: &mut TestAppContext) {
        let (_runtime, screen, handle) = mount(cx, "talos-wk-fra1-03");
        cx.update_window(handle.into(), |_, window, cx| {
            window.render_frame(cx);
            // Cluster-scoped: a silent target node still shows workloads.
            let data = screen.read(cx).loader.data().expect("workloads load");
            let flannel = data
                .snapshot
                .namespaces
                .iter()
                .flat_map(|ns| &ns.problem_pods)
                .find(|pod| pod.name.starts_with("kube-flannel"))
                .expect("flannel pod on the degraded worker");
            assert_eq!(flannel.node.as_deref(), Some("talos-wk-fra1-02"));
            window.find("workload-summary");
            assert!(window.try_find("screen-retry").is_none());
        })
        .unwrap();
    }

    #[gpui_kit::test]
    fn kubernetes_unavailable_offers_retry_not_failure(cx: &mut TestAppContext) {
        let (_runtime, screen, handle) = mount(cx, "talos-cp-fra1-01");
        cx.update_window(handle.into(), |_, window, cx| {
            screen.update(cx, |screen, cx| {
                let target = screen.source.as_ref().unwrap().target.clone();
                screen.loader.reset();
                screen
                    .loader
                    .resolve(target, Err("no kubeconfig selected".into()));
                cx.notify();
            });
            window.render_frame(cx);
            window.find("k8s-unavailable");
            window.find("screen-retry");
            assert!(window.try_find("workload-list").is_none());
        })
        .unwrap();
    }

    #[gpui_kit::test]
    fn partial_results_name_what_is_missing(cx: &mut TestAppContext) {
        let (_runtime, screen, handle) = mount(cx, "talos-cp-fra1-01");
        cx.update_window(handle.into(), |_, window, cx| {
            screen.update(cx, |screen, cx| {
                let target = screen.source.as_ref().unwrap().target.clone();
                let mut data: WorkloadData = (**screen.loader.data().unwrap()).clone();
                data.unavailable.push(WorkloadSourceError {
                    source: WorkloadSource::Pods,
                    message: "forbidden".into(),
                });
                screen.loader.resolve(target, Ok(Arc::new(data)));
                cx.notify();
            });
            window.render_frame(cx);
            window.find("partial-notice");
            window.find("workload-list");
        })
        .unwrap();
    }

    #[gpui_kit::test]
    fn changing_target_drops_old_data(cx: &mut TestAppContext) {
        let (_runtime, screen, handle) = mount(cx, "talos-cp-fra1-01");
        cx.update_window(handle.into(), |_, window, cx| {
            screen.update(cx, |screen, cx| {
                screen.selected = Some(ItemKey::Namespace("shop".into()));
                // Same target, fresh handles: data and selection stay.
                screen.set_source(Some(source("talos-cp-fra1-01")), window, cx);
                assert!(screen.loader.data().is_some());
                assert!(screen.selected.is_some());
                screen.set_source(Some(source("talos-wk-fra1-02")), window, cx);
                assert!(screen.loader.data().is_none());
                assert!(screen.selected.is_none());
            });
        })
        .unwrap();
    }

    #[gpui_kit::test]
    fn rows_are_cached_until_the_data_or_the_filters_change(cx: &mut TestAppContext) {
        let (_runtime, screen, handle) = mount(cx, "talos-cp-fra1-01");
        cx.update_window(handle.into(), |_, window, cx| {
            window.render_frame(cx);
            let computed = || crate::desktop::probe::count("workloads.rows");
            let first = screen.read(cx).rows(cx);
            let base = computed();
            window.render_frame(cx);
            window.press("down", cx);
            let again = screen.read(cx).rows(cx);
            assert!(std::rc::Rc::ptr_eq(&first, &again));
            assert_eq!(computed(), base, "nothing it depends on changed");
            // Unhealthy only.
            window.click("only-unhealthy", cx);
            let narrowed = screen.read(cx).rows(cx);
            assert_eq!(computed(), base + 1);
            assert!(narrowed.len() < first.len());
            window.click("only-unhealthy", cx);
            // Collapsing a namespace.
            let namespace = screen.read(cx).loader.data().unwrap().snapshot.namespaces[0]
                .name
                .clone();
            screen.update(cx, |screen, cx| {
                screen.collapsed.insert(namespace);
                cx.notify();
            });
            let collapsed = screen.read(cx).rows(cx);
            assert!(computed() > base + 1);
            assert!(collapsed.len() < first.len());
            let settled = computed();
            screen.read(cx).rows(cx);
            assert_eq!(computed(), settled);
            // The filter text.
            screen.update(cx, |screen, cx| {
                screen.collapsed.clear();
                screen
                    .query
                    .update(cx, |input, cx| input.set_value("cert", window, cx));
            });
            screen.read(cx).rows(cx);
            assert!(computed() > settled);
            let settled = computed();
            // A new snapshot.
            screen.update(cx, |screen, cx| screen.refresh(window, cx));
            screen.read(cx).rows(cx);
            assert_eq!(computed(), settled + 1);
        })
        .unwrap();
    }
}
