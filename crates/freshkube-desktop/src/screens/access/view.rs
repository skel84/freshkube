use super::*;
use freshkube_ui::{
    inspector::{self, Inspector},
    page::{self, PageHeader},
    palette::palette,
    table::DataTable,
};
use gpui_kit::assets::IconName;
use gpui_kit::component::{
    Disableable, Sizable,
    button::{Button, ButtonVariants},
    h_flex, v_flex,
};

impl AccessScreen {
    fn header(&self, window: &mut Window, cx: &mut Context<Self>) -> Div {
        let selected = self.selected_row().is_some();
        let copy = Button::new("access-copy")
            .ghost()
            .small()
            .h(dp(ui::CONTROL_HEIGHT))
            .label("Copy as access request")
            .tooltip(
                "Copy what the selected row lacks, as a request to paste to whoever grants access",
            )
            .disabled(self.requestable().is_none())
            .on_click(cx.listener(|this, _, _, cx| this.copy_request(cx)));
        let recheck = Button::new("access-recheck")
            .ghost()
            .small()
            .h(dp(ui::CONTROL_HEIGHT))
            .label("Recheck")
            .tooltip(if selected {
                "Ask the cluster again about the selected row"
            } else {
                "Ask the cluster again about every row in view"
            })
            .disabled(self.source.is_none())
            .on_click(cx.listener(|this, _, _, cx| this.recheck(cx)));
        let refresh = Button::new("access-refresh")
            .ghost()
            .small()
            .h(dp(ui::CONTROL_HEIGHT))
            .icon(ui::refresh_icon(
                self.namespaces.is_loading() || self.board.in_flight() > 0,
                cx,
            ))
            .accessibility_label("Refresh your access")
            .tooltip("Read the namespaces and who you are again")
            .disabled(self.source.is_none() || self.namespaces.is_loading())
            .on_click(cx.listener(|this, _, _, cx| this.refresh_all(cx)));
        PageHeader::new(PREFIX, "Your access")
            .control(copy)
            .control(recheck)
            .control(refresh)
            .render(window, cx)
    }

    fn lacking(&self, row: &display::Row, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let p = palette(cx);
        row.detail
            .lacking
            .iter()
            .map(|(label, ask, lines)| {
                let value = v_flex()
                    .gap(dp(2.))
                    .children(lines.iter().cloned().map(|line| {
                        div()
                            .whitespace_normal()
                            .text_color(if *ask { p.muted } else { p.ink_2 })
                            .child(line)
                    }));
                div()
                    .id(SharedString::from(format!(
                        "access-detail-{}",
                        label.to_lowercase().replace([' ', '·'], "-")
                    )))
                    .test_support()
                    .text_size(dp(12.))
                    .child(freshkube_ui::ui::named_field(
                        format!("{label} · {}", if *ask { "ask" } else { "no" }).into(),
                        value,
                        cx,
                    ))
                    .into_any_element()
            })
            .collect()
    }

