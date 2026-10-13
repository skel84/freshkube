//! Machines from both summaries, with one retained pane for their details.
mod cards;
mod header;
mod join;
mod metrics;
mod projection;
mod resource;
mod table;
#[cfg(test)]
mod tests;
mod view;

use super::system_services::Reading;
use super::{NodeView, Page, Pilot};
use crate::{
    resources::{DetailEvent, DetailPane, Tab, detail::DetailTarget, model::ResourceIdentity},
    ui,
};
use freshkube_core::monitoring::history::Subject;
use gpui_kit::{
    component::input::{InputEvent, InputState},
    *,
};
pub(crate) use join::{NodeKey, NodeRow};
use std::{sync::Arc, time::Duration};

/// The columns the table hides until the user shows them.
const DEFAULT_HIDDEN: [table::Field; 1] = [table::Field::Load];

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum NodeTab {
    #[default]
    Overview,
    Pods,
    Services,
    Processes,
    Storage,
    Network,
    Diagnostics,
    Logs,
    Events,
    Yaml,
}

impl NodeTab {
    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::Overview => "Overview",
            Self::Pods => "Pods",
            Self::Services => "System services",
            Self::Processes => "Processes",
            Self::Storage => "Storage",
            Self::Network => "Network",
            Self::Diagnostics => "Diagnostics",
            Self::Logs => "Logs",
            Self::Events => "Events",
            Self::Yaml => "YAML",
        }
    }
    pub(super) fn id(self) -> &'static str {
        match self {
            Self::Overview => "node-tab-overview",
            Self::Pods => "node-tab-pods",
            Self::Services => "node-tab-services",
            Self::Processes => "node-tab-processes",
            Self::Storage => "node-tab-storage",
            Self::Network => "node-tab-network",
            Self::Diagnostics => "node-tab-diagnostics",
            Self::Logs => "node-tab-logs",
            Self::Events => "node-tab-events",
            Self::Yaml => "node-tab-yaml",
        }
    }
    pub(super) fn screen(self) -> Option<super::pages::PageId> {
        match self {
            Self::Processes => Some(super::pages::PageId::Processes),
            Self::Storage => Some(super::pages::PageId::Storage),
            Self::Network => Some(super::pages::PageId::Network),
            Self::Diagnostics => Some(super::pages::PageId::Diagnostics),
            _ => None,
        }
    }
}

