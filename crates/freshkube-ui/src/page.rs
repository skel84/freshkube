//! The page frame every page shares: edge to edge, or padded for a page of
//! cards; the card a body sits in; and `PageHeader`
//! (docs/DESIGN.md#components).
use gpui_kit::base::ObservedElement as Observed;
use gpui_kit::component::{h_flex, v_flex};
use gpui_kit::prelude::*;
use std::cell::Cell;
use std::rc::Rc;

use gpui_kit::{
    AnyElement, App, Div, ElementId, Pixels, SharedString, Stateful, TestSupportExt, Window,
    canvas, div, px,
};

use crate::palette::palette;
use crate::ui::{dp, toolbar_label};

/// Left and right padding of a [`padded`] page.
pub const PAGE_PADDING: f32 = 26.;
/// Padding above a padded page's header and below its body.
pub const PAGE_TOP: f32 = 22.;
pub const PAGE_BOTTOM: f32 = 18.;
/// Between a padded page's header, any banners and the body.
pub const PAGE_GAP: f32 = 14.;
/// Around the content of a [`page`]'s panes: its header, a banner, a grid.
pub const PANE_PADDING: f32 = 12.;
/// Above and below that content.
pub const PANE_PADDING_Y: f32 = 10.;
/// A toolbar row's height: the header's row, and its secondary row.
pub const TOOLBAR_HEIGHT: f32 = 38.;
/// Below this width, the header's own (the page's inside its insets), the
/// header stacks its parts. Pods' toolbar with all four status chips takes
/// about 870 dp on one row at 13 px.
pub const HEADER_NARROW: f32 = 880.;
/// The header's filter beside the title, when it fits.
const FILTER_WIDTH: f32 = 150.;

/// A page's frame, without margins: its header, any banners and its panes
/// stack edge to edge, and the caller puts what isn't a pane, such as the
/// header, in an [`inset`]. The caller adds scrolling.
pub fn page(id: impl Into<ElementId>) -> Observed<Stateful<Div>> {
    v_flex().id(id).test_support().size_full().min_h_0()
}

/// Content inside a [`page`]'s pane: padded 12 at the sides and 10 above
/// and below.
pub fn inset() -> Div {
    div()
        .flex_none()
        .min_w_0()
        .px(dp(PANE_PADDING))
        .py(dp(PANE_PADDING_Y))
}

/// A [`page`]'s toolbar: its [`PageHeader`], padded 12 at the sides, with a
/// hairline under it.
pub fn toolbar(cx: &App) -> Div {
    div()
        .flex_none()
        .min_w_0()
        .px(dp(PANE_PADDING))
        .border_b_1()
        .border_color(palette(cx).line)
}

/// The frame of a page of cards, which keeps its margins until it moves to
/// [`page`]: padding and the gap between its parts. The caller adds
/// scrolling.
pub fn padded(id: impl Into<ElementId>) -> Observed<Stateful<Div>> {
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
/// it, its state and when it last changed. A line too long for the page
/// wraps between its parts and keeps each whole.
pub fn meta_line(id: impl Into<ElementId>, cx: &App) -> Observed<Stateful<Div>> {
    h_flex()
        .id(id)
        .test_support()
        .flex_none()
        .flex_wrap()
        .gap_y(dp(2.))
        .whitespace_nowrap()
        .text_size(dp(11.))
        .text_color(palette(cx).muted)
}

/// A page's header, a toolbar: the title as its leading label, the filter
/// and the status chips, then the controls at the right, each
/// [`CONTROL_HEIGHT`](crate::ui::CONTROL_HEIGHT) high, on a row
/// [`TOOLBAR_HEIGHT`] high, with the meta line below. Its ids derive from the
/// page's prefix: it draws `<prefix>-title`, `<prefix>-toolbar` (the row),
/// `<prefix>-slot-<n>` (each control's box) and `<prefix>-scope`, and
/// [`id`](Self::id) names the parts the caller builds. It lays them out on
/// one row when the content is at least [`HEADER_NARROW`] wide, wrapping the
/// controls below when they don't fit, and stacked otherwise, where the
/// controls wrap among themselves. A page without margins puts it in a
/// [`toolbar`].
///
/// A page with a [`secondary`](Self::secondary) row renders with
/// [`render_fit`](Self::render_fit), which places the controls by what fits.
pub struct PageHeader {
    prefix: SharedString,
    title: SharedString,
    title_id: SharedString,
    scope_id: SharedString,
    filter: Option<Div>,
    chips: Option<AnyElement>,
    controls: Vec<AnyElement>,
    secondary: Option<AnyElement>,
    meta: Vec<AnyElement>,
    narrow: bool,
}

/// Where a header with a secondary row puts its controls.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum ControlsRow {
    /// At the right of the title's row, as a header without one does.
    #[default]
    Title,
    /// At the right of the secondary row.
    Secondary,
    /// Left-aligned on a row of their own, below the secondary row.
    Own,
}

