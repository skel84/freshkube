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

use super::{NodeView, Page, Pilot};
use crate::{
    resources::{DetailPane, Tab, detail::DetailTarget, model::ResourceIdentity},
    ui,
};
use freshkube_core::monitoring::history::Subject;
use gpui_kit::{
    component::{
        input::{InputEvent, InputState},
        resizable::ResizableState,
    },
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
    pub(super) fn screen(self) -> Option<super::pages::ScreenKind> {
        match self {
            Self::Processes => Some(super::pages::ScreenKind::Processes),
            Self::Storage => Some(super::pages::ScreenKind::Storage),
            Self::Network => Some(super::pages::ScreenKind::Network),
            Self::Diagnostics => Some(super::pages::ScreenKind::Diagnostics),
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
    meta: SharedString,
    _query_subscription: Subscription,
    table: freshkube_ui::table::TableState,
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
    tab_scroll: ScrollHandle,
    scroll: UniformListScrollHandle,
    /// The frame's scroll, used while the window is short.
    pub(super) page_scroll: ScrollHandle,
    /// The node pane's scroll, used while the window is short on Logs.
    pub(super) pane_scroll: ScrollHandle,
    split: Entity<ResizableState>,
    pub(super) document: Entity<DetailPane>,
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
        window: &mut Window,
        cx: &mut Context<Pilot>,
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
                |pilot, input, event, window, cx| match event {
                    InputEvent::Change => {
                        pilot.node_workspace.query_text = input.read(cx).value().to_lowercase();
                        pilot.node_workspace.rebuild_lines();
                        pilot.node_workspace.table.reveal(0, ScrollStrategy::Top);
                        pilot
                            .node_workspace
                            .scroll
                            .scroll_to_item(0, ScrollStrategy::Top);
                        cx.notify();
                    }
                    InputEvent::PressEnter { .. } => window.focus(&pilot.node_focus, cx),
                    _ => {}
                },
            );
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
            meta: "Not connected".into(),
            _query_subscription: subscription,
            table: freshkube_ui::table::TableState::new("nodes"),
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
            tab_scroll: ScrollHandle::new(),
            scroll: UniformListScrollHandle::new(),
            page_scroll: ScrollHandle::new(),
            pane_scroll: ScrollHandle::new(),
            split: cx.new(|_| ResizableState::default()),
            document: cx.new(|cx| DetailPane::new(runtime, window, cx)),
        }
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
        self.inline_tabs = tabs
            .iter()
            .copied()
            .filter(|tab| !self.more || !matches!(tab, NodeTab::Events | NodeTab::Yaml))
            .collect();
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

    pub(super) fn rebuild_joined_nodes(&mut self) {
        self.column_state
            .prepare(self.kubernetes_summary.data().map(|s| s.as_ref()));
        let kubernetes = self
            .kubernetes_summary
            .data()
            .and_then(|summary| summary.nodes.loaded());
        let rows = join::join(
            &self.nodes,
            kubernetes.map(Vec::as_slice).unwrap_or_default(),
            self.overview.data().is_some(),
            self.kubernetes_summary
                .data()
                .is_some_and(|summary| summary.nodes.is_current()),
        );
        self.node_workspace.set_rows(
            rows,
            self.overview.data().is_some() && !self.overview.is_stale(),
        );
        if self
            .node_workspace
            .selected
            .as_ref()
            .is_some_and(|key| !self.node_workspace.rows.iter().any(|row| &row.key == key))
        {
            self.node_workspace.selected = None;
            self.node_workspace.open = false;
        }
        self.node_workspace.empty = if self.node_workspace.rows.is_empty() {
            let error = self
                .kubernetes_summary
                .data()
                .and_then(|summary| summary.nodes.error())
                .or_else(|| self.kubernetes_summary.error())
                .or_else(|| self.overview.error());
            Some(if let Some(error) = error {
                Empty::Failed(error.to_owned().into())
            } else if kubernetes.is_some() {
                Empty::Loaded
            } else {
                Empty::Loading
            })
        } else {
            None
        };
        self.rebuild_nodes_meta();
        self.node_workspace.rebuild_lines();
        self.node_workspace
            .rebuild_columns(self.kubernetes_only.is_none());
        self.node_workspace.sync_tabs();
        self.overview_display = crate::presentation::overview::Overview::build(
            &self.node_workspace.rows,
            &self.nodes,
            self.kubernetes_summary
                .data()
                .map(|summary| summary.as_ref()),
            self.overview.data(),
            self.fixture,
            self.kubernetes_only.is_some(),
        );
        if self.overview.is_stale() {
            let reason = self.overview.error().unwrap_or("The refresh failed.");
            self.overview_display =
                std::mem::take(&mut self.overview_display).talos_stale(reason.to_owned().into());
        }
        self.rail_marks = crate::desktop::shell::RailMarks::from_cards(
            &self.overview_display.cards,
            self.fixture,
        );
        self.attention = crate::presentation::attention::build(
            &self.node_workspace.rows,
            self.kubernetes_summary
                .data()
                .map(|summary| summary.as_ref()),
            self.overview.data(),
            chrono::Utc::now(),
        );
    }

    pub(super) fn open_node(&mut self, key: NodeKey, window: &mut Window, cx: &mut Context<Self>) {
        if !self.node_workspace.rows.iter().any(|row| row.key == key) {
            return;
        }
        if let Some(name) = &key.talos {
            self.select_node_by_name(name.clone(), window, cx);
        }
        self.node_workspace.selected = Some(key);
        if self.node_workspace.view == NodeView::Table {
            self.node_workspace.show_selected_healthy();
            freshkube_ui::table::reveal(self, ScrollStrategy::Nearest);
        }
        self.node_workspace.open = true;
        self.node_workspace
            .page_scroll
            .set_offset(point(px(0.), px(0.)));
        self.node_workspace
            .pane_scroll
            .set_offset(point(px(0.), px(0.)));
        self.node_workspace.sync_tabs();
        self.navigate(Page::Nodes, window, cx);
        self.activate_node_tab(window, cx);
    }

    pub(super) fn close_node(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.node_workspace.open = false;
        self.node_workspace
            .page_scroll
            .set_offset(point(px(0.), px(0.)));
        self.node_workspace
            .pane_scroll
            .set_offset(point(px(0.), px(0.)));
        if self.node_workspace.view == NodeView::Table {
            self.node_workspace.show_selected_healthy();
            freshkube_ui::table::reveal(self, ScrollStrategy::Nearest);
        }
        self.sync_node_visibility(window, cx);
        window.focus(&self.node_focus, cx);
        cx.notify();
    }

    pub(super) fn show_node_tab(
        &mut self,
        tab: NodeTab,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.node_workspace.tabs.contains(&tab) {
            return;
        }
        self.node_workspace.tab = tab;
        self.node_workspace
            .pane_scroll
            .set_offset(point(px(0.), px(0.)));
        if let Some(index) = self
            .node_workspace
            .inline_tabs
            .iter()
            .position(|candidate| *candidate == tab)
        {
            self.node_workspace.tab_scroll.scroll_to_item(index);
        }
        self.activate_node_tab(window, cx);
        cx.notify();
    }

    pub(super) fn sync_node_visibility(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.sync_node_metrics(window, cx);
        let shown = self.page == Page::Nodes && self.node_workspace.open;
        self.node_pods.update(cx, |pods, cx| {
            pods.set_visible(
                shown && self.node_workspace.tab == NodeTab::Pods,
                window,
                cx,
            )
        });
        self.node_workspace.document.update(cx, |pane, cx| {
            pane.set_active(
                shown && matches!(self.node_workspace.tab, NodeTab::Events | NodeTab::Yaml),
                cx,
            )
        });
        self.logs.update(cx, |logs, cx| {
            logs.set_visible(shown && self.node_workspace.tab == NodeTab::Logs, cx)
        });
        let subject = self
            .node_workspace
            .row()
            .and_then(|row| row.key.kubernetes.clone())
            .map(|name| Subject::Node { name });
        let overview = shown && self.node_workspace.tab == NodeTab::Overview;
        self.node_history.update(cx, |history, cx| {
            history.set_subject(subject, cx);
            history.set_visible(overview, cx);
        });
    }

    fn activate_node_tab(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(row) = self.node_workspace.row().cloned() {
            self.node_pods.update(cx, |pods, cx| {
                pods.set_node(row.key.kubernetes.as_deref(), window, cx)
            });
            if matches!(self.node_workspace.tab, NodeTab::Events | NodeTab::Yaml)
                && let (Some(node), Some(source)) = (&row.kubernetes, self.kube_source())
            {
                let tab = if self.node_workspace.tab == NodeTab::Events {
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
                self.node_workspace.document.update(cx, |pane, cx| {
                    pane.set_context(source.context, cx);
                    pane.embed_node(tab, cx);
                    pane.open(target, source.access, "", Duration::ZERO, cx);
                });
            }
        }
        self.sync_node_visibility(window, cx);
        if let Some(screen) = self.active_screen() {
            screen.set_embedded(true, cx);
            screen.activate(window, cx);
        }
        if self.node_workspace.tab == NodeTab::Logs {
            self.logs
                .update(cx, |logs, cx| logs.focus_lines(window, cx));
        } else if let Some(screen) = self.active_screen() {
            screen.focus(window, cx);
        } else {
            window.focus(&self.node_focus, cx);
        }
    }

    pub(super) fn step_joined_node(
        &mut self,
        delta: isize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        use freshkube_ui::table::{self, TableSource};
        let key = if self.node_workspace.selected.is_none() {
            // Preserve the existing first-row choice for either arrow, skipping groups.
            table::step(self, 1, cx)
        } else {
            table::step(self, delta, cx)
        };
        let Some(key) = key else { return };
        if self.node_workspace.open {
            self.open_node(key, window, cx);
        } else {
            self.node_workspace.selected = Some(key);
            cx.notify();
        }
        if self.node_workspace.open || self.node_workspace.view == NodeView::Cards {
            if let Some(line) = self
                .node_workspace
                .selected
                .as_ref()
                .and_then(|key| self.line_of(key))
            {
                let columns = if self.node_workspace.open {
                    1
                } else {
                    cards::card_columns(window)
                };
                self.node_workspace
                    .scroll
                    .scroll_to_item(line / columns, ScrollStrategy::Nearest);
            }
        } else {
            table::reveal(self, ScrollStrategy::Nearest);
        }
    }

    pub(super) fn step_node_tab(
        &mut self,
        delta: isize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let tabs = &self.node_workspace.tabs;
        let index = tabs
            .iter()
            .position(|tab| *tab == self.node_workspace.tab)
            .unwrap_or(0);
        let next = (index as isize + delta).rem_euclid(tabs.len() as isize) as usize;
        self.show_node_tab(tabs[next], window, cx);
    }

    pub(super) fn node_back(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.node_workspace.tab != NodeTab::Overview {
            self.show_node_tab(NodeTab::Overview, window, cx);
        } else {
            self.close_node(window, cx);
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
        OpenNode
    ]
);
