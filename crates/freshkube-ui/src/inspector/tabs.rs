//! The inspector's tabs: a row under its heading, each 28 high. A row too
//! wide for the inspector scrolls sideways; a cut end fades under a chevron
//! that pages to the tabs beyond it, and the active tab is always in view.
use std::cell::Cell;
use std::rc::Rc;

use gpui_kit::assets::IconName;
use gpui_kit::base::{ElementExt as _, ObservedElement as Observed};
use gpui_kit::component::{ActiveTheme as _, Icon, h_flex};
use gpui_kit::prelude::*;
use gpui_kit::{
    AnyElement, App, Bounds, Div, ElementId, EntityId, FontWeight, Hsla, Pixels, Role,
    ScrollHandle, SharedString, Stateful, TestSupportExt, Window, div, linear_color_stop,
    linear_gradient, point, transparent_black,
};

use super::UNDER_HEADING;
use crate::page::PANE_PADDING;
use crate::palette::palette;
use crate::ui::{dp, dp_px};

/// A tab's height, and the space at its sides.
pub const TAB_HEIGHT: f32 = 28.;
pub(super) const TAB_PADDING: f32 = 10.;
/// How far a cut end's fade and chevron reach in from the strip's edge, and
/// the chevron's share of it.
const END: f32 = 40.;
const CHEVRON: f32 = 24.;

/// One of an inspector's tabs: 28 high, its label 12.5, and a 2 px accent
/// underline with semibold ink when `active`, muted ink otherwise. The
/// caller adds what it does: focus, a tooltip, a click, a count or mark.
pub fn tab(
    id: impl Into<ElementId>,
    label: impl Into<SharedString>,
    active: bool,
    cx: &App,
) -> Observed<Stateful<Div>> {
    let p = palette(cx);
    let label = label.into();
    h_flex()
        .id(id.into())
        .test_support()
        .role(Role::Tab)
        .aria_selected(active)
        .aria_label(label.clone())
        .focus_visible(|style| style.bg(p.hover))
        .flex_none()
        .h(dp(TAB_HEIGHT))
        .px(dp(TAB_PADDING))
        .gap(dp(6.))
        .cursor_pointer()
        .text_size(dp(12.5))
        .border_b_2()
        .map(|this| {
            if active {
                this.border_color(p.accent)
                    .text_color(p.ink)
                    .font_weight(FontWeight::SEMIBOLD)
            } else {
                this.border_color(transparent_black())
                    .text_color(p.muted)
                    .hover(|style| style.text_color(p.ink))
            }
        })
        .child(label)
}

/// A tab strip's memory between frames, kept on the page's entity like an
/// [`InspectorSplit`](super::InspectorSplit): how far its row scrolled, how
/// wide it was, which tab is active and which ends are cut. A new object
/// with other tabs takes a new one.
#[derive(Clone, Default)]
pub struct TabStrip(Rc<Strip>);

#[derive(Default)]
struct Strip {
    scroll: ScrollHandle,
    /// The row's width at the last frame; a new one reveals the active tab.
    width: Cell<Option<Pixels>>,
    active: Cell<Option<usize>>,
    /// The tab to bring into view at the next frame.
    reveal: Cell<Option<usize>>,
    edges: Cell<Edges>,
    /// The view that draws the strip, to redraw it from a click or prepaint.
    view: Cell<Option<EntityId>>,
}

/// Which ends of a strip have tabs cut off.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Edges {
    pub earlier: bool,
    pub later: bool,
}

impl TabStrip {
    /// The row the tabs go in, `active` being the shown tab's position in
    /// it. Its children must be the tabs alone, in order; the caller adds
    /// its key context and actions.
    pub fn row(&self, id: impl Into<ElementId>, active: usize) -> Observed<Stateful<Div>> {
        let strip = &self.0;
        if strip.active.replace(Some(active)) != Some(active) {
            strip.reveal.set(Some(active));
        }
        h_flex()
            .id(id.into())
            .test_support()
            .flex_1()
            .min_w_0()
            .overflow_x_scroll()
            .track_scroll(&strip.scroll)
    }

