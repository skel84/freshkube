use super::*;
use display::{Detail, Row};
use freshkube_ui::{
    inspector::{self, Inspector},
    page::{self, PageHeader},
    palette::palette,
    table::{DataTable, TableSource},
};
use gpui_kit::assets::IconName;
use gpui_kit::component::{
    Disableable, Icon, Sizable,
    button::{Button, ButtonVariants},
    h_flex,
    input::Input,
    v_flex,
};

impl CertificatesScreen {
    fn tool(
        &self,
        id: &'static str,
        label: &'static str,
        icon: IconName,
        narrow: bool,
        enabled: bool,
    ) -> Button {
        let button = Button::new(id)
            .ghost()
            .small()
            .h(dp(ui::CONTROL_HEIGHT))
            .disabled(!enabled);
        if narrow {
            button.icon(icon).accessibility_label(label).tooltip(label)
        } else {
            button.label(label)
        }
    }

    fn header(&self, window: &mut Window, cx: &mut Context<Self>) -> Div {
        let filter = div().w(dp(190.)).child(
            Input::new(&self.filter_input)
                .id("certificates-filter")
                .aria_label("Filter by name, host, Gateway, Ingress, issuer or state")
                .small()
                .h(dp(ui::CONTROL_HEIGHT))
                .cleanable(true)
                .prefix(Icon::new(IconName::Search).size(dp(14.))),
        );
        // Under the width where the toolbar has room for words, an icon
        // alone with its tooltip and label.
        let narrow = super::super::page_width(window) < NARROW_TOOLBAR;
        let row = self.selected_row();
        let read = self
            .tool(
                "certificates-read-secret",
                "Read Secret",
                IconName::Eye,
                narrow,
                row.is_some_and(|row| row.can_read) && self.secret_job.is_none(),
            )
            .on_click(cx.listener(|this, _, _, cx| this.read_secret(cx)));
        let open = self
            .tool(
                "certificates-open",
                "Open Certificate",
                IconName::ExternalLink,
                narrow,
                row.is_some_and(|row| row.certificate.is_some()),
            )
            .on_click(cx.listener(|this, _, _, cx| this.open(cx)));
        let refresh = Button::new("certificates-refresh")
            .ghost()
            .small()
            .h(dp(ui::CONTROL_HEIGHT))
            .icon(ui::refresh_icon(self.loader.is_loading(), cx))
            .accessibility_label("Refresh")
            .tooltip("Refresh")
            .disabled(self.source.is_none() || self.loader.is_loading())
            .on_click(cx.listener(|this, _, _, cx| this.read(cx)));
        PageHeader::new("certificates", "Certificates")
            .filter(filter)
            .control(read)
            .control(open)
            .control(refresh)
            .render(window, cx)
    }

    fn section(
        &self,
        id: &str,
        title: &str,
        lines: Vec<AnyElement>,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        v_flex()
            .id(SharedString::from(format!("certificates-detail-{id}")))
            .test_support()
            .gap(dp(6.))
            .text_size(dp(12.))
            .child(ui::caption(title, cx))
            .children(lines)
            .into_any_element()
    }

    fn line(&self, id: String, words: String, muted: bool, cx: &mut Context<Self>) -> AnyElement {
        let p = palette(cx);
        div()
            .id(SharedString::from(id))
            .test_support()
            .whitespace_normal()
            .min_w_0()
            .when(muted, |this| this.text_color(p.muted))
            .child(words)
            .into_any_element()
    }

    fn more_line(&self, id: &str, count: usize, cx: &mut Context<Self>) -> Option<AnyElement> {
        (count > 0).then(|| self.line(format!("{id}-more"), format!("and {count} more"), true, cx))
    }

    fn lines_of(
        &self,
        id: &str,
        words: &[String],
        more: usize,
        cx: &mut Context<Self>,
    ) -> Vec<AnyElement> {
        let mut lines: Vec<AnyElement> = words
            .iter()
            .enumerate()
            .map(|(ix, w)| self.line(format!("{id}-{ix}"), w.clone(), false, cx))
            .collect();
        lines.extend(self.more_line(id, more, cx));
        lines
    }

