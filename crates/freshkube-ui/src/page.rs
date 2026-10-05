//! The page frame every page shares: edge to edge, or padded for a page of
//! cards; the card a body sits in; and `PageHeader`
//! (docs/DESIGN.md#components).
use gpui_kit::assets::IconName;
use gpui_kit::base::ObservedElement as Observed;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::menu::{DropdownMenu, PopupMenu, PopupMenuItem};
use gpui_kit::component::{Sizable, h_flex, v_flex};
use gpui_kit::prelude::*;
use std::cell::{Cell, RefCell};
use std::rc::Rc;

use gpui_kit::{
    Anchor, AnyElement, App, Context, Div, ElementId, Pixels, SharedString, Stateful,
    TestSupportExt, Window, canvas, div, px,
};

use crate::palette::palette;
use crate::ui::{CONTROL_HEIGHT, dp, toolbar_label};

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
/// The header's filter beside the title, at its full width.
const FILTER_WIDTH: f32 = 150.;
/// The filter's width when the row is full with every control folded.
const FILTER_MIN: f32 = 96.;

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

/// Adds a control's items to a menu. A control the header folds into its
/// "…" menu shows there as these items; a control that is itself a menu
/// builds its own from the same function, so the two can't drift apart.
pub type MenuItems = Rc<dyn Fn(PopupMenu, &mut Window, &mut Context<PopupMenu>) -> PopupMenu>;

/// What a control does when it's pressed, from its button or its item in
/// the "…" menu: one handler for both.
pub type Handler = Rc<dyn Fn(&mut Window, &mut App)>;

/// A [`Handler`] that runs `f` on the view behind `cx`, as long as it lives.
pub fn handler<V: 'static>(
    cx: &Context<V>,
    f: impl Fn(&mut V, &mut Window, &mut Context<V>) + 'static,
) -> Handler {
    let view = cx.entity().downgrade();
    Rc::new(move |window, cx| {
        _ = view.update(cx, |view, cx| f(view, window, cx));
    })
}

/// The folded form of a button: one item, `label`, that runs `handler`.
pub fn item(label: impl Into<SharedString>, handler: Handler) -> MenuItems {
    let label = label.into();
    Rc::new(move |menu, _, _| {
        let handler = handler.clone();
        menu.item(
            PopupMenuItem::new(label.clone()).on_click(move |_, window, cx| handler(window, cx)),
        )
    })
}

/// The folded form of a button that shows whether it's on, such as one of a
/// segment's: `label`, checked when it is, running `handler`.
pub fn checked_item(label: impl Into<SharedString>, checked: bool, handler: Handler) -> MenuItems {
    let label = label.into();
    Rc::new(move |menu, _, _| {
        let handler = handler.clone();
        menu.item(
            PopupMenuItem::new(label.clone())
                .checked(checked)
                .on_click(move |_, window, cx| handler(window, cx)),
        )
    })
}

/// The folded form of a button that is disabled: `label`, greyed.
pub fn disabled_item(label: impl Into<SharedString>) -> MenuItems {
    let label = label.into();
    Rc::new(move |menu, _, _| menu.item(PopupMenuItem::new(label.clone()).disabled(true)))
}

/// The folded form of a control that opens a menu or picks one of several:
/// its `items` under `label`.
pub fn submenu(label: impl Into<SharedString>, items: MenuItems) -> MenuItems {
    let label = label.into();
    Rc::new(move |menu, window, cx| {
        let items = items.clone();
        menu.submenu(label.clone(), window, cx, move |menu, window, cx| {
            items(menu, window, cx)
        })
    })
}

