//! Shared Fog presentation, with normal Kit controls for interactive content.
use super::*;

/// A status's glyph tone; an absent signal has none.
pub(super) fn tone(state: Status) -> Option<Tone> {
    match state {
        Status::Absent => None,
        Status::Ok => Some(Tone::Good),
        Status::Warning => Some(Tone::Warn),
        Status::Critical => Some(Tone::Crit),
        Status::LogError | Status::Info => Some(Tone::Info),
        _ => Some(Tone::Unknown),
    }
}
pub(super) fn status(state: Status, cx: &App) -> AnyElement {
    match tone(state) {
        None => text("—").text_color(palette(cx).muted).into_any_element(),
        Some(tone) => div()
            .children(ui::status_glyph(tone, cx))
            .into_any_element(),
    }
}
pub(super) fn text(value: impl Into<SharedString>) -> Div {
    div().min_w_0().child(value.into())
}
pub(super) fn mono(value: impl Into<SharedString>) -> Div {
    text(value).font_family(MONO_FONT).text_size(dp(12.5))
}
pub(super) fn muted(value: impl Into<SharedString>, cx: &App) -> Div {
    let p = palette(cx);
    text(value)
        .text_color(p.muted)
        .text_size(dp(12.))
        .group_hover("fog-control", |style| style.text_color(p.ink_2))
}
pub(super) fn card(title: impl Into<SharedString>, cx: &App) -> Div {
    let p = palette(cx);
    v_flex()
        .min_w_0()
        .rounded(px(12.))
        .bg(p.surface)
        .border_1()
        .border_color(p.line)
        .child(
            h_flex().h(dp(42.)).px(dp(14.)).flex_none().child(
                text(title)
                    .font_weight(ui::HEADING_WEIGHT)
                    .text_size(dp(14.)),
            ),
        )
}
/// Whether an inspector fits beside its table on this edge-to-edge page.
pub(super) fn inspector_beside(window: &Window) -> bool {
    freshkube_ui::page::page_width(window) >= freshkube_ui::inspector::SPLIT_WIDTH
}
impl ObservabilityPage {
    /// Whether a short window scrolls the frame around an inspector's
    /// split: when the inspector stacks under its table, and always on
    /// Traces, whose heatmap sits above it. Otherwise the split fills.
    pub(super) fn inspector_scrolls(&self, window: &Window) -> bool {
        freshkube_ui::page::is_short(window)
            && (!inspector_beside(window) || self.destination == Destination::Traces)
    }

    /// `split` as tall as the frame leaves it, or, while the frame
    /// scrolls, as tall as `inspector::short_height`.
    pub(super) fn sized_split(&self, split: AnyElement, open: bool, window: &Window) -> AnyElement {
        if !self.inspector_scrolls(window) {
            return split;
        }
        let height = freshkube_ui::inspector::short_height(inspector_beside(window), open);
        div()
            .flex()
            .flex_none()
            .h(dp(height))
            .child(split)
            .into_any_element()
    }
}
pub(super) fn body() -> Div {
    v_flex().p(dp(14.)).gap(dp(12.)).min_w_0()
}
pub(super) fn line() -> Div {
    h_flex().gap(dp(8.)).min_w_0()
}
pub(super) fn action(id: impl Into<ElementId>, label: impl Into<SharedString>) -> Button {
    Button::new(id)
        .outline()
        .group("fog-control")
        .small()
        .label(label)
}
pub(super) fn pair(label: &'static str, value: impl Into<SharedString>, cx: &App) -> Div {
    line()
        .child(muted(label, cx).w(dp(112.)).flex_none())
        .child(mono(value).flex_1())
}