pub(super) struct Nodes {
    pub(super) rows: Arc<Vec<NodeRow>>,
    empty: Option<Empty>,
    lines: Vec<usize>,
    items: Vec<projection::Item>,
    counts: [usize; 4],
    group_counts: [usize; 4],
    filter: Option<projection::Status>,
    healthy_open: bool,
    healthy_folded: bool,
    query: Entity<InputState>,
    query_text: String,
    search_keys: Vec<String>,
    /// The status bar's segment while Nodes shows.
    status: freshkube_ui::status::Segment,
    _query_subscription: Subscription,
    /// The node document's Escape, stepping back from its last level.
    _document_subscription: Subscription,
    table: freshkube_ui::table::TableState,
    /// What the table shows until the summaries answer, and their motion,
    /// drawn over the table.
    loading: freshkube_ui::table::LoadingRows,
    loading_motion: Entity<freshkube_ui::table::LoadingMotion>,
    all_columns: Vec<table::Column>,
    menu_columns: Arc<Vec<(table::Field, SharedString)>>,
    columns: Vec<table::Column>,
    hidden_columns: std::collections::BTreeSet<table::Field>,
    table_width: f32,
    metrics: metrics::Metrics,
    resource_cells: std::collections::HashMap<NodeKey, resource::RowResources>,
    talos_current: bool,
    pub(super) selected: Option<NodeKey>,
    pub(super) open: bool,
    pub(super) expanded: bool,
    pub(super) tab: NodeTab,
    tabs: Vec<NodeTab>,
    inline_tabs: Vec<NodeTab>,
    more: bool,
    pub(super) view: NodeView,
    tab_focus: FocusHandle,
    /// Tracked by the whole pane, never focused itself: whether the
    /// keyboard is already somewhere in it.
    pane_focus: FocusHandle,
    /// The tab strip's scroll; the strip brings the active tab into view.
    pub(super) tab_strip: freshkube_ui::inspector::TabStrip,
    scroll: UniformListScrollHandle,
    /// The frame's scroll, used while the window is short.
    pub(super) page_scroll: ScrollHandle,
    /// The table and the node's inspector; the inspector's width is
    /// remembered in `navigation.json`.
    split: freshkube_ui::inspector::InspectorSplit,
    /// The Logs tab's body, which scrolls inside the inspector while the
    /// window is short: Talos' toolbar can take most of its height.
    pub(super) logs_scroll: ScrollHandle,
    /// The Logs tab's body while it scrolls, as last measured: its room and
    /// the log's toolbar and notices.
    pub(super) logs_height: Option<Pixels>,
    pub(super) document: Entity<DetailPane>,
    /// The table's and the cards' keyboard focus.
    pub(super) focus: FocusHandle,
    /// The pane's Pods tab: the Pods list scoped to the node.
    pub(super) pods: Entity<crate::resources::ResourcesScreen>,
    /// The pane's Overview CPU and memory charts.
    pub(super) history: Entity<crate::monitoring::history::HistoryView>,
    /// Each node's load averages over the last refreshes, for the cards'
    /// sparklines.
    pub(super) load_history: crate::presentation::LoadHistory,
    /// The pane's System services tab.
    pub(super) services: Entity<super::services::NodeServices>,
    /// The pane's Logs tab: the shell's Talos log view.
    logs: Entity<crate::logs::LogPanel>,
    /// Whether the log view collects, for the Logs tab's dot, and whether
    /// the history chart shows, for the Overview tab. Neither is drawn
    /// while another tab shows, so Nodes hears them change itself.
    collecting: bool,
    chart: bool,
    _watches: [Subscription; 2],
    /// The Talos screens the pane's tabs embed, by their pages.
    pub(super) screens:
        std::collections::BTreeMap<super::pages::PageId, crate::screens::ScreenHandle>,
    /// What needs attention, as the shell last derived it, for the pane's
    /// Overview.
    pub(super) attention: Arc<crate::presentation::attention::Attention>,
    /// Why that list may be out of date.
    pub(super) attention_stale: Option<SharedString>,
}

/// What a click or key in Nodes asks of the shell, which owns the node's
/// selection across pages, the Talos reads and navigation.
#[derive(Clone, Debug)]
pub(super) enum NodesEvent {
    /// Open a node's pane, and with `focus` move the keyboard into it.
    Open {
        key: NodeKey,
        focus: bool,
    },
    /// Step the selection through the list.
    Step(isize),
    Close,
    /// Escape: one level back.
    Back,
    StepTab(isize),
    /// Show a tab and move the keyboard into it.
    Tab(NodeTab),
    Refresh,
    /// An attention row's destination.
    Destination(crate::presentation::attention::Destination),
}

impl EventEmitter<NodesEvent> for Nodes {}
impl EventEmitter<crate::screens::ScreenEvent> for Nodes {}

/// Nodes draws the rows and status line the shell hands it, and reads its
/// nodes' use itself while its page shows.
impl crate::screens::ScreenPanel for Nodes {
    /// Nodes reads no Talos target; the shell picks it from Nodes' events.
    fn set_source(
        &mut self,
        _: Option<crate::screens::ScreenSource>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) {
    }

    /// The Kubernetes connection it reads nodes' use from.
    fn set_page_source(
        &mut self,
        source: crate::screens::PageSource,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.set_metrics_source(source.kube, window, cx)
    }

    /// Shown on its page and around a pane screen alike, so the use read
    /// follows the page, not the pane.
    fn set_visible(&mut self, visible: bool, window: &mut Window, cx: &mut Context<Self>) {
        self.set_metrics_visible(visible, window, cx)
    }

    fn activate(&mut self, _: &mut Window, _: &mut Context<Self>) {}

    fn refresh(&mut self, _: &mut Window, _: &mut Context<Self>) {}

    /// The line the shell derives from the Talos overview and the summary
    /// session (`Pilot::rebuild_nodes_meta`).
    fn status(&mut self, _: &App) -> Option<freshkube_ui::status::Segment> {
        Some(self.status.clone())
    }

