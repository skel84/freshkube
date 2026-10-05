//! Pinning inside the table's sideways scroll. `Pinned` keeps a child at
//! the scroll's left edge, `Passing` clips a cell that passes under the
//! pinned ones, and `Watch` draws the table again when what it built no
//! longer matches the scroll. Each acts while the frame is laid out, after
//! the scroll has clamped its offset, so nothing trails a drag by a frame.
use gpui_kit::{
    AnyElement, App, Bounds, ContentMask, Element, ElementId, EntityId, GlobalElementId, Hsla,
    InspectorElementId, IntoElement, LayoutId, Pixels, ScrollHandle, Window, fill, point, px, size,
};

use crate::ui::dp_px;

/// How far a sideways scroll has moved the table left, or `None` at its
/// left edge. A clamped offset can be `-0`, which `Pixels` orders below
/// zero, so anything under half a pixel counts as no scroll.
pub(super) fn scrolled_by(scroll: &ScrollHandle) -> Option<Pixels> {
    let shift = -scroll.offset().x;
    (shift >= px(0.5)).then_some(shift)
}

/// Whether a pinned run `run` dp wide stays at the left edge: the table is
/// scrolled, and the run takes at most two thirds of its visible width.
/// Wider, pinned cells would hide what the scroll is for; they scroll with
/// the rest.
pub(super) fn pins(scroll: &ScrollHandle, run: f32, window: &Window) -> bool {
    scrolled_by(scroll).is_some() && dp_px(run, window) * 1.5 <= scroll.bounds().size.width
}

/// A child that stays at the scroll's left edge.
pub(super) struct Pinned {
    scroll: ScrollHandle,
    child: AnyElement,
    /// A row's leading cells, `run` dp wide, drawn after the rest over the
    /// empty cells that keep their place, with a hairline on their right.
    /// They pin only while [`pins`] holds; otherwise they stay over their
    /// place.
    overlay: Option<(f32, Hsla)>,
}

impl Pinned {
    /// A child that always stays in view, as a group's line does.
    pub(super) fn new(scroll: &ScrollHandle, child: impl IntoElement) -> Self {
        Self {
            scroll: scroll.clone(),
            child: child.into_any_element(),
            overlay: None,
        }
    }

    /// Pinned cells `run` dp wide over the empty ones that keep their
    /// place, with a `rule` between them and the cells passing under them.
    pub(super) fn overlay(
        scroll: &ScrollHandle,
        run: f32,
        child: impl IntoElement,
        rule: Hsla,
    ) -> Self {
        Self {
            overlay: Some((run, rule)),
            ..Self::new(scroll, child)
        }
    }
}

impl IntoElement for Pinned {
    type Element = Self;

    fn into_element(self) -> Self {
        self
    }
}

impl Element for Pinned {
    type RequestLayoutState = ();
    /// How far the child moved right.
    type PrepaintState = Pixels;

    fn id(&self) -> Option<ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static core::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, ()) {
        (self.child.request_layout(window, cx), ())
    }

    fn prepaint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        _: Bounds<Pixels>,
        _: &mut (),
        window: &mut Window,
        cx: &mut App,
    ) -> Pixels {
        // The scrolling element's own prepaint runs before its children's:
        // the offset is clamped, zero or negative, and its bounds are this
        // frame's.
        let shift = match self.overlay {
            Some((run, _)) if !pins(&self.scroll, run, window) => None,
            _ => scrolled_by(&self.scroll),
        }
        .unwrap_or_default();
        window.with_element_offset(point(shift, px(0.)), |window| {
            self.child.prepaint(window, cx)
        });
        shift
    }

    fn paint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _: &mut (),
        shift: &mut Pixels,
        window: &mut Window,
        cx: &mut App,
    ) {
        self.child.paint(window, cx);
        if let Some((_, rule)) = self.overlay.filter(|_| *shift > px(0.)) {
            let right = bounds.right() + *shift;
            window.paint_quad(fill(
                Bounds::new(
                    point(right - px(1.), bounds.top()),
                    size(px(1.), bounds.size.height),
                ),
                rule,
            ));
        }
    }
}

