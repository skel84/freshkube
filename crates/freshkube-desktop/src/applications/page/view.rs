//! The page: its breadcrumb header, then the parts table edge to edge with
//! the selection in the Inspector, beside it on a wide page and under it on
//! a narrow one.
use super::*;
use crate::screens::{field, mono};
use crate::ui::{self, dp};
use freshkube_ui::inspector::{self, Inspector};
use freshkube_ui::page::{self, PageHeader};
use freshkube_ui::palette::palette;
use freshkube_ui::tooltip::FollowTooltip as _;
use gpui_kit::component::{h_flex, v_flex};

impl ApplicationPage {
    fn render_header(&self, window: &mut Window, cx: &mut Context<Self>) -> Div {
        let this = cx.entity().downgrade();
        PageHeader::new(PREFIX, self.name.clone())
            .parent("back", "Applications", move |_, _, cx| {
                _ = this.update(cx, |_, cx| cx.emit(ApplicationEvent::Back));
            })
            .crumb(self.what.clone())
            .render(window, cx)
    }

    /// The selected part's details, then its link: each side, why, and
    /// any lower rule's claim.
    fn render_details(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let row = self.selected_row()?;
        let p = palette(cx);
        let title = div()
            .id("application-detail-title")
            .test_support()
            .aria_label(row.name.clone())
            .min_w_0()
            .text_size(dp(14.))
            .font_weight(FontWeight::SEMIBOLD)
            .truncate()
            .follow_tooltip(row.name.clone())
            .child(row.name.clone());
        let heading = h_flex()
            .gap_2()
            .min_w_0()
            .child(title)
            .child(table::link_mark(row.confidence, row.word, cx));
        let fields = |rows: &[(&'static str, SharedString)], cx: &App| {
            rows.iter()
                .map(|(label, value)| field(label, mono(value.clone()).whitespace_normal(), cx))
                .collect::<Vec<_>>()
        };
        let lower = row.lower.iter().enumerate().map(|(ix, lower)| {
            div()
                .id(SharedString::from(format!("application-detail-lower-{ix}")))
                .test_support()
                .aria_label(lower.clone())
                .text_size(dp(12.5))
                .text_color(p.muted)
                .child(lower.clone())
        });
        let link = v_flex()
            .id("application-detail-link")
            .test_support()
            .gap(dp(8.))
            .child(ui::caption("Link", cx))
            .children(fields(&row.link, cx))
            .children(lower);
        Some(
            Inspector::new("application-detail")
                .heading(heading)
                .children(fields(&row.fields, cx))
                .child(link)
                .render(cx)
                .into_any_element(),
        )
    }

    fn render_table(&mut self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let table = div()
            .id("application-table")
            .test_support()
            .flex()
            .flex_col()
            .size_full()
            .min_w_0()
            .min_h_0()
            .child(kit::data_table(self, window, cx).flex_1().min_h_0())
            .into_any_element();
        let beside = crate::screens::page_width(window) >= inspector::SPLIT_WIDTH;
        let details = self.render_details(cx);
        inspector::split(
            "application-split",
            &self.split,
            beside,
            table,
            details,
            window,
        )
    }
}

impl Render for ApplicationPage {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let _span = crate::perf::span("page.render");
        let header = self.render_header(window, cx);
        let body = self.render_table(window, cx);
        let short = page::is_short(window);
        // The keys live on a wrapper drawn in every state.
        div()
            .key_context(CONTEXT)
            .track_focus(&self.focus)
            .on_action(cx.listener(|this, _: &NextPart, _, cx| this.step(1, cx)))
            .on_action(cx.listener(|this, _: &PreviousPart, _, cx| this.step(-1, cx)))
            .on_action(cx.listener(|this, _: &Back, _, cx| this.back(cx)))
            .flex()
            .flex_col()
            .size_full()
            .min_h_0()
            .child(
                page::page("application-page")
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
