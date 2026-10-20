//! Chain's canvas and view switch. The page owns selection and Inspector;
//! the graph owns only layout, focus and sideways scrolling.
use freshkube_ui::graph::{
    self, Cursor, GraphCanvas, GraphEdge, GraphSource, GraphState, GraphText,
};
use freshkube_ui::page;
use gpui_kit::component::{Sizable, button::Button, h_flex};

use super::*;
use crate::change_view::ChangeView;
use crate::ui::{self, dp};

pub(super) fn state(cx: &mut App) -> GraphState<SharedString> {
    graph::bind_keys(cx);
    GraphState::new(
        "change-chain",
        GraphText {
            nouns: ("hops", "links"),
            title: "Link".into(),
            problems: "".into(),
            caption: "".into(),
        },
    )
    .unpaged()
    .with_keys(cx)
}

impl ChangePage {
    // A first answer, or opening the Inspector, changes the pan's bounds.
    // Reveal again after layout, only while this is still the selection.
    pub(super) fn reveal_chain_when_drawn(&self, window: &Window, cx: &mut Context<Self>) {
        let selected = self.selected.clone();
        let this = cx.entity().downgrade();
        window.on_next_frame(move |window, cx| {
            _ = this.update(cx, |page, cx| {
                if page.visible
                    && page.view == ChangeView::Chain
                    && page.selected == selected
                    && let Some(key) = page
                        .selected
                        .as_ref()
                        .and_then(|key| page.trail.cards.get(key))
                {
                    page.graph.reveal_node(key, window);
                    cx.notify();
                }
            });
        });
    }

    pub(super) fn derive_chain(&mut self) {
        self.trail = super::trail::Trail::derive(&self.change, &self.rows);
        self.graph
            .set(self.trail.nodes.clone(), self.trail.edges.clone());
        self.dim_chain();
    }

    pub(super) fn dim_chain(&mut self) {
        let dimmed = self
            .trail
            .nodes
            .iter()
            .filter(|node| {
                self.filter.is_some_and(|tone| {
                    !self.rows.iter().any(|row| {
                        self.trail.cards.get(&row.key) == Some(&node.key)
                            && status(row.tone) == tone
                    })
                })
            })
            .map(|node| node.key.clone());
        self.graph.set_dimmed(dimmed);
    }

    pub(super) fn set_view(
        &mut self,
        view: ChangeView,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.view == view {
            return;
        }
        self.view = view;
        self.view_store.save(view, cx);
        if view == ChangeView::List {
            if self.link_selected {
                self.detail_scroll.set_offset(point(px(0.), px(0.)));
            }
            if let Some(row) = self
                .selected
                .as_ref()
                .and_then(|key| self.rows.iter().find(|row| &row.key == key))
            {
                self.folded.remove(&row.group);
                if self.filter.is_some_and(|tone| tone != status(row.tone)) {
                    self.filter = None;
                }
                self.derive();
                self.dim_chain();
            }
            kit::reveal(self, ScrollStrategy::Nearest);
            kit::reveal_when_settled(
                cx.entity().downgrade(),
                |page| page.view == ChangeView::List && page.selected.is_some(),
                window,
            );
        }
        self.focus(window, cx);
        // Schedule the reveal from render, after the canvas is actually
        // drawn. A callback queued here can precede its first layout.
        self.chain_reveal_pending = view == ChangeView::Chain;
        cx.notify();
    }

    pub(super) fn toggle_view(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.set_view(
            if self.view == ChangeView::List {
                ChangeView::Chain
            } else {
                ChangeView::List
            },
            window,
            cx,
        );
    }

