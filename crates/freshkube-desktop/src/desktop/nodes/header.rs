//! Nodes' shared header, with cached source and freshness metadata.
use super::projection::Status;
use super::*;
use crate::ui::dp;
use freshkube_ui::{page, table};
use gpui_kit::base::Selectable;
use gpui_kit::{
    assets::IconName,
    component::{
        Icon, Sizable,
        button::{Button, ButtonGroup, ButtonVariants},
        h_flex,
        input::Input,
    },
    prelude::*,
};

impl Pilot {
    pub(super) fn nodes_header(&self, window: &mut Window, cx: &mut Context<Self>) -> Div {
        let header = page::PageHeader::new("nodes", "Nodes");
        let segment = ButtonGroup::new("nodes-view")
            .outline()
            .small()
            .child(
                Button::new("nodes-view-cards")
                    .h(dp(crate::ui::CONTROL_HEIGHT))
                    .label("Cards")
                    .selected(self.node_workspace.view == NodeView::Cards),
            )
            .child(
                Button::new("nodes-view-table")
                    .h(dp(crate::ui::CONTROL_HEIGHT))
                    .label("Table")
                    .selected(self.node_workspace.view == NodeView::Table),
            )
            .on_click(cx.listener(|view, choice: &Vec<usize>, _, cx| {
                view.node_workspace.view = if choice.first() == Some(&0) {
                    NodeView::Cards
                } else {
                    NodeView::Table
                };
                cx.notify();
            }));
        let filter = div()
            .key_context("NodeWorkspaceFilter")
            .on_action(cx.listener(|view, _: &BackNode, window, cx| {
                view.node_workspace.query_text.clear();
                view.node_workspace
                    .query
                    .update(cx, |input, cx| input.set_value("", window, cx));
                view.node_workspace.rebuild_lines();
                window.focus(&view.node_focus, cx);
                cx.notify();
            }))
            .child(
                Input::new(&self.node_workspace.query)
                    .id(header.id("filter"))
                    .small()
                    .h(dp(crate::ui::CONTROL_HEIGHT))
                    .cleanable(true)
                    .aria_label("Filter nodes by name, address or role")
                    .prefix(Icon::new(IconName::Search).size(dp(14.))),
            );
        let chips = table::status_chips(
            header.id("chips"),
            Status::ALL.map(|status| {
                table::status_chip(
                    status.id(),
                    status.tone(),
                    self.node_workspace.counts[status.index()],
                    status.what(),
                    self.node_workspace.filter == Some(status),
                    cx,
                )
                .on_click(cx.listener(move |view, _, _, cx| {
                    view.node_workspace.filter = if view.node_workspace.filter == Some(status) {
                        None
                    } else {
                        Some(status)
                    };
                    view.node_workspace.rebuild_lines();
                    view.node_workspace.table.reveal(0, ScrollStrategy::Top);
                    view.node_workspace
                        .scroll
                        .scroll_to_item(0, ScrollStrategy::Top);
                    cx.notify();
                }))
            }),
            cx,
        );
        let header = header
            .filter(filter)
            .meta([self.node_workspace.meta.clone().into_any_element()])
            .chips(Some(
                h_flex().gap(dp(8.)).flex_wrap().child(segment).child(chips),
            ));
        let header = if self.node_workspace.view == NodeView::Table {
            let items = self.nodes_columns_items(cx);
            header.foldable(
                self.nodes_columns_menu(items.clone()),
                page::submenu("Columns", items),
            )
        } else {
            header
        };
        let refresh = page::handler(cx, |view: &mut Self, window, cx| view.refresh(window, cx));
        let button = Button::new("nodes-refresh")
            .ghost()
            .small()
            .size(dp(crate::ui::CONTROL_HEIGHT))
            .icon(IconName::RefreshCw)
            .accessibility_label("Refresh nodes")
            .tooltip("Refresh nodes")
            .on_click({
                let refresh = refresh.clone();
                move |_, window, cx| refresh(window, cx)
            });
        header
            .foldable(button, page::item("Refresh", refresh))
            .render(window, cx)
    }
}

impl Pilot {
    pub(super) fn rebuild_nodes_meta(&mut self) {
        let source = if self.fixture {
            "Example data"
        } else {
            self.applied.context.as_deref().unwrap_or("Not connected")
        };
        let mut parts = vec![source.to_owned()];
        if self.kubernetes_only.is_none() {
            parts.push(format!("Talos · {}", source_status(&self.overview)));
        }
        let nodes = self.kubernetes_summary.data().map(|summary| &summary.nodes);
        let state = match nodes {
            Some(nodes) if nodes.is_current() => source_status(&self.kubernetes_summary),
            Some(nodes) => format!(
                "last known · {}",
                nodes.error().unwrap_or("awaiting current nodes")
            ),
            None => source_status(&self.kubernetes_summary),
        };
        parts.push(format!("Kubernetes · {state}"));
        self.node_workspace.meta = parts.join(" · ").into();
    }
}

fn source_status<T, I: Clone + Eq>(snapshot: &crate::state::Snapshot<T, I>) -> String {
    let state = if snapshot.is_loading() {
        "loading"
    } else if snapshot.is_stale() {
        "last known"
    } else if snapshot.data().is_some() {
        "current"
    } else {
        "unavailable"
    };
    let mut text = state.to_owned();
    if let Some(time) = snapshot.last_successful() {
        let time: chrono::DateTime<chrono::Local> = time.into();
        text.push_str(&format!(" {}", time.format("%H:%M:%S")));
    }
    if let Some(error) = snapshot.error() {
        text.push_str(&format!(" · {error}"));
    }
    text
}
