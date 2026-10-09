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
use gpui_kit::component::{Disableable, IconName, Sizable, button::Button, h_flex, v_flex};

/// A field's value: names in the monospace face, prose in the UI's.
fn value_text(label: &str, value: &SharedString) -> Div {
    match label {
        "Cluster" | "Namespace" => mono(value.clone()).whitespace_normal(),
        _ => div().child(value.clone()),
    }
}

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
        if !self.inspect {
            return None;
        }
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
                .map(|(label, value)| field(label, value_text(label, value), cx))
                .collect::<Vec<_>>()
        };
        // A lower rule's claim follows Why in its value column.
        let lower = row.lower.iter().enumerate().map(|(ix, lower)| {
            div()
                .id(SharedString::from(format!("application-detail-lower-{ix}")))
                .test_support()
                .aria_label(lower.clone())
                .text_color(p.muted)
                .child(lower.clone())
        });
        let why = row.why.clone().map(|why| {
            field(
                "Why",
                v_flex()
                    .id("application-detail-why")
                    .test_support()
                    .gap(dp(4.))
                    .child(why)
                    .children(lower),
                cx,
            )
        });
        let link = v_flex()
            .id("application-detail-link")
            .test_support()
            .gap(dp(8.))
            .child(ui::caption("Link", cx))
            .children(fields(&row.link, cx))
            .children(why);
        Some(
            Inspector::new("application-detail")
                .heading(heading)
                .child(render_open(row, cx).children(render_follow(row, cx)))
                .children(fields(&row.fields, cx))
                .child(link)
                .render(cx)
                .into_any_element(),
        )
    }

    /// The list's refresh that failed over an earlier read, as the list
    /// says it: what shows is the last read.
    fn render_stale(&self, cx: &App) -> Option<AnyElement> {
        let (text, label) = self.stale.as_ref()?;
        Some(
            page::inset()
                .child(
                    ui::warning_banner(Some("Couldn't read again".into()), text.clone(), None, cx)
                        .id("application-stale")
                        .aria_label(label.clone())
                        .test_support()
                        .role(Role::Status),
                )
                .into_any_element(),
        )
    }

    /// The parts' table with the Inspector beside or under it, and the
    /// height the split keeps while a short page scrolls its frame.
    fn render_table(&mut self, window: &mut Window, cx: &mut Context<Self>) -> (AnyElement, f32) {
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
        let least = self.split.short_height(beside, details.is_some());
        let split = inspector::split(
            "application-split",
            &self.split,
            beside,
            table,
            details,
            window,
        );
        (split, least)
    }
}

impl Render for ApplicationPage {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let _span = crate::perf::span("page.render");
        let header = self.render_header(window, cx);
        let (body, least) = self.render_table(window, cx);
        let short = page::is_short(window);
        // The keys live on a wrapper drawn in every state.
        div()
            .key_context(CONTEXT)
            .track_focus(&self.focus)
            .on_action(cx.listener(|this, _: &NextPart, _, cx| this.step(1, cx)))
            .on_action(cx.listener(|this, _: &PreviousPart, _, cx| this.step(-1, cx)))
            .on_action(cx.listener(|this, _: &Back, _, cx| this.back(cx)))
            .on_action(cx.listener(|this, _: &OpenPart, _, cx| this.open_part(cx)))
            .on_action(cx.listener(|this, _: &FollowFreight, _, cx| this.follow(cx)))
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
                    .children(self.render_stale(cx))
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .flex_1()
                            .min_h_0()
                            // A short page scrolls its frame down to the
                            // split's least heights, the Inspector's too.
                            .when(short, |this| this.min_h(dp(least)))
                            .child(body),
                    ),
            )
    }
}

/// Open in Resources, greyed out with why for a part in a cluster that
/// isn't open.
fn render_open(row: &PartRow, cx: &mut Context<ApplicationPage>) -> Div {
    let tip = row.open_tip.clone();
    h_flex().gap(dp(6.)).flex_wrap().child(
        Button::new("application-detail-open")
            .outline()
            .xsmall()
            .icon(IconName::ExternalLink)
            .label("Open in Resources")
            .disabled(row.closed.is_some())
            // O only for a part that opens: a greyed-out one says why alone.
            .when_else(
                row.closed.is_some(),
                |button| button.tooltip(tip.clone()),
                |button| button.tooltip_with_action(tip.clone(), &OpenPart, Some(CONTEXT)),
            )
            .on_click(cx.listener(|this, _, _, cx| this.open_part(cx))),
    )
}

/// Follow, for a Kargo Stage whose change was read: the change page on
/// that Stage.
fn render_follow(row: &PartRow, cx: &mut Context<ApplicationPage>) -> Option<impl IntoElement> {
    let freight = row.follows.clone()?;
    let tip = format!("Follow Freight {freight} from its commit to the pods on each Stage");
    Some(
        Button::new("application-detail-follow")
            .outline()
            .xsmall()
            .label(format!("Follow {freight}"))
            .tooltip_with_action(tip, &FollowFreight, Some(CONTEXT))
            .on_click(cx.listener(|this, _, _, cx| this.follow(cx))),
    )
}
