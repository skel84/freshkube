//! The inspector (docs/DESIGN.md#components): the pane that shows a table's
//! selected row. It takes the split's trailing pane without a card, divided
//! from the table by the resize handle's hairline: beside the table on a
//! wide page, under it on a narrow one. The page keeps an
//! [`InspectorSplit`], which remembers how wide the user made it.
mod tabs;
pub(crate) use tabs::bare_strip;

pub use tabs::{Edges, TAB_HEIGHT, TabStrip, tab};

use std::cell::Cell;
use std::ops::Range;
use std::rc::Rc;

use gpui_kit::base::ObservedElement as Observed;
use gpui_kit::component::resizable::{
    ResizablePanelEvent, ResizableState, h_resizable, resizable_panel, v_resizable,
};
use gpui_kit::component::{h_flex, v_flex};
use gpui_kit::prelude::*;
use gpui_kit::{
    AnyElement, App, AppContext, Div, Entity, Pixels, SharedString, Stateful, Subscription,
    TestSupportExt, Window, div,
};

use crate::page::{PANE_PADDING, SHORT_LIST_HEIGHT};
use crate::palette::palette;
use crate::ui::{BASE_TEXT, dp, dp_px};

/// The least page width, in dp, at which the inspector sits beside the
/// table; on a narrower page it opens under it.
pub const SPLIT_WIDTH: f32 = 900.;
/// The inspector's width beside the table until the user drags it.
pub const WIDTH: f32 = 460.;
/// The least width the inspector keeps beside the table.
pub const MIN_WIDTH: f32 = 320.;
/// The least width the table keeps beside the inspector.
pub const LIST_MIN_WIDTH: f32 = 320.;
/// Stacked, the table and the inspector share the height one to two, so a
/// short window still leaves the inspector room for a few lines.
pub const STACKED_LIST_HEIGHT: f32 = 190.;
pub const STACKED_HEIGHT: f32 = 380.;
/// The least heights the table and the inspector keep when stacked.
pub const LIST_MIN_HEIGHT: f32 = 96.;
pub const MIN_HEIGHT: f32 = 220.;
/// How tall the split is beside the table while a short page scrolls.
pub const SHORT_HEIGHT: f32 = 360.;
/// Between the heading and the body's parts.
const GAP: f32 = 12.;
/// Between the heading and a banner or the tabs under it.
const UNDER_HEADING: f32 = 8.;

/// The inspector's frame: a heading, then a body that scrolls on its own,
/// both padded 12, on whatever the split sits on. A banner and a tab strip
/// may sit between them, and a footer under the body. Its parts' ids are
/// `<id>-heading`, `<id>-banner`, `<id>-tabs`, `<id>-body` (or
/// `<id>-content`) and `<id>-footer`.
pub struct Inspector {
    id: SharedString,
    heading: Option<AnyElement>,
    banner: Option<AnyElement>,
    tabs: Option<(TabStrip, AnyElement)>,
    body: Vec<AnyElement>,
    content: Option<AnyElement>,
    footer: Option<AnyElement>,
}

impl Inspector {
    pub fn new(id: impl Into<SharedString>) -> Self {
        Self {
            id: id.into(),
            heading: None,
            banner: None,
            tabs: None,
            body: vec![],
            content: None,
            footer: None,
        }
    }

    /// What the inspector shows, at the top: a title and its tags.
    pub fn heading(mut self, heading: impl IntoElement) -> Self {
        self.heading = Some(heading.into_any_element());
        self
    }

    /// A notice under the heading, such as a stale or deleted banner.
    pub fn banner(mut self, banner: Option<impl IntoElement>) -> Self {
        self.banner = banner.map(IntoElement::into_any_element);
        self
    }

