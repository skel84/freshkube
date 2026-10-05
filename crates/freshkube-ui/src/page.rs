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
/// one row when the content is at least [`HEADER_NARROW`] wide, wrapping the
/// controls below when they don't fit, and stacked otherwise, where the
/// controls wrap among themselves.
pub struct PageHeader {
    prefix: SharedString,
    title: SharedString,
    title_id: SharedString,
    scope_id: SharedString,
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
        let prefix = prefix.into();
        Self {
            title_id: format!("{prefix}-title").into(),
            scope_id: format!("{prefix}-scope").into(),
            prefix,
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
        let (title_id, scope_id) = (self.title_id, self.scope_id);
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
        // Each control sits in a box of its own size: a control whose root
        // fills its parent, such as a select, would otherwise take a whole
        // line of a wrapping row. The controls wrap among themselves when
        // even a line of their own is too narrow.
        let controls = h_flex().flex_wrap().gap(dp(8.)).children(
            self.controls
                .into_iter()
                .map(|control| div().flex_none().child(control)),
        );
        let toolbar = if narrow {
            v_flex()
                .w_full()
                .gap(dp(8.))
                .child(leading.w_full())
                .children(self.chips)
                .child(controls.w_full())
        } else {
            // The controls start at their own width and fill what's left of
            // the line, at its right; when they don't fit beside the title
            // and chips they take a line of their own.
            h_flex()
                .w_full()
                .flex_wrap()
                .gap(dp(8.))
                .child(leading)
                .children(self.chips)
                .child(controls.flex_grow_1().max_w_full().justify_end())
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

#[cfg(test)]
mod tests {
    use gpui_kit::component::Root;
    use gpui_kit::test::TestWindowExt;
    use gpui_kit::{
        AppContext, Bounds, Context, IntoElement, Pixels, Render, TestAppContext, Window, px, size,
    };

    use super::*;

    /// Applications' controls, each filling its parent around a fixed-width
    /// part, as a select does.
    const CONTROLS: [f32; 7] = [160., 160., 132., 20., 80., 100., 20.];

    struct Header {
        narrow: bool,
    }

    impl Render for Header {
        fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
            let header = PageHeader::new("app", "Applications", self.narrow);
            let controls = CONTROLS
                .iter()
                .enumerate()
                .map(|(ix, width)| {
                    div()
                        .id(header.id(&format!("control-{ix}")))
                        .test_support()
                        .size_full()
                        .child(div().w(dp(*width)).h(dp(20.)))
                })
                .collect::<Vec<_>>();
            controls
                .into_iter()
                .fold(header, PageHeader::control)
                .render(cx)
                .w_full()
        }
    }

    /// The title and controls the header drew at `width` × 560 and 20 px text.
    fn draw(
        cx: &mut TestAppContext,
        width: f32,
        narrow: bool,
    ) -> (Bounds<Pixels>, Vec<Bounds<Pixels>>) {
        cx.update(|cx| {
            gpui_kit::init(cx);
            crate::theme::install(cx);
            crate::text_size::install(None, cx);
            crate::text_size::set(20., cx);
            cx.set_reduce_motion(true);
        });
        let handle = cx.open_window(size(px(width), px(560.)), |window, cx| {
            let view = cx.new(|_| Header { narrow });
            Root::new(view, window, cx)
        });
        cx.update_window(handle.into(), |_, window, cx| {
            window.render_frame(cx);
            let title = window.find("app-title").bounds();
            let controls = (0..CONTROLS.len())
                .map(|ix| window.find(format!("app-control-{ix}")).bounds())
                .collect();
            (title, controls)
        })
        .unwrap()
    }

    /// Every control inside the page at its own width: one that fills its
    /// parent doesn't take the line.
    fn assert_inside(width: f32, controls: &[Bounds<Pixels>]) {
        let scale = controls[0].size.width / CONTROLS[0];
        assert!(
            scale > px(1.),
            "the controls are drawn larger than at 14 px"
        );
        for (control, own) in controls.iter().zip(CONTROLS) {
            assert!(
                control.left() >= px(0.) && control.right() <= px(width),
                "{width}: {control:?}"
            );
            assert!(
                (control.size.width - scale * own).abs() < px(0.5),
                "{width}: {control:?}"
            );
        }
    }

    #[gpui_kit::test]
    fn narrow_controls_wrap_inside_the_page(cx: &mut TestAppContext) {
        let (title, controls) = draw(cx, 760., true);
        assert_inside(760., &controls);
        assert!(controls[0].top() > title.bottom());
        assert!(
            controls
                .iter()
                .any(|control| control.top() > controls[0].bottom())
        );
    }

    #[gpui_kit::test]
    fn wide_controls_wrap_below_the_title_at_the_right(cx: &mut TestAppContext) {
        let (title, controls) = draw(cx, 1000., false);
        assert_inside(1000., &controls);
        assert!(
            controls
                .iter()
                .all(|control| control.top() > title.bottom())
        );
        assert_eq!(controls.last().unwrap().right(), px(1000.));
    }

    #[gpui_kit::test]
    fn wide_controls_stay_beside_the_title_when_they_fit(cx: &mut TestAppContext) {
        let (title, controls) = draw(cx, 1800., false);
        assert_inside(1800., &controls);
        assert!(
            controls
                .iter()
                .all(|control| control.top() < title.bottom())
        );
        assert_eq!(controls.last().unwrap().right(), px(1800.));
    }
}
