//! The page's two views, Overview and All hops: the switch, its saved
//! choice, and what each does when it is chosen. The page owns the
//! selection and the Inspector; both views draw them.
use freshkube_ui::page;
use gpui_kit::component::{Sizable, button::Button, h_flex};

use super::*;
use crate::change_view::ChangeView;
use crate::ui::{self, dp};

/// The views in the switch: the view, the id part and the label.
const VIEWS: [(ChangeView, &str, &str); 2] = [
    (ChangeView::Overview, "overview", "Overview"),
    (ChangeView::Hops, "hops", "All hops"),
];

impl ChangePage {
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
        self.matrix_stale.set(true);
        self.view_store.save(view, cx);
        if view == ChangeView::Hops {
            if self.link_selected {
                self.detail_scroll.set_offset(point(px(0.), px(0.)));
            }
            // The selection is shown: its Stage unfolds and a filter that
            // hides it gives way.
            if let Some(row) = self
                .selected
                .as_ref()
                .and_then(|key| self.rows.iter().find(|row| &row.key == key))
            {
                self.folded.remove(&row.group);
                if self
                    .filter
                    .is_some_and(|tone| tone != self.groups[row.group].counts_as)
                {
                    self.filter = None;
                }
                self.derive();
            }
            kit::reveal(self, ScrollStrategy::Nearest);
            kit::reveal_when_settled(
                cx.entity().downgrade(),
                |page| page.view == ChangeView::Hops && page.selected.is_some(),
                window,
            );
        }
        self.focus(window, cx);
        cx.notify();
    }

    pub(super) fn toggle_view(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.set_view(
            if self.view == ChangeView::Overview {
                ChangeView::Hops
            } else {
                ChangeView::Overview
            },
            window,
            cx,
        );
    }

    pub(super) fn view_control(&self, cx: &mut Context<Self>) -> (Div, page::MenuItems) {
        let control = h_flex()
            .gap(dp(2.))
            .children(VIEWS.map(|(view, id, label)| {
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
        let overview = page::handler(cx, |this: &mut Self, window, cx| {
            this.set_view(ChangeView::Overview, window, cx)
        });
        let hops = page::handler(cx, |this: &mut Self, window, cx| {
            this.set_view(ChangeView::Hops, window, cx)
        });
        let fold = std::rc::Rc::new(
            move |menu: gpui_kit::component::menu::PopupMenu,
                  window: &mut Window,
                  cx: &mut Context<gpui_kit::component::menu::PopupMenu>| {
                let menu = page::item("Overview", overview.clone())(menu, window, cx);
                page::item("All hops", hops.clone())(menu, window, cx)
            },
        );
        (control, fold)
    }
}
