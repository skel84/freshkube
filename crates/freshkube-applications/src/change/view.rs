//! The page: its breadcrumb header with the hops counted by state, then
//! the trail edge to edge with the selection in the Inspector, beside it on
//! a wide page and under it on a narrow one; or, when the change couldn't
//! be read, the state that says why in place of the trail.
use super::*;
use crate::ui::{self, dp};
use freshkube_ui::page::{self, PageHeader};
use freshkube_ui::palette::palette;
use gpui_kit::assets::IconName;
use gpui_kit::component::{
    Disableable, Sizable,
    button::{Button, ButtonVariants},
    v_flex,
};

/// The tallest the banners under the fix bar grow, in dp, before they scroll.
const NOTES_MAX_HEIGHT: f32 = 100.;
/// The least a capped block keeps, in dp: a banner's first line, so the
/// banners stay on the page when the trail needs the room.
const NOTES_MIN_HEIGHT: f32 = 40.;
/// What the table keeps in view under the capped banners at rest, in dp:
/// its header and two rows.
const TABLE_VISIBLE: f32 = 90.;

impl ChangePage {
    fn render_header(&self, window: &mut Window, cx: &mut Context<Self>) -> Div {
        let this = cx.entity().downgrade();
        let header = PageHeader::new(PREFIX, self.change.freight.clone()).parent(
            "back",
            self.parent
                .clone()
                .unwrap_or_else(|| self.change.project.clone().into()),
            move |_, _, cx| {
                _ = this.update(cx, |_, cx| cx.emit(ChangeEvent::Back));
            },
        );
        let chips = kit::status_chips(
            header.id("tally"),
            TONES
                .iter()
                .enumerate()
                .filter(|(ix, (tone, ..))| {
                    // A chip shows only what it counts, and while it filters.
                    self.counts[*ix] > 0 || self.filter == Some(*tone)
                })
                .map(|(ix, (tone, id, what))| {
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
        let reading = self.loader.is_loading();
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
        let (view, view_fold) = self.view_control(cx);
        let mut header = header
            .chips(Some(chips))
            .foldable(kargo, kargo_fold)
            .foldable(copy, copy_fold);
        if let Some(menu) = self.follow.clone() {
            header = header.control(menu);
        }
        header
            .control(refresh)
            .foldable(view, view_fold)
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
        let label = "Copy Kargo link";
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

    /// A refresh that failed over an earlier answer, then what the change
    /// may be missing.
    fn render_notes(
        &self,
        short: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let refresh = self.stale.as_ref().map(|(text, label)| {
            ui::warning_banner(Some("Couldn't read again".into()), text.clone(), None, cx)
                .id("change-stale")
                .aria_label(label.clone())
                .test_support()
                .role(Role::Status)
        });
        let missing = self.missing.as_ref().map(|(text, label)| {
            ui::warning_banner(Some("May be missing".into()), text.clone(), None, cx)
                .id("change-missing")
                .aria_label(label.clone())
                .test_support()
                .role(Role::Status)
        });
        // The placeholder for the pull request card: a plain banner, in the
        // shared look, until the card is designed.
        let pull = self.pull.line().map(|line| {
            ui::banner(line.tone, None, line.text.clone(), None, cx)
                .id("change-pull")
                .aria_label(line.text.clone())
                .test_support()
                .role(Role::Status)
        });
        if refresh.is_none() && missing.is_none() && pull.is_none() {
            return None;
        }
        let notes = page::inset()
            .id("change-notes")
            .test_support()
            .flex()
            .flex_col()
            .gap(dp(page::PANE_PADDING_Y))
            // A short window caps the block, so the banners leave the trail
            // its room; any other shows every banner whole.
            .when(short, |this| {
                let scroll = self.notes_scroll.clone();
                this.max_h(self.notes_cap(window))
                    .overflow_y_scroll()
                    .restrict_scroll_to_axis()
                    .track_scroll(&self.notes_scroll)
                    .on_scroll_wheel(move |event, window, cx| {
                        // The block keeps the wheel until it reaches its end
                        // in that direction, instead of also scrolling the page.
                        let delta = event.delta.pixel_delta(window.line_height()).y;
                        let (offset, most) = (scroll.offset().y, scroll.max_offset().y);
                        let more = if delta < gpui_kit::px(0.) {
                            offset > -most
                        } else {
                            offset < gpui_kit::px(0.)
                        };
                        if most > gpui_kit::px(0.) && more {
                            cx.stop_propagation();
                        }
                    })
            })
            .children(refresh)
            .children(missing)
            .children(pull);
        // The edge below a capped block is its own element, which the block's
        // scrolling can't move, so a clipped banner reads as scrollable. The
        // 1 px edge is not counted in the cap.
        Some(
            v_flex()
                .child(notes)
                .when(short, |this| {
                    this.child(
                        div()
                            .flex_none()
                            .h(gpui_kit::px(1.))
                            .w_full()
                            .bg(palette(cx).line),
                    )
                })
                .into_any_element(),
        )
    }

    /// How tall the capped banners may be in a short window: what is left of
    /// the page's visible height once the table's header and two rows fit
    /// under them, between [`NOTES_MIN_HEIGHT`] and [`NOTES_MAX_HEIGHT`] dp.
    /// The room is the last layout's, so a change asks for one more frame.
    fn notes_cap(&self, window: &mut Window) -> Pixels {
        let (least, most) = (
            freshkube_ui::ui::dp_px(NOTES_MIN_HEIGHT, window),
            freshkube_ui::ui::dp_px(NOTES_MAX_HEIGHT, window),
        );
        let (page, notes) = (self.page_scroll.bounds(), self.notes_scroll.bounds());
        if page.size.height <= Pixels::ZERO || notes.size.height <= Pixels::ZERO {
            return most;
        }
        let top = notes.top() - self.page_scroll.offset().y;
        let room = page.bottom() - top - freshkube_ui::ui::dp_px(TABLE_VISIBLE, window);
        let cap = room.max(least).min(most);
        if self.notes_cap_used.replace(cap) != cap {
            window.on_next_frame(|window, _| window.refresh());
        }
        cap
    }

    /// In place of the trail when nothing could be read: why, and Retry.
    fn render_failed(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let why = self.failure()?.to_owned();
        let retrying = self.loader.is_loading();
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
        let beside = freshkube_ui::page::page_width(window) >= inspector::SPLIT_WIDTH;
        let details = self.render_detail(cx);
        inspector::split("change-split", &self.split, beside, table, details, window)
    }
}

impl Render for ChangePage {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let _span = crate::perf::span("page.render");
        let header = self.render_header(window, cx);
        let short = page::is_short(window);
        let notes = self.render_notes(short, window, cx);
        let summary = self.render_summary(window, cx);
        let body = match self.render_failed(cx) {
            Some(failed) => failed,
            None if self.view == ChangeView::Chain
                && self.loading_rows().is_none()
                && !self.rows.is_empty() =>
            {
                self.render_chain(window, cx)
            }
            None => self.render_table(window, cx),
        };
        let least = self.split.short_height(
            page::page_width(window) >= inspector::SPLIT_WIDTH,
            self.selected.is_some(),
        );
        // The keys live on a wrapper drawn in every state.
        div()
            .key_context(CONTEXT)
            .track_focus(&self.focus)
            .on_action(cx.listener(|this, _: &NextHop, window, cx| this.step(1, window, cx)))
            .on_action(cx.listener(|this, _: &PreviousHop, window, cx| this.step(-1, window, cx)))
            .on_action(cx.listener(|this, _: &ToggleView, window, cx| this.toggle_view(window, cx)))
            .on_action(cx.listener(|this, _: &Back, _, cx| this.back(cx)))
            .on_action(cx.listener(|this, _: &OpenHop, window, cx| {
                if this.focus.is_focused(window)
                    || this
                        .graph
                        .focus_handle()
                        .is_some_and(|focus| focus.is_focused(window))
                {
                    this.open_selected(cx);
                } else {
                    cx.propagate();
                }
            }))
            .flex()
            .flex_col()
            .size_full()
            .min_h_0()
            .min_w_0()
            .child(
                page::page("change-page")
                    .min_w_0()
                    .w_full()
                    .track_scroll(&self.page_scroll)
                    .when(short, |this| {
                        this.overflow_y_scroll().restrict_scroll_to_axis()
                    })
                    .child(page::toolbar(cx).child(header))
                    .children(summary)
                    .children(notes)
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .flex_1()
                            .min_h_0()
                            // A short page scrolls the complete split, including the
                            // Inspector footer; shrinking this wrapper hides its tail.
                            .when(short, |this| this.flex_none().h(dp(least)))
                            .child(body),
                    ),
            )
    }
}