    /// Which ends had tabs cut off at the last frame.
    pub fn edges(&self) -> Edges {
        self.0.edges.get()
    }

    /// After the row's prepaint: brings a tab into view if one is due,
    /// notes which ends are cut, and asks for a frame when either changed,
    /// since nothing else would draw one.
    fn measure(&self, window: &mut Window) {
        let strip = &self.0;
        strip.view.set(Some(window.current_view()));
        let viewport = strip.scroll.bounds();
        if strip.width.replace(Some(viewport.size.width)) != Some(viewport.size.width) {
            strip.reveal.set(strip.reveal.get().or(strip.active.get()));
        }
        let mut redraw = false;
        if let Some(ix) = strip.reveal.get()
            && let Some(item) = strip.scroll.bounds_for_item(ix)
        {
            strip.reveal.set(None);
            redraw |= self.bring_in(ix, item, viewport, window);
        }
        let max = strip.scroll.max_offset().x.max(Pixels::ZERO);
        let offset = strip.scroll.offset().x;
        let edges = Edges {
            earlier: offset < -px_half(),
            later: offset > -max + px_half(),
        };
        redraw |= strip.edges.replace(edges) != edges;
        if redraw {
            let view = window.current_view();
            window.on_next_frame(move |_, cx| cx.notify(view));
        }
    }

    /// Scrolls the least that shows tab `ix` whole and clear of a cut
    /// end's fade; returns whether the row moved.
    fn bring_in(
        &self,
        ix: usize,
        item: Bounds<Pixels>,
        viewport: Bounds<Pixels>,
        window: &Window,
    ) -> bool {
        let scroll = &self.0.scroll;
        let max = scroll.max_offset().x.max(Pixels::ZERO);
        let last = scroll.children_count().saturating_sub(1);
        let clear = dp_px(END - PANE_PADDING, window);
        let (lead, trail) = (
            if ix == 0 { Pixels::ZERO } else { clear },
            if ix >= last { Pixels::ZERO } else { clear },
        );
        let current = scroll.offset();
        let mut offset = current.x;
        if item.right() + offset > viewport.right() - trail {
            offset = viewport.right() - trail - item.right();
        }
        if item.left() + offset < viewport.left() + lead {
            offset = viewport.left() + lead - item.left();
        }
        let offset = offset.clamp(-max, Pixels::ZERO);
        if offset == current.x {
            return false;
        }
        scroll.set_offset(point(offset, current.y));
        true
    }

    /// A chevron's click pages the row: › puts the first tab cut at the
    /// right at the left, clear of the ‹ fade, so every tab that fits after
    /// it comes in; ‹ puts the first cut at the left at the right. A tab too
    /// wide for that comes in at the chevron's side instead, so a click
    /// always moves the row.
    fn step(&self, later: bool, window: &mut Window, cx: &mut App) {
        let scroll = &self.0.scroll;
        let viewport = scroll.bounds();
        let current = scroll.offset();
        let offset = current.x;
        let clear = dp_px(END - PANE_PADDING, window);
        // A tab whose label is clear of the fade shows, though its padding
        // may lie under it; counting it as cut would move the row a pixel.
        let shown = clear - dp_px(TAB_PADDING, window);
        let last = scroll.children_count().saturating_sub(1);
        let items = (0..=last).filter_map(|ix| Some((ix, scroll.bounds_for_item(ix)?)));
        let cut = if later {
            items
                .filter(|(_, item)| item.right() + offset > viewport.right() - shown + px_half())
                .min_by_key(|(ix, _)| *ix)
        } else {
            items
                .filter(|(_, item)| item.left() + offset < viewport.left() + shown - px_half())
                .max_by_key(|(ix, _)| *ix)
        };
        let Some((ix, item)) = cut else {
            return;
        };
        let lead = if ix == 0 { Pixels::ZERO } else { clear };
        let trail = if ix >= last { Pixels::ZERO } else { clear };
        let at_left = viewport.left() + lead - item.left();
        let at_right = viewport.right() - trail - item.right();
        let paged = if later {
            at_left.min(at_right)
        } else {
            at_right.max(at_left)
        };
        let max = scroll.max_offset().x.max(Pixels::ZERO);
        let paged = paged.clamp(-max, Pixels::ZERO);
        if paged == offset {
            return;
        }
        scroll.set_offset(point(paged, current.y));
        if let Some(view) = self.0.view.get() {
            cx.notify(view);
        }
    }
}