/// A page's header, a toolbar: the title as its leading label, the filter
/// and the status chips, then the controls at the right, each
/// [`CONTROL_HEIGHT`] high, on a row [`TOOLBAR_HEIGHT`] high, with the meta
/// line below. Its ids derive from the page's prefix: it draws
/// `<prefix>-title`, `<prefix>-toolbar` (the row), `<prefix>-controls`,
/// `<prefix>-slot-<n>` (the box of the nth control the page added),
/// `<prefix>-more`, `<prefix>-secondary` and `<prefix>-scope`, and
/// [`id`](Self::id) names the parts the caller builds. A page without
/// margins puts it in a [`toolbar`].
///
/// The row never wraps. When the parts don't fit, the controls added with
/// [`foldable`](Self::foldable) fold into a "…" menu at the row's right,
/// the rightmost first, and show there as their menu form. When they don't
/// fit even with all of them folded, the chips take a row of their own under
/// the toolbar's, and the controls unfold again as far as the row then
/// allows. The header measures its parts as it draws and keeps their widths
/// under its prefix, so the page holds no state for it; when they call for
/// another placement it draws again on the next frame. The first frame a
/// prefix draws folds nothing, until its parts have been measured.
pub struct PageHeader {
    prefix: SharedString,
    title: SharedString,
    title_id: SharedString,
    scope_id: SharedString,
    filter: Option<Div>,
    chips: Option<AnyElement>,
    controls: Vec<Control>,
    secondary: Option<AnyElement>,
    meta: Vec<AnyElement>,
}

/// A control, with its menu form if it folds.
struct Control {
    element: AnyElement,
    fold: Option<MenuItems>,
}

/// Where the header puts its parts.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Placement {
    /// How many of the foldable controls, counted from the right, are in the
    /// "…" menu.
    folded: usize,
    /// Whether the chips have a row of their own, under the toolbar's.
    chips_below: bool,
}

/// The natural widths of a header's parts, the gap between them and the
/// header's own width. Each part sits in a box of its own size wherever it
/// goes, and the filter counts at its full width, so the widths don't
/// depend on the placement they decide and the header can't flip between
/// two placements.
#[derive(Clone, Debug, Default, PartialEq)]
struct HeaderWidths {
    /// The title and filter.
    leading: Pixels,
    /// The chips, or `None` without any.
    chips: Option<Pixels>,
    /// Each control, and whether it folds.
    controls: Vec<(Pixels, bool)>,
    /// The "…" button.
    more: Pixels,
    gap: Pixels,
    width: Pixels,
}

impl HeaderWidths {
    /// The fewest folded controls that fit the chips on the toolbar's row;
    /// else the chips on a row of their own, and the fewest that fit then.
    fn place(&self) -> Placement {
        // The foldable controls, the rightmost first.
        let foldable: Vec<usize> = (0..self.controls.len())
            .rev()
            .filter(|ix| self.controls[*ix].1)
            .collect();
        let below = if self.chips.is_some() {
            &[false, true][..]
        } else {
            &[false][..]
        };
        for &chips_below in below {
            for folded in 0..=foldable.len() {
                if self.row(&foldable[..folded], chips_below) <= self.width {
                    return Placement {
                        folded,
                        chips_below,
                    };
                }
            }
        }
        Placement {
            folded: foldable.len(),
            chips_below: self.chips.is_some(),
        }
    }

    /// The toolbar's row with the `folded` controls in the menu.
    fn row(&self, folded: &[usize], chips_below: bool) -> Pixels {
        let gap = self.gap;
        let chips = self
            .chips
            .filter(|_| !chips_below)
            .map_or(px(0.), |chips| gap + chips);
        let controls = self
            .controls
            .iter()
            .enumerate()
            .filter(|(ix, _)| !folded.contains(ix))
            .fold(px(0.), |sum, (_, (width, _))| sum + gap + *width);
        let more = if folded.is_empty() {
            px(0.)
        } else {
            gap + self.more
        };
        self.leading + chips + controls + more
    }
}

/// What a header keeps between frames, under its prefix: its parts' widths
/// in rems, as they last drew, and the placement they chose.
#[derive(Default)]
struct Fit {
    label: Cell<f32>,
    chips: Cell<f32>,
    controls: RefCell<Vec<f32>>,
    placement: Cell<Placement>,
}

