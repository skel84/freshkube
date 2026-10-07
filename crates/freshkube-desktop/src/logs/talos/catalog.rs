//! The service catalog: one pill per service the node reports, whose main
//! part starts or stops collecting it and whose eye shows or hides its
//! lines. The pills take the rows the panel has room for, as the workload
//! logs' containers do: two at most, and none in a short window. A "+N"
//! chip lists every service; when no row shows, it reads "3 of 12
//! services" and joins the collection row, so the picker takes no row of
//! its own.

use std::hash::{DefaultHasher, Hash, Hasher};

use gpui_kit::assets::IconName;
use gpui_kit::{
    Anchor, AnyElement, App, AvailableSpace, Context, FocusHandle, KeyBinding, Pixels, Role,
    ScrollHandle, SharedString, TestSupportExt, Toggled, WeakEntity, Window,
    component::{
        Icon, Sizable,
        button::Button,
        h_flex,
        popover::Popover,
        scroll::{Scrollbar, ScrollbarMode},
        v_flex,
    },
    div,
    prelude::*,
    px, size,
};

use freshkube_core::logs::ServiceId;

use super::{Collection, LogPanel};
use crate::logs::{chip_rows, chips_that_fit};
use crate::palette::palette;
use crate::ui;

/// One service as its pill draws it.
#[derive(Clone)]
struct ServiceChip {
    service: ServiceId,
    collecting: bool,
    showing: bool,
    /// Its retained lines.
    count: usize,
    /// Not collected while 16 others are: it can't be picked.
    full: bool,
}