    /// A row of [`tab`]s under the heading, with a hairline below: `row`
    /// is `strip`'s [`TabStrip::row`] with the tabs in it. It scrolls
    /// sideways when the inspector is too narrow for it.
    pub fn tabs(mut self, strip: &TabStrip, row: impl IntoElement) -> Self {
        self.tabs = Some((strip.clone(), row.into_any_element()));
        self
    }

    /// One part of the body, under the heading and the parts before it.
    pub fn child(mut self, child: impl IntoElement) -> Self {
        self.body.push(child.into_any_element());
        self
    }

    pub fn children(mut self, children: impl IntoIterator<Item = impl IntoElement>) -> Self {
        self.body
            .extend(children.into_iter().map(IntoElement::into_any_element));
        self
    }

    /// In place of a body: one element that fills the rest and lays itself
    /// out, unpadded and unscrolled, for a view that scrolls itself (a
    /// virtual list, a log, a terminal). The parts given to [`child`] are
    /// then not drawn.
    ///
    /// [`child`]: Self::child
    pub fn content(mut self, content: impl IntoElement) -> Self {
        self.content = Some(content.into_any_element());
        self
    }

    /// A line under the body, with a hairline above.
    pub fn footer(mut self, footer: Option<impl IntoElement>) -> Self {
        self.footer = footer.map(IntoElement::into_any_element);
        self
    }

    pub fn render(self, cx: &App) -> Observed<Stateful<Div>> {
        let line = palette(cx).line;
        let part = |part: &str| SharedString::from(format!("{}-{part}", self.id));
        let body = match self.content {
            Some(content) => div()
                .id(part("content"))
                .test_support()
                .flex()
                .flex_col()
                .flex_1()
                .min_h_0()
                .min_w_0()
                .child(content),
            None => v_flex()
                .id(part("body"))
                .test_support()
                .flex_1()
                .min_h_0()
                .min_w_0()
                .overflow_y_scroll()
                .gap(dp(GAP))
                .p(dp(PANE_PADDING))
                .children(self.body),
        };
        v_flex()
            .id(self.id.clone())
            .test_support()
            .size_full()
            .min_w_0()
            .min_h_0()
            .children(self.heading.map(|content| {
                h_flex()
                    .id(part("heading"))
                    .test_support()
                    .flex_none()
                    .min_w_0()
                    .items_start()
                    .px(dp(PANE_PADDING))
                    .pt(dp(PANE_PADDING))
                    .child(content)
            }))
            .children(self.banner.map(|banner| {
                div()
                    .id(part("banner"))
                    .test_support()
                    .flex_none()
                    .px(dp(PANE_PADDING))
                    .pt(dp(UNDER_HEADING))
                    .child(banner)
            }))
            .children(
                self.tabs
                    .map(|(strip, row)| tabs::strip(part("tabs"), &strip, row, cx)),
            )
            .child(body)
            .children(self.footer.map(|footer| {
                div()
                    .id(part("footer"))
                    .test_support()
                    .flex_none()
                    .px(dp(PANE_PADDING))
                    .py(dp(6.))
                    .border_t_1()
                    .border_color(line)
                    .child(footer)
            }))
    }
}

/// A stacked pane's height in dp: where it starts, the least it keeps and,
/// if any, the most it takes.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Pane {
    pub start: f32,
    pub least: f32,
    pub most: Option<f32>,
}

impl Pane {
    pub const fn new(start: f32) -> Self {
        Self {
            start,
            least: 0.,
            most: None,
        }
    }

    pub const fn least(mut self, least: f32) -> Self {
        self.least = least;
        self
    }

    pub const fn most(mut self, most: f32) -> Self {
        self.most = Some(most);
        self
    }

    fn range(&self, window: &Window) -> Range<Pixels> {
        let most = self.most.map_or(Pixels::MAX, |most| dp_px(most, window));
        dp_px(self.least, window)..most
    }
}

/// The heights of a stacked split's table (`lead`) and inspector
/// (`trail`). The default shares the height one to two: 190 and 380,
/// keeping 96 and 220.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Stacked {
    pub lead: Pane,
    pub trail: Pane,
}