    /// The table's while it draws loading rows; the open pane's screen's
    /// once the pane covers the table or its tab shows a Talos screen.
    fn loading_motion(&self, cx: &App) -> Option<Entity<freshkube_ui::table::LoadingMotion>> {
        let table = self.view == NodeView::Table
            && !(self.open && self.expanded)
            && freshkube_ui::table::TableSource::loading(self).is_some();
        if table {
            return Some(self.loading_motion.clone());
        }
        self.pane_handle()?.loading_motion(cx)
    }
}

#[derive(Clone, Debug)]
enum Empty {
    Loading,
    Failed(SharedString),
    Loaded,
}

impl Nodes {
    pub(super) fn new(
        runtime: tokio::runtime::Handle,
        logs: Entity<crate::logs::LogPanel>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        cx.bind_keys([KeyBinding::new(
            "escape",
            BackNode,
            Some("NodeWorkspaceFilter"),
        )]);
        let query = cx.new(|cx| InputState::new(window, cx).placeholder("Filter nodes"));
        let subscription =
            cx.subscribe_in(
                &query,
                window,
                |nodes, input, event, window, cx| match event {
                    InputEvent::Change => {
                        nodes.query_text = input.read(cx).value().to_lowercase();
                        nodes.rebuild_lines();
                        nodes.table.reveal(0, ScrollStrategy::Top);
                        nodes.scroll.scroll_to_item(0, ScrollStrategy::Top);
                        cx.notify();
                    }
                    InputEvent::PressEnter { .. } => window.focus(&nodes.focus, cx),
                    _ => {}
                },
            );
        let document = cx.new(|cx| DetailPane::new(runtime.clone(), window, cx));
        let document_subscription = cx.subscribe(&document, |_, _, event, cx| {
            if matches!(event, DetailEvent::Leave) {
                cx.emit(NodesEvent::Back);
            }
        });
        let loading = freshkube_ui::table::LoadingRows::new("nodes");
        let loading_motion = cx.new(|_| loading.motion(freshkube_ui::table::Look::Pulse));
        let history =
            cx.new(|_| crate::monitoring::history::HistoryView::new(runtime.clone(), "node"));
        // Seeded from the entities, so the flags hold whatever they start as.
        let collecting = {
            use crate::logs::TalosPanel as _;
            logs.read(cx).collecting_count() > 0
        };
        let chart = history.read(cx).shows();
        let watches = [
            cx.observe(&logs, |nodes, logs, cx| {
                use crate::logs::TalosPanel as _;
                let collecting = logs.read(cx).collecting_count() > 0;
                if nodes.collecting != collecting {
                    nodes.collecting = collecting;
                    cx.notify();
                }
            }),
            cx.observe(&history, |nodes, history, cx| {
                let chart = history.read(cx).shows();
                if nodes.chart != chart {
                    nodes.chart = chart;
                    cx.notify();
                }
            }),
        ];
        Self {
            rows: Arc::new(Vec::new()),
            empty: Some(Empty::Loading),
            lines: Vec::new(),
            items: Vec::new(),
            counts: [0; 4],
            group_counts: [0; 4],
            filter: None,
            healthy_open: false,
            healthy_folded: false,
            query,
            query_text: String::new(),
            search_keys: Vec::new(),
            status: freshkube_ui::status::Segment::new(None::<SharedString>, ["Not connected"]),
            _query_subscription: subscription,
            _document_subscription: document_subscription,
            table: freshkube_ui::table::TableState::new("nodes"),
            loading,
            loading_motion,
            all_columns: Vec::new(),
            menu_columns: Arc::new(Vec::new()),
            columns: Vec::new(),
            hidden_columns: std::collections::BTreeSet::from(DEFAULT_HIDDEN),
            table_width: 0.,
            metrics: metrics::Metrics::new(runtime.clone()),
            resource_cells: Default::default(),
            talos_current: false,
            selected: None,
            open: false,
            expanded: false,
            tab: NodeTab::Overview,
            tabs: Vec::new(),
            inline_tabs: Vec::new(),
            more: false,
            view: NodeView::Table,
            tab_focus: cx.focus_handle(),
            pane_focus: cx.focus_handle(),
            tab_strip: Default::default(),
            scroll: UniformListScrollHandle::new(),
            page_scroll: ScrollHandle::new(),
            logs_scroll: ScrollHandle::new(),
            logs_height: None,
            split: freshkube_ui::inspector::InspectorSplit::new("nodes", cx),
            document,
            focus: cx.focus_handle(),
            pods: cx.new(|cx| crate::resources::ResourcesScreen::new(runtime.clone(), window, cx)),
            history,
            load_history: Default::default(),
            services: cx.new(|cx| super::services::NodeServices::new(window, cx)),
            logs,
            collecting,
            chart,
            _watches: watches,
            screens: Default::default(),
            attention: Default::default(),
            attention_stale: None,
        }
    }
    /// The Talos screen the open pane shows, when its tab is one.
    pub(super) fn pane_screen(&self) -> Option<super::pages::PageId> {
        self.open.then(|| self.tab.screen()).flatten()
    }

