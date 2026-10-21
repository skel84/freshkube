use super::*;
use freshkube_ui::{
    inspector::{self, Inspector},
    page::{self, PageHeader},
    table::DataTable,
};
use gpui_kit::assets::IconName;
use gpui_kit::component::{
    button::{Button, ButtonVariants},
    h_flex,
    input::Input,
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
        let namespace = div().w(dp(170.)).child(
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
        header
            .filter(namespace)
            .control(modes)
            .control(open)
            .control(refresh)
            .render(window, cx)
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
        Some(
            Inspector::new("databases-detail")
                .heading(
                    div()
                        .id("databases-detail-title")
                        .aria_label(row.identity.name.clone())
                        .test_support()
                        .child(row.identity.name.clone()),
                )
                .children(row.fields.iter().map(|(label, value)| {
                    freshkube_ui::ui::named_field(
                        label.clone().into(),
                        ui::mono(value.clone()).whitespace_normal(),
                        cx,
                    )
                }))
                .render(cx)
                .into_any_element(),
        )
    }
    pub(super) fn retry(&self, cx: &mut Context<Self>) -> Button {
        Button::new("databases-retry")
            .small()
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
            .on_action(cx.listener(|this, _: &Find, window, cx| {
                this.namespace_input
                    .update(cx, |input, cx| input.focus(window, cx))
            }))
            .child(
                page::page("databases-page")
                    .overflow_y_scroll()
                    .restrict_scroll_to_axis()
                    .child(page::toolbar(cx).child(header))
                    .when(notice.is_some() || failure.is_some(), |this| {
                        this.child(page::inset().children(notice).children(failure))
                    })
                    .child(body),
            )
    }
}