    fn details(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let selected = self.selected.as_ref()?;
        let Some(row) = self.display.rows.iter().find(|row| &row.key == selected) else {
            return Some(
                Inspector::new("access-detail")
                    .child("No longer listed here.")
                    .render(cx)
                    .into_any_element(),
            );
        };
        let heading = div()
            .id("access-detail-title")
            .aria_label(row.label.clone())
            .test_support()
            .flex_1()
            .w_full()
            .min_w_0()
            .truncate()
            .child(row.label.clone());
        let verdict = h_flex()
            .id("access-detail-verdict")
            .test_support()
            .aria_label(row.detail.headline.clone())
            .gap(dp(8.))
            .items_start()
            .children(ui::status_glyph(row.tone, cx))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .font_weight(FontWeight::MEDIUM)
                    .whitespace_normal()
                    .child(row.detail.headline.clone()),
            );
        let failure = row.detail.failure.clone().map(|words| {
            div()
                .id("access-detail-failure")
                .test_support()
                .text_size(dp(12.))
                .text_color(palette(cx).ink_2)
                .whitespace_normal()
                .child(words)
        });
        let note = row.detail.note.clone().map(|words| {
            div()
                .id("access-detail-note")
                .test_support()
                .text_size(dp(12.))
                .text_color(palette(cx).muted)
                .whitespace_normal()
                .child(format!("The server said: {words}"))
        });
        let copy = Button::new("access-detail-copy")
            .small()
            .outline()
            .label("Copy as access request")
            .disabled(row.detail.denied.is_empty())
            .on_click(cx.listener(|this, _, _, cx| this.copy_request(cx)));
        let recheck = Button::new("access-detail-recheck")
            .small()
            .outline()
            .label("Recheck")
            .on_click(cx.listener(|this, _, _, cx| this.recheck(cx)));
        Some(
            Inspector::new("access-detail")
                .selection(selected)
                .heading(heading)
                .child(verdict)
                .children(failure)
                .children(self.lacking(row, cx))
                .children(note)
                .footer(Some(h_flex().gap(dp(8.)).child(copy).child(recheck)))
                .render(cx)
                .into_any_element(),
        )
    }

    fn retry(&self, cx: &mut Context<Self>) -> Button {
        Button::new("access-retry")
            .icon(IconName::RefreshCw)
            .label("Retry")
            .on_click(cx.listener(|this, _, _, cx| this.refresh_all(cx)))
    }

    fn banners(&self, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let mut banners = Vec::new();
        if let Some(changed) = self.board.changed() {
            let words = match (&changed.from, &changed.to) {
                (Some(from), Some(to)) => {
                    format!(
                        "You are now {to}, not {from}. Earlier answers were dropped and are read again."
                    )
                }
                _ => "Your access changed. Earlier answers were dropped and are read again."
                    .to_owned(),
            };
            banners.push(
                ui::warning_banner(
                    Some("Identity changed".into()),
                    words,
                    Some(
                        Button::new("access-changed-dismiss")
                            .small()
                            .outline()
                            .label("Dismiss")
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.board.dismiss_changed();
                                cx.notify();
                            }))
                            .into_any_element(),
                    ),
                    cx,
                )
                .id("access-changed")
                .test_support()
                .into_any_element(),
            );
        }
        if self.display.incomplete() {
            banners.push(
                ui::banner(
                    ui::Tone::Info,
                    Some("Rules incomplete".into()),
                    "The cluster could not list every rule (a webhook authorizer, for example). Cells that read \"ask\" are not a no: the check before each action decides.",
                    None,
                    cx,
                )
                .id("access-incomplete")
                .test_support()
                .into_any_element(),
            );
        }
        if let Some((reason, names)) = self
            .namespaces
            .data()
            .and_then(|namespaces| Some((namespaces.refused.as_ref()?, &namespaces.names)))
        {
            let scope = match names.first() {
                Some(own) => format!(
                    "Only {own}, the namespace this context's kubeconfig names, is checked."
                ),
                None => {
                    "This context's kubeconfig names no namespace, so none is checked.".to_owned()
                }
            };
            banners.push(
                ui::warning_banner(
                    Some("Can't list namespaces".into()),
                    format!("{reason}. {scope} Others are not guessed at."),
                    Some(
                        Button::new("access-refused-retry")
                            .small()
                            .outline()
                            .label("Retry")
                            .on_click(cx.listener(|this, _, _, cx| this.refresh_all(cx)))
                            .into_any_element(),
                    ),
                    cx,
                )
                .id("access-refused")
                .test_support()
                .into_any_element(),
            );
        }
        if let Some(namespaces) = self.namespaces.data().filter(|n| n.more) {
            banners.push(
                ui::banner(
                    ui::Tone::Info,
                    Some("Namespaces cut".into()),
                    format!(
                        "Showing the first {} namespaces; the cluster has more.",
                        namespaces.names.len()
                    ),
                    None,
                    cx,
                )
                .id("access-cut")
                .test_support()
                .into_any_element(),
            );
        }
        if let Some(error) = self
            .namespaces
            .error()
            .filter(|_| self.namespaces.data().is_some())
        {
            banners.push(
                ui::warning_banner(
                    Some("Couldn't read again".into()),
                    error.to_owned(),
                    None,
                    cx,
                )
                .id("access-stale")
                .test_support()
                .into_any_element(),
            );
        }
        banners
    }

    fn body(&self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let table = div()
            .id("access-table")
            .test_support()
            .size_full()
            .flex()
            .flex_col()
            .min_w_0()
            .min_h_0()
            .child(DataTable::new().render(self, window, cx).flex_1().min_h_0())
            .into_any_element();
        inspector::split(
            "access-split",
            &self.split,
            super::super::page_width(window) >= inspector::SPLIT_WIDTH,
            table,
            self.details(cx),
            window,
        )
    }
}