    /// Whether the open pane shows the node's Talos log.
    pub(super) fn shows_logs(&self) -> bool {
        self.open && self.tab == NodeTab::Logs
    }

    /// The header's breadcrumb after Nodes: the open node and its tab.
    pub(super) fn crumb(&self) -> Option<(SharedString, &'static str)> {
        self.open.then(|| {
            let name = self.row().map(|row| row.name.clone()).unwrap_or_default();
            (name, self.tab.label())
        })
    }

    /// The Talos screen the open pane shows, as its page's handle.
    fn pane_handle(&self) -> Option<&crate::screens::ScreenHandle> {
        self.screens.get(&self.pane_screen()?)
    }

    pub(super) fn row(&self) -> Option<&NodeRow> {
        self.rows
            .iter()
            .find(|row| Some(&row.key) == self.selected.as_ref())
    }
    fn sync_tabs(&mut self) {
        let mut tabs = vec![NodeTab::Overview];
        if let Some(row) = self.row() {
            if row.kubernetes.is_some() {
                tabs.push(NodeTab::Pods);
            }
            if row.talos.is_some() {
                tabs.extend([
                    NodeTab::Services,
                    NodeTab::Processes,
                    NodeTab::Storage,
                    NodeTab::Network,
                    NodeTab::Diagnostics,
                    NodeTab::Logs,
                ]);
            }
            if row.kubernetes.is_some() {
                tabs.extend([NodeTab::Events, NodeTab::Yaml]);
            }
        }
        if !tabs.contains(&self.tab) {
            self.tab = NodeTab::Overview;
        }
        self.more = self
            .row()
            .is_some_and(|row| row.talos.is_some() && row.kubernetes.is_some());
        let inline: Vec<_> = tabs
            .iter()
            .copied()
            .filter(|tab| !self.more || !matches!(tab, NodeTab::Events | NodeTab::Yaml))
            .collect();
        // Other tabs start unscrolled.
        if inline != self.inline_tabs {
            self.tab_strip = Default::default();
        }
        self.inline_tabs = inline;
        self.tabs = tabs;
    }
}

impl Pilot {
    pub(super) fn open_node_by_name(
        &mut self,
        name: &str,
        tab: NodeTab,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(key) = self
            .node_workspace
            .read(cx)
            .rows
            .iter()
            .find(|row| {
                row.key.talos.as_deref() == Some(name)
                    || row.key.kubernetes.as_deref() == Some(name)
            })
            .map(|row| row.key.clone())
        {
            self.open_node(key, window, cx);
            self.show_node_tab(tab, window, cx);
        }
    }

