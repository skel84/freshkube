use super::*;
use freshkube_ui::{
    inspector::{self, Inspector},
    page::{self, PageHeader},
    palette::palette,
    table::DataTable,
};
use gpui_kit::assets::IconName;
use gpui_kit::component::{
    button::{Button, ButtonVariants},
    h_flex,
    input::Input,
    v_flex,
};

impl DatabasesScreen {
    fn header(&self, window: &mut Window, cx: &mut Context<Self>) -> Div {
        let header = PageHeader::new(PREFIX, "Databases");
        let modes = h_flex().gap_1().children(
            [Mode::Clusters, Mode::Backups, Mode::Schedules]
                .into_iter()
                .map(|mode| {
                    ui::segment(
                        Button::new(format!("databases-view-{}", mode.label().to_lowercase()))
                            .small()
                            .h(dp(ui::CONTROL_HEIGHT))
                            .label(mode.label())
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.mode(mode, cx);
                                window.focus(&this.focus, cx);
                            })),
                        self.mode == mode,
                        cx,
                    )
                }),
        );
        let filter = div().w(dp(170.)).child(
            Input::new(&self.filter_input)
                .id("databases-filter")
                .aria_label("Filter by name, namespace, engine, primary or state")
                .small()
                .h(dp(ui::CONTROL_HEIGHT)),
        );
        let namespace = div().w(dp(170.)).h(dp(ui::CONTROL_HEIGHT)).child(
            Input::new(&self.namespace_input)
                .id("databases-namespace")
                .aria_label("Namespace; blank reads all namespaces; Enter applies")
                .small()
                .h(dp(ui::CONTROL_HEIGHT)),
        );
        let refresh = Button::new("databases-refresh")
            .ghost()
            .small()
            .h(dp(ui::CONTROL_HEIGHT))
            .icon(ui::refresh_icon(self.loader.is_loading(), cx))
            .accessibility_label("Refresh databases")
            .tooltip("Refresh databases")
            .disabled(self.source.is_none() || self.loader.is_loading())
            .on_click(cx.listener(|this, _, _, cx| this.read(cx)));
        let open = Button::new("databases-open")
            .ghost()
            .small()
            .h(dp(ui::CONTROL_HEIGHT))
            .label("Open in Resources")
            .disabled(self.selected.is_none())
            .on_click(cx.listener(|this, _, _, cx| this.open(cx)));
        let observe = Button::new("databases-open-observability")
            .ghost()
            .small()
            .h(dp(ui::CONTROL_HEIGHT))
            .label("Open in Observability")
            .disabled(self.selected_observe().is_none())
            .on_click(cx.listener(|this, _, _, cx| this.open_observability(cx)));
        header
            .filter(filter)
            .control(namespace)
            .control(modes)
            .control(open)
            .control(observe)
            .control(refresh)
            .render(window, cx)
    }

    /// The verdict, then why and what next; the groups follow it.
    fn verdict(&self, row: &display::Row, cx: &mut Context<Self>) -> AnyElement {
        let p = palette(cx);
        let verdict = &row.verdict;
        let lines = |title: &'static str, lines: &[SharedString], cx: &mut Context<Self>| {
            (!lines.is_empty()).then(|| {
                v_flex()
                    .id(SharedString::from(format!(
                        "databases-detail-{}",
                        title.to_lowercase()
                    )))
                    .test_support()
                    .gap(dp(4.))
                    .text_size(dp(12.))
                    .child(ui::caption(title, cx))
                    .children(
                        lines
                            .iter()
                            .cloned()
                            .map(|line| div().text_color(p.ink_2).whitespace_normal().child(line)),
                    )
            })
        };
        v_flex()
            .id("databases-detail-verdict")
            .test_support()
            .aria_label(verdict.headline.clone())
            .gap(dp(8.))
            .child(
                h_flex()
                    .gap(dp(8.))
                    .items_start()
                    .children(ui::status_glyph(verdict.tone, cx))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .font_weight(FontWeight::MEDIUM)
                            .whitespace_normal()
                            .child(verdict.headline.clone()),
                    )
                    .children(verdict.source.map(|source| {
                        ui::tag(ui::Tone::Outline, None, source, cx)
                            .id("databases-detail-source")
                            .aria_label(source)
                            .test_support()
                    })),
            )
            .children(lines("Why", &verdict.why, cx))
            .children(lines("Next", &verdict.next, cx))
            .into_any_element()
    }

    fn section(&self, section: &display::Section, cx: &mut Context<Self>) -> AnyElement {
        let p = palette(cx);
        v_flex()
            .id(SharedString::from(format!(
                "databases-detail-section-{}",
                section.id
            )))
            .test_support()
            .gap(dp(7.))
            .text_size(dp(12.))
            .child(ui::caption(section.title, cx))
            .children(section.fields.iter().map(|field| {
                let value = if field.monospace {
                    ui::mono(field.value.clone())
                } else {
                    div().child(field.value.clone())
                }
                .whitespace_normal();
                let value = match (field.tone, field.dim) {
                    (_, true) => value.text_color(p.muted),
                    (Some(ui::Tone::Crit), _) => value.text_color(p.crit_ink),
                    (Some(ui::Tone::Warn), _) => value.text_color(p.warn_ink),
                    (Some(ui::Tone::Good), _) => value.text_color(p.good_ink),
                    _ => value,
                };
                freshkube_ui::ui::named_field(field.label.clone().into(), value, cx)
            }))
            .into_any_element()
    }

    fn details(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let selected = self.selected.as_ref()?;
        let Some(row) = self.display.rows.iter().find(|row| &row.key == selected) else {
            return Some(
                Inspector::new("databases-detail")
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
                    .id("databases-detail-title")
                    .aria_label(row.identity.name.clone())
                    .test_support()
                    .truncate()
                    .child(row.identity.name.clone()),
            )
            // The engine shows here when the table has no room for its column.
            .children((!self.display.engine_shown()).then(|| {
                div()
                    .id("databases-detail-engine")
                    .aria_label(row.engine.clone())
                    .test_support()
                    .text_size(dp(12.))
                    .text_color(p.muted)
                    .truncate()
                    .child(row.engine.clone())
            }));
        let observe = row.observe.is_some().then(|| {
            Button::new("databases-detail-observe")
                .small()
                .outline()
                .label("Open in Observability")
                .on_click(cx.listener(|this, _, _, cx| this.open_observability(cx)))
        });
        Some(
            Inspector::new("databases-detail")
                .heading(heading)
                .child(self.verdict(row, cx))
                .children(row.sections.iter().map(|section| self.section(section, cx)))
                .footer(observe)
                .render(cx)
                .into_any_element(),
        )
    }
    pub(super) fn retry(&self, cx: &mut Context<Self>) -> Button {
        Button::new("databases-retry")
            .icon(IconName::RefreshCw)
            .label("Retry")
            .on_click(cx.listener(|this, _, _, cx| this.read(cx)))
    }
    fn body(&self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let table = div()
            .id("databases-table")
            .test_support()
            .size_full()
            .flex()
            .flex_col()
            .min_w_0()
            .min_h_0()
            .child(DataTable::new().render(self, window, cx).flex_1().min_h_0())
            .into_any_element();
        inspector::split(
            "databases-split",
            &self.split,
            super::super::page_width(window) >= inspector::SPLIT_WIDTH,
            table,
            self.details(cx),
            window,
        )
    }
}
impl Render for DatabasesScreen {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.sync();
        // With the Inspector open the table has less room: the engine then
        // moves into the Inspector's header.
        let engine = self.selected.is_none()
            || super::super::page_width(window) >= table::ENGINE_COLUMN_WIDTH;
        self.display.set_engine_shown(engine);
        let loading = self.loader.is_loading() && self.loader.data().is_none();
        self.loading.show(loading);
        let header = self.header(window, cx);
        let state = if self.source.is_none() {
            Some("Not connected".to_owned())
        } else if self.loader.data().is_none() && !loading {
            Some(
                self.loader
                    .error()
                    .map(ToString::to_string)
                    .unwrap_or_else(|| "Waiting to read databases".into()),
            )
        } else {
            None
        };
        let notice = self.display.notice.as_ref().map(|words| {
            ui::warning_banner(Some("Read coverage".into()), words.clone(), None, cx)
                .id("databases-coverage")
                .aria_label(words.clone())
                .test_support()
        });
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
                .id("databases-stale")
                .test_support()
            });
        let body = if let Some(words) = state {
            page::inset()
                .child(ui::empty_state(
                    IconName::Database,
                    "Databases",
                    words,
                    None,
                    if self.loader.error().is_some() && self.source.is_some() {
                        vec![self.retry(cx).into_any_element()]
                    } else {
                        Vec::new()
                    },
                    cx,
                ))
                .into_any_element()
        } else {
            self.body(window, cx)
        };
        let body = div()
            .key_context(CONTEXT)
            .track_focus(&self.focus)
            .flex()
            .flex_col()
            .flex_1()
            .min_h_0()
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
            .on_action(cx.listener(|this, _: &OpenResource, _, cx| this.open(cx)))
            .on_action(
                cx.listener(|this, _: &OpenObservability, _, cx| this.open_observability(cx)),
            )
            .on_action(cx.listener(|this, _: &Find, window, cx| {
                this.filter_input
                    .update(cx, |input, cx| input.focus(window, cx))
            }))
            .child(
                page::page("databases-page")
                    .overflow_y_scroll()
                    .restrict_scroll_to_axis()
                    .child(page::toolbar(cx).child(header))
                    .when(notice.is_some() || failure.is_some(), |this| {
                        // Capped, so a long coverage line leaves the table and
                        // the Inspector their room in a short window.
                        this.child(
                            page::inset()
                                .max_h(dp(72.))
                                .id("databases-notices")
                                .test_support()
                                .overflow_y_scroll()
                                .children(notice)
                                .children(failure),
                        )
                    })
                    .child(body),
            )
    }
}
