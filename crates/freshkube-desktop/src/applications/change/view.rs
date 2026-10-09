//! The page: its breadcrumb header with the hops counted by state, then
//! the trail edge to edge with the selection in the Inspector, beside it on
//! a wide page and under it on a narrow one; or, when the change couldn't
//! be read, the state that says why in place of the trail.
use super::*;
use crate::ui::{self, dp};
use freshkube_ui::page::{self, PageHeader};
use gpui_kit::assets::IconName;
use gpui_kit::component::{
    Disableable, Sizable,
    button::{Button, ButtonVariants},
};

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
        let reading = self.pending;
        let label = "Read the change again";
        let refresh = Button::new(header.id("refresh"))
            .ghost()
            .small()
            .size(dp(ui::CONTROL_HEIGHT))
            .icon(ui::refresh_icon(reading, cx))
            .accessibility_label(label)
            .tooltip_with_action(
                if reading { "Reading…" } else { label },
                &page::Refresh,
                Some(page::SHELL_CONTEXT),
            )
            .disabled(reading)
            // The button and ⌘R are one action, which the shell runs.
            .on_click(page::dispatch(page::Refresh, &self.focus));
        let (kargo, kargo_fold) = self.render_open_in_kargo(&header, cx);
        let (copy, copy_fold) = self.render_copy_link(&header, cx);
        header
            .chips(Some(chips))
            .foldable(kargo, kargo_fold)
            .foldable(copy, copy_fold)
            .control(refresh)
            .render(window, cx)
    }

    /// Open in Kargo ↗: the Freight's page, at the address the cluster
    /// records; greyed out with why without one.
    fn render_open_in_kargo(
        &self,
        header: &PageHeader,
        cx: &mut Context<Self>,
    ) -> (Button, page::MenuItems) {
        let label = "Open in Kargo ↗";
        let open = page::handler(cx, |this: &mut Self, _, cx| this.open_page(cx));
        let button = Button::new(header.id("open-in-kargo"))
            .ghost()
            .small()
            .h(dp(ui::CONTROL_HEIGHT))
            .label(label)
            .accessibility_label("Open the Freight in Kargo")
            .on_click({
                let open = open.clone();
                move |_, window, cx| open(window, cx)
            });
        self.linked(button, label, open)
    }

    /// Copy link: the same address, to the clipboard.
    fn render_copy_link(
        &self,
        header: &PageHeader,
        cx: &mut Context<Self>,
    ) -> (Button, page::MenuItems) {
        let label = "Copy link";
        let copy = page::handler(cx, |this: &mut Self, _, cx| this.copy_link(cx));
        let (icon, shown, said) = if self.copied {
            (
                IconName::Check,
                "Copied",
                "Copied the Freight's Kargo address",
            )
        } else {
            (IconName::Copy, label, "Copy the Freight's Kargo address")
        };
        let button = Button::new(header.id("copy-link"))
            .ghost()
            .small()
            .h(dp(ui::CONTROL_HEIGHT))
            .icon(icon)
            .label(shown)
            .accessibility_label(said)
            .on_click({
                let copy = copy.clone();
                move |_, window, cx| copy(window, cx)
            });
        self.linked(button, label, copy)
    }

    /// A header control on the Freight's Kargo address: its tooltip shows
    /// the address, or, greyed out, why there's none; folded, the same.
    fn linked(
        &self,
        button: Button,
        label: &str,
        handler: page::Handler,
    ) -> (Button, page::MenuItems) {
        match &self.page_link {
            Ok(address) => (button.tooltip(address.clone()), page::item(label, handler)),
            Err(why) => (
                button.disabled(true).tooltip(why.clone()),
                page::disabled_item(label),
            ),
        }
    }

    /// A refresh that failed over an earlier answer.
    fn render_stale(&self, cx: &App) -> Option<AnyElement> {
        let (text, label) = self.stale.clone()?;
        Some(
            page::inset()
                .child(
                    ui::warning_banner(Some("Couldn't read again".into()), text, None, cx)
                        .id("change-stale")
                        .aria_label(label)
                        .test_support()
                        .role(Role::Status),
                )
                .into_any_element(),
        )
    }

    /// In place of the trail when nothing could be read: why, and Retry.
    fn render_failed(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let why = self.failure()?.to_owned();
        let retrying = self.pending;
        let label = if retrying { "Retrying…" } else { "Retry" };
        let retry = Button::new("change-retry")
            .primary()
            .icon(ui::refresh_icon(retrying, cx))
            .label(label)
            .accessibility_label(label)
            .disabled(retrying)
            .on_click(cx.listener(|this, _, _, cx| this.refresh(cx)))
            .into_any_element();
        let state = ui::empty_state(
            IconName::CircleDashed,
            "Couldn't read the change",
            "Nothing of it was read; the trail shows once the Freight answers.",
            Some(why),
            vec![retry],
            cx,
        );
        Some(
            page::inset()
                .flex_1()
                .child(state.id("change-failed").test_support().role(Role::Status))
                .into_any_element(),
        )
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
        let stale = self.render_stale(cx);
        let body = match self.render_failed(cx) {
            Some(failed) => failed,
            None => self.render_table(window, cx),
        };
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
                    .children(stale)
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
