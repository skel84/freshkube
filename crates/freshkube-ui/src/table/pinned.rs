//! `Pinned`: a child of the table's sideways scroll that stays at the
//! scroll's left edge. It moves its child back by the scroll offset while
//! the frame is laid out, after the scroll has clamped it, so the child
//! never trails a drag by a frame.
use gpui_kit::{
    AnyElement, App, Bounds, Element, ElementId, EntityId, GlobalElementId, Hsla,
    InspectorElementId, IntoElement, LayoutId, Pixels, ScrollHandle, Window, fill, point, px, size,
};

/// How far a sideways scroll has moved the table left, or `None` at its
/// left edge. A clamped offset can be `-0`, which `Pixels` orders below
/// zero, so anything under half a pixel counts as no scroll.
pub(super) fn scrolled_by(scroll: &ScrollHandle) -> Option<Pixels> {
    let shift = -scroll.offset().x;
    (shift >= px(0.5)).then_some(shift)
}

pub(super) struct Pinned {
    scroll: ScrollHandle,
    child: AnyElement,
    /// A row's leading cells, drawn after the rest over the empty cells
    /// that keep their place. They pin only while the table is scrolled
    /// and they take at most two thirds of its width, with a hairline on
    /// their right; otherwise they stay over their place.
    overlay: Option<Hsla>,
    /// The view to draw again when the scroll turns out to be at its left
    /// edge, as after a resize clamped it there, so the rows draw unpinned.
    settle: Option<EntityId>,
}

impl Pinned {
    /// A child that always stays in view, as a group's line does.
    pub(super) fn new(scroll: &ScrollHandle, child: impl IntoElement) -> Self {
        Self {
            scroll: scroll.clone(),
            child: child.into_any_element(),
            overlay: None,
            settle: None,
        }
    }

    /// Pinned cells over the empty ones that keep their place, with a
    /// `rule` between them and the cells passing under them.
    pub(super) fn overlay(scroll: &ScrollHandle, child: impl IntoElement, rule: Hsla) -> Self {
        Self {
            overlay: Some(rule),
            ..Self::new(scroll, child)
        }
    }

    pub(super) fn settle(mut self, view: EntityId) -> Self {
        self.settle = Some(view);
        self
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
        bounds: Bounds<Pixels>,
        _: &mut (),
        window: &mut Window,
        cx: &mut App,
    ) -> Pixels {
        // The scrolling element's own prepaint runs before its children's:
        // the offset is clamped, zero or negative, and its bounds are this
        // frame's.
        let mut shift = scrolled_by(&self.scroll);
        if shift.is_none()
            && let Some(view) = self.settle.take()
        {
            window.on_next_frame(move |_, cx| cx.notify(view));
        }
        // Wider, pinned cells would hide what the scroll is for; they
        // scroll with the rest.
        if self.overlay.is_some() && bounds.size.width * 1.5 > self.scroll.bounds().size.width {
            shift = None;
        }
        let shift = shift.unwrap_or_default();
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
        if let Some(rule) = self.overlay.filter(|_| *shift > px(0.)) {
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
