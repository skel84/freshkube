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
    /// The status bar's segment while Nodes shows.
    pub(super) status: freshkube_ui::status::Segment,
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
        let document = cx.new(|cx| DetailPane::new(runtime.clone(), window, cx));
        let document_subscription =
            cx.subscribe_in(&document, window, |pilot, _, event, window, cx| {
                if matches!(event, DetailEvent::Leave) {
                    pilot.node_back(window, cx);
                }
            });
        let loading = freshkube_ui::table::LoadingRows::new("nodes");
        let loading_motion = cx.new(|_| loading.motion(freshkube_ui::table::Look::Pulse));
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
            split: {
                let file = crate::navigation_file::NavigationFile::global(cx);
                freshkube_ui::inspector::InspectorSplit::new(
                    file.inspector_width("nodes"),
                    move |width, cx| file.set_inspector_width("nodes", width, cx),
                    cx,
                )
            },
            document,
        }
    }
    /// The motion over the table's loading rows, which the shell mounts
    /// beside the cached page, so its frames redraw neither.
    pub(super) fn loading_motion(&self) -> &Entity<freshkube_ui::table::LoadingMotion> {
        &self.loading_motion
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
    pub(super) fn nodes_wait(&self) -> bool {
        matches!(self.node_workspace.empty, Some(Empty::Loading))
    }
    pub(super) fn rebuild_joined_nodes(&mut self, cx: &mut Context<Self>) {
        // System services waits as long as the Talos overview has neither
        // answered nor failed, and says why when nothing can answer.
        let failure = self
            .config_error
            .as_deref()
            .or_else(|| self.overview.error());
        let reading = if self.kubernetes_only.is_some() || self.overview.data().is_some() {
            Reading::Answered
        } else if let Some(failure) = failure {
            Reading::Failed(failure.to_owned().into())
        } else {
            Reading::Waiting
        };
        self.system_services
            .update(cx, |services, cx| services.set_reading(reading, cx));
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
                .or(self.config_error.as_deref())
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
            // Only a stacked inspector shrinks the table it opens under.
            if crate::screens::page_width(window) < freshkube_ui::inspector::SPLIT_WIDTH {
                reveal_when_settled(cx.entity().downgrade(), None, false, 6, window);
            }
        }
        self.node_workspace.open = true;
        // A short page scrolls its frame down to the inspector, or to the
        // top when the inspector fills it, so its heading stays in view.
        if self.node_workspace.expanded {
            self.node_workspace
                .page_scroll
                .set_offset(point(px(0.), px(0.)));
        } else {
            self.node_workspace.page_scroll.scroll_to_bottom();
        }
        self.node_workspace
            .logs_scroll
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
        if self.node_workspace.view == NodeView::Table {
            self.node_workspace.show_selected_healthy();
            freshkube_ui::table::reveal(self, ScrollStrategy::Nearest);
        }
        self.sync_node_visibility(window, cx);
        window.focus(&self.node_focus, cx);
        cx.notify();
    }

    pub(super) fn toggle_node_expanded(&mut self, cx: &mut Context<Self>) {
        if !self.node_workspace.open {
            return;
        }
        let expanded = !self.node_workspace.expanded;
        self.node_workspace.expanded = expanded;
        // A short page keeps its heading in view: expanded from the top,
        // collapsed down to the inspector, as opening a node does.
        if expanded {
            self.node_workspace
                .page_scroll
                .set_offset(point(px(0.), px(0.)));
        } else {
            self.node_workspace.page_scroll.scroll_to_bottom();
        }
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
            .logs_scroll
            .set_offset(point(px(0.), px(0.)));
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

    /// Moves the keyboard into the node's pane, as Enter moves it into
    /// Resources' drawer (#340): to the shown tab's list, document, log or
    /// screen, or to the tab strip when the tab has none of its own.
    pub(super) fn focus_node_pane(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        match self.node_workspace.tab {
            NodeTab::Logs => self
                .logs
                .update(cx, |logs, cx| logs.focus_lines(window, cx)),
            NodeTab::Pods => self.node_pods.update(cx, |pods, cx| pods.focus(window, cx)),
            NodeTab::Events | NodeTab::Yaml => self
                .node_workspace
                .document
                .update(cx, |pane, cx| pane.focus(window, cx)),
            _ => {
                // Most screens keep no keyboard of their own, so the tab
                // strip takes it; a screen with its own then takes it from
                // there. Always the strip first: a previous tab's list,
                // log or screen may hold it, and isn't drawn next frame.
                window.focus(&self.node_workspace.tab_focus, cx);
                if let Some(screen) = self.active_screen() {
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
        // The cards, or their roster beside a node's pane, scroll their own
        // grid; the table reveals its row.
        if self.node_workspace.view == NodeView::Cards {
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
                self.node_workspace.scroll.scroll_to_item(
                    freshkube_ui::grid::row_of(line, columns),
                    ScrollStrategy::Nearest,
                );
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

    /// Escape steps back one level, as on Resources: from the pane it hands
    /// the keyboard to the table, leaving the pane open on its tab (an
    /// expanded pane first gives the table its room back); on the table it
    /// clears the filter, then closes the pane.
    pub(super) fn node_back(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let nodes = &self.node_workspace;
        if nodes.open && (nodes.expanded || !self.node_focus.is_focused(window)) {
            if nodes.expanded {
                self.toggle_node_expanded(cx);
            }
            window.focus(&self.node_focus, cx);
            cx.notify();
        } else if !nodes.query_text.is_empty() {
            self.clear_node_filter(window, cx);
        } else if nodes.open {
            self.close_node(window, cx);
        }
    }

    /// Escape in the filter clears it; in an empty filter it hands the
    /// keyboard back to the table.
    pub(super) fn leave_node_filter(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.node_workspace.query_text.is_empty() {
            window.focus(&self.node_focus, cx);
        } else {
            self.clear_node_filter(window, cx);
        }
    }

    fn clear_node_filter(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.node_workspace.query_text.clear();
        // Setting the value from code emits no change event.
        self.node_workspace
            .query
            .update(cx, |input, cx| input.set_value("", window, cx));
        self.node_workspace.rebuild_lines();
        cx.notify();
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

/// Reveals the selected row on each of the next frames until the table's
/// height has changed and then held: a stacked inspector's split learns its
/// heights while drawing and applies them on a later frame, and a row
/// revealed in the taller table would end up under it. Each reveal asks for
/// the next frame, so the split's change is drawn; `frames` bounds the wait
/// when the height never changes.
fn reveal_when_settled(
    view: WeakEntity<Pilot>,
    last: Option<Pixels>,
    changed: bool,
    frames: usize,
    window: &Window,
) {
    window.on_next_frame(move |window, cx| {
        // Closed or expanded, there is no table to reveal it in.
        let Some(height) = view
            .update(cx, |pilot, cx| {
                if !pilot.node_workspace.open || pilot.node_workspace.expanded {
                    return None;
                }
                freshkube_ui::table::reveal(pilot, ScrollStrategy::Nearest);
                cx.notify();
                Some(
                    (pilot.node_workspace.table.scroll.0.borrow().last_item_size)
                        .map(|size| size.item.height),
                )
            })
            .ok()
            .flatten()
        else {
            return;
        };
        let settled = changed && height == last;
        if frames > 1 && !settled {
            let changed = changed || (last.is_some() && height != last);
            reveal_when_settled(view, height, changed, frames - 1, window);
        }
    });
}