    fn sections(&self, row: &Row, detail: &Detail, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let id = &row.id;
        let mut status = vec![
            h_flex()
                .gap(dp(6.))
                .child(ui::status_mark(
                    format!("{id}-detail-state"),
                    row.tone,
                    row.state,
                    cx,
                ))
                .child(div().child(row.state))
                .into_any_element(),
        ];
        if !detail.reason.is_empty() {
            status.push(self.line(format!("{id}-reason"), detail.reason.clone(), false, cx));
        }
        for field in &row.fields {
            status.push(
                ui::named_field(
                    field.label.into(),
                    div().whitespace_normal().child(field.value.clone()),
                    cx,
                )
                .into_any_element(),
            );
        }
        status.push(self.line(format!("{id}-source"), detail.source.clone(), true, cx));
        let mut sections = vec![self.section("status", "Status", status, cx)];
        let names = match &detail.names {
            Ok(names) if !names.is_empty() => {
                self.lines_of(&format!("{id}-name"), names, detail.names_more, cx)
            }
            Ok(_) => vec![self.line(
                format!("{id}-no-names"),
                "The certificate names no host.".into(),
                true,
                cx,
            )],
            Err(words) => vec![self.line(format!("{id}-names-unknown"), words.clone(), true, cx)],
        };
        sections.push(self.section("names", "Names", names, cx));
        if !detail.wrong.is_empty() {
            let mut lines = vec![self.line(
                format!("{id}-wrong-why"),
                "Served under this certificate, which does not name them:".into(),
                true,
                cx,
            )];
            lines.extend(self.lines_of(
                &format!("{id}-wrong"),
                &detail.wrong,
                detail.wrong_more,
                cx,
            ));
            sections.push(self.section("wrong", "Wrong name", lines, cx));
        }
        let served = if detail.served.is_empty() {
            vec![self.line(
                format!("{id}-not-served"),
                "No Gateway listener or Ingress read names this Secret.".into(),
                true,
                cx,
            )]
        } else {
            self.lines_of(
                &format!("{id}-served"),
                &detail.served,
                detail.served_more,
                cx,
            )
        };
        sections.push(self.section("served", "Served by", served, cx));
        if !detail.chain.is_empty() {
            let steps = detail
                .chain
                .iter()
                .enumerate()
                .map(|(ix, step)| {
                    h_flex()
                        .id(SharedString::from(format!("{id}-chain-{ix}")))
                        .test_support()
                        .gap(dp(6.))
                        .items_start()
                        // Steps with no glyph (not used, not reached) keep its
                        // width, so every step's text starts at one edge.
                        .child(div().w(dp(10.)).flex_none().child(ui::status_mark(
                            format!("{id}-chain-{ix}-state"),
                            step.tone,
                            step.state,
                            cx,
                        )))
                        .child(
                            div()
                                .min_w_0()
                                .whitespace_normal()
                                .child(format!("{} · {}", step.text, step.state)),
                        )
                        .into_any_element()
                })
                .collect();
            sections.push(self.section("chain", "Issuance", steps, cx));
        }
        sections
    }

    fn details(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let selected = self.selected.as_ref()?;
        let detail_id = SharedString::from("certificates-detail");
        let Some(row) = self.display.rows.iter().find(|row| &row.key == selected) else {
            return Some(
                Inspector::new(detail_id)
                    .child("No longer listed here.")
                    .render(cx)
                    .into_any_element(),
            );
        };
        let p = palette(cx);
        let heading = v_flex()
            .flex_1()
            .w_full()
            .min_w_0()
            .gap(dp(2.))
            .child(
                div()
                    .id("certificates-detail-title")
                    .aria_label(row.label.clone())
                    .test_support()
                    .truncate()
                    .child(row.label.clone()),
            )
            .child(
                div()
                    .id("certificates-detail-name")
                    .test_support()
                    .text_size(dp(12.))
                    .text_color(p.muted)
                    .truncate()
                    .child(row.cells[0].clone()),
            );
        let sections = self.sections(row, &row.detail, cx);
        let footer = h_flex()
            .gap_2()
            .child(
                Button::new("certificates-detail-read")
                    .small()
                    .outline()
                    .label("Read Secret")
                    .disabled(!row.can_read || self.secret_job.is_some())
                    .on_click(cx.listener(|this, _, _, cx| this.read_secret(cx))),
            )
            .child(
                Button::new("certificates-detail-open")
                    .small()
                    .outline()
                    .label("Open Certificate")
                    .disabled(row.certificate.is_none())
                    .on_click(cx.listener(|this, _, _, cx| this.open(cx))),
            );
        Some(
            Inspector::new(detail_id)
                .selection(selected)
                .heading(heading)
                .children(sections)
                .footer(Some(footer))
                .render(cx)
                .into_any_element(),
        )
    }