impl Render for AccessScreen {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.sync();
        let loading = self.namespaces.is_loading() && self.namespaces.data().is_none();
        self.loading.show(loading);
        let header = self.header(window, cx);
        let state = if self.source.is_none() {
            Some("Not connected".to_owned())
        } else if self.namespaces.data().is_none() && !loading {
            Some(
                self.namespaces
                    .error()
                    .map(|error| format!("Couldn't list namespaces: {error}"))
                    .unwrap_or_else(|| "Waiting to read your access".into()),
            )
        } else {
            None
        };
        let banners = self.banners(cx);
        let short = page::is_short(window);
        let body = if let Some(words) = state {
            page::inset()
                .child(
                    ui::empty_state(
                        IconName::ShieldCheck,
                        "Your access",
                        words,
                        None,
                        if self.source.is_some() && self.namespaces.error().is_some() {
                            vec![self.retry(cx).into_any_element()]
                        } else {
                            Vec::new()
                        },
                        cx,
                    )
                    .h_auto()
                    .id("access-empty-state")
                    .test_support(),
                )
                .into_any_element()
        } else {
            self.body(window, cx)
        };
        let least = self.split.short_height(
            super::super::page_width(window) >= inspector::SPLIT_WIDTH,
            self.selected.is_some(),
        );
        let body = div()
            .key_context(CONTEXT)
            .track_focus(&self.focus)
            .flex()
            .flex_col()
            .flex_1()
            .min_h_0()
            .when(short, |this| this.min_h(dp(least)))
            .child(body);
        div()
            .size_full()
            .flex()
            .flex_col()
            .min_h_0()
            .on_action(cx.listener(|this, _: &Next, _, cx| this.step(1, cx)))
            .on_action(cx.listener(|this, _: &Previous, _, cx| this.step(-1, cx)))
            .on_action(cx.listener(|this, _: &First, _, cx| this.jump(false, cx)))
            .on_action(cx.listener(|this, _: &Last, _, cx| this.jump(true, cx)))
            .on_action(cx.listener(|this, _: &Close, _, cx| {
                if this.selected.take().is_some() {
                    cx.notify()
                } else {
                    cx.propagate()
                }
            }))
            .on_action(cx.listener(|this, _: &CopyRequest, _, cx| this.copy_request(cx)))
            .on_action(cx.listener(|this, _: &Recheck, _, cx| this.recheck(cx)))
            .child(
                page::page("access-page")
                    .overflow_y_scroll()
                    .restrict_scroll_to_axis()
                    .child(page::toolbar(cx).child(header))
                    .when(!banners.is_empty(), |this| {
                        this.child(
                            page::capped_notes(
                                "access-notices",
                                if short { dp(80.).into() } else { Length::Auto },
                                &self.notes_scroll,
                                banners,
                                Some(palette(cx).line),
                                None,
                                cx,
                            )
                            .flex_none()
                            .min_w_0()
                            .px(dp(page::PANE_PADDING))
                            .py(dp(page::PANE_PADDING_Y)),
                        )
                    })
                    .child(body),
            )
    }
}