/// The natural widths of a fitted header's parts, measured as it draws.
/// Each part sits in a box of its own size wherever the controls go, so the
/// widths don't depend on the placement they decide, and the header can't
/// flip between two placements.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct HeaderWidths {
    /// The title and filter.
    leading: Pixels,
    /// The chips, or `None` without any.
    chips: Option<Pixels>,
    /// The secondary row, at most the header's width.
    secondary: Pixels,
    /// The controls side by side, with the gaps between them.
    controls: Pixels,
    /// The header's own width.
    width: Pixels,
}

impl HeaderWidths {
    /// Where the controls go: beside the title and chips when they fit
    /// there, else beside the secondary row, else on a row of their own.
    fn place(&self, gap: Pixels) -> ControlsRow {
        let chips = self.chips.map_or(px(0.), |chips| gap + chips);
        if self.leading + chips + gap + self.controls <= self.width {
            ControlsRow::Title
        } else if self.secondary + gap + self.controls <= self.width {
            ControlsRow::Secondary
        } else {
            ControlsRow::Own
        }
    }
}

/// What a fitted header keeps between frames: the widths its parts
/// measured, and the placement they chose.
#[derive(Default)]
struct Fit {
    leading: Cell<Pixels>,
    chips: Cell<Pixels>,
    secondary: Cell<Pixels>,
    controls: Cell<Pixels>,
    placement: Cell<ControlsRow>,
}

/// `element` in a box of its own width, whose width lands in `slot` each
/// time it prepaints.
fn measured(element: impl IntoElement, slot: impl Fn(Pixels) + 'static) -> Div {
    div().flex_none().relative().child(element).child(
        canvas(move |bounds, _, _| slot(bounds.size.width), |_, _, _, _| {})
            .absolute()
            .size_full(),
    )
}

/// One of a toolbar's rows: [`TOOLBAR_HEIGHT`] high, its parts centred.
fn row(content: impl IntoElement) -> Div {
    h_flex()
        .w_full()
        .min_h(dp(TOOLBAR_HEIGHT))
        .items_center()
        .child(content)
}

/// Each control in a box of its own width, `<prefix>-slot-<n>`: a
/// control whose root fills its parent, such as a select, would otherwise
/// take a whole line of a wrapping row.
fn boxed(prefix: &str, controls: Vec<AnyElement>) -> Vec<AnyElement> {
    controls
        .into_iter()
        .enumerate()
        .map(|(ix, control)| {
            div()
                .id(SharedString::from(format!("{prefix}-slot-{ix}")))
                .test_support()
                .flex_none()
                .child(control)
                .into_any_element()
        })
        .collect()
}