impl Default for Stacked {
    fn default() -> Self {
        Self {
            lead: Pane::new(STACKED_LIST_HEIGHT).least(LIST_MIN_HEIGHT),
            trail: Pane::new(STACKED_HEIGHT).least(MIN_HEIGHT),
        }
    }
}

impl Stacked {
    /// How tall a split with these heights is on a page that scrolls its
    /// frame, where it has no room to fill: the least heights stacked,
    /// [`SHORT_HEIGHT`] beside, and a table alone keeps a few rows.
    pub fn short_height(&self, beside: bool, open: bool) -> f32 {
        match (open, beside) {
            (true, false) => self.lead.least + self.trail.least,
            (true, true) => SHORT_HEIGHT,
            (false, _) => SHORT_LIST_HEIGHT,
        }
    }
}

/// What a page keeps for its split: Kit's panel states for either
/// arrangement, and the inspector's width beside the table in dp, so it
/// scales with the text size.
pub struct InspectorSplit {
    beside: Entity<ResizableState>,
    stacked: Entity<ResizableState>,
    width: Rc<Cell<f32>>,
    heights: Stacked,
    /// The stacked table's height from [`Self::lead_start`], if the page
    /// gives one.
    lead: Cell<Option<f32>>,
    /// Whether the user has dragged the stacked split, after which its
    /// sizes are theirs.
    dragged: Rc<Cell<bool>>,
    _resized: Subscription,
    _dragged: Subscription,
}

impl InspectorSplit {
    /// A split whose inspector starts `width` dp wide, the width the user
    /// left it at; [`WIDTH`] without one. `remember` hears the new width
    /// once each drag ends, to save it.
    pub fn new(
        width: Option<f32>,
        remember: impl Fn(f32, &mut App) + 'static,
        cx: &mut App,
    ) -> Self {
        let beside = cx.new(|_| ResizableState::default());
        let stacked = cx.new(|_| ResizableState::default());
        let width = Rc::new(Cell::new(start_width(width)));
        // Kit tells the state once a drag ends, not while it moves.
        let _resized = cx.subscribe(&beside, {
            let width = width.clone();
            move |state, _: &ResizablePanelEvent, cx| {
                let Some(&size) = state.read(cx).sizes().get(1) else {
                    return;
                };
                let dp = f32::from(size) * BASE_TEXT / crate::text_size::current(cx);
                width.set(dp);
                remember(dp, cx);
            }
        });
        let dragged = Rc::new(Cell::new(false));
        let _dragged = cx.subscribe(&stacked, {
            let dragged = dragged.clone();
            move |_, _: &ResizablePanelEvent, _| dragged.set(true)
        });
        Self {
            beside,
            stacked,
            width,
            heights: Stacked::default(),
            lead: Cell::new(None),
            dragged,
            _resized,
            _dragged,
        }
    }

    /// The heights of the table and the inspector when stacked, in place
    /// of the default one to two.
    pub fn stacked(mut self, heights: Stacked) -> Self {
        self.heights = heights;
        self
    }

    /// Stacked, the table starts `start` dp high, kept within its least and
    /// most, and the inspector takes the rest: for a table whose height
    /// follows its data, given while rendering. A new start lays the split
    /// out again, until the user drags it; then their sizes win.
    pub fn lead_start(&self, start: f32, cx: &mut App) {
        let lead = self.heights.lead;
        let start = start.min(lead.most.unwrap_or(f32::MAX)).max(lead.least);
        if self.dragged.get() || self.lead.get() == Some(start) {
            return;
        }
        self.lead.set(Some(start));
        // Kit keeps the sizes it first laid out; forget them.
        self.stacked.update(cx, |state, _| state.clear());
    }

    /// How tall this split is on a page that scrolls its frame; see
    /// [`Stacked::short_height`].
    pub fn short_height(&self, beside: bool, open: bool) -> f32 {
        self.heights.short_height(beside, open)
    }

