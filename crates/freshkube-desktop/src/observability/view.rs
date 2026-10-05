//! Shared Fog presentation, with normal Kit controls for interactive content.
use super::*;

pub(super) fn status(state: Status, cx: &App) -> AnyElement {
    let p = palette(cx);
    match state {
        Status::Absent => text("—").text_color(p.muted).into_any_element(),
        Status::LogError | Status::Info => div()
            .size(dp(8.))
            .rounded_full()
            .bg(p.accent)
            .flex_none()
            .into_any_element(),
        _ => div()
            .children(ui::status_glyph(
                match state {
                    Status::Ok => Tone::Good,
                    Status::Warning => Tone::Warn,
                    Status::Critical => Tone::Crit,
                    _ => Tone::Unknown,
                },
                cx,
            ))
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
    pub(super) fn breadcrumbs(
        &self,
        root: &'static str,
        destination: Destination,
        cx: &Context<Self>,
    ) -> Div {
        let app = self.selected_app.as_ref();
        line()
            .child(
                Button::new("obs-breadcrumb")
                    .ghost()
                    .group("fog-control")
                    .small()
                    .label(root)
                    .text_color(palette(cx).accent)
                    .on_click(cx.listener(move |this, _, _, cx| this.open(destination, cx))),
            )
            .child(muted("/", cx))
            .child(muted(
                app.and_then(|a| a.namespace())
                    .unwrap_or("External / unmapped")
                    .to_string(),
                cx,
            ))
            .child(muted("/", cx))
            .child(status(
                self.selected_application()
                    .map_or(Status::Unknown, |a| a.status),
                cx,
            ))
    }
}

/// An application as the pickers name it: namespace / name.
/// From this content width a list's detail sits beside it.
const SPLIT_WIDTH: f32 = 900.;
const PANE_WIDTH: f32 = 460.;
const PANE_MIN_WIDTH: f32 = 320.;
const SPLIT_GAP: f32 = 14.;

/// Whether a list's detail fits beside it.
pub(super) fn beside(window: &Window) -> bool {
    crate::screens::content_width(window) >= SPLIT_WIDTH
}

/// A list with its detail beside it on a wide page and below it on a narrow
/// one, at DESIGN.md's detail pane sizes. The pane's size is fixed until
/// change 9's Inspector.
pub(super) fn split(
    id: &'static str,
    beside: bool,
    table: AnyElement,
    pane: Option<AnyElement>,
) -> AnyElement {
    div()
        .id(id)
        .test_support()
        .flex()
        .items_start()
        .gap(dp(SPLIT_GAP))
        .when_else(beside, |this| this.flex_row(), |this| this.flex_col())
        .child(
            div()
                .min_w_0()
                .when_else(beside, |this| this.flex_1(), |this| this.w_full())
                .child(table),
        )
        .children(pane.map(|pane| {
            div()
                .when_else(
                    beside,
                    |this| this.w(dp(PANE_WIDTH)).min_w(dp(PANE_MIN_WIDTH)).flex_none(),
                    |this| this.w_full(),
                )
                .child(pane)
        }))
        .into_any_element()
}

pub(super) fn app_label(id: &freshkube_core::coroot::AppId) -> String {
    match id.namespace() {
        Some(namespace) if !namespace.is_empty() => format!("{namespace} / {}", id.name()),
        _ => id.name().to_string(),
    }
}

impl ObservabilityPage {
    /// A live evidence page's application picker and a way back to its report.
    pub(super) fn evidence_header(&self, id: &'static str, cx: &Context<Self>) -> Div {
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