/// The meta line under a toolbar's rows.
fn meta_row(id: SharedString, parts: Vec<AnyElement>, cx: &App) -> Observed<Stateful<Div>> {
    meta_line(id, cx).pb(dp(8.)).children(parts)
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
            secondary: None,
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

    /// One control at the right, in order: source or namespace, columns,
    /// time range, refresh.
    pub fn control(mut self, control: impl IntoElement) -> Self {
        self.controls.push(control.into_any_element());
        self
    }

    /// A left-aligned row below the title's, such as category segments,
    /// before the meta line. A header with one renders with
    /// [`render_fit`](Self::render_fit).
    pub fn secondary(mut self, row: impl IntoElement) -> Self {
        self.secondary = Some(row.into_any_element());
        self
    }

    /// The meta line's parts: source, count, state, time.
    pub fn meta(mut self, parts: impl IntoIterator<Item = AnyElement>) -> Self {
        self.meta.extend(parts);
        self
    }

    pub fn render(self, cx: &App) -> Div {
        debug_assert!(
            self.secondary.is_none(),
            "a header with a secondary row renders with render_fit"
        );
        let narrow = self.narrow;
        let toolbar_id = self.id("toolbar");
        let (title_id, scope_id) = (self.title_id, self.scope_id);
        let leading = h_flex()
            .gap(dp(8.))
            .min_w_0()
            .child(toolbar_label(self.title, cx).id(title_id).test_support())
            .children(self.filter.map(|filter| {
                filter
                    .when_else(
                        narrow,
                        |this| this.flex_1(),
                        |this| this.w(dp(FILTER_WIDTH)),
                    )
                    .min_w_0()
            }));
        // The controls wrap among themselves when even a line of their own
        // is too narrow.
        let has_controls = !self.controls.is_empty();
        let controls = h_flex()
            .flex_wrap()
            .items_center()
            .gap(dp(8.))
            .children(boxed(&self.prefix, self.controls));
        let toolbar = if narrow {
            // Stacked, a part a page doesn't have takes no row.
            v_flex()
                .w_full()
                .child(row(leading.w_full()).id(toolbar_id).test_support())
                .children(self.chips.map(row))
                .when(has_controls, |this| this.child(row(controls.w_full())))
                .into_any_element()
        } else {
            // The controls start at their own width and fill what's left of
            // the line, at its right; when they don't fit beside the title
            // and chips they take a line of their own.
            row(h_flex()
                .w_full()
                .flex_wrap()
                .items_center()
                .gap(dp(8.))
                .child(leading)
                .children(self.chips)
                .child(controls.flex_grow_1().max_w_full().justify_end()))
            .id(toolbar_id)
            .test_support()
            .into_any_element()
        };
        let meta = (!self.meta.is_empty()).then(|| meta_row(scope_id, self.meta, cx));
        v_flex().w_full().flex_none().child(toolbar).children(meta)
    }

    /// The header with its [`secondary`](Self::secondary) row: the title's
    /// row, the secondary row, then the meta line. Wide, the controls stay at
    /// the right of the title's row when they fit beside the title, filter
    /// and chips; otherwise at the right of the secondary row when they fit
    /// there; otherwise left-aligned on a row of their own below it, where
    /// they wrap if they must. Narrow, the parts stack: title and filter,
    /// chips, the secondary row, then the controls.
    ///
    /// The header measures its parts as it draws and keeps the widths under
    /// its prefix, so the page holds no state for it. When they call for
    /// another placement it draws again on the next frame; a resize or a new
    /// text size measures again. The first frame a prefix draws puts the
    /// controls on the title's row, until its parts have been measured. The
    /// secondary row is `<prefix>-secondary` and the controls' row
    /// `<prefix>-controls`.
    pub fn render_fit(self, window: &mut Window, cx: &mut App) -> Div {
        let fit = window.use_keyed_state(self.id("fit"), cx, |_, _| Rc::new(Fit::default()));
        let state = fit.read(cx).clone();
        let placement = if self.narrow {
            ControlsRow::Own
        } else {
            state.placement.get()
        };
        let gap = dp(8.);
        let (controls_id, secondary_id) = (self.id("controls"), self.id("secondary"));
        let toolbar_id = self.id("toolbar");
        let (title_id, scope_id) = (self.title_id, self.scope_id);
        let leading = h_flex()
            .gap(gap)
            .items_center()
            .child(toolbar_label(self.title, cx).id(title_id).test_support())
            .children(self.filter.map(|filter| {
                filter
                    .when_else(
                        self.narrow,
                        |this| this.flex_1(),
                        |this| this.w(dp(FILTER_WIDTH)),
                    )
                    .min_w_0()
            }));
        let controls = {
            let state = state.clone();
            // Each control in a box of its own width, as in `render`; their
            // sum is the same whichever row they sit on.
            h_flex()
                .gap(gap)
                .items_center()
                .children(boxed(&self.prefix, self.controls))
                .on_children_prepainted(move |bounds, window, _| {
                    let gaps =
                        gap.to_pixels(window.rem_size()) * bounds.len().saturating_sub(1) as f32;
                    let widths = bounds.iter().map(|bounds| bounds.size.width);
                    state
                        .controls
                        .set(widths.fold(gaps, |sum, width| sum + width));
                })
                .id(controls_id)
                .test_support()
        };
        let secondary = self.secondary.map(|row| {
            let state = state.clone();
            measured(row, move |width| state.secondary.set(width))
                .id(secondary_id)
                .test_support()
                .max_w_full()
        });
        let meta = (!self.meta.is_empty()).then(|| meta_row(scope_id, self.meta, cx));
        if self.narrow {
            return v_flex()
                .w_full()
                .flex_none()
                .child(
                    row(leading.w_full().min_w_0())
                        .id(toolbar_id)
                        .test_support(),
                )
                .children(self.chips.map(row))
                .children(secondary.map(row))
                .child(row(controls.w_full().flex_wrap()))
                .children(meta);
        }
        let has_chips = self.chips.is_some();
        let leading = {
            let state = state.clone();
            measured(leading, move |width| state.leading.set(width))
        };
        let chips = self.chips.map(|chips| {
            let state = state.clone();
            measured(chips, move |width| state.chips.set(width))
        });
        // The controls at the right of a row, after whatever else is on it.
        let at_right = |controls: Observed<Stateful<Div>>| {
            h_flex().flex_1().justify_end().child(controls.flex_none())
        };
        let (title_controls, secondary_controls, own_controls) = match placement {
            ControlsRow::Title => (Some(controls), None, None),
            ControlsRow::Secondary => (None, Some(controls), None),
            ControlsRow::Own => (None, None, Some(controls)),
        };
        let title_row = row(h_flex()
            .w_full()
            .flex_wrap()
            .items_center()
            .gap(gap)
            .child(leading)
            .children(chips)
            .children(title_controls.map(at_right)))
        .id(toolbar_id)
        .test_support();
        let secondary_row = (secondary.is_some() || secondary_controls.is_some()).then(|| {
            row(h_flex()
                .w_full()
                .items_center()
                .gap(gap)
                .children(secondary)
                .children(secondary_controls.map(at_right)))
        });
        // Decides after every part above has measured itself, since children
        // prepaint in order.
        let decide = canvas(
            move |bounds, window, _| {
                let widths = HeaderWidths {
                    leading: state.leading.get(),
                    chips: has_chips.then(|| state.chips.get()),
                    secondary: state.secondary.get(),
                    controls: state.controls.get(),
                    width: bounds.size.width,
                };
                let next = widths.place(gap.to_pixels(window.rem_size()));
                if next != state.placement.get() {
                    state.placement.set(next);
                    // A notify while prepainting would schedule no frame.
                    window.on_next_frame(move |_, cx| fit.update(cx, |_, cx| cx.notify()));
                }
            },
            |_, _, _, _| {},
        )
        .absolute()
        .size_full();
        v_flex()
            .w_full()
            .flex_none()
            .relative()
            .child(title_row)
            .children(secondary_row)
            .children(own_controls.map(|controls| row(controls.w_full().flex_wrap())))
            .children(meta)
            .child(decide)
    }
}