    pub(super) fn view_control(&self, cx: &mut Context<Self>) -> (Div, page::MenuItems) {
        let options = [
            (ChangeView::List, "list", "List"),
            (ChangeView::Chain, "chain", "Chain"),
        ];
        let control = h_flex()
            .gap(dp(2.))
            .children(options.map(|(view, id, label)| {
                ui::segment(
                    Button::new(SharedString::from(format!("change-view-{id}"))),
                    self.view == view,
                    cx,
                )
                .small()
                .h(dp(ui::CONTROL_HEIGHT))
                .label(label)
                .accessibility_label(format!("{label} view"))
                .on_click(cx.listener(move |this, _, window, cx| this.set_view(view, window, cx)))
            }));
        let list = page::handler(cx, |this: &mut Self, window, cx| {
            this.set_view(ChangeView::List, window, cx)
        });
        let chain = page::handler(cx, |this: &mut Self, window, cx| {
            this.set_view(ChangeView::Chain, window, cx)
        });
        let fold = std::rc::Rc::new(
            move |menu: gpui_kit::component::menu::PopupMenu,
                  window: &mut Window,
                  cx: &mut Context<gpui_kit::component::menu::PopupMenu>| {
                let menu = page::item("List", list.clone())(menu, window, cx);
                page::item("Chain", chain.clone())(menu, window, cx)
            },
        );
        (control, fold)
    }

    pub(super) fn render_chain(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        if std::mem::take(&mut self.chain_reveal_pending) {
            self.reveal_chain_when_drawn(window, cx);
        }
        let canvas = GraphCanvas::render(self, window, cx);
        let legend = kit::legend_line(
            "change-chain-legend",
            "━ Confirmed · ╌ Claimed · ┄ Unknown ⓘ",
            "Confirmed: the key matches on both sides. Claimed: only a label, annotation or attestation says so. Unknown: a side can't be read.",
            cx,
        );
        let chain = div()
            .id("change-chain")
            .test_support()
            .flex()
            .flex_col()
            .size_full()
            .min_w_0()
            .min_h_0()
            .child(canvas)
            .child(GraphCanvas::legend("change-chain-footer", legend, cx))
            .into_any_element();
        let details = self.render_detail(cx);
        inspector::split(
            "change-split",
            &self.split,
            page::page_width(window) >= inspector::SPLIT_WIDTH,
            chain,
            details,
            window,
        )
    }
}

impl GraphSource for ChangePage {
    fn word_details(&self) -> bool {
        true
    }
    type Key = SharedString;
    fn graph(&self) -> &GraphState<SharedString> {
        &self.graph
    }
    fn graph_mut(&mut self) -> &mut GraphState<SharedString> {
        &mut self.graph
    }
    fn graph_split(&self) -> &InspectorSplit {
        &self.split
    }
    fn highlighted(&self) -> Option<&SharedString> {
        self.selected
            .as_ref()
            .and_then(|key| self.trail.cards.get(key))
    }
    fn cursor(&self) -> Option<Cursor<SharedString>> {
        let key = self
            .selected
            .as_ref()
            .and_then(|key| self.trail.cards.get(key))?
            .clone();
        if self.link_selected
            && let Some(edge) = self.trail.edges.iter().find(|edge| edge.to == key)
        {
            return Some(Cursor::Link(edge.from.clone(), key));
        }
        Some(Cursor::Node(key))
    }
    fn open(&mut self, key: &SharedString, window: &mut Window, cx: &mut Context<Self>) {
        if self.selected.as_ref() != Some(key) || self.link_selected {
            self.select(key, window, cx);
        }
        self.focus(window, cx);
    }
    fn step_to(
        &mut self,
        cursor: Cursor<SharedString>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let (key, link) = match cursor {
            Cursor::Node(key) => (key, false),
            Cursor::Link(_, to) => (to, true),
        };
        self.select(&key, window, cx);
        self.link_selected = link;
        if link {
            self.detail_scroll.set_offset(point(px(0.), px(0.)));
        }
        if link && let Some(edge) = self.trail.edges.iter().find(|edge| edge.to == key) {
            self.graph.select(&edge.from, &edge.to);
        }
        self.detail = self.derive_detail();
        cx.notify();
    }
    fn inspector_head(
        &self,
        _: Option<&GraphEdge<SharedString>>,
        _: &mut Context<Self>,
    ) -> AnyElement {
        div().into_any_element()
    }
    fn empty(&self, _: &mut Context<Self>) -> AnyElement {
        div().child("No hops").into_any_element()
    }
}
