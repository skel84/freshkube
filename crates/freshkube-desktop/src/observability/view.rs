//! Shared Fog presentation, with normal Kit controls for interactive content.
use super::*;

pub(super) fn ink(status: Status, cx: &App) -> Hsla {
    let p = palette(cx);
    match status {
        Status::Ok => p.good,
        Status::Warning => p.warn_ink,
        Status::Critical => p.crit_ink,
        Status::Unknown | Status::Absent => p.muted,
        Status::Integration => p.integration,
        Status::LogError | Status::Info => p.accent,
    }
}
pub(super) fn status(state: Status, cx: &App) -> AnyElement {
    let p = palette(cx);
    match state {
        Status::Integration => div()
            .size(dp(9.))
            .border(px(1.5))
            .border_color(p.integration)
            .rounded(px(1.))
            .flex_none()
            .into_any_element(),
        Status::Absent => text("—").text_color(p.muted).into_any_element(),
        Status::LogError | Status::Info => div()
            .size(dp(8.))
            .rounded_full()
            .bg(p.accent)
            .flex_none()
            .into_any_element(),
        _ => ui::status_glyph(
            match state {
                Status::Ok => Tone::Good,
                Status::Warning => Tone::Warn,
                Status::Critical => Tone::Crit,
                _ => Tone::Unknown,
            },
            cx,
        )
        .unwrap(),
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
pub(super) fn section(title: impl Into<SharedString>) -> Div {
    h_flex().gap(dp(12.)).min_w_0().child(ui::page_title(title))
}
pub(super) fn pair(label: &'static str, value: impl Into<SharedString>, cx: &App) -> Div {
    line()
        .child(muted(label, cx).w(dp(112.)).flex_none())
        .child(mono(value).flex_1())
}

impl ObservabilityPage {
    pub(super) fn render_unavailable(&self, cx: &Context<Self>) -> AnyElement {
        v_flex().id("obs-integration-required").test_support().gap(dp(16.)).max_w(dp(600.)).pt(dp(60.))
            .child(line().child(status(Status::Integration,cx)).child(ui::page_title("Integration required")))
            .child(text("Connect Coroot to inspect application health, service dependencies and supported report evidence."))
            .child(muted("Enter a reachable Coroot URL, connect, and explicitly choose a project above.",cx))
            .child(action("obs-open-dashboards","Open Prometheus dashboards").on_click(cx.listener(|_,_,_,cx|cx.emit(ObservabilityEvent::Dashboards)))).into_any_element()
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
            .child(ui::page_title(
                app.map_or("Select an application", |a| a.name())
                    .to_string(),
            ))
            .child(status(
                self.selected_application()
                    .map_or(Status::Unknown, |a| a.status),
                cx,
            ))
    }
}