#[cfg(test)]
mod tests {
    use gpui_kit::component::Root;
    use gpui_kit::test::TestWindowExt;
    use gpui_kit::{
        AnyWindowHandle, AppContext, Bounds, Context, IntoElement, Pixels, Render, TestAppContext,
        Window, px, size,
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

    /// A header like Applications': chips and categories of fixed widths,
    /// and `CONTROLS`: about 1,240 dp beside the title, 1,088 beside the
    /// categories.
    struct Fitted {
        narrow: bool,
    }

    impl Render for Fitted {
        fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
            let header = PageHeader::new("app", "Applications", self.narrow);
            let part =
                |id: SharedString, width: f32| div().id(id).test_support().w(dp(width)).h(dp(20.));
            let chips = part(header.id("chips"), 220.);
            let categories = part(header.id("categories"), 360.);
            let controls = (0..CONTROLS.len())
                .map(|ix| part(header.id(&format!("control-{ix}")), CONTROLS[ix]))
                .collect::<Vec<_>>();
            controls
                .into_iter()
                .fold(header, PageHeader::control)
                .filter(div().h(dp(28.)))
                .chips(Some(chips))
                .secondary(categories)
                .meta([div().child("Example data").into_any_element()])
                .render_fit(window, cx)
                .w_full()
        }
    }

    /// Where the fitted header put its parts at `width` × 560 and `text` px,
    /// once it settled, with how many redraws it asked for on the way.
    struct Placed {
        title: Bounds<Pixels>,
        categories: Bounds<Pixels>,
        controls: Bounds<Pixels>,
        scope: Bounds<Pixels>,
        redraws: usize,
    }