/// A cell that passes under a pinned run `run` dp wide, which starts
/// `inset` right of the scroll's left edge. While the run pins, the cell is
/// clipped at its right edge: neither its paint nor its hitboxes reach under
/// the run, so there only the row and the pinned cells take the mouse.
pub(super) struct Passing {
    scroll: ScrollHandle,
    run: f32,
    inset: Pixels,
    /// A header's cell, whose label stays beside the run while any of the
    /// cell shows; a row's long text shows its tail there, and a short
    /// label would otherwise leave its column unnamed.
    sticky: bool,
    child: AnyElement,
}

impl Passing {
    pub(super) fn new(
        scroll: &ScrollHandle,
        run: f32,
        inset: Pixels,
        child: impl IntoElement,
    ) -> Self {
        Self {
            scroll: scroll.clone(),
            run,
            inset,
            sticky: false,
            child: child.into_any_element(),
        }
    }

    /// A header cell that moves right to stay beside the run, clipped to
    /// its own place.
    pub(super) fn sticky(scroll: &ScrollHandle, run: f32, child: impl IntoElement) -> Self {
        Self {
            sticky: true,
            ..Self::new(scroll, run, px(0.), child)
        }
    }
}

impl IntoElement for Passing {
    type Element = Self;

    fn into_element(self) -> Self {
        self
    }
}

impl Element for Passing {
    type RequestLayoutState = ();
    /// The clip, while the run pins.
    type PrepaintState = Option<ContentMask<Pixels>>;

    fn id(&self) -> Option<ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static core::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, ()) {
        (self.child.request_layout(window, cx), ())
    }

    fn prepaint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _: &mut (),
        window: &mut Window,
        cx: &mut App,
    ) -> Option<ContentMask<Pixels>> {
        if !pins(&self.scroll, self.run, window) {
            self.child.prepaint(window, cx);
            return None;
        }
        let view = self.scroll.bounds();
        let left = (view.left() + self.inset + dp_px(self.run, window)).min(view.right());
        let mut right = view.right();
        let mut shift = px(0.);
        if self.sticky {
            shift = (left - bounds.left()).clamp(px(0.), bounds.size.width);
            right = right.min(bounds.right());
        }
        let mask = ContentMask {
            bounds: Bounds::from_corners(
                point(left, view.top()),
                point(right.max(left), view.bottom()),
            ),
        };
        window.with_content_mask(Some(mask), |window| {
            window.with_element_offset(point(shift, px(0.)), |window| {
                self.child.prepaint(window, cx)
            })
        });
        Some(mask)
    }

    fn paint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        _: Bounds<Pixels>,
        _: &mut (),
        mask: &mut Option<ContentMask<Pixels>>,
        window: &mut Window,
        cx: &mut App,
    ) {
        window.with_content_mask(*mask, |window| self.child.paint(window, cx));
    }
}

/// The scrolled table's header, which notes whether the table was built
/// pinned, from the scroll's last frame. When this frame's scroll says
/// otherwise, as after a resize or a scroll clamped to the left edge, the
/// `view` draws again with what this frame says.
pub(super) struct Watch {
    scroll: ScrollHandle,
    run: f32,
    pinned: bool,
    view: EntityId,
    child: AnyElement,
}

impl Watch {
    pub(super) fn new(
        scroll: &ScrollHandle,
        run: f32,
        pinned: bool,
        view: EntityId,
        child: impl IntoElement,
    ) -> Self {
        Self {
            scroll: scroll.clone(),
            run,
            pinned,
            view,
            child: child.into_any_element(),
        }
    }
}

impl IntoElement for Watch {
    type Element = Self;

    fn into_element(self) -> Self {
        self
    }
}

impl Element for Watch {
    type RequestLayoutState = ();
    type PrepaintState = ();

    fn id(&self) -> Option<ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static core::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, ()) {
        (self.child.request_layout(window, cx), ())
    }

    fn prepaint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        _: Bounds<Pixels>,
        _: &mut (),
        window: &mut Window,
        cx: &mut App,
    ) {
        // The run's pins and passes act on this frame's scroll already; the
        // new build only brings the tree in line, so it can't loop.
        let unscrolled = scrolled_by(&self.scroll).is_none();
        if unscrolled || pins(&self.scroll, self.run, window) != self.pinned {
            let view = self.view;
            window.on_next_frame(move |_, cx| cx.notify(view));
        }
        self.child.prepaint(window, cx);
    }

    fn paint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        _: Bounds<Pixels>,
        _: &mut (),
        _: &mut (),
        window: &mut Window,
        cx: &mut App,
    ) {
        self.child.paint(window, cx);
    }
}
