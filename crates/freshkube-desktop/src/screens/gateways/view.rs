use super::*;
use display::{ClassDetail, Detail, GatewayDetail, Row};
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

impl GatewaysScreen {
    fn header(&self, window: &mut Window, cx: &mut Context<Self>) -> Div {
        let prefix = self.mode.prefix();
        let filter = div().w(dp(190.)).child(
            Input::new(&self.filter_input)
                .id(SharedString::from(format!("{prefix}-filter")))
                .aria_label(self.mode.filter_hint())
                .small()
                .h(dp(ui::CONTROL_HEIGHT))
                .cleanable(true)
                .prefix(Icon::new(IconName::Search).size(dp(14.))),
        );
        // Under the width where the toolbar has room for words, an icon
        // alone with its tooltip and label.
        let narrow = super::super::page_width(window) < NARROW_TOOLBAR;
        let openable = self.selected_row().is_some_and(|row| row.openable);
        let label = self.mode.open_label();
        let button = Button::new(SharedString::from(format!("{prefix}-open")))
            .ghost()
            .small()
            .h(dp(ui::CONTROL_HEIGHT))
            .disabled(!openable);
        let open = if narrow {
            button
                .icon(IconName::ExternalLink)
                .accessibility_label(label)
                .tooltip(label)
        } else {
            button.label(label)
        }
        .on_click(cx.listener(|this, _, _, cx| this.open(cx)));
        let refresh = Button::new(SharedString::from(format!("{prefix}-refresh")))
            .ghost()
            .small()
            .h(dp(ui::CONTROL_HEIGHT))
            .icon(ui::refresh_icon(self.loader.is_loading(), cx))
            .accessibility_label("Refresh")
            .tooltip("Refresh")
            .disabled(self.source.is_none() || self.loader.is_loading())
            .on_click(cx.listener(|this, _, _, cx| this.read(cx)));
        PageHeader::new(prefix, self.mode.title())
            .filter(filter)
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
            .id(SharedString::from(format!(
                "{}-detail-{id}",
                self.mode.prefix()
            )))
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

    fn status_lines(
        &self,
        row: &Row,
        state: &'static str,
        reason: &str,
        owner: Option<&'static str>,
        says: Option<&String>,
        cx: &mut Context<Self>,
    ) -> Vec<AnyElement> {
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
                .child(div().child(state))
                .into_any_element(),
            self.line(format!("{id}-reason"), reason.to_owned(), false, cx),
        ];
        if let Some(whose) = owner {
            status.push(
                ui::named_field("Whose problem".into(), div().child(whose), cx).into_any_element(),
            );
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
        // What the controller reports stays on a line of its own.
        if let Some(says) = says {
            status.push(
                ui::named_field(
                    "Controller reports".into(),
                    div().whitespace_normal().child(says.clone()),
                    cx,
                )
                .into_any_element(),
            );
        }
        status
    }

    fn more_line(&self, id: &str, count: usize, cx: &mut Context<Self>) -> Option<AnyElement> {
        (count > 0).then(|| self.line(format!("{id}-more"), format!("and {count} more"), true, cx))
    }

    fn gateway_sections(
        &self,
        row: &Row,
        detail: &GatewayDetail,
        cx: &mut Context<Self>,
    ) -> Vec<AnyElement> {
        let id = &row.id;
        let status = self.status_lines(
            row,
            detail.state,
            &detail.reason,
            detail.owner,
            detail.says.as_ref(),
            cx,
        );
        let mut sections = vec![self.section("status", "Status", status, cx)];
        let muted = palette(cx).muted;
        let mut listeners = detail
            .listeners
            .iter()
            .enumerate()
            .map(|(ix, block)| {
                v_flex()
                    .id(SharedString::from(format!("{id}-listener-{ix}")))
                    .test_support()
                    .gap(dp(1.))
                    .children(block.lines.iter().enumerate().map(|(n, w)| {
                        div()
                            .whitespace_normal()
                            .min_w_0()
                            .when(n > 0, |this| this.text_color(muted))
                            .child(w.clone())
                    }))
                    .into_any_element()
            })
            .collect::<Vec<_>>();
        if listeners.is_empty() {
            listeners.push(self.line(
                format!("{id}-no-listeners"),
                "None declared.".into(),
                true,
                cx,
            ));
        }
        listeners.extend(self.more_line(&format!("{id}-listeners"), detail.listeners_more, cx));
        sections.push(self.section("listeners", "Listeners", listeners, cx));
        let mut routes = detail
            .routes
            .iter()
            .enumerate()
            .map(|(ix, r)| {
                h_flex()
                    .id(SharedString::from(format!("{id}-route-{ix}")))
                    .test_support()
                    .gap(dp(6.))
                    .items_start()
                    .child(ui::status_mark(
                        format!("{id}-route-{ix}-state"),
                        r.tone,
                        r.state,
                        cx,
                    ))
                    .child(div().min_w_0().whitespace_normal().child(r.text.clone()))
                    .into_any_element()
            })
            .collect::<Vec<_>>();
        if routes.is_empty() {
            routes.push(self.line(
                format!("{id}-no-routes"),
                "No route names this Gateway.".into(),
                true,
                cx,
            ));
        }
        routes.extend(self.more_line(&format!("{id}-routes"), detail.routes_more, cx));
        sections.push(self.section("routes", &detail.routes_title, routes, cx));
        if !detail.contested.is_empty() {
            let mut lines = detail
                .contested
                .iter()
                .enumerate()
                .map(|(ix, words)| {
                    self.line(format!("{id}-contested-{ix}"), words.clone(), false, cx)
                })
                .collect::<Vec<_>>();
            lines.extend(self.more_line(&format!("{id}-contested"), detail.contested_more, cx));
            sections.push(self.section("contested", "Contested hostnames", lines, cx));
        }
        sections
    }

    fn class_sections(
        &self,
        row: &Row,
        detail: &ClassDetail,
        cx: &mut Context<Self>,
    ) -> Vec<AnyElement> {
        let id = &row.id;
        let status = self.status_lines(
            row,
            detail.state,
            &detail.reason,
            None,
            detail.says.as_ref(),
            cx,
        );
        let mut gateways = detail
            .gateways
            .iter()
            .enumerate()
            .map(|(ix, (line, state, tone))| {
                h_flex()
                    .id(SharedString::from(format!("{id}-gateway-{ix}")))
                    .test_support()
                    .gap(dp(6.))
                    .child(ui::status_mark(
                        format!("{id}-gateway-{ix}-state"),
                        *tone,
                        *state,
                        cx,
                    ))
                    .child(div().child(line.clone()))
                    .into_any_element()
            })
            .collect::<Vec<_>>();
        if gateways.is_empty() {
            gateways.push(self.line(
                format!("{id}-no-gateways"),
                "No Gateway uses this class.".into(),
                true,
                cx,
            ));
        }
        gateways.extend(self.more_line(&format!("{id}-gateways"), detail.gateways_more, cx));
        vec![
            self.section("status", "Status", status, cx),
            self.section("gateways", "Gateways", gateways, cx),
        ]
    }

    fn details(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let prefix = self.mode.prefix();
        let selected = self.selected.as_ref()?;
        let detail_id = SharedString::from(format!("{prefix}-detail"));
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
                    .id(SharedString::from(format!("{prefix}-detail-title")))
                    .aria_label(row.label.clone())
                    .test_support()
                    .truncate()
                    .child(row.label.clone()),
            )
            .child(
                div()
                    .id(SharedString::from(format!("{prefix}-detail-name")))
                    .test_support()
                    .text_size(dp(12.))
                    .text_color(p.muted)
                    .truncate()
                    .child(row.cells[0].clone()),
            );
        let sections = match &row.detail {
            Detail::Gateway(entry) => self.gateway_sections(row, entry, cx),
            Detail::Class(entry) => self.class_sections(row, entry, cx),
        };
        let open = h_flex().gap_2().child(
            Button::new(SharedString::from(format!("{prefix}-detail-open")))
                .small()
                .outline()
                .label(self.mode.open_label())
                .disabled(!row.openable)
                .on_click(cx.listener(|this, _, _, cx| this.open(cx))),
        );
        Some(
            Inspector::new(detail_id)
                .selection(selected)
                .heading(heading)
                .children(sections)
                .footer(Some(open))
                .render(cx)
                .into_any_element(),
        )
    }