    /// The fitted header in a `width` × 560 window at `text` px.
    fn open(cx: &mut TestAppContext, width: f32, text: f32) -> AnyWindowHandle {
        cx.update(|cx| {
            gpui_kit::init(cx);
            crate::theme::install(cx);
            crate::text_size::install(None, cx);
            crate::text_size::set(text, cx);
            cx.set_reduce_motion(true);
        });
        // As a page decides it: the content's width in dp.
        let narrow = width * crate::ui::BASE_TEXT / text < HEADER_NARROW;
        cx.open_window(size(px(width), px(560.)), |window, cx| {
            let view = cx.new(|_| Fitted { narrow });
            Root::new(view, window, cx)
        })
        .into()
    }

    /// Draws until the header asks for no more frames, then reads where its
    /// parts are. Only `simulate_next_frame` stands in for the platform: no
    /// input reaches the window.
    fn settle(cx: &mut TestAppContext, handle: AnyWindowHandle) -> Placed {
        let mut redraws = 0;
        for _ in 0..4 {
            cx.run_until_parked();
            let asked = cx
                .update_window(handle, |_, window, cx| {
                    window.render_frame(cx);
                    window.simulate_next_frame(cx)
                })
                .unwrap();
            if asked == 0 {
                break;
            }
            redraws += asked;
        }
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            // Settled: drawing again asks for nothing more.
            assert_eq!(window.simulate_next_frame(cx), 0, "the header keeps moving");
            Placed {
                title: window.find("app-title").bounds(),
                categories: window.find("app-secondary").bounds(),
                controls: window.find("app-controls").bounds(),
                scope: window.find("app-scope").bounds(),
                redraws,
            }
        })
        .unwrap()
    }

    fn place(cx: &mut TestAppContext, width: f32, text: f32) -> Placed {
        let handle = open(cx, width, text);
        settle(cx, handle)
    }

    #[test]
    fn the_controls_go_where_they_fit() {
        let gap = px(8.);
        let widths = |width: f32| HeaderWidths {
            leading: px(300.),
            chips: Some(px(200.)),
            secondary: px(400.),
            controls: px(500.),
            width: px(width),
        };
        // 300 + 8 + 200 + 8 + 500 beside the title, 400 + 8 + 500 below it.
        assert_eq!(widths(1016.).place(gap), ControlsRow::Title);
        assert_eq!(widths(1015.).place(gap), ControlsRow::Secondary);
        assert_eq!(widths(908.).place(gap), ControlsRow::Secondary);
        assert_eq!(widths(907.).place(gap), ControlsRow::Own);
        let bare = HeaderWidths {
            chips: None,
            ..widths(808.)
        };
        assert_eq!(bare.place(gap), ControlsRow::Title);
    }

    #[gpui_kit::test]
    fn at_1280_and_the_default_text_size_the_controls_stay_beside_the_title(
        cx: &mut TestAppContext,
    ) {
        let placed = place(cx, 1280., crate::ui::BASE_TEXT);
        assert!(placed.controls.top() < placed.title.bottom());
        assert_eq!(placed.controls.right(), px(1280.));
        assert!(placed.categories.top() >= placed.title.bottom());
        assert_eq!(placed.categories.left(), placed.title.left());
        assert!(placed.scope.top() >= placed.categories.bottom());
        assert_eq!(placed.redraws, 0, "the first frame was already right");
    }

    #[gpui_kit::test]
    fn one_size_up_the_controls_follow_the_categories_without_input(cx: &mut TestAppContext) {
        let placed = place(cx, 1280., 14.);
        assert!(placed.controls.top() >= placed.title.bottom());
        assert_eq!(placed.controls.top(), placed.categories.top());
        assert!(placed.controls.left() > placed.categories.right());
        assert_eq!(placed.controls.right(), px(1280.));
        assert!(placed.scope.top() >= placed.controls.bottom());
        // The first frame drew them beside the title; it asked for the next.
        assert_eq!(placed.redraws, 1);
    }

    #[gpui_kit::test]
    fn a_new_text_size_moves_the_controls_and_back_without_input(cx: &mut TestAppContext) {
        let handle = open(cx, 1280., crate::ui::BASE_TEXT);
        let beside_title = settle(cx, handle);
        assert!(beside_title.controls.top() < beside_title.title.bottom());
        cx.update(|cx| crate::text_size::set(14., cx));
        let after_categories = settle(cx, handle);
        assert_eq!(after_categories.redraws, 1);
        assert_eq!(
            after_categories.controls.top(),
            after_categories.categories.top()
        );
        assert_eq!(after_categories.controls.right(), px(1280.));
        cx.update(|cx| crate::text_size::set(crate::ui::BASE_TEXT, cx));
        let back = settle(cx, handle);
        assert_eq!(back.redraws, 1);
        assert!(back.controls.top() < back.title.bottom());
        assert_eq!(back.controls.right(), px(1280.));
    }

    #[gpui_kit::test]
    fn when_neither_row_has_room_the_controls_take_their_own(cx: &mut TestAppContext) {
        let placed = place(cx, 1000., crate::ui::BASE_TEXT);
        assert!(placed.controls.top() >= placed.categories.bottom());
        assert_eq!(placed.controls.left(), placed.title.left());
        assert!(placed.scope.top() >= placed.controls.bottom());
        assert_eq!(placed.redraws, 1);
    }

    #[gpui_kit::test]
    fn narrow_the_parts_stack_and_the_controls_wrap(cx: &mut TestAppContext) {
        let placed = place(cx, 760., 20.);
        assert!(placed.categories.top() >= placed.title.bottom());
        assert!(placed.controls.top() >= placed.categories.bottom());
        assert_eq!(placed.controls.left(), placed.title.left());
        assert!(placed.controls.right() <= px(760.));
        assert!(placed.scope.top() >= placed.controls.bottom());
        assert_eq!(placed.redraws, 0);
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

    #[gpui_kit::test]
    fn the_header_is_a_toolbar_row_with_a_label_and_rows_under_it(cx: &mut TestAppContext) {
        // At 13 px a dp is a pixel; 1800 wide leaves the controls beside
        // the title, the categories on a row of their own under it.
        let handle = open(cx, 1800., 13.);
        let placed = settle(cx, handle);
        cx.update_window(handle, |_, window, _| {
            let row = window.find("app-toolbar").bounds();
            assert_eq!(row.size.height, px(TOOLBAR_HEIGHT));
            assert!((placed.title.center().y - row.center().y).abs() < px(0.5));
            assert!((placed.controls.center().y - row.center().y).abs() < px(0.5));
            // The categories take a second row of the same height.
            assert_eq!(
                placed.categories.top(),
                row.bottom() + (px(TOOLBAR_HEIGHT) - px(20.)) / 2.
            );
            for ix in 0..CONTROLS.len() {
                let slot = window.find(format!("app-slot-{ix}")).bounds();
                assert!(slot.top() >= row.top() && slot.bottom() <= row.bottom());
            }
            assert!(placed.scope.top() >= placed.categories.bottom());
        })
        .unwrap();
    }

    /// A header with no controls and a meta line of `parts` like parts.
    struct Bare {
        parts: usize,
    }

    impl Render for Bare {
        fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
            let meta = (0..self.parts)
                .map(|ix| {
                    div()
                        .id(SharedString::from(format!("part-{ix}")))
                        .test_support()
                        .child(format!("part {ix} of the line ·"))
                        .into_any_element()
                })
                .collect::<Vec<_>>();
            PageHeader::new("bare", "prod-fra", true)
                .meta(meta)
                .render(cx)
                .w_full()
        }
    }

    #[gpui_kit::test]
    fn narrow_a_header_without_controls_keeps_the_meta_under_the_title_and_wraps_it(
        cx: &mut TestAppContext,
    ) {
        cx.update(|cx| {
            gpui_kit::init(cx);
            crate::theme::install(cx);
            crate::text_size::install(None, cx);
            crate::text_size::set(20., cx);
            cx.set_reduce_motion(true);
        });
        let handle = cx.open_window(size(px(480.), px(560.)), |window, cx| {
            let view = cx.new(|_| Bare { parts: 12 });
            Root::new(view, window, cx)
        });
        cx.update_window(handle.into(), |_, window, cx| {
            window.render_frame(cx);
            let row = window.find("bare-toolbar").bounds();
            let scope = window.find("bare-scope").bounds();
            assert_eq!(scope.top(), row.bottom(), "an empty row sits between");
            let parts = (0..12)
                .map(|ix| window.find(format!("part-{ix}")).bounds())
                .collect::<Vec<_>>();
            let one_line = parts[0].size.height;
            for part in &parts {
                assert!(part.right() <= px(480.), "{part:?} leaves the page");
                // Whole: a part wraps as one, never inside itself.
                assert_eq!(part.size.height, one_line);
            }
            assert!(parts[11].top() > parts[0].top(), "the line didn't wrap");
        })
        .unwrap();
    }
}
