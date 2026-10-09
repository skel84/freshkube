//! The page: its breadcrumb header with the hops counted by state, then
//! the trail edge to edge with the selection in the Inspector, beside it on
//! a wide page and under it on a narrow one.
use super::*;
use crate::ui::dp;
use freshkube_ui::page::{self, PageHeader};

impl ChangePage {
    fn render_header(&self, window: &mut Window, cx: &mut Context<Self>) -> Div {
        let this = cx.entity().downgrade();
        let header = PageHeader::new(PREFIX, self.change.freight.clone()).parent(
            "back",
            self.change.project.clone(),
            move |_, _, cx| {
                _ = this.update(cx, |_, cx| cx.emit(ChangeEvent::Back));
            },
        );
        let chips = kit::status_chips(
            header.id("tally"),
            TONES.iter().enumerate().map(|(ix, (tone, id, what))| {
                let tone = *tone;
                kit::status_chip(
                    header.id(&format!("tally-{id}")),
                    tone,
                    self.counts[ix],
                    what,
                    self.filter == Some(tone),
                    cx,
                )
                .on_click(cx.listener(move |this, _, _, cx| this.toggle_filter(tone, cx)))
            }),
            cx,
        );
        header.chips(Some(chips)).render(window, cx)
    }

    fn render_table(&mut self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let table = div()
            .id("change-table")
            .test_support()
            .flex()
            .flex_col()
            .size_full()
            .min_w_0()
            .min_h_0()
            .child(kit::data_table(self, window, cx).flex_1().min_h_0())
            .into_any_element();
        let beside = crate::screens::page_width(window) >= inspector::SPLIT_WIDTH;
        let details = self.render_detail(cx);
        inspector::split("change-split", &self.split, beside, table, details, window)
    }
}

impl Render for ChangePage {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let _span = crate::perf::span("page.render");
        let header = self.render_header(window, cx);
        let body = self.render_table(window, cx);
        let short = page::is_short(window);
        // The keys live on a wrapper drawn in every state.
        div()
            .key_context(CONTEXT)
            .track_focus(&self.focus)
            .on_action(cx.listener(|this, _: &NextHop, window, cx| this.step(1, window, cx)))
            .on_action(cx.listener(|this, _: &PreviousHop, window, cx| this.step(-1, window, cx)))
            .on_action(cx.listener(|this, _: &Back, _, cx| this.back(cx)))
            .on_action(cx.listener(|this, _: &OpenHop, _, cx| this.open_selected(cx)))
            .flex()
            .flex_col()
            .size_full()
            .min_h_0()
            .child(
                page::page("change-page")
                    .track_scroll(&self.page_scroll)
                    .when(short, |this| {
                        this.overflow_y_scroll().restrict_scroll_to_axis()
                    })
                    .child(page::toolbar(cx).child(header))
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .flex_1()
                            .min_h_0()
                            .when(short, |this| this.min_h(dp(page::SHORT_LIST_HEIGHT)))
                            .child(body),
                    ),
            )
    }
}