    fn body(&self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let table = div()
            .id("certificates-table")
            .test_support()
            .size_full()
            .flex()
            .flex_col()
            .min_w_0()
            .min_h_0()
            .child(DataTable::new().render(self, window, cx).flex_1().min_h_0())
            .into_any_element();
        inspector::split(
            SharedString::from("certificates-split"),
            &self.split,
            super::super::page_width(window) >= inspector::SPLIT_WIDTH,
            table,
            self.details(cx),
            window,
        )
    }
}

impl Render for CertificatesScreen {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.sync();
        let loading = self.loader.is_loading() && self.loader.data().is_none();
        self.loading.show(loading);
        let header = self.header(window, cx);
        // The lists refused are an error with Retry; so is a read that failed
        // outright. Never an empty table.
        let error = self.display.error.clone().or_else(|| {
            self.loader
                .data()
                .is_none()
                .then(|| self.loader.error().map(ToString::to_string))
                .flatten()
        });
        let state = if self.source.is_none() {
            Some("Not connected".to_owned())
        } else if let Some(error) = error {
            Some(error)
        } else if self.loader.data().is_none() && !loading {
            Some("Waiting to read certificates".to_owned())
        } else {
            None
        };
        // When nothing could be read the error says it all; the banners would
        // repeat the same sentences.
        let notices = self
            .display
            .notices
            .iter()
            .filter(|_| self.display.error.is_none())
            .enumerate()
            .map(|(ix, (title, words))| {
                ui::warning_banner(Some((*title).into()), words.clone(), None, cx)
                    .id(SharedString::from(format!("certificates-notice-{ix}")))
                    .aria_label(words.clone())
                    .test_support()
            })
            .collect::<Vec<_>>();
        let failure = self
            .loader
            .error()
            .filter(|_| self.loader.data().is_some())
            .map(|error| {
                ui::warning_banner(
                    Some("Couldn't read again".into()),
                    error.to_owned(),
                    None,
                    cx,
                )
                .id("certificates-stale")
                .test_support()
            });
        let empty = !loading && self.display.lines.is_empty();
        let short = page::is_short(window);
        let retryable = self.source.is_some()
            && (self.display.error.is_some() || self.loader.error().is_some());
        let body = if let Some(words) = state {
            page::inset()
                .child(
                    ui::empty_state(
                        IconName::ShieldCheck,
                        "Certificates",
                        words,
                        None,
                        if retryable {
                            vec![self.retry(cx).into_any_element()]
                        } else {
                            Vec::new()
                        },
                        cx,
                    )
                    .h_auto()
                    .id("certificates-state")
                    .test_support(),
                )
                .into_any_element()
        } else if empty {
            page::inset().children(self.empty(cx)).into_any_element()
        } else {
            self.body(window, cx)
        };
        let least = self.split.short_height(
            super::super::page_width(window) >= inspector::SPLIT_WIDTH,
            self.selected.is_some(),
        );
        let has_notes = !notices.is_empty() || failure.is_some();
        let body = div()
            .key_context("Certificates")
            .track_focus(&self.focus)
            .flex()
            .flex_col()
            .flex_1()
            .min_h_0()
            .when(short, |this| {
                this.min_h(dp(least)).when(empty, |this| this.flex_none())
            })
            .child(body);
        div()
            .size_full()
            .flex()
            .flex_col()
            .min_h_0()
            .on_action(cx.listener(|this, _: &Next, _, cx| this.step(1, cx)))
            .on_action(cx.listener(|this, _: &Previous, _, cx| this.step(-1, cx)))
            .on_action(cx.listener(|this, _: &First, _, cx| this.step(isize::MIN, cx)))
            .on_action(cx.listener(|this, _: &Last, _, cx| this.step(isize::MAX, cx)))
            .on_action(cx.listener(|this, _: &Close, _, cx| {
                if this.selected.take().is_some() {
                    cx.notify()
                } else {
                    cx.propagate()
                }
            }))
            .on_action(cx.listener(|this, _: &OpenObject, _, cx| this.open(cx)))
            .on_action(cx.listener(|this, _: &ReadSecret, _, cx| this.read_secret(cx)))
            .on_action(cx.listener(|this, _: &Find, window, cx| {
                this.filter_input
                    .update(cx, |input, cx| input.focus(window, cx))
            }))
            .child(
                page::page("certificates-page")
                    .overflow_y_scroll()
                    .restrict_scroll_to_axis()
                    .child(page::toolbar(cx).child(header))
                    .when(has_notes, |this| {
                        this.child(
                            page::capped_notes(
                                "certificates-notices",
                                if short { dp(80.).into() } else { Length::Auto },
                                &self.notes_scroll,
                                notices.into_iter().chain(failure),
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