/// `element` in a box of its own width, whose width lands in `slot` each
/// time it prepaints.
fn measured(element: impl IntoElement, slot: impl Fn(Pixels, &Window) + 'static) -> Div {
    div().flex_none().relative().child(element).child(
        canvas(
            move |bounds, window, _| slot(bounds.size.width, window),
            |_, _, _, _| {},
        )
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

/// The meta line under a toolbar's rows.
fn meta_row(id: SharedString, parts: Vec<AnyElement>, cx: &App) -> Observed<Stateful<Div>> {
    meta_line(id, cx).pb(dp(8.)).children(parts)
}

impl PageHeader {
    pub fn new(prefix: impl Into<SharedString>, title: impl Into<SharedString>) -> Self {
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

    /// The segment and status chips after the title. They never fold.
    pub fn chips(mut self, chips: Option<impl IntoElement>) -> Self {
        self.chips = chips.map(IntoElement::into_any_element);
        self
    }

    /// A control at the right that never folds. Controls go in order:
    /// source or namespace, columns, time range, refresh.
    pub fn control(mut self, control: impl IntoElement) -> Self {
        self.controls.push(Control {
            element: control.into_any_element(),
            fold: None,
        });
        self
    }

    /// A control at the right that folds into the "…" menu when the row is
    /// full, where it shows as `items`: built with [`item`] from the
    /// button's own [`Handler`], or with [`submenu`] from the items its own
    /// menu shows.
    pub fn foldable(mut self, control: impl IntoElement, items: MenuItems) -> Self {
        self.controls.push(Control {
            element: control.into_any_element(),
            fold: Some(items),
        });
        self
    }

    /// A left-aligned row below the toolbar's, such as category segments,
    /// before the meta line: `<prefix>-secondary`.
    pub fn secondary(mut self, row: impl IntoElement) -> Self {
        self.secondary = Some(row.into_any_element());
        self
    }

    /// The meta line's parts: source, count, state, time.
    pub fn meta(mut self, parts: impl IntoIterator<Item = AnyElement>) -> Self {
        self.meta.extend(parts);
        self
    }

    pub fn render(self, window: &mut Window, cx: &mut App) -> Div {
        let fit = window.use_keyed_state(self.id("fit"), cx, |_, _| Rc::new(Fit::default()));
        let state = fit.read(cx).clone();
        let placement = state.placement.get();
        let gap = dp(8.);
        let (toolbar_id, controls_id, more_id) =
            (self.id("toolbar"), self.id("controls"), self.id("more"));
        let (secondary_id, title_id, scope_id) =
            (self.id("secondary"), self.title_id, self.scope_id);
        let has_filter = self.filter.is_some();
        let leading = h_flex()
            .flex_shrink(1.)
            .min_w_0()
            .gap(gap)
            .items_center()
            .child({
                let state = state.clone();
                measured(
                    toolbar_label(self.title, cx).id(title_id).test_support(),
                    move |width, window| state.label.set(width / window.rem_size()),
                )
            })
            .children(self.filter.map(|filter| {
                // Shrinks only when the row is full with every control folded.
                filter
                    .w(dp(FILTER_WIDTH))
                    .min_w(dp(FILTER_MIN))
                    .flex_shrink(1.)
            }));
        let has_chips = self.chips.is_some();
        let chips = self.chips.map(|chips| {
            let state = state.clone();
            measured(chips, move |width, window| {
                state.chips.set(width / window.rem_size())
            })
        });
        let (chips_row, chips_below) = if placement.chips_below {
            (None, chips.map(row))
        } else {
            (chips, None)
        };
        // The controls that fold, the rightmost first, up to the placement's.
        let foldable: Vec<bool> = self.controls.iter().map(|c| c.fold.is_some()).collect();
        let folded: Vec<usize> = (0..foldable.len())
            .rev()
            .filter(|ix| foldable[*ix])
            .take(placement.folded)
            .collect();
        let mut shown = Vec::new();
        let mut slots = Vec::new();
        let mut forms = Vec::new();
        for (ix, control) in self.controls.into_iter().enumerate() {
            if folded.contains(&ix) {
                forms.extend(control.fold);
                continue;
            }
            shown.push(ix);
            slots.push(
                div()
                    .id(SharedString::from(format!("{}-slot-{ix}", self.prefix)))
                    .test_support()
                    .flex_none()
                    .child(control.element),
            );
        }
        let more = (!forms.is_empty()).then(|| {
            Button::new(more_id)
                .ghost()
                .small()
                .size(dp(CONTROL_HEIGHT))
                .icon(IconName::Ellipsis)
                .tooltip("More")
                .dropdown_menu_with_anchor(Anchor::TopRight, move |mut menu, window, cx| {
                    for form in &forms {
                        menu = form(menu, window, cx);
                    }
                    menu
                })
        });
        let controls = {
            let state = state.clone();
            let count = foldable.len();
            h_flex()
                .flex_1()
                .justify_end()
                .items_center()
                .gap(gap)
                .children(slots)
                .children(more)
                .on_children_prepainted(move |bounds, window, _| {
                    let mut widths = state.controls.borrow_mut();
                    widths.resize(count, 0.);
                    for (ix, bounds) in shown.iter().zip(bounds) {
                        widths[*ix] = bounds.size.width / window.rem_size();
                    }
                })
                .id(controls_id)
                .test_support()
        };
        let toolbar = row(h_flex()
            .w_full()
            .min_w_0()
            .items_center()
            .gap(gap)
            .child(leading)
            .children(chips_row)
            .child(controls))
        .id(toolbar_id)
        .test_support();
        let secondary = self
            .secondary
            .map(|secondary| row(div().id(secondary_id).test_support().child(secondary)));
        let meta = (!self.meta.is_empty()).then(|| meta_row(scope_id, self.meta, cx));
        // Decides after every part above has measured itself, since children
        // prepaint in order.
        let decide = canvas(
            move |bounds, window, _| {
                let rem = window.rem_size();
                let gap = gap.to_pixels(rem);
                let filter = if has_filter {
                    gap + dp(FILTER_WIDTH).to_pixels(rem)
                } else {
                    px(0.)
                };
                let widths = HeaderWidths {
                    leading: rem * state.label.get() + filter,
                    chips: has_chips.then(|| rem * state.chips.get()),
                    controls: state
                        .controls
                        .borrow()
                        .iter()
                        .zip(&foldable)
                        .map(|(width, folds)| (rem * *width, *folds))
                        .collect(),
                    more: dp(CONTROL_HEIGHT).to_pixels(rem),
                    gap,
                    width: bounds.size.width,
                };
                let next = widths.place();
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
            .child(toolbar)
            .children(chips_below)
            .children(secondary)
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

    /// Applications-like controls: the first never folds, the rest do.
    const CONTROLS: [f32; 5] = [160., 132., 80., 100., 24.];

    #[test]
    fn controls_fold_right_to_left_then_the_chips_move_down() {
        let widths = |width: f32| HeaderWidths {
            leading: px(300.),
            chips: Some(px(200.)),
            controls: vec![(px(100.), false), (px(100.), true), (px(50.), true)],
            more: px(24.),
            gap: px(8.),
            width: px(width),
        };
        let at = |width: f32| widths(width).place();
        let placed = |folded, chips_below| Placement {
            folded,
            chips_below,
        };
        // 300 + 208 + 108 + 108 + 58: everything on one row.
        assert_eq!(at(782.), placed(0, false));
        // The rightmost folds first, and the menu takes 32.
        assert_eq!(at(781.), placed(1, false));
        assert_eq!(at(756.), placed(1, false));
        assert_eq!(at(755.), placed(2, false));
        // 300 + 208 + 108 + 32: the control that doesn't fold stays.
        assert_eq!(at(648.), placed(2, false));
        // Then the chips go down, and the row unfolds as far as it can.
        assert_eq!(at(647.), placed(0, true));
        assert_eq!(at(573.), placed(1, true));
        assert_eq!(at(440.), placed(2, true));
        assert_eq!(at(100.), placed(2, true));
        let bare = HeaderWidths {
            chips: None,
            ..widths(300.)
        };
        assert_eq!(bare.place(), placed(2, false));
    }

    /// A header like Applications': a filter, chips and categories of fixed
    /// widths, and `CONTROLS`, whose folded forms count their presses.
    struct Fitted {
        pressed: Rc<Cell<usize>>,
    }

    impl Render for Fitted {
        fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
            let header = PageHeader::new("app", "Applications");
            let part =
                |id: SharedString, width: f32| div().id(id).test_support().w(dp(width)).h(dp(20.));
            let chips = part(header.id("chips"), 220.);
            let categories = part(header.id("categories"), 360.);
            let mut header = header
                .filter(div().h(dp(28.)))
                .chips(Some(chips))
                .secondary(categories)
                .meta([div().child("Example data").into_any_element()]);
            for (ix, width) in CONTROLS.into_iter().enumerate() {
                let control = part(header.id(&format!("control-{ix}")), width);
                header = if ix == 0 {
                    header.control(control)
                } else {
                    let pressed = self.pressed.clone();
                    let handler: Handler = Rc::new(move |_, _| pressed.set(pressed.get() + ix));
                    header.foldable(control, item(format!("Control {ix}"), handler))
                };
            }
            header.render(window, cx).w_full()
        }
    }

    /// Where the header put its parts, once it settled, with how many
    /// frames asked for another on the way.
    struct Placed {
        title: Bounds<Pixels>,
        toolbar: Bounds<Pixels>,
        chips: Bounds<Pixels>,
        categories: Bounds<Pixels>,
        /// The controls still on the row, by index.
        shown: Vec<(usize, Bounds<Pixels>)>,
        more: Option<Bounds<Pixels>>,
        scope: Bounds<Pixels>,
        redraws: usize,
    }

    /// The header in a `width` × 560 window at `text` px.
    fn open(cx: &mut TestAppContext, width: f32, text: f32) -> (AnyWindowHandle, Rc<Cell<usize>>) {
        cx.update(|cx| {
            gpui_kit::init(cx);
            crate::theme::install(cx);
            crate::text_size::install(None, cx);
            crate::text_size::set(text, cx);
            cx.set_reduce_motion(true);
        });
        let pressed = Rc::new(Cell::new(0));
        let handle = cx.open_window(size(px(width), px(560.)), |window, cx| {
            let view = cx.new(|_| Fitted {
                pressed: pressed.clone(),
            });
            Root::new(view, window, cx)
        });
        (handle.into(), pressed)
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
            // Kit's "…" button asks for a frame of its own when it appears.
            redraws += 1;
        }
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            // Settled: drawing again asks for nothing more.
            assert_eq!(window.simulate_next_frame(cx), 0, "the header keeps moving");
            Placed {
                title: window.find("app-title").bounds(),
                toolbar: window.find("app-toolbar").bounds(),
                chips: window.find("app-chips").bounds(),
                categories: window.find("app-secondary").bounds(),
                shown: (0..CONTROLS.len())
                    .filter_map(|ix| {
                        let slot = window.try_find(format!("app-slot-{ix}"))?;
                        Some((ix, slot.bounds()))
                    })
                    .collect(),
                more: window.try_find("app-more").map(|more| more.bounds()),
                scope: window.find("app-scope").bounds(),
                redraws,
            }
        })
        .unwrap()
    }

    fn place(cx: &mut TestAppContext, width: f32, text: f32) -> Placed {
        let (handle, _) = open(cx, width, text);
        settle(cx, handle)
    }

    /// Every control left on the row sits inside it, in order, the last at
    /// the right edge, or the "…" button there after them.
    fn assert_one_row(placed: &Placed, width: f32, text: f32) {
        let row = placed.toolbar;
        let height = px(TOOLBAR_HEIGHT) * text / crate::ui::BASE_TEXT;
        assert!((row.size.height - height).abs() < px(0.5), "{row:?}");
        let mut right = placed.title.right();
        for (ix, slot) in &placed.shown {
            assert!(
                slot.top() >= row.top() && slot.bottom() <= row.bottom(),
                "{ix}"
            );
            assert!(slot.left() > right, "{ix} overlaps what's before it");
            right = slot.right();
        }
        let last = placed.more.unwrap_or(placed.shown.last().unwrap().1);
        assert!((last.right() - px(width)).abs() < px(0.5), "{last:?}");
    }

    #[gpui_kit::test]
    fn wide_nothing_folds_and_the_parts_share_one_row(cx: &mut TestAppContext) {
        let placed = place(cx, 1800., crate::ui::BASE_TEXT);
        assert_eq!(placed.shown.len(), CONTROLS.len());
        assert!(placed.more.is_none());
        assert_one_row(&placed, 1800., crate::ui::BASE_TEXT);
        assert!(placed.chips.top() >= placed.toolbar.top());
        assert!(placed.chips.bottom() <= placed.toolbar.bottom());
        // The categories take a second row of the same height.
        assert_eq!(
            placed.categories.top(),
            placed.toolbar.bottom() + (px(TOOLBAR_HEIGHT) - px(20.)) / 2.
        );
        assert!(placed.scope.top() >= placed.categories.bottom());
        assert_eq!(placed.redraws, 0, "the first frame was already right");
    }

    #[gpui_kit::test]
    fn a_full_row_folds_its_rightmost_controls_into_the_menu(cx: &mut TestAppContext) {
        // The title, filter and chips take about 470 dp, the controls 530.
        let placed = place(cx, 900., crate::ui::BASE_TEXT);
        assert!(placed.more.is_some());
        let shown: Vec<_> = placed.shown.iter().map(|(ix, _)| *ix).collect();
        assert!(shown.len() < CONTROLS.len() && shown.contains(&0));
        assert_eq!(
            shown,
            (0..shown.len()).collect::<Vec<_>>(),
            "folded from the right"
        );
        assert_one_row(&placed, 900., crate::ui::BASE_TEXT);
        assert!(placed.chips.bottom() <= placed.toolbar.bottom());
        assert_eq!(placed.redraws, 1);
    }

    #[gpui_kit::test]
    fn narrow_the_chips_take_a_row_and_the_rest_fold(cx: &mut TestAppContext) {
        let placed = place(cx, 760., 20.);
        assert!(placed.more.is_some());
        assert_one_row(&placed, 760., 20.);
        // The chips' row centres them: (38 - 20) / 2 dp below the toolbar.
        let below = placed.chips.top() - placed.toolbar.bottom();
        assert!((below - px(9.) * 20. / 13.).abs() < px(0.5), "{below:?}");
        assert!(placed.categories.top() >= placed.chips.bottom());
        assert!(placed.scope.top() >= placed.categories.bottom());
    }

    #[gpui_kit::test]
    fn a_new_text_size_folds_and_unfolds_without_input(cx: &mut TestAppContext) {
        let (handle, _) = open(cx, 1280., crate::ui::BASE_TEXT);
        let before = settle(cx, handle);
        assert!(before.more.is_none());
        cx.update(|cx| crate::text_size::set(20., cx));
        let larger = settle(cx, handle);
        assert!(larger.more.is_some());
        assert_one_row(&larger, 1280., 20.);
        cx.update(|cx| crate::text_size::set(crate::ui::BASE_TEXT, cx));
        let back = settle(cx, handle);
        assert!(back.more.is_none());
        assert_eq!(back.shown.len(), CONTROLS.len());
    }

    #[gpui_kit::test]
    fn the_menu_lists_the_folded_controls_and_runs_their_handlers(cx: &mut TestAppContext) {
        let (handle, pressed) = open(cx, 760., 20.);
        let placed = settle(cx, handle);
        let folded: Vec<usize> = (1..CONTROLS.len())
            .filter(|ix| placed.shown.iter().all(|(shown, _)| shown != ix))
            .collect();
        assert!(!folded.is_empty());
        cx.update_window(handle, |_, window, cx| {
            window.click("app-more", cx);
            window.render_frame(cx);
            // In the order the controls were added: the last is the last item.
            window.within("popup-menu").click(folded.len() - 1, cx);
            window.render_frame(cx);
        })
        .unwrap();
        assert_eq!(pressed.get(), *folded.last().unwrap());
    }

    /// A header with no controls and a meta line of `parts` like parts.
    struct Bare {
        parts: usize,
    }

    impl Render for Bare {
        fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
            let meta = (0..self.parts)
                .map(|ix| {
                    div()
                        .id(SharedString::from(format!("part-{ix}")))
                        .test_support()
                        .child(format!("part {ix} of the line ·"))
                        .into_any_element()
                })
                .collect::<Vec<_>>();
            PageHeader::new("bare", "prod-fra")
                .meta(meta)
                .render(window, cx)
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
            assert!(window.try_find("bare-more").is_none());
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
