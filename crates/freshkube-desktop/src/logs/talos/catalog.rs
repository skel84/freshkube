//! The service catalog: one pill per service the node reports, whose main
//! part starts or stops collecting it and whose eye shows or hides its
//! lines. The pills are free functions over a weak handle, so a popover
//! can draw them as well as the panel.

use gpui_kit::assets::IconName;
use gpui_kit::{
    App, Context, Role, SharedString, TestSupportExt, Toggled, WeakEntity,
    component::{Icon, h_flex},
    div,
    prelude::*,
};

use freshkube_core::logs::ServiceId;

use super::{Collection, LogPanel};
use crate::palette::palette;
use crate::ui;

/// One service as its pill draws it.
#[derive(Clone)]
pub(super) struct ServiceChip {
    pub(super) service: ServiceId,
    pub(super) collecting: bool,
    pub(super) showing: bool,
    /// Its retained lines.
    pub(super) count: usize,
    /// Not collected while 16 others are: it can't be picked.
    pub(super) full: bool,
}

impl ServiceChip {
    pub(super) fn of(view: &LogPanel, service: &ServiceId) -> Self {
        let collecting = view.source().collecting.contains(service);
        Self {
            service: service.clone(),
            collecting,
            showing: view.shown().contains(service),
            count: view.service_count(service),
            full: !collecting && view.source().collecting.len() >= 16,
        }
    }
}

/// Every service's pill, wrapped.
pub(super) fn catalog(view: &LogPanel, cx: &mut Context<LogPanel>) -> gpui_kit::Div {
    let handle = cx.entity().downgrade();
    h_flex().flex_wrap().gap(ui::dp(6.)).children(
        view.source()
            .services
            .iter()
            .map(|service| service_chip(&ServiceChip::of(view, service), handle.clone(), cx)),
    )
}

/// One service's pill: whether it collects, and whether its lines show.
pub(super) fn service_chip(
    chip: &ServiceChip,
    view: WeakEntity<LogPanel>,
    cx: &App,
) -> gpui_kit::Div {
    let p = palette(cx);
    h_flex()
        .h(ui::dp(26.))
        .rounded_full()
        .border_1()
        .border_color(if chip.collecting {
            p.accent_line
        } else {
            p.line_strong
        })
        .bg(if chip.collecting {
            p.accent_soft
        } else {
            p.surface
        })
        .overflow_hidden()
        .child(collect_toggle(chip, view.clone(), cx))
        .when(chip.collecting || chip.count > 0, |this| {
            this.child(show_toggle(chip, view, cx))
        })
}

/// The pill's main part, which starts or stops collecting the service.
fn collect_toggle(chip: &ServiceChip, view: WeakEntity<LogPanel>, cx: &App) -> impl IntoElement {
    let p = palette(cx);
    let service = chip.service.clone();
    let (collecting, count, full) = (chip.collecting, chip.count, chip.full);
    h_flex()
        .id(SharedString::from(format!("collect-{}", service.as_str())))
        .test_support()
        .role(Role::CheckBox)
        .aria_toggled(if collecting {
            Toggled::True
        } else {
            Toggled::False
        })
        .aria_label(format!("Collect {}", service.as_str()))
        .tab_index(0)
        .h_full()
        .pl(ui::dp(10.))
        .pr(ui::dp(if collecting || count > 0 { 4. } else { 10. }))
        .gap(ui::dp(5.))
        .when(!full, |this| this.cursor_pointer())
        .when(full, |this| this.opacity(0.5))
        .font_family(ui::MONO_FONT)
        .text_size(ui::dp(12.))
        .text_color(if collecting { p.ink } else { p.muted })
        .when(collecting, |this| {
            this.child(
                Icon::new(IconName::Check)
                    .size(ui::dp(13.))
                    .text_color(p.accent),
            )
        })
        .child(
            div()
                .when(!chip.showing, |this| {
                    this.line_through().text_color(p.muted)
                })
                .child(service.as_str().to_owned()),
        )
        .when(count > 0, |this| {
            this.child(
                div()
                    .text_size(ui::dp(10.5))
                    .text_color(p.muted)
                    .child(count.to_string()),
            )
        })
        .when(!full, |this| {
            this.on_click(move |_, _, cx| {
                _ = view.update(cx, |this, cx| {
                    let checked = !this.source().collecting.contains(&service);
                    this.toggle_collection(service.clone(), checked, cx)
                });
            })
        })
}

/// The eye after it, which shows or hides the service's lines.
fn show_toggle(chip: &ServiceChip, view: WeakEntity<LogPanel>, cx: &App) -> impl IntoElement {
    let p = palette(cx);
    let service = chip.service.clone();
    let showing = chip.showing;
    h_flex()
        .id(SharedString::from(format!("show-{}", service.as_str())))
        .test_support()
        .role(Role::CheckBox)
        .aria_toggled(if showing {
            Toggled::True
        } else {
            Toggled::False
        })
        .aria_label(format!(
            "{} {} lines",
            if showing { "Hide" } else { "Show" },
            service.as_str()
        ))
        .tab_index(0)
        .h_full()
        .pl(ui::dp(4.))
        .pr(ui::dp(9.))
        .cursor_pointer()
        .child(
            Icon::new(if showing {
                IconName::Eye
            } else {
                IconName::EyeOff
            })
            .size(ui::dp(13.))
            .text_color(p.muted),
        )
        .on_click(move |_, _, cx| {
            _ = view.update(cx, |this, cx| this.toggle_shown(&service, cx));
        })
}