impl ServiceChip {
    fn of(view: &LogPanel, service: &ServiceId) -> Self {
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

/// A pill's own context, deeper than the list's popover: Enter and Space
/// toggle the pill there instead of closing the list.
const PILL_CONTEXT: &str = "LogServicePill";

gpui_kit::actions!(talos_logs, [TogglePill]);

pub(super) fn key_bindings() -> [KeyBinding; 2] {
    [
        KeyBinding::new("enter", TogglePill, Some(PILL_CONTEXT)),
        KeyBinding::new("space", TogglePill, Some(PILL_CONTEXT)),
    ]
}

/// What the panel's controls draw of the catalog: the chips that fit, and
/// whether "+N" stands for the rest or for every service.
#[derive(Default)]
pub(super) struct Picker {
    /// The pills that fit their rows, from the start of the catalog.
    pub(super) fits: usize,
    /// Rows for the pills: none puts the picker on the collection row.
    pub(super) rows: usize,
    /// Whether "+N"'s list is open.
    pub(super) open: bool,
    /// Each pill's width, then "+N"'s, as last measured.
    widths: Vec<Pixels>,
    more_width: Pixels,
    /// What the widths were measured for: see [`measure_key`].
    measured: Option<u64>,
    /// The open list's, which holds the keyboard when nothing had it before.
    focus: Option<FocusHandle>,
    /// The open list's pills, which scroll in a short window.
    scroll: ScrollHandle,
}

/// The "Services" caption before the rows, and the gap after it.
const LABEL_WIDTH: f32 = 90.;

/// What "+N" reads: the hidden count, or how many of every service are
/// collected when no row shows.
fn more_label(hidden: usize, total: usize, collecting: usize) -> SharedString {
    if hidden == total {
        format!(
            "{collecting} of {total} {}",
            if total == 1 { "service" } else { "services" }
        )
        .into()
    } else {
        format!("+{hidden}").into()
    }
}

/// How many pills fit the room. The pills are measured again only when
/// something that sizes them changes: a pill's count grows as its lines
/// arrive, but its width only with another digit.
pub(super) fn fit(
    view: &mut LogPanel,
    width: Pixels,
    window: &mut Window,
    cx: &mut Context<LogPanel>,
) {
    if view.source().picker.focus.is_none() {
        view.source_mut().picker.focus = Some(cx.focus_handle());
    }
    // A short window scrolls the log's body, and the panel lays out taller
    // than the room it shows in: the pills take no row there.
    let rows = if freshkube_ui::page::is_short(window) {
        0
    } else {
        chip_rows(view.panel_height(), window)
    };
    let services = view.source().services.len();
    let fits = if rows == 0 || services == 0 {
        0
    } else {
        let key = measure_key(view, window.rem_size());
        if view.source().picker.measured != Some(key) {
            let (widths, more) = measure(view, window, cx);
            let picker = &mut view.source_mut().picker;
            picker.widths = widths;
            picker.more_width = more;
            picker.measured = Some(key);
        }
        let picker = &view.source().picker;
        // The caption before the rows, the panel's border, and a little
        // slack for rounding.
        let room = width - ui::dp_px(LABEL_WIDTH + 8., window);
        chips_that_fit(
            &picker.widths,
            picker.more_width,
            ui::dp_px(6., window),
            room,
            rows,
        )
    };
    let picker = &mut view.source_mut().picker;
    picker.rows = rows;
    picker.fits = fits;
    // With every pill in its row there's no "+N" to hold the list open.
    if fits >= services {
        picker.open = false;
    }
}

/// What sizes the pills: each service's name, whether it collects (its
/// check and eye), the digits of its count, and the text size.
fn measure_key(view: &LogPanel, rem: Pixels) -> u64 {
    let mut hasher = DefaultHasher::new();
    f32::from(rem).to_bits().hash(&mut hasher);
    for service in &view.source().services {
        let count = view.service_count(service);
        service.as_str().hash(&mut hasher);
        view.source().collecting.contains(service).hash(&mut hasher);
        (count > 0).then(|| count.ilog10()).hash(&mut hasher);
    }
    hasher.finish()
}

/// Each pill's width, then "+N"'s at its widest.
fn measure(
    view: &LogPanel,
    window: &mut Window,
    cx: &mut Context<LogPanel>,
) -> (Vec<Pixels>, Pixels) {
    let handle = cx.entity().downgrade();
    let services = &view.source().services;
    let mut elements: Vec<AnyElement> = services
        .iter()
        .map(|service| {
            service_chip(&ServiceChip::of(view, service), "", handle.clone(), cx).into_any_element()
        })
        .collect();
    elements.push(
        more_button(
            "logs-services-more-measure",
            more_label(services.len(), services.len() + 1, 0),
        )
        .into_any_element(),
    );
    let mut widths: Vec<Pixels> = elements
        .iter_mut()
        .map(|element| {
            element
                .layout_as_root(
                    size(AvailableSpace::MinContent, AvailableSpace::MinContent),
                    window,
                    cx,
                )
                .width
        })
        .collect();
    let more = widths.pop().unwrap_or_default();
    (widths, more)
}

/// The pills that fit, then "+N"; `None` when no row shows them.
pub(super) fn rows(view: &LogPanel, cx: &mut Context<LogPanel>) -> Option<AnyElement> {
    let picker = &view.source().picker;
    if picker.rows == 0 {
        return None;
    }
    let services = &view.source().services;
    let shown = picker.fits.min(services.len());
    let handle = cx.entity().downgrade();
    Some(
        h_flex()
            .id("logs-services-chips")
            .w_full()
            .min_w_0()
            .flex_wrap()
            .gap(ui::dp(6.))
            .children(services[..shown].iter().map(|service| {
                service_chip(&ServiceChip::of(view, service), "", handle.clone(), cx)
            }))
            .when(shown < services.len(), |this| {
                this.child(picker_button(view, services.len() - shown, cx))
            })
            .into_any_element(),
    )
}

/// The picker alone, for the collection row when no row shows the pills.
pub(super) fn compact(view: &LogPanel, cx: &mut Context<LogPanel>) -> Option<AnyElement> {
    let services = view.source().services.len();
    (view.source().picker.rows == 0 && services > 0).then(|| picker_button(view, services, cx))
}

fn more_button(id: &'static str, label: SharedString) -> Button {
    Button::new(id)
        .outline()
        .small()
        .accessibility_label(label.clone())
        .label(label)
}

/// "+N", or "3 of 12 services", which lists every service.
fn picker_button(view: &LogPanel, hidden: usize, cx: &mut Context<LogPanel>) -> AnyElement {
    let services = view.source().services.len();
    let label = more_label(hidden, services, view.source().collecting.len());
    let handle = cx.entity().downgrade();
    let open_handle = handle.clone();
    let focus = view
        .source()
        .picker
        .focus
        .clone()
        .unwrap_or_else(|| cx.focus_handle());
    let list_focus = focus.clone();
    let scroll = view.source().picker.scroll.clone();
    // The list's pills are made only while it's open.
    let chips: Vec<ServiceChip> = if view.source().picker.open {
        view.source()
            .services
            .iter()
            .map(|service| ServiceChip::of(view, service))
            .collect()
    } else {
        Vec::new()
    };
    Popover::new("logs-services-popover")
        .anchor(Anchor::TopLeft)
        .open(view.source().picker.open)
        .track_focus(&focus)
        .on_open_change(move |open, window, cx| {
            let open = *open;
            _ = open_handle.update(cx, |view, cx| {
                view.source_mut().picker.open = open;
                // The list gives the keyboard back to what had it; a mouse
                // open leaves nothing to give it to, so the lines take it.
                if !open && list_focus.contains_focused(window, cx) {
                    view.focus_lines(window, cx);
                }
                cx.notify();
            });
        })
        .trigger(
            more_button("logs-services-more", label)
                .when(hidden == services, |button| {
                    button.icon(IconName::ChevronDown)
                })
                .tooltip("Every service: what to collect, and what to show"),
        )
        .content(move |_, window, cx| list(&chips, &handle, &focus, &scroll, window, cx))
        .into_any_element()
}

/// Kit's margin between a popover and the window's edges.
const LIST_MARGIN: Pixels = px(8.);

/// The open list: a note, then every service's pill, scrolling with a
/// scrollbar when the window is too short for them.
fn list(
    chips: &[ServiceChip],
    view: &WeakEntity<LogPanel>,
    focus: &FocusHandle,
    scroll: &ScrollHandle,
    window: &mut Window,
    cx: &mut App,
) -> AnyElement {
    // Inside the margins, the popover's border and its padding.
    let max_height =
        window.viewport_size().height - LIST_MARGIN * 2. - px(2.) - window.rem_size() * 1.5;
    v_flex()
        .id("logs-services-list")
        .test_support()
        .role(Role::Group)
        .aria_label("Services to collect and show")
        .track_focus(focus)
        .w(ui::dp(340.))
        .max_h(max_height)
        .gap(ui::dp(8.))
        .child(
            div()
                .flex_none()
                .text_size(ui::dp(12.))
                .text_color(palette(cx).muted)
                .child(
                    "Collect up to 16 services. The eye hides a service's lines without stopping collection.",
                ),
        )
        .child(
            v_flex()
                .relative()
                .flex_shrink(1.)
                .min_h_0()
                .child(
                    div()
                        .id("logs-services-list-scroll")
                        .test_support()
                        .flex_shrink(1.)
                        .min_h_0()
                        .overflow_y_scroll()
                        .restrict_scroll_to_axis()
                        .track_scroll(scroll)
                        .child(
                            // Clear of the scrollbar.
                            h_flex()
                                .flex_wrap()
                                .gap(ui::dp(6.))
                                .pr(ui::dp(12.))
                                .children(chips.iter().map(|chip| {
                                    service_chip(chip, "list-", view.clone(), cx)
                                })),
                        ),
                )
                .child(
                    Scrollbar::vertical(scroll)
                        .id("logs-services-list-scrollbar")
                        .mode(ScrollbarMode::Always),
                ),
        )
        .into_any_element()
}

/// One service's pill: whether it collects, and whether its lines show.
/// `prefix` keeps the list's pills apart from the rows' when both draw.
fn service_chip(
    chip: &ServiceChip,
    prefix: &str,
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
        .child(collect_toggle(chip, prefix, view.clone(), cx))
        .when(chip.collecting || chip.count > 0, |this| {
            this.child(show_toggle(chip, prefix, view, cx))
        })
}

/// The pill's main part, which starts or stops collecting the service.
fn collect_toggle(
    chip: &ServiceChip,
    prefix: &str,
    view: WeakEntity<LogPanel>,
    cx: &App,
) -> impl IntoElement {
    let p = palette(cx);
    let service = chip.service.clone();
    let (collecting, count, full) = (chip.collecting, chip.count, chip.full);
    h_flex()
        .id(SharedString::from(format!(
            "{prefix}collect-{}",
            service.as_str()
        )))
        .test_support()
        .role(Role::CheckBox)
        .aria_toggled(if collecting {
            Toggled::True
        } else {
            Toggled::False
        })
        .aria_label(format!("Collect {}", service.as_str()))
        // A full pill can't be pressed, so the keyboard passes it by.
        .when(!full, |this| this.tab_index(0))
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
            let toggle = move |cx: &mut App| {
                _ = view.update(cx, |this, cx| {
                    let checked = !this.source().collecting.contains(&service);
                    this.toggle_collection(service.clone(), checked, cx)
                });
            };
            let key = toggle.clone();
            this.key_context(PILL_CONTEXT)
                .on_action(move |_: &TogglePill, _, cx| key(cx))
                .on_click(move |_, _, cx| toggle(cx))
        })
}

/// The eye after it, which shows or hides the service's lines.
fn show_toggle(
    chip: &ServiceChip,
    prefix: &str,
    view: WeakEntity<LogPanel>,
    cx: &App,
) -> impl IntoElement {
    let p = palette(cx);
    let service = chip.service.clone();
    let showing = chip.showing;
    h_flex()
        .id(SharedString::from(format!(
            "{prefix}show-{}",
            service.as_str()
        )))
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
        .key_context(PILL_CONTEXT)
        .on_action({
            let view = view.clone();
            let service = service.clone();
            move |_: &TogglePill, _, cx| {
                _ = view.update(cx, |this, cx| this.toggle_shown(&service, cx));
            }
        })
        .on_click(move |_, _, cx| {
            _ = view.update(cx, |this, cx| this.toggle_shown(&service, cx));
        })
}