    fn body(&self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let prefix = self.mode.prefix();
        let table = div()
            .id(SharedString::from(format!("{prefix}-table")))
            .test_support()
            .size_full()
            .flex()
            .flex_col()
            .min_w_0()
            .min_h_0()
            .child(DataTable::new().render(self, window, cx).flex_1().min_h_0())
            .into_any_element();
        inspector::split(
            SharedString::from(format!("{prefix}-split")),
            &self.split,
            super::super::page_width(window) >= inspector::SPLIT_WIDTH,
            table,
            self.details(cx),
            window,
        )
    }
}

impl Render for GatewaysScreen {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.sync();
        let prefix = self.mode.prefix();
        let loading = self.loader.is_loading() && self.loader.data().is_none();
        self.loading.show(loading);
        let header = self.header(window, cx);
        // The page's own list refused is an error with Retry; so is a read
        // that failed outright. Never an empty table.
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
            Some(format!(
                "Waiting to read {}",
                self.mode.title().to_lowercase()
            ))
        } else {
            None
        };
        let notices = self
            .display
            .notices
            .iter()
            .enumerate()
            .map(|(ix, words)| {
                ui::warning_banner(Some("Read coverage".into()), words.clone(), None, cx)
                    .id(SharedString::from(format!("{prefix}-notice-{ix}")))
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
                .id(SharedString::from(format!("{prefix}-stale")))
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
                        IconName::Network,
                        self.mode.title(),
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
                    .id(SharedString::from(format!("{prefix}-state")))
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
            .key_context(self.mode.context())
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
            .on_action(cx.listener(|this, _: &Find, window, cx| {
                this.filter_input
                    .update(cx, |input, cx| input.focus(window, cx))
            }))
            .child(
                page::page(SharedString::from(format!("{prefix}-page")))
                    .overflow_y_scroll()
                    .restrict_scroll_to_axis()
                    .child(page::toolbar(cx).child(header))
                    .when(has_notes, |this| {
                        this.child(
                            page::capped_notes(
                                SharedString::from(format!("{prefix}-notices")),
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
