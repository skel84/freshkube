use super::*;
use crate::resources::route_trace::verdict_tone;
use display::{Detail, Row};
use freshkube_core::resources::traffic::routes::{ClassEntry, GatewayEntry, Owner};
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

fn owner_words(owner: Option<Owner>) -> Option<&'static str> {
    owner.map(|owner| match owner {
        Owner::Platform => "the platform's",
        Owner::Yours => "yours",
        Owner::Controller => "the controller's",
        Owner::ThisMachine => "this machine's",
    })
}

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

    fn gateway_sections(
        &self,
        row: &Row,
        entry: &GatewayEntry,
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
                .child(div().child(entry.state.label()))
                .into_any_element(),
            self.line(format!("{id}-reason"), entry.reason.clone(), false, cx),
        ];
        if let Some(whose) = owner_words(entry.owner) {
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
        if let Some(says) = &entry.says {
            status.push(
                ui::named_field(
                    "Controller reports".into(),
                    div().whitespace_normal().child(says.clone()),
                    cx,
                )
                .into_any_element(),
            );
        }
        let mut sections = vec![self.section("status", "Status", status, cx)];
        let listeners = entry
            .listeners
            .iter()
            .enumerate()
            .map(|(ix, l)| {
                let host = l.hostname.as_deref().unwrap_or("any host");
                let attached = match l.attached {
                    Some(n) => format!("{n} attached"),
                    None => "no status from the controller".to_owned(),
                };
                let mut words = vec![
                    format!("{} · {}/{} · {host}", l.name, l.port, l.protocol),
                    format!("allows {}", l.allows),
                    format!("{} · {attached}", l.state.label()),
                ];
                if let Some(says) = &l.says {
                    words.push(format!("controller: {says}"));
                }
                if !l.certificates.is_empty() {
                    words.push(format!(
                        "certificate {} (by name only)",
                        l.certificates.join(", ")
                    ));
                }
                v_flex()
                    .id(SharedString::from(format!("{id}-listener-{ix}")))
                    .test_support()
                    .gap(dp(1.))
                    .children(words.into_iter().enumerate().map(|(n, w)| {
                        div()
                            .whitespace_normal()
                            .min_w_0()
                            .when(n > 0, |this| this.text_color(palette(cx).muted))
                            .child(w)
                    }))
                    .into_any_element()
            })
            .collect::<Vec<_>>();
        if listeners.is_empty() {
            sections.push(self.section(
                "listeners",
                "Listeners",
                vec![self.line(
                    format!("{id}-no-listeners"),
                    "None declared.".into(),
                    true,
                    cx,
                )],
                cx,
            ));
        } else {
            sections.push(self.section("listeners", "Listeners", listeners, cx));
        }
        let routes = entry
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
                        verdict_tone(r.verdict),
                        r.verdict.label(),
                        cx,
                    ))
                    .child(
                        div()
                            .min_w_0()
                            .whitespace_normal()
                            .child(format!("{}/{} — {}", r.namespace, r.name, r.line)),
                    )
                    .into_any_element()
            })
            .collect::<Vec<_>>();
        sections.push(if routes.is_empty() {
            self.section(
                "routes",
                "Routes",
                vec![self.line(
                    format!("{id}-no-routes"),
                    "No route names this Gateway.".into(),
                    true,
                    cx,
                )],
                cx,
            )
        } else {
            self.section(
                "routes",
                &format!("Routes · {}", entry.routes_line()),
                routes,
                cx,
            )
        });
        if !entry.contested.is_empty() {
            let lines = entry
                .contested
                .iter()
                .enumerate()
                .map(|(ix, c)| {
                    self.line(
                        format!("{id}-contested-{ix}"),
                        format!("{} is also named by {}", c.host, c.also.join(", ")),
                        false,
                        cx,
                    )
                })
                .collect();
            sections.push(self.section("contested", "Contested hostnames", lines, cx));
        }
        sections
    }

    fn class_sections(
        &self,
        row: &Row,
        entry: &ClassEntry,
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
                .child(div().child(entry.state.label()))
                .into_any_element(),
            self.line(format!("{id}-reason"), entry.reason.clone(), false, cx),
        ];
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
        if let Some(says) = &entry.says {
            status.push(
                ui::named_field(
                    "Controller reports".into(),
                    div().whitespace_normal().child(says.clone()),
                    cx,
                )
                .into_any_element(),
            );
        }
        let gateways = entry
            .gateways
            .iter()
            .enumerate()
            .map(|(ix, (key, state))| {
                h_flex()
                    .id(SharedString::from(format!("{id}-gateway-{ix}")))
                    .test_support()
                    .gap(dp(6.))
                    .child(ui::status_mark(
                        format!("{id}-gateway-{ix}-state"),
                        display::gateway_tone(*state),
                        state.label(),
                        cx,
                    ))
                    .child(div().child(format!("{key} · {}", state.label())))
                    .into_any_element()
            })
            .collect::<Vec<_>>();
        vec![
            self.section("status", "Status", status, cx),
            if gateways.is_empty() {
                self.section(
                    "gateways",
                    "Gateways",
                    vec![self.line(
                        format!("{id}-no-gateways"),
                        "No Gateway uses this class.".into(),
                        true,
                        cx,
                    )],
                    cx,
                )
            } else {
                self.section("gateways", "Gateways", gateways, cx)
            },
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