    /// Whether the Nodes table shows its loading rows.
    #[cfg(test)]
    pub(super) fn nodes_wait(&self, cx: &App) -> bool {
        matches!(self.node_workspace.read(cx).empty, Some(Empty::Loading))
    }
    /// Derives where the Talos overview stands, for System services and for
    /// every page that needs a node, whenever the overview changes.
    pub(super) fn publish_reading(&mut self, cx: &mut Context<Self>) {
        // System services waits as long as the Talos overview has neither
        // answered nor failed, and says why when nothing can answer.
        let failure = self
            .config_error
            .as_deref()
            .or_else(|| self.overview.error().map(String::as_str));
        let reading = if self.kubernetes_only.is_some() || self.overview.data().is_some() {
            Reading::Answered
        } else if let Some(failure) = failure {
            Reading::Failed(failure.to_owned().into())
        } else {
            Reading::Waiting
        };
        if crate::screens::set_reading(reading.clone(), cx) {
            for (_, screen) in self.pages.iter().filter(|(id, _)| id.is_screen()) {
                App::notify(cx, screen.view().entity_id());
            }
        }
        self.system_services
            .update(cx, |services, cx| services.set_reading(reading, cx));
    }

    pub(super) fn rebuild_joined_nodes(&mut self, cx: &mut Context<Self>) {
        let shown = self.shown_page(cx);
        self.publish_reading(cx);
        self.navigation.column_changed();
        self.navigation.column_state.prepare(
            self.registry
                .active()
                .kubernetes_summary
                .data()
                .map(|s| s.as_ref()),
        );
        let kubernetes = self
            .registry
            .active()
            .kubernetes_summary
            .data()
            .and_then(|summary| summary.nodes.loaded());
        let rows = join::join(
            &self.nodes,
            kubernetes.map(Vec::as_slice).unwrap_or_default(),
            self.overview.data().is_some(),
            self.registry
                .active()
                .kubernetes_summary
                .data()
                .is_some_and(|summary| summary.nodes.is_current()),
        );
        let talos_current = self.overview.data().is_some() && !self.overview.is_stale();
        let loaded = kubernetes.is_some();
        let error: Option<SharedString> = self
            .registry
            .active()
            .kubernetes_summary
            .data()
            .and_then(|summary| summary.nodes.error())
            .or_else(|| {
                self.registry
                    .active()
                    .kubernetes_summary
                    .error()
                    .map(String::as_str)
            })
            .or(self.config_error.as_deref())
            .or_else(|| self.overview.error().map(String::as_str))
            .map(|error| error.to_owned().into());
        let talos = self.kubernetes_only.is_none();
        self.node_workspace.update(cx, |nodes, cx| {
            nodes.set_rows(rows, talos_current);
            if nodes
                .selected
                .as_ref()
                .is_some_and(|key| !nodes.rows.iter().any(|row| &row.key == key))
            {
                nodes.selected = None;
                nodes.open = false;
            }
            nodes.empty = nodes.rows.is_empty().then_some(match error {
                Some(error) => Empty::Failed(error),
                None if loaded => Empty::Loaded,
                None => Empty::Loading,
            });
            nodes.rebuild_lines();
            nodes.rebuild_columns(talos);
            nodes.sync_tabs();
            cx.notify();
        });
        self.rebuild_nodes_meta(cx);
        // A node that left closes the pane, and one that lost its Talos
        // side drops its Talos tabs: the pane's screen hides.
        if self.shown_page(cx) != shown {
            self.sync_visible_soon(cx);
        }
        let rows = self.node_workspace.read(cx).rows.clone();
        let mut display = crate::presentation::overview::Overview::build(
            &rows,
            &self.nodes,
            self.registry
                .active()
                .kubernetes_summary
                .data()
                .map(|summary| summary.as_ref()),
            self.overview.data(),
            self.fixture,
            self.kubernetes_only.is_some(),
        );
        if self.overview.is_stale() {
            let reason = self
                .overview
                .error()
                .map(String::as_str)
                .unwrap_or("The refresh failed.");
            display = display.talos_stale(reason.to_owned().into());
        }
        self.navigation.column_changed();
        self.navigation.rail_marks =
            crate::desktop::shell::RailMarks::from_cards(&display.cards, self.fixture);
        let attention = Arc::new(crate::presentation::attention::build(
            &rows,
            self.registry
                .active()
                .kubernetes_summary
                .data()
                .map(|summary| summary.as_ref()),
            self.overview.data(),
            self.source().kube_identity.as_deref(),
            chrono::Utc::now(),
        ));
        self.node_workspace
            .update(cx, |nodes, _| nodes.attention = attention.clone());
        self.overview_page.update(cx, |overview, cx| {
            overview.set_display(Arc::new(display), cx);
            overview.set_attention(attention, cx);
        });
    }