fn px_half() -> Pixels {
    Pixels::from(0.5)
}

/// The strip under the inspector's heading: `row` with a hairline below,
/// on the background its fades end in, and a fade and chevron over each
/// cut end.
///
/// The strip alone paints, so an inspector in a card (the Application
/// report's traces) keeps the card's surface. Its colour assumes the page's
/// surface, which every inspector with tabs sits on; an inspector that may
/// sit on a card must set the strip's colour with its own, or the fades end
/// in the wrong colour.
pub(super) fn strip(
    id: SharedString,
    strip: &TabStrip,
    row: AnyElement,
    cx: &App,
) -> Observed<Stateful<Div>> {
    bare_strip(id, strip, row, cx)
        .mt(dp(UNDER_HEADING))
        .border_b_1()
        .border_color(palette(cx).line)
}

/// [`strip`] without the gap above it or the hairline below, for a bar
/// that draws its own, such as the dock's.
pub(crate) fn bare_strip(
    id: SharedString,
    strip: &TabStrip,
    row: AnyElement,
    cx: &App,
) -> Observed<Stateful<Div>> {
    let background = cx.theme().background;
    let edges = strip.edges();
    let measure = strip.clone();
    let wheel = strip.clone();
    h_flex()
        .id(id.clone())
        .test_support()
        .relative()
        .flex_none()
        .min_w_0()
        .px(dp(PANE_PADDING))
        .bg(background)
        .child(row)
        .on_prepaint(move |_, window, _| measure.measure(window))
        // gpui gives a wheel to every scroller under the pointer; a cut
        // strip keeps it, so the page behind doesn't scroll too.
        .on_scroll_wheel(move |_, _, cx| {
            let edges = wheel.edges();
            if edges.earlier || edges.later {
                cx.stop_propagation();
            }
        })
        .children(
            edges
                .earlier
                .then(|| end(&id, false, strip, background, cx)),
        )
        .children(edges.later.then(|| end(&id, true, strip, background, cx)))
}

/// A cut end: a fade from clear into `background`, then a chevron on it
/// that pages the row toward it. It isn't focusable: the keyboard moves
/// along the tabs themselves, and the strip follows the active one.
fn end(
    id: &str,
    later: bool,
    strip: &TabStrip,
    background: Hsla,
    cx: &App,
) -> Observed<Stateful<Div>> {
    let p = palette(cx);
    let clear = Hsla {
        a: 0.,
        ..background
    };
    let fade = div().h_full().w(dp(END - CHEVRON)).bg(linear_gradient(
        if later { 90. } else { 270. },
        linear_color_stop(clear, 0.),
        linear_color_stop(background, 1.),
    ));
    let chevron = h_flex()
        .h_full()
        .w(dp(CHEVRON))
        .justify_center()
        .bg(background)
        .child(
            Icon::new(if later {
                IconName::ChevronRight
            } else {
                IconName::ChevronLeft
            })
            .size(dp(14.))
            .text_color(p.muted),
        );
    let strip = strip.clone();
    h_flex()
        .id(SharedString::from(format!(
            "{id}-{}",
            if later { "later" } else { "earlier" }
        )))
        .test_support()
        .role(Role::Button)
        .aria_label(if later { "More tabs" } else { "Earlier tabs" })
        .absolute()
        .top_0()
        .h(dp(TAB_HEIGHT))
        .w(dp(END))
        .map(|this| if later { this.right_0() } else { this.left_0() })
        .occlude()
        .cursor_pointer()
        .map(|this| {
            if later {
                this.child(fade).child(chevron)
            } else {
                this.child(chevron).child(fade)
            }
        })
        .on_click(move |_, window, cx| strip.step(later, window, cx))
}