    /// The inspector's width beside the table, in dp, as the user last
    /// left it: what it starts at and what is saved.
    pub fn width(&self) -> f32 {
        self.width.get()
    }

    /// The inspector's width beside the table as Kit last laid it out, in
    /// dp. Kit rescales both panels when the window or the text size
    /// changes and says nothing, so [`Self::width`], which changes only
    /// when a drag ends, can be stale; anything that lays out by the
    /// inspector's width reads this. [`Self::width`] until the split
    /// first lays out.
    pub fn live_width(&self, cx: &App) -> f32 {
        match self.beside.read(cx).sizes().as_slice() {
            [_, size] => f32::from(*size) * BASE_TEXT / crate::text_size::current(cx),
            _ => self.width(),
        }
    }

    /// Kit's state for the split beside the table, for tests that resize it.
    #[cfg(any(test, feature = "testing"))]
    pub fn beside_state(&self) -> &Entity<ResizableState> {
        &self.beside
    }

    /// Kit's state for the stacked split, for tests that resize it.
    #[cfg(any(test, feature = "testing"))]
    pub fn stacked_state(&self) -> &Entity<ResizableState> {
        &self.stacked
    }
}

/// A remembered width, or [`WIDTH`]; never narrower than [`MIN_WIDTH`].
fn start_width(width: Option<f32>) -> f32 {
    match width {
        Some(width) if width.is_finite() => width.max(MIN_WIDTH),
        _ => WIDTH,
    }
}

/// How tall a split with the default heights is on a page that scrolls
/// its frame. A page gives its split this height, or the panes take their
/// contents' heights.
pub fn short_height(beside: bool, open: bool) -> f32 {
    Stacked::default().short_height(beside, open)
}

/// The table and its inspector, in a split with the id `id`: beside it,
/// resizable, when `beside`, as on a page at least 900 wide; otherwise
/// under it. Without an inspector the table fills the split. Both run edge
/// to edge, and Kit's handle draws the hairline between them. The split
/// fills its parent; on a page that scrolls its frame, the parent gives it
/// [`InspectorSplit::short_height`].
pub fn split(
    id: impl Into<SharedString>,
    split: &InspectorSplit,
    beside: bool,
    table: AnyElement,
    inspector: Option<AnyElement>,
    window: &Window,
) -> AnyElement {
    let least = split.short_height(beside, inspector.is_some());
    let frame = div()
        .id(id.into())
        .test_support()
        .flex()
        .flex_col()
        .flex_1()
        .w_full()
        .min_w_0()
        .min_h(dp(least));
    let Some(inspector) = inspector else {
        return frame.child(table).into_any_element();
    };
    let dp = |n: f32| dp_px(n, window);
    let panels = if beside {
        h_resizable("inspector-split")
            .with_state(&split.beside)
            .child(
                resizable_panel()
                    .size_range(dp(LIST_MIN_WIDTH)..Pixels::MAX)
                    .child(table),
            )
            .child(
                resizable_panel()
                    .size(dp(split.width()))
                    .size_range(dp(MIN_WIDTH)..Pixels::MAX)
                    .flex_none()
                    .child(inspector),
            )
    } else {
        let Stacked { lead, trail } = split.heights;
        // A table that gives its height keeps it, and the inspector fills.
        let given = split.lead.get();
        v_resizable("inspector-split-stacked")
            .with_state(&split.stacked)
            .child(
                resizable_panel()
                    .size(dp(given.unwrap_or(lead.start)))
                    .size_range(lead.range(window))
                    .child(table),
            )
            .child(
                resizable_panel()
                    .when(given.is_none(), |this| this.size(dp(trail.start)))
                    .size_range(trail.range(window))
                    .child(inspector),
            )
    };
    frame.child(panels).into_any_element()
}

#[cfg(test)]
mod tests;