    pub(super) fn open_node(&mut self, key: NodeKey, window: &mut Window, cx: &mut Context<Self>) {
        if !self
            .node_workspace
            .read(cx)
            .rows
            .iter()
            .any(|row| row.key == key)
        {
            return;
        }
        if let Some(name) = &key.talos {
            self.select_node_by_name(name.clone(), window, cx);
        }
        self.node_workspace
            .update(cx, |nodes, cx| nodes.open(key, window, cx));
        self.navigate(Page::Nodes, window, cx);
        self.activate_node_tab(window, cx);
    }

    pub(super) fn close_node(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.node_workspace.update(cx, |nodes, cx| nodes.close(cx));
        self.sync_node_visibility(window, cx);
        let focus = self.node_workspace.read(cx).focus.clone();
        window.focus(&focus, cx);
        cx.notify();
    }

    pub(super) fn toggle_node_expanded(&mut self, cx: &mut Context<Self>) {
        self.node_workspace
            .update(cx, |nodes, cx| nodes.toggle_expanded(cx));
        cx.notify();
    }

    pub(super) fn show_node_tab(
        &mut self,
        tab: NodeTab,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let shown = self.node_workspace.update(cx, |nodes, cx| {
            if !nodes.tabs.contains(&tab) {
                return false;
            }
            nodes.tab = tab;
            nodes.logs_scroll.set_offset(point(px(0.), px(0.)));
            cx.notify();
            true
        });
        if !shown {
            return;
        }
        self.activate_node_tab(window, cx);
        cx.notify();
    }

    pub(super) fn sync_node_visibility(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let nodes = self.node_workspace.read(cx);
        let shown = self.navigation.page == Page::Nodes && nodes.open;
        let tab = nodes.tab;
        let (pods, document, history) = (
            nodes.pods.clone(),
            nodes.document.clone(),
            nodes.history.clone(),
        );
        let subject = nodes
            .row()
            .and_then(|row| row.key.kubernetes.clone())
            .map(|name| Subject::Node { name });
        pods.update(cx, |pods, cx| {
            pods.set_visible(shown && tab == NodeTab::Pods, window, cx)
        });
        document.update(cx, |pane, cx| {
            pane.set_active(shown && matches!(tab, NodeTab::Events | NodeTab::Yaml), cx)
        });
        self.logs.update(cx, |logs, cx| {
            logs.set_visible(shown && tab == NodeTab::Logs, cx)
        });
        let overview = shown && tab == NodeTab::Overview;
        history.update(cx, |history, cx| {
            history.set_subject(subject, cx);
            history.set_visible(overview, cx);
        });
        // The pane's Talos screens show with their tab.
        self.sync_visible(window, cx);
    }

    fn activate_node_tab(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let nodes = self.node_workspace.read(cx);
        let (row, tab, pods, document, focus) = (
            nodes.row().cloned(),
            nodes.tab,
            nodes.pods.clone(),
            nodes.document.clone(),
            nodes.focus.clone(),
        );
        if let Some(row) = row {
            pods.update(cx, |pods, cx| {
                pods.set_node(row.key.kubernetes.as_deref(), window, cx)
            });
            if matches!(tab, NodeTab::Events | NodeTab::Yaml)
                && let (Some(node), Some(source)) = (&row.kubernetes, self.source().kube)
            {
                let tab = if tab == NodeTab::Events {
                    Tab::Events
                } else {
                    Tab::Yaml
                };
                let target = DetailTarget {
                    kind: freshkube_core::resources::builtin("nodes").expect("node kind"),
                    identity: ResourceIdentity {
                        connection: source.id,
                        resource: "nodes".into(),
                        namespace: String::new(),
                        name: node.name.clone(),
                        uid: node.uid.clone(),
                    },
                };
                document.update(cx, |pane, cx| {
                    pane.set_context(source.context, cx);
                    pane.embed_node(tab, cx);
                    pane.open(target, source.access, "", Duration::ZERO, cx);
                });
            }
        }
        self.sync_node_visibility(window, cx);
        if let Some(screen) = self.active_screen(cx) {
            screen.set_embedded(true, cx);
            screen.activate(window, cx);
        }
        if tab == NodeTab::Logs {
            self.logs
                .update(cx, |logs, cx| logs.focus_lines(window, cx));
        } else if let Some(screen) = self.active_screen(cx) {
            screen.focus(window, cx);
        } else {
            window.focus(&focus, cx);
        }
    }

