//! Machines from both summaries, with one retained pane for their details.
mod join;
#[cfg(test)]
mod tests;
mod view;

use super::{NodeView, Page, Pilot};
use crate::{
    resources::{DetailPane, Tab, detail::DetailTarget, model::ResourceIdentity},
    ui,
};
use gpui_kit::{component::resizable::ResizableState, *};
use join::{NodeKey, NodeRow};
use std::{sync::Arc, time::Duration};

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
    empty: Option<(SharedString, SharedString)>,
    pub(super) selected: Option<NodeKey>,
    pub(super) open: bool,
    pub(super) expanded: bool,
    pub(super) tab: NodeTab,
    tabs: Vec<NodeTab>,
    inline_tabs: Vec<NodeTab>,
    more: bool,
    view: NodeView,
    tab_focus: FocusHandle,
    tab_scroll: ScrollHandle,
    scroll: UniformListScrollHandle,
    split: Entity<ResizableState>,
    pub(super) document: Entity<DetailPane>,
}

impl Nodes {
    pub(super) fn new(
        runtime: tokio::runtime::Handle,
        window: &mut Window,
        cx: &mut Context<Pilot>,
    ) -> Self {
        Self {
            rows: Arc::new(Vec::new()),
            empty: Some((
                "Waiting for nodes".into(),
                "The cluster summaries have not answered yet.".into(),
            )),
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
        let kubernetes = self
            .kubernetes_summary
            .data()
            .and_then(|summary| summary.nodes.loaded());
        self.node_workspace.rows = Arc::new(join::join(
            &self.nodes,
            kubernetes.map(Vec::as_slice).unwrap_or_default(),
            self.overview.data().is_some(),
            kubernetes.is_some(),
        ));
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
                ("Nodes unavailable".into(), error.to_owned().into())
            } else if kubernetes.is_some() {
                (
                    "No nodes reported".into(),
                    "No nodes were reported for this context.".into(),
                )
            } else {
                (
                    "Waiting for nodes".into(),
                    "The cluster summaries have not answered yet.".into(),
                )
            })
        } else {
            None
        };
        self.node_workspace.sync_tabs();
    }

    pub(super) fn open_node(&mut self, key: NodeKey, window: &mut Window, cx: &mut Context<Self>) {
        if !self.node_workspace.rows.iter().any(|row| row.key == key) {
            return;
        }
        if let Some(name) = &key.talos {
            self.select_node_by_name(name.clone(), window, cx);
        }
        self.node_workspace.selected = Some(key);
        self.node_workspace.open = true;
        self.node_workspace.sync_tabs();
        self.navigate(Page::Nodes, window, cx);
        self.activate_node_tab(window, cx);
    }

    pub(super) fn close_node(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.node_workspace.open = false;
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
        let count = self.node_workspace.rows.len();
        if count == 0 {
            return;
        }
        let index = self
            .node_workspace
            .rows
            .iter()
            .position(|row| Some(&row.key) == self.node_workspace.selected.as_ref());
        let next = index
            .map(|index| index.saturating_add_signed(delta).min(count - 1))
            .unwrap_or(0);
        let key = self.node_workspace.rows[next].key.clone();
        self.node_workspace
            .scroll
            .scroll_to_item(next, ScrollStrategy::Nearest);
        if self.node_workspace.open {
            self.open_node(key, window, cx);
        } else {
            self.node_workspace.selected = Some(key);
            cx.notify();
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