impl ObservabilityPage {
    /// The one empty state before Coroot answers: what it adds, then the
    /// form or project picker that gets there.
    pub(super) fn render_unavailable(&self, cx: &Context<Self>) -> AnyElement {
        v_flex()
            .id("obs-integration-required")
            .test_support()
            .role(Role::Status)
            .aria_label("Coroot connection required")
            .gap(dp(16.))
            .child(ui::empty_state(IconName::Waypoints, "Connect Coroot",
                "Application health, service dependencies and report evidence, read from your Coroot server.",
                None, vec![], cx).h_auto())
            .child(self.render_connection(cx))
            .child(line()
                .child(muted("Prometheus dashboards work without Coroot.", cx))
                .child(Button::new("obs-open-dashboards").ghost().small().label("Open dashboards")
                    .on_click(cx.listener(|_, _, _, cx| cx.emit(ObservabilityEvent::Dashboards)))))
            .into_any_element()
    }
}

/// An application as the pickers name it: namespace / name.
/// From this content width a list's detail sits beside it.
pub(super) fn app_label(id: &freshkube_core::coroot::AppId) -> String {
    match id.namespace() {
        Some(namespace) if !namespace.is_empty() => format!("{namespace} / {}", id.name()),
        _ => id.name().to_string(),
    }
}

impl ObservabilityPage {
    /// A live evidence page's application picker and a way back to its report.
    pub(super) fn evidence_header(&self, id: &'static str, cx: &Context<Self>) -> Div {
        // Embedded in an application's report, the page already names it.
        if self.destination == Destination::Application {
            return div();
        }
        // Kit's Select fills its parent, so a box sets its size in the row.
        let picker = div().w(dp(300.)).flex_none().child(
            Select::new(&self.app_select)
                .id(id)
                .small()
                .menu_width(dp(420.))
                .placeholder("Choose an application")
                .search_placeholder("Find an application")
                .accessibility_label("Application")
                .disabled(self.app_choices.is_empty()),
        );
        line()
            .flex_wrap()
            .child(picker)
            .when_some(self.selected_application(), |this, app| {
                this.child(status(app.status, cx))
            })
            .child(div().flex_1())
            .when(self.selected_app.is_some(), |this| {
                this.child(
                    action(
                        SharedString::from(format!("{id}-report")),
                        "Application report",
                    )
                    .on_click(
                        cx.listener(|this, _, _, cx| this.open(Destination::Application, cx)),
                    ),
                )
            })
    }

    /// Brings the application picker up to date with the applications and
    /// the selection. Runs from render, and does work only after a change.
    pub(super) fn sync_app_select(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let stale = !Rc::ptr_eq(&self.app_select_source, &self.app_choices);
        let selected = self.app_select.read(cx).selected_value().cloned();
        if !stale && selected == self.selected_app {
            return;
        }
        let items = SearchableVec::new(
            self.app_choices
                .iter()
                .map(|(value, label)| AppChoice {
                    label: label.clone(),
                    value: value.clone(),
                })
                .collect::<Vec<_>>(),
        );
        self.app_select_source = self.app_choices.clone();
        let current = self.selected_app.clone();
        self.app_select.update(cx, |state, cx| {
            if stale {
                state.set_items(items, window, cx);
            }
            match &current {
                Some(app) => state.set_selected_value(app, window, cx),
                None => state.set_selected_index(None, window, cx),
            }
        });
    }

    pub(super) fn choose_app(&mut self, id: freshkube_core::coroot::AppId, cx: &mut Context<Self>) {
        if self.selected_app.as_ref() == Some(&id) {
            return;
        }
        self.selected_app = Some(id);
        self.report_snapshot = None;
        self.app_page = None;
        self.forget_wanted();
        self.refresh(cx);
    }
}

/// One application in the Traces and Profiling picker.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct AppChoice {
    label: SharedString,
    value: freshkube_core::coroot::AppId,
}

impl SelectItem for AppChoice {
    type Value = freshkube_core::coroot::AppId;

    fn title(&self) -> SharedString {
        self.label.clone()
    }

    fn value(&self) -> &Self::Value {
        &self.value
    }
}