    /// Moves the keyboard into the node's pane, as Enter moves it into
    /// Resources' drawer (#340): to the shown tab's list, document, log or
    /// screen, or to the tab strip when the tab has none of its own.
    pub(super) fn focus_node_pane(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let nodes = self.node_workspace.read(cx);
        let (tab, pods, document, tab_focus) = (
            nodes.tab,
            nodes.pods.clone(),
            nodes.document.clone(),
            nodes.tab_focus.clone(),
        );
        match tab {
            NodeTab::Logs => self
                .logs
                .update(cx, |logs, cx| logs.focus_lines(window, cx)),
            NodeTab::Pods => pods.update(cx, |pods, cx| pods.focus(window, cx)),
            NodeTab::Events | NodeTab::Yaml => {
                document.update(cx, |pane, cx| pane.focus(window, cx))
            }
            _ => {
                // Most screens keep no keyboard of their own, so the tab
                // strip takes it; a screen with its own then takes it from
                // there. Always the strip first: a previous tab's list,
                // log or screen may hold it, and isn't drawn next frame.
                window.focus(&tab_focus, cx);
                if let Some(screen) = self.active_screen(cx) {
                    screen.focus(window, cx);
                }
            }
        }
    }

    pub(super) fn step_joined_node(
        &mut self,
        delta: isize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let (key, open) = self.node_workspace.update(cx, |nodes, cx| {
            use freshkube_ui::table;
            let key = if nodes.selected.is_none() {
                // Preserve the existing first-row choice for either arrow, skipping groups.
                table::step(nodes, 1, cx)
            } else {
                table::step(nodes, delta, cx)
            };
            (key, nodes.open)
        });
        let Some(key) = key else { return };
        if open {
            self.open_node(key, window, cx);
        } else {
            self.node_workspace.update(cx, |nodes, cx| {
                nodes.selected = Some(key);
                cx.notify();
            });
            cx.notify();
        }
        self.node_workspace
            .update(cx, |nodes, _| nodes.reveal_selected(window));
    }

    pub(super) fn step_node_tab(
        &mut self,
        delta: isize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let nodes = self.node_workspace.read(cx);
        let tabs = &nodes.tabs;
        let index = tabs.iter().position(|tab| *tab == nodes.tab).unwrap_or(0);
        let next = (index as isize + delta).rem_euclid(tabs.len() as isize) as usize;
        let next = tabs[next];
        self.show_node_tab(next, window, cx);
    }

    /// Escape steps back one level, as on Resources: from the pane it hands
    /// the keyboard to the table, leaving the pane open on its tab (an
    /// expanded pane first gives the table its room back); on the table it
    /// clears the filter, then closes the pane.
    pub(super) fn node_back(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let nodes = self.node_workspace.read(cx);
        let (open, expanded, filtered) = (nodes.open, nodes.expanded, !nodes.query_text.is_empty());
        let focus = nodes.focus.clone();
        if open && (expanded || !focus.is_focused(window)) {
            if expanded {
                self.toggle_node_expanded(cx);
            }
            window.focus(&focus, cx);
            cx.notify();
        } else if filtered {
            self.node_workspace
                .update(cx, |nodes, cx| nodes.clear_filter(window, cx));
        } else if open {
            self.close_node(window, cx);
        }
    }

    /// Runs what a click or key in Nodes asked for.
    pub(super) fn nodes_event(
        &mut self,
        event: &NodesEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match event.clone() {
            NodesEvent::Open { key, focus } => {
                self.open_node(key, window, cx);
                if focus {
                    self.focus_node_pane(window, cx);
                }
            }
            NodesEvent::Step(delta) => self.step_joined_node(delta, window, cx),
            NodesEvent::Close => self.close_node(window, cx),
            NodesEvent::Back => self.node_back(window, cx),
            NodesEvent::StepTab(delta) => self.step_node_tab(delta, window, cx),
            NodesEvent::Tab(tab) => {
                self.show_node_tab(tab, window, cx);
                self.focus_node_pane(window, cx);
            }
            NodesEvent::Refresh => self.refresh_now(window, cx),
            NodesEvent::Destination(destination) => self.open_destination(destination, window, cx),
        }
    }
}

