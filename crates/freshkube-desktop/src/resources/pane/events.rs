//! The Events tab: the events recorded about the object, newest first.

use freshkube_core::resources::ObjectEvent;
use gpui_kit::assets::IconName;
use gpui_kit::component::{h_flex, v_flex};
use gpui_kit::prelude::*;
use gpui_kit::*;

use super::{DetailPane, local_time};
use crate::palette::palette;
use crate::resources::detail::{Detail, EventsRead, MAX_EVENTS};
use crate::ui::{self, Tone, dp};

/// One event as listed, derived when the events change.
pub(super) struct EventLine {
    warning: bool,
    /// What a screen reader says for the row.
    label: SharedString,
    reason: SharedString,
    message: SharedString,
    detail: SharedString,
}

impl EventLine {
    pub(super) fn new(event: &ObjectEvent) -> Self {
        let when = match (event.count, event.last_seen.or(event.first_seen)) {
            (count, Some(last)) if count > 1 => {
                format!("{count} times, last at {}", local_time(last))
            }
            (_, Some(last)) => local_time(last),
            (count, None) if count > 1 => format!("{count} times"),
            _ => String::new(),
        };
        let detail = [when.as_str(), &event.source, &event.field_path]
            .into_iter()
            .filter(|text| !text.is_empty())
            .collect::<Vec<_>>()
            .join(" · ");
        Self {
            warning: event.is_warning(),
            label: format!("{}: {}", event.reason, event.message).into(),
            reason: event.reason.clone().into(),
            message: event.message.clone().into(),
            detail: detail.into(),
        }
    }
}

impl DetailPane {
    pub(super) fn events(&self, detail: &Detail, cx: &mut Context<Self>) -> AnyElement {
        let p = palette(cx);
        let events = &detail.events;
        let state = |id: &'static str, element: Div| {
            element
                .id(id)
                .test_support()
                .role(Role::Status)
                .into_any_element()
        };
        match events.read() {
            EventsRead::Loading => {
                return state(
                    "detail-events-loading",
                    v_flex()
                        .py_2()
                        .gap_3()
                        .children((0..4).map(|_| ui::skeleton(relative(0.8), dp(12.)))),
                );
            }
            EventsRead::Refused(reason) => {
                return state(
                    "detail-events-refused",
                    ui::empty_state(
                        IconName::ShieldX,
                        "Not permitted to list events",
                        "The identity may not list events here. That says nothing about whether any exist.",
                        Some(reason.clone()),
                        Vec::new(),
                        cx,
                    ),
                );
            }
            EventsRead::Failed(reason) => {
                return state(
                    "detail-events-failed",
                    ui::empty_state(
                        IconName::CircleDashed,
                        "Couldn't list events",
                        "Nothing is known yet, so nothing is shown as missing. Listing retries by itself.",
                        Some(reason.clone()),
                        Vec::new(),
                        cx,
                    ),
                );
            }
            EventsRead::Loaded | EventsRead::Stale(_) => {}
        }
        let stale = match events.read() {
            EventsRead::Stale(reason) => Some(
                div()
                    .id("detail-events-stale")
                    .test_support()
                    .role(Role::Status)
                    .child(ui::warning_banner(
                        Some("The events watch was interrupted; reconnecting.".into()),
                        format!("Showing events as last seen. {reason}"),
                        None,
                        cx,
                    )),
            ),
            _ => None,
        };
        let body = if self.event_lines.is_empty() {
            div()
                .id("detail-events-empty")
                .test_support()
                .text_size(dp(12.5))
                .text_color(p.muted)
                .child("No events recorded. Kubernetes keeps events for about an hour, so a quiet object has none.")
                .into_any_element()
        } else {
            v_flex()
                .gap_3()
                .children(self.event_lines.iter().enumerate().map(|(ix, line)| {
                    v_flex()
                        .id(("detail-event", ix))
                        .test_support()
                        .aria_label(line.label.clone())
                        .gap_0p5()
                        .child(
                            h_flex()
                                .gap_2()
                                .child(if line.warning {
                                    ui::tag(Tone::Warn, None, "Warning", cx)
                                } else {
                                    ui::tag(Tone::Outline, None, "Normal", cx)
                                })
                                .child(
                                    div()
                                        .text_size(dp(12.5))
                                        .font_weight(FontWeight::SEMIBOLD)
                                        .child(line.reason.clone()),
                                ),
                        )
                        .child(div().text_size(dp(12.5)).child(line.message.clone()))
                        .child(
                            div()
                                .text_size(dp(11.5))
                                .text_color(p.muted)
                                .child(line.detail.clone()),
                        )
                }))
                .when(events.len() > MAX_EVENTS, |this| {
                    this.child(
                        div()
                            .id("detail-events-capped")
                            .test_support()
                            .text_size(dp(12.))
                            .text_color(p.muted)
                            .child(format!(
                                "Showing the newest {MAX_EVENTS} of {} events.",
                                events.len()
                            )),
                    )
                })
                .into_any_element()
        };
        v_flex()
            .id("detail-events")
            .test_support()
            .gap_3()
            .children(stale)
            .child(body)
            .into_any_element()
    }
}
