//! The page frame every page shares: its padding, the card its body sits
//! in and `PageHeader` (docs/DESIGN.md#components).
use gpui_kit::base::ObservedElement as Observed;
use gpui_kit::component::{h_flex, v_flex};
use gpui_kit::prelude::*;
use gpui_kit::{AnyElement, App, Div, ElementId, SharedString, Stateful, TestSupportExt, div, px};

use crate::palette::palette;
use crate::ui::{dp, page_title};

/// Left and right padding of every page.
pub const PAGE_PADDING: f32 = 26.;
/// Padding above the header and below the body.
pub const PAGE_TOP: f32 = 22.;
pub const PAGE_BOTTOM: f32 = 18.;
/// Between the header, any banners and the body.
pub const PAGE_GAP: f32 = 14.;
/// Below this content width the header stacks its parts.
pub const HEADER_NARROW: f32 = 920.;
/// The header's filter beside the title, when it fits.
const FILTER_WIDTH: f32 = 150.;

/// A page's frame: padding and the gap between its parts. The caller adds
/// scrolling.
pub fn page(id: impl Into<ElementId>) -> Observed<Stateful<Div>> {
    v_flex()
        .id(id)
        .test_support()
        .size_full()
        .min_h_0()
        .px(dp(PAGE_PADDING))
        .pt(dp(PAGE_TOP))
        .pb(dp(PAGE_BOTTOM))
        .gap(dp(PAGE_GAP))
}

/// A card: a 12 px radius, a hairline and the card surface.
pub fn card(cx: &App) -> Div {
    let p = palette(cx);
    v_flex()
        .rounded(px(12.))
        .border_1()
        .border_color(p.line)
        .bg(p.surface)
        .min_w_0()
}

/// The line under a page header: where the data comes from, how much of
/// it, its state and when it last changed.
pub fn meta_line(id: impl Into<ElementId>, cx: &App) -> Observed<Stateful<Div>> {
    h_flex()
        .id(id)
        .test_support()
        .flex_none()
        .text_size(dp(11.))
        .text_color(palette(cx).muted)
}

/// A page's header: the title and filter, the status chips, then the
/// controls at the right, with the meta line below. Its ids derive from the
/// page's prefix: it draws `<prefix>-title` and `<prefix>-scope`, and
/// [`id`](Self::id) names the parts the caller builds. It lays them out on
/// one row when the content is at least [`HEADER_NARROW`] wide and stacked
/// otherwise.
pub struct PageHeader {
    prefix: SharedString,
    title: SharedString,
    filter: Option<Div>,
    chips: Option<AnyElement>,
    controls: Vec<AnyElement>,
    meta: Vec<AnyElement>,
    narrow: bool,
}

impl PageHeader {
    pub fn new(
        prefix: impl Into<SharedString>,
        title: impl Into<SharedString>,
        narrow: bool,
    ) -> Self {
        Self {
            prefix: prefix.into(),
            title: title.into(),
            filter: None,
            chips: None,
            controls: Vec::new(),
            meta: Vec::new(),
            narrow,
        }
    }

    /// `<prefix>-<part>`, for the filter, chips and controls.
    pub fn id(&self, part: &str) -> SharedString {
        format!("{}-{part}", self.prefix).into()
    }

    /// The filter beside the title; the header sizes it.
    pub fn filter(mut self, filter: Div) -> Self {
        self.filter = Some(filter);
        self
    }

    /// The segment and status chips after the title.
    pub fn chips(mut self, chips: Option<impl IntoElement>) -> Self {
        self.chips = chips.map(IntoElement::into_any_element);
        self
    }

    /// One control at the right, in order: source or namespace, density,
    /// columns, time range, refresh.
    pub fn control(mut self, control: impl IntoElement) -> Self {
        self.controls.push(control.into_any_element());
        self
    }

    /// The meta line's parts: source, count, state, time.
    pub fn meta(mut self, parts: impl IntoIterator<Item = AnyElement>) -> Self {
        self.meta.extend(parts);
        self
    }

    pub fn render(self, cx: &App) -> Div {
        let narrow = self.narrow;
        let title_id = self.id("title");
        let scope_id = self.id("scope");
        let leading = h_flex()
            .gap(dp(8.))
            .min_w_0()
            .child(page_title(self.title).id(title_id).test_support())
            .children(self.filter.map(|filter| {
                filter
                    .when_else(
                        narrow,
                        |this| this.flex_1(),
                        |this| this.w(dp(FILTER_WIDTH)),
                    )
                    .min_w_0()
            }));
        let controls = h_flex().flex_none().gap(dp(8.)).children(self.controls);
        let toolbar = if narrow {
            v_flex()
                .w_full()
                .gap(dp(8.))
                .child(leading.w_full())
                .children(self.chips)
                .child(controls)
        } else {
            h_flex()
                .w_full()
                .gap(dp(8.))
                .child(leading)
                .children(self.chips)
                .child(div().flex_1())
                .child(controls)
        };
        let meta = (!self.meta.is_empty()).then(|| meta_line(scope_id, cx).children(self.meta));
        v_flex()
            .w_full()
            .flex_none()
            .gap(dp(8.))
            .child(toolbar)
            .children(meta)
    }
}