impl Nodes {
    /// Selects the node and opens its pane on it.
    fn open(&mut self, key: NodeKey, window: &mut Window, cx: &mut Context<Self>) {
        self.selected = Some(key);
        if self.view == NodeView::Table {
            self.show_selected_healthy();
            freshkube_ui::table::reveal(self, ScrollStrategy::Nearest);
            // Only a stacked inspector shrinks the table it opens under.
            if crate::screens::page_width(window) < freshkube_ui::inspector::SPLIT_WIDTH {
                // Closed or expanded, there is no table to reveal it in.
                freshkube_ui::table::reveal_when_settled(
                    cx.entity().downgrade(),
                    |nodes| nodes.open && !nodes.expanded,
                    window,
                );
            }
        }
        self.open = true;
        // A short page scrolls its frame down to the inspector, or to the
        // top when the inspector fills it, so its heading stays in view.
        if self.expanded {
            self.page_scroll.set_offset(point(px(0.), px(0.)));
        } else {
            self.page_scroll.scroll_to_bottom();
        }
        self.logs_scroll.set_offset(point(px(0.), px(0.)));
        self.sync_tabs();
        cx.notify();
    }

    fn close(&mut self, cx: &mut Context<Self>) {
        self.open = false;
        self.page_scroll.set_offset(point(px(0.), px(0.)));
        if self.view == NodeView::Table {
            self.show_selected_healthy();
            freshkube_ui::table::reveal(self, ScrollStrategy::Nearest);
        }
        cx.notify();
    }

    pub(super) fn toggle_expanded(&mut self, cx: &mut Context<Self>) {
        if !self.open {
            return;
        }
        self.expanded = !self.expanded;
        // A short page keeps its heading in view: expanded from the top,
        // collapsed down to the inspector, as opening a node does.
        if self.expanded {
            self.page_scroll.set_offset(point(px(0.), px(0.)));
        } else {
            self.page_scroll.scroll_to_bottom();
        }
        cx.notify();
    }

    /// The cards, or their roster beside a node's pane, scroll their own
    /// grid to the selection; the table reveals its row.
    fn reveal_selected(&mut self, window: &Window) {
        use freshkube_ui::table::{self, TableSource};
        if self.view == NodeView::Cards {
            if let Some(line) = self.selected.as_ref().and_then(|key| self.line_of(key)) {
                let columns = if self.open {
                    1
                } else {
                    cards::card_columns(window)
                };
                self.scroll.scroll_to_item(
                    freshkube_ui::grid::row_of(line, columns),
                    ScrollStrategy::Nearest,
                );
            }
        } else {
            table::reveal(self, ScrollStrategy::Nearest);
        }
    }

    /// Escape in the filter clears it; in an empty filter it hands the
    /// keyboard back to the table.
    pub(super) fn leave_filter(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.query_text.is_empty() {
            window.focus(&self.focus, cx);
        } else {
            self.clear_filter(window, cx);
        }
    }

    fn clear_filter(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.query_text.clear();
        // Setting the value from code emits no change event.
        self.query
            .update(cx, |input, cx| input.set_value("", window, cx));
        self.rebuild_lines();
        cx.notify();
    }

    /// Why the attention list may be out of date; redraws only for a
    /// change.
    pub(super) fn set_attention_stale(
        &mut self,
        stale: Option<SharedString>,
        cx: &mut Context<Self>,
    ) {
        if self.attention_stale != stale {
            self.attention_stale = stale;
            cx.notify();
        }
    }
}

gpui_kit::actions!(
    node_workspace,
    [
        ExpandNode,
        CloseNode,
        BackNode,
        NextNodeTab,
        PreviousNodeTab,
        OpenNode,
        ToggleHealthyNodes
    ]
);
