use super::*;
use crate::resources::route_trace;
use freshkube_ui::{
    inspector::{self, Inspector},
    page::{self, PageHeader},
    palette::palette,
    table::{DataTable, TableSource},
};
use gpui_kit::assets::IconName;
use gpui_kit::component::{
    Disableable, Sizable,
    button::{Button, ButtonVariants},
    h_flex,
    input::Input,
    v_flex,
};

impl RoutesScreen {
    fn header(&self, window: &mut Window, cx: &mut Context<Self>) -> Div {
        let filter = div().w(dp(190.)).child(
            Input::new(&self.filter_input)
                .id("routes-filter")
                .aria_label("Filter by host, name, gateway, backend or where the trace stops")
                .small()
                .h(dp(ui::CONTROL_HEIGHT)),
        );
        let trace = Button::new("routes-open-trace")
            .ghost()
            .small()
            .h(dp(ui::CONTROL_HEIGHT))
            .label("Open trace")
            .disabled(self.selected.is_none())
            .on_click(cx.listener(|this, _, _, cx| this.open(crate::resources::Tab::Trace, cx)));
        let route = Button::new("routes-open-route")
            .ghost()
            .small()
            .h(dp(ui::CONTROL_HEIGHT))
            .label("Open route")
            .disabled(self.selected.is_none())
            .on_click(cx.listener(|this, _, _, cx| this.open(crate::resources::Tab::Overview, cx)));
        let refresh = Button::new("routes-refresh")
            .ghost()
            .small()
            .h(dp(ui::CONTROL_HEIGHT))
            .icon(ui::refresh_icon(self.loader.is_loading(), cx))
            .accessibility_label("Refresh routes")
            .tooltip("Refresh routes")
            .disabled(self.source.is_none() || self.loader.is_loading())
            .on_click(cx.listener(|this, _, _, cx| this.read(cx)));
        PageHeader::new(PREFIX, "Routes")
            .filter(filter)
            .control(trace)
            .control(route)
            .control(refresh)
            .render(window, cx)
    }

    fn details(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let selected = self.selected.as_ref()?;
        let Some(row) = self.display.rows.iter().find(|row| &row.key == selected) else {
            return Some(
                Inspector::new("routes-detail")
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
                    .id("routes-detail-title")
                    .aria_label(row.label.clone())
                    .test_support()
                    .truncate()
                    .child(row.label.clone()),
            )
            .child(
                div()
                    .id("routes-detail-name")
                    .test_support()
                    .text_size(dp(12.))
                    .text_color(p.muted)
                    .truncate()
                    .child(row.cells[1].clone()),
            );
        let fields = v_flex()
            .id("routes-detail-fields")
            .test_support()
            .gap(dp(7.))
            .text_size(dp(12.))
            .child(ui::caption("Route", cx))
            .children(row.fields.iter().map(|field| {
                freshkube_ui::ui::named_field(
                    field.label.into(),
                    div().whitespace_normal().child(field.value.clone()),
                    cx,
                )
            }));
        let open =
            h_flex()
                .gap_2()
                .child(
                    Button::new("routes-detail-trace")
                        .small()
                        .outline()
                        .label("Open trace")
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.open(crate::resources::Tab::Trace, cx)
                        })),
                )
                .child(
                    Button::new("routes-detail-route")
                        .small()
                        .ghost()
                        .label("Open route")
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.open(crate::resources::Tab::Overview, cx)
                        })),
                );
        Some(
            Inspector::new("routes-detail")
                .selection(selected)
                .heading(heading)
                .child(route_trace::compact(&row.model, cx))
                .children(route_trace::backends(&row.model, cx))
                .children(route_trace::proposals(&row.model, cx))
                .child(fields)
                .footer(Some(open))
                .render(cx)
                .into_any_element(),
        )
    }

    fn body(&self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let table = div()
            .id("routes-table")
            .test_support()
            .size_full()
            .flex()
            .flex_col()
            .min_w_0()
            .min_h_0()
            .child(DataTable::new().render(self, window, cx).flex_1().min_h_0())
            .into_any_element();
        inspector::split(
            "routes-split",
            &self.split,
            super::super::page_width(window) >= inspector::SPLIT_WIDTH,
            table,
            self.details(cx),
            window,
        )
    }
}

impl Render for RoutesScreen {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.sync();
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
                    .unwrap_or_else(|| "Waiting to read routes".into()),
            )
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
                    .id(SharedString::from(format!("routes-notice-{ix}")))
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
                .id("routes-stale")
                .test_support()
            });
        let empty = !loading && self.display.lines.is_empty();
        let short = page::is_short(window);
        let body = if let Some(words) = state {
            page::inset()
                .child(
                    ui::empty_state(
                        IconName::Network,
                        "Routes",
                        words,
                        None,
                        if self.loader.error().is_some() && self.source.is_some() {
                            vec![self.retry(cx).into_any_element()]
                        } else {
                            Vec::new()
                        },
                        cx,
                    )
                    .h_auto(),
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
            .key_context(CONTEXT)
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
            .on_action(
                cx.listener(|this, _: &OpenTrace, _, cx| {
                    this.open(crate::resources::Tab::Trace, cx)
                }),
            )
            .on_action(cx.listener(|this, _: &OpenRoute, _, cx| {
                this.open(crate::resources::Tab::Overview, cx)
            }))
            .on_action(cx.listener(|this, _: &Find, window, cx| {
                this.filter_input
                    .update(cx, |input, cx| input.focus(window, cx))
            }))
            .child(
                page::page("routes-page")
                    .overflow_y_scroll()
                    .restrict_scroll_to_axis()
                    .child(page::toolbar(cx).child(header))
                    .when(has_notes, |this| {
                        this.child(
                            page::capped_notes(
                                "routes-notices",
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
