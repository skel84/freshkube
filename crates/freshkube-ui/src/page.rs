//! The page frame every page shares: edge to edge, or padded for a page of
//! cards; the card a body sits in; and `PageHeader`
//! (docs/DESIGN.md#components).
use gpui_kit::assets::IconName;
use gpui_kit::base::ObservedElement as Observed;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::menu::{DropdownMenu, PopupMenu, PopupMenuItem};
use gpui_kit::component::{ActiveTheme, Sizable, h_flex, v_flex};
use gpui_kit::prelude::*;
use std::cell::{Cell, RefCell};
use std::rc::Rc;

use gpui_kit::{
    Action, Anchor, AnyElement, App, ClickEvent, Context, Div, ElementId, FocusHandle, Pixels,
    Point, SharedString, Stateful, TestSupportExt, Window, canvas, div, point, px,
};

use crate::menu::{self, MenuAction};
use crate::palette::palette;
use crate::ui::{CONTROL_HEIGHT, Tone, badge_dot, dp, dp_px, status_glyph, toolbar_label};

gpui_kit::actions!(
    pilot,
    [
        /// Reads the shown page again: `⌘R`, and a page's Refresh button
        /// or its folded entry, wherever the page's crate lives.
        Refresh
    ]
);

/// The app shell's key context, where [`Refresh`]'s key is bound: a
/// Refresh button's `tooltip_with_action` names it to find the key.
pub const SHELL_CONTEXT: &str = "Freshkube";

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
/// A window shorter than this, in dp, scrolls a table page's frame: see
/// [`is_short`].
pub const SHORT_HEIGHT: f32 = 620.;
/// The least height a table page's list keeps while its frame scrolls.
pub const SHORT_LIST_HEIGHT: f32 = 180.;
/// A toolbar row's height: the header's row, and its secondary row.
pub const TOOLBAR_HEIGHT: f32 = 38.;
/// The app frame's header, above every page, in dp, its bottom hairline
/// included: one row of [`APP_HEADER_CONTROL`] controls with even padding.
pub const APP_HEADER_HEIGHT: f32 = 44.;
/// The height of the header's controls, in dp: the context switcher and
/// Search everything, and the slot of Refresh's countdown ring.
pub const APP_HEADER_CONTROL: f32 = 28.;
/// Where the header's content starts on macOS, in points: clear of the
/// traffic lights, which don't scale with the text size.
pub const TRAFFIC_LIGHT_INSET: f32 = 80.;
/// The close button's left edge from the window's, in points. AppKit's
/// button frames are 14 × 16 with the 12 pt circle 1 in and 2 down, so the
/// circle starts at 15, as far as it sits from the header's top at 13 px.
const TRAFFIC_LIGHT_X: f32 = 14.;
/// The traffic light circle's diameter, and how far below its button
/// frame's top it starts, in points.
const TRAFFIC_LIGHT: f32 = 12.;
const TRAFFIC_LIGHT_TOP: f32 = 2.;
// The zoom button's circle, two 20 pt steps from the close button's, ends
// at 67 pt: the header's content starts at least 12 pt after it.
const _: () =
    assert!(TRAFFIC_LIGHT_INSET - (TRAFFIC_LIGHT_X + 1. + 2. * 20. + TRAFFIC_LIGHT) >= 12.);

/// Where macOS puts the close button's frame so the traffic lights are
/// centred on the header, whose dp lengths are `rem` pixels per 13: the
/// lights stay one size, so a fixed position centres them at only one
/// text size, and the shell places them again when the text size changes.
pub fn traffic_light_position(rem: f32) -> Point<Pixels> {
    let header = APP_HEADER_HEIGHT * rem / crate::ui::BASE_TEXT;
    // Centre the circle on the header less its 1 px hairline.
    let top = (header - 1. - TRAFFIC_LIGHT) / 2. - TRAFFIC_LIGHT_TOP;
    point(px(TRAFFIC_LIGHT_X), px(top))
}
/// The app frame's status bar, below every page, in dp.
pub const STATUS_BAR_HEIGHT: f32 = 28.;
/// The icon rail's width, and the navigation column's beside it, in dp.
pub const RAIL_WIDTH: f32 = 64.;
pub const COLUMN_WIDTH: f32 = 208.;
thread_local! {
    /// The width of the navigation beside the page in dp: the rail, and the
    /// column when it shows. Windows draw on one thread, and the shell sets
    /// it whenever the column shows or hides.
    static CHROME_WIDTH: Cell<f32> = const { Cell::new(RAIL_WIDTH + COLUMN_WIDTH) };
}

pub fn set_chrome_width(width: f32) {
    CHROME_WIDTH.set(width);
}

/// The width of a page in `dp` (pixels at the default text size): the
/// window less the rail and column. A page without margins has all of it,
/// so its breakpoints compare this; a larger text size leaves less room, as
/// a narrower window would.
pub fn page_width(window: &Window) -> f32 {
    let viewport = window.viewport_size().width / dp_px(1., window);
    (viewport - CHROME_WIDTH.get()).max(240.)
}

/// The width inside a page's [`inset`], such as its header's, in `dp`.
pub fn inset_width(window: &Window) -> f32 {
    (page_width(window) - PANE_PADDING * 2.).max(240.)
}

/// The width inside a [padded] page's margins in `dp`, for choosing between
/// side-by-side and stacked layouts there.
pub fn content_width(window: &Window) -> f32 {
    (page_width(window) - PAGE_PADDING * 2.).max(240.)
}

/// The header's filter beside the title, at its full width.
const FILTER_WIDTH: f32 = 150.;
/// The filter's width when the row is full with every control folded.
const FILTER_MIN: f32 = 96.;

/// A page's frame, without margins: its header, any banners and its panes
/// stack edge to edge, and the caller puts what isn't a pane, such as the
/// header, in an [`inset`]. The caller adds scrolling: a table page scrolls
/// its frame when the window [`is_short`], and its list keeps at least
/// [`SHORT_LIST_HEIGHT`].
pub fn page(id: impl Into<ElementId>) -> Observed<Stateful<Div>> {
    v_flex().id(id).test_support().size_full().min_h_0()
}

/// Whether the window, less the dock under the page, is too short for a
/// table page's header and a usable list together, so the page scrolls its
/// frame instead: under [`SHORT_HEIGHT`].
pub fn is_short(window: &Window) -> bool {
    window.viewport_size().height - crate::ui::dp_px(below(), window)
        < crate::ui::dp_px(SHORT_HEIGHT, window)
}

thread_local! {
    /// The height in dp taken under the page by the dock, which the shell
    /// sets as it draws. Windows draw on one thread.
    static BELOW: std::cell::Cell<f32> = const { std::cell::Cell::new(0.) };
}

/// Tells pages how much of the window's height, in dp, the dock takes
/// under them, so a page under an open dock lays out as in a window that
/// much shorter.
pub fn set_below(height: f32) {
    BELOW.set(height.max(0.));
}

/// The height in dp the dock takes under the page.
pub fn below() -> f32 {
    BELOW.get()
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

/// The folded form of a button whose command has a key: `label`, running
/// `action` on `focus`, the page's list where the key is bound, so the
/// entry shows the key the button's tooltip names.
pub fn action_item(
    label: impl Into<SharedString>,
    action: impl Action,
    focus: &FocusHandle,
) -> MenuItems {
    action_entry(MenuAction::new(label, action), focus)
}

/// [`action_item`] for a button that shows whether it's on: checked when
/// it is.
pub fn checked_action_item(
    label: impl Into<SharedString>,
    checked: bool,
    action: impl Action,
    focus: &FocusHandle,
) -> MenuItems {
    action_entry(MenuAction::new(label, action).checked(checked), focus)
}

/// The folded form of a button whose command has a key, as `entry` says,
/// such as one disabled while nothing is selected; it keeps its key.
pub fn action_entry(entry: MenuAction, focus: &FocusHandle) -> MenuItems {
    let focus = focus.clone();
    Rc::new(move |menu, window, cx| {
        menu::actions(menu, vec![entry.clone()], &focus, |_| true, window, cx)
    })
}

/// A button's click that runs `action` on `focus`, the page's, so the
/// button, its folded entry and its key are one action: a Refresh button's
/// click is ⌘R's. Give the button `tooltip_with_action` with the same
/// action, so its tooltip names the key.
pub fn dispatch(
    action: impl Action,
    focus: &FocusHandle,
) -> impl Fn(&ClickEvent, &mut Window, &mut App) + 'static {
    let focus = focus.clone();
    move |_, window, cx| focus.dispatch_action(&action, window, cx)
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

/// The most of a value [`submenu_value`] shows after its label.
const VALUE_CHARS: usize = 24;

/// The folded form of a control that picks a value, such as a namespace:
/// `label · value`, the value cut to 24 characters with "…", over `items`.
/// A cut value shows whole, greyed, at the top of the submenu.
pub fn submenu_value(
    label: impl Into<SharedString>,
    value: impl Into<SharedString>,
    items: MenuItems,
) -> MenuItems {
    let value = value.into();
    let (label, cut) = value_label(&label.into(), &value);
    submenu(
        label,
        if cut {
            Rc::new(move |menu, window, cx| {
                let menu = menu
                    .item(PopupMenuItem::new(value.clone()).disabled(true))
                    .separator();
                items(menu, window, cx)
            })
        } else {
            items
        },
    )
}

/// The folded form of a Columns menu: `Columns · N hidden` while any is
/// hidden, noted while the hidden set isn't the page's default.
pub fn columns_fold(items: MenuItems, hidden: usize, at_default: bool) -> Fold {
    let items = match hidden {
        0 => submenu("Columns", items),
        n => submenu_value("Columns", format!("{n} hidden"), items),
    };
    Fold::from(items).changed((!at_default).then(|| hidden_note(hidden)))
}

/// `2 columns hidden`, as the "…" tooltip lists it.
fn hidden_note(hidden: usize) -> SharedString {
    match hidden {
        0 => "no columns hidden".into(),
        1 => "1 column hidden".into(),
        n => format!("{n} columns hidden").into(),
    }
}

/// `label · value`, and whether the value was cut to [`VALUE_CHARS`].
fn value_label(label: &str, value: &str) -> (SharedString, bool) {
    match value.char_indices().nth(VALUE_CHARS - 1) {
        Some((end, _)) if value.chars().count() > VALUE_CHARS => {
            (format!("{label} · {}…", &value[..end]).into(), true)
        }
        _ => (format!("{label} · {value}").into(), false),
    }
}

/// A control's folded form, and a short note while the control isn't at
/// its default, such as `Namespace payments`. While the control is folded,
/// a note puts a dot on the "…" button and joins its tooltip.
pub struct Fold {
    items: MenuItems,
    changed: Option<SharedString>,
}

impl Fold {
    /// The note, or `None` while the control is at its default.
    pub fn changed(mut self, note: Option<SharedString>) -> Self {
        self.changed = note;
        self
    }
}

impl From<MenuItems> for Fold {
    fn from(items: MenuItems) -> Self {
        Self {
            items,
            changed: None,
        }
    }
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
    parent: Option<Parent>,
    untitled: bool,
}

/// A control, with its menu form if it folds.
struct Control {
    element: AnyElement,
    fold: Option<Fold>,
}

/// What a click on a breadcrumb's parent does.
type OnClick = Box<dyn Fn(&ClickEvent, &mut Window, &mut App)>;

/// A breadcrumb's parent: the collection the page sits in.
struct Parent {
    id: SharedString,
    label: SharedString,
    on_click: OnClick,
    /// Muted crumbs between the parent and the title, such as a namespace.
    trail: Vec<SharedString>,
    /// The title's status glyph.
    glyph: Option<Tone>,
}

/// A crumb after the parent, then its faint "/".
fn crumb(text: SharedString, cx: &App) -> [Div; 2] {
    let p = palette(cx);
    [
        div().flex_none().text_color(p.muted).child(text),
        div().flex_none().text_color(p.faint).child("/"),
    ]
}

/// A breadcrumb's parts as [`title`] draws them, without ids or a click,
/// at their full width.
fn breadcrumb(parent: &Parent, text: SharedString, cx: &App) -> Div {
    h_flex()
        .gap(dp(6.))
        .items_center()
        .text_size(dp(13.))
        .line_height(dp(18.))
        .whitespace_nowrap()
        .child(div().flex_none().child(parent.label.clone()))
        .child(div().flex_none().child("/"))
        .children(parent.trail.iter().flat_map(|t| crumb(t.clone(), cx)))
        .children(parent.glyph.and_then(|tone| status_glyph(tone, cx)))
        .child(toolbar_label(text, cx))
}

/// The toolbar label, after its breadcrumb when it has one: the parent
/// muted at 13, a faint "/", then the label.
fn title(parent: Option<Parent>, text: SharedString, id: SharedString, cx: &App) -> AnyElement {
    let title = toolbar_label(text, cx).id(id).test_support();
    let Some(parent) = parent else {
        return title.into_any_element();
    };
    let p = palette(cx);
    let accent = p.accent;
    let on_click = parent.on_click;
    let trail = parent.trail.into_iter().flat_map(|t| crumb(t, cx));
    let glyph = parent.glyph.and_then(|tone| status_glyph(tone, cx));
    h_flex()
        .gap(dp(6.))
        .items_center()
        .min_w_0()
        .text_size(dp(13.))
        .line_height(dp(18.))
        .child(
            div()
                .id(parent.id)
                .flex_none()
                .text_color(p.muted)
                .cursor_pointer()
                .hover(move |style| style.text_color(accent))
                .on_click(move |event, window, cx| on_click(event, window, cx))
                .child(parent.label)
                .test_support(),
        )
        .child(div().flex_none().text_color(p.faint).child("/"))
        .children(trail)
        .children(glyph)
        // The label's own box hugs its text, so it measures as it does
        // without a breadcrumb; the wrapper keeps 120 of it in view.
        .child(
            div()
                .flex()
                .min_w(dp(120.))
                .child(title.flex_shrink(1.).min_w_0().truncate()),
        )
        .into_any_element()
}

/// Where the header puts its parts.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Placement {
    /// How many of the foldable controls, counted from the right, are in the
    /// "…" menu.
    folded: usize,
    /// Whether the chips have a row of their own, under the toolbar's.
    chips_below: bool,
    /// Whether the controls have a row of their own, under the chips': only
    /// when even a full fold leaves them no room beside the title.
    controls_below: bool,
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
    /// else the chips on a row of their own, and the fewest that fit then;
    /// else the controls on a row of their own too, folded as that row needs.
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
                        controls_below: false,
                    };
                }
            }
        }
        let chips_below = self
            .chips
            .is_some_and(|chips| self.leading + self.gap + chips > self.width);
        let folded = (0..=foldable.len())
            .find(|folded| self.controls(&foldable[..*folded]) - self.gap <= self.width)
            .unwrap_or(foldable.len());
        Placement {
            folded,
            chips_below,
            controls_below: true,
        }
    }

    /// The controls left after folding `folded`, and the menu if any fold,
    /// each after a gap.
    fn controls(&self, folded: &[usize]) -> Pixels {
        let gap = self.gap;
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
        controls + more
    }

    /// The toolbar's row with the `folded` controls in the menu.
    fn row(&self, folded: &[usize], chips_below: bool) -> Pixels {
        let gap = self.gap;
        let chips = self
            .chips
            .filter(|_| !chips_below)
            .map_or(px(0.), |chips| gap + chips);
        self.leading + chips + self.controls(folded)
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
            parent: None,
            untitled: false,
        }
    }

    /// Leaves the title out, for a screen embedded where a tab already
    /// names it, such as a node's tab in the node inspector. The toolbar
    /// starts with the filter, then the chips.
    pub fn untitled(mut self, untitled: bool) -> Self {
        self.untitled = untitled;
        self
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
    /// menu shows. A [`Fold`] adds what the control is set to while it
    /// isn't at its default.
    pub fn foldable(mut self, control: impl IntoElement, fold: impl Into<Fold>) -> Self {
        self.controls.push(Control {
            element: control.into_any_element(),
            fold: Some(fold.into()),
        });
        self
    }

    /// A left-aligned row below the toolbar's, such as category segments,
    /// before the meta line: `<prefix>-secondary`.
    pub fn secondary(mut self, row: impl IntoElement) -> Self {
        self.secondary = Some(row.into_any_element());
        self
    }

    /// A breadcrumb before the title, for a page inside a collection (a
    /// dashboard, an application report): the parent as a link back to it,
    /// with the id `<prefix>-<part>`, then a faint `/`. The title keeps its
    /// id and truncates, at least 120 wide.
    pub fn parent(
        mut self,
        part: &str,
        label: impl Into<SharedString>,
        on_click: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.parent = Some(Parent {
            id: self.id(part),
            label: label.into(),
            on_click: Box::new(on_click),
            trail: Vec::new(),
            glyph: None,
        });
        self
    }

    /// A muted crumb after the parent and before the title, such as an
    /// application's namespace. Needs a [`parent`](Self::parent) first.
    pub fn crumb(mut self, text: impl Into<SharedString>) -> Self {
        if let Some(parent) = &mut self.parent {
            parent.trail.push(text.into());
        }
        self
    }

    /// A status glyph just before the title, for a page about one thing
    /// whose health the breadcrumb states. Needs a [`parent`](Self::parent).
    pub fn glyph(mut self, tone: Tone) -> Self {
        if let Some(parent) = &mut self.parent {
            parent.glyph = Some(tone);
        }
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
        let (toolbar_id, controls_id, more_id, dot_id) = (
            self.id("toolbar"),
            self.id("controls"),
            self.id("more"),
            self.id("more-dot"),
        );
        let (secondary_id, title_id, scope_id) =
            (self.id("secondary"), self.title_id, self.scope_id);
        let has_filter = self.filter.is_some();
        let untitled = self.untitled;
        if untitled {
            state.label.set(0.);
        }
        let leading = h_flex()
            .flex_shrink(1.)
            .min_w_0()
            .gap(gap)
            .items_center()
            .when(!untitled, |this| {
                this.child({
                    let state = state.clone();
                    let slot = move |width: Pixels, window: &Window| {
                        state.label.set(width / window.rem_size())
                    };
                    match self.parent.as_ref() {
                        None => measured(
                            toolbar_label(self.title, cx).id(title_id).test_support(),
                            slot,
                        ),
                        // A breadcrumb's title truncates, at least 120 wide, when
                        // even a full fold leaves it no room, so what counts is
                        // the width of an unseen copy that never shrinks.
                        // Its own least width is the parent's, the "/" and 120.
                        Some(parent) => div()
                            .relative()
                            .flex_shrink(1.)
                            .child(
                                measured(breadcrumb(parent, self.title.clone(), cx), slot)
                                    .absolute()
                                    .top_0()
                                    .left_0()
                                    .opacity(0.),
                            )
                            .child(title(self.parent, self.title, title_id, cx)),
                    }
                })
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
        let mut notes = Vec::new();
        for (ix, control) in self.controls.into_iter().enumerate() {
            if folded.contains(&ix) {
                if let Some(fold) = control.fold {
                    forms.push(fold.items);
                    notes.extend(fold.changed);
                }
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
            let dot = (!notes.is_empty()).then(|| {
                badge_dot(Tone::Accent, Some(cx.theme().background), cx)
                    .id(dot_id)
                    .test_support()
                    .absolute()
                    .top_0()
                    .right_0()
            });
            // What the folded controls are set to, when any isn't at its
            // default: "More · Namespace payments · 2 columns hidden".
            let tip: SharedString = std::iter::once(SharedString::from("More"))
                .chain(notes)
                .collect::<Vec<_>>()
                .join(" · ")
                .into();
            div()
                .relative()
                .flex_none()
                .child(
                    Button::new(more_id)
                        .ghost()
                        .small()
                        .size(dp(CONTROL_HEIGHT))
                        .icon(IconName::Ellipsis)
                        .accessibility_label(tip.clone())
                        .tooltip(tip)
                        .dropdown_menu_with_anchor(
                            Anchor::TopRight,
                            move |mut menu, window, cx| {
                                // Escape gives the keyboard back, also
                                // when no folded control has a key.
                                menu::return_focus(None, window, cx);
                                for form in &forms {
                                    menu = form(menu, window, cx);
                                }
                                menu
                            },
                        ),
                )
                .children(dot)
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
        let (controls_row, controls_below) = if placement.controls_below {
            (None, Some(row(controls)))
        } else {
            (Some(controls), None)
        };
        let toolbar = row(h_flex()
            .w_full()
            .min_w_0()
            .items_center()
            .gap(gap)
            .when(!untitled || has_filter, |this| this.child(leading))
            .children(chips_row)
            .children(controls_row))
        .id(toolbar_id)
        .test_support();
        let secondary = self.secondary.map(|secondary| {
            // As wide as the header, so a row that wraps or pushes part
            // of itself right, as Monitoring's variables do, has room.
            row(div()
                .id(secondary_id)
                .test_support()
                .flex_1()
                .min_w_0()
                .child(secondary))
        });
        let meta = (!self.meta.is_empty()).then(|| meta_row(scope_id, self.meta, cx));
        // Decides after every part above has measured itself, since children
        // prepaint in order.
        let decide = canvas(
            move |bounds, window, _| {
                let rem = window.rem_size();
                let gap = gap.to_pixels(rem);
                let filter = match (has_filter, untitled) {
                    (true, false) => gap + dp(FILTER_WIDTH).to_pixels(rem),
                    (true, true) => dp(FILTER_WIDTH).to_pixels(rem),
                    (false, _) => px(0.),
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
            .children(controls_below)
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

    /// The traffic lights' circles sit centred on the header less its
    /// hairline at every text size, and their left edge as far from the
    /// window's as from the header's top at the default size.
    #[test]
    fn traffic_lights_centre_on_the_header_at_every_text_size() {
        for rem in crate::text_size::STEPS {
            let header = APP_HEADER_HEIGHT * rem / crate::ui::BASE_TEXT - 1.;
            let at = traffic_light_position(rem);
            let top = f32::from(at.y) + TRAFFIC_LIGHT_TOP;
            let bottom = header - top - TRAFFIC_LIGHT;
            assert!(
                (top - bottom).abs() < 0.01,
                "{rem}: {top} above, {bottom} below"
            );
        }
        let at = traffic_light_position(crate::ui::BASE_TEXT);
        let (left, top) = (f32::from(at.x) + 1., f32::from(at.y) + TRAFFIC_LIGHT_TOP);
        assert!(
            (left - top).abs() <= 0.5,
            "{left} from the left, {top} from the top"
        );
    }

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
            controls_below: false,
        };
        let below = |folded, chips_below| Placement {
            controls_below: true,
            ..placed(folded, chips_below)
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
        // Then the controls take a row of their own too: 100 + 8 + 100 + 8
        // + 50 whole, 100 + 8 + 100 + 8 + 24 with one folded, 100 + 8 + 24
        // with both.
        assert_eq!(at(439.), below(0, true));
        assert_eq!(at(265.), below(1, true));
        assert_eq!(at(132.), below(2, true));
        assert_eq!(at(100.), below(2, true));
        let bare = |width| {
            HeaderWidths {
                chips: None,
                ..widths(width)
            }
            .place()
        };
        assert_eq!(bare(440.), placed(2, false));
        assert_eq!(bare(439.), below(0, false));
        // A wide control that doesn't fold goes down alone, and the chips
        // stay beside the title while they fit there: 300 + 108 of 450.
        let wide = HeaderWidths {
            chips: Some(px(100.)),
            controls: vec![(px(300.), false)],
            ..widths(450.)
        };
        assert_eq!(wide.place(), below(0, false));
    }

    /// A header like Applications': a filter, chips and categories of fixed
    /// widths, and `CONTROLS`, whose folded forms count their presses.
    struct Fitted {
        pressed: Rc<Cell<usize>>,
        /// A long title after a breadcrumb, which truncates on a full row.
        breadcrumb: bool,
        /// Whether each control is off its default: then it has a note.
        changed: Rc<Cell<bool>>,
    }

    impl Render for Fitted {
        fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
            let header =
                if self.breadcrumb {
                    PageHeader::new("app", "Kubernetes compute resources by namespace and pod")
                        .parent("parent", "Dashboards", |_, _, _| {})
                } else {
                    PageHeader::new("app", "Applications")
                };
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
                    let note = self.changed.get().then(|| format!("note {ix}").into());
                    header.foldable(
                        control,
                        Fold::from(item(format!("Control {ix}"), handler)).changed(note),
                    )
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
        open_titled(cx, width, text, false)
    }

    fn open_titled(
        cx: &mut TestAppContext,
        width: f32,
        text: f32,
        breadcrumb: bool,
    ) -> (AnyWindowHandle, Rc<Cell<usize>>) {
        cx.update(|cx| {
            gpui_kit::init(cx);
            crate::theme::install(cx);
            crate::text_size::install(None, cx);
            crate::text_size::set(text, cx);
            cx.set_reduce_motion(true);
        });
        another(cx, width, breadcrumb)
    }

    /// One more such window, in the app as it is.
    fn another(
        cx: &mut TestAppContext,
        width: f32,
        breadcrumb: bool,
    ) -> (AnyWindowHandle, Rc<Cell<usize>>) {
        another_with(cx, width, breadcrumb, Rc::default())
    }

    /// The same, its controls off their defaults while `changed` is set.
    fn another_with(
        cx: &mut TestAppContext,
        width: f32,
        breadcrumb: bool,
        changed: Rc<Cell<bool>>,
    ) -> (AnyWindowHandle, Rc<Cell<usize>>) {
        let pressed = Rc::new(Cell::new(0));
        let handle = cx.open_window(size(px(width), px(560.)), |window, cx| {
            let view = cx.new(|_| Fitted {
                pressed: pressed.clone(),
                breadcrumb,
                changed: changed.clone(),
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

    /// What a settled header shows: the controls left on the row, and
    /// whether the chips took a row of their own.
    fn layout(placed: &Placed) -> (Vec<usize>, bool) {
        let shown = placed.shown.iter().map(|(ix, _)| *ix).collect();
        (shown, placed.chips.top() >= placed.toolbar.bottom())
    }

    /// The layout a breadcrumb header opened fresh at `width`, at the app's
    /// text size, settles on.
    fn fresh(cx: &mut TestAppContext, width: f32) -> (Vec<usize>, bool) {
        let (handle, _) = another(cx, width, true);
        layout(&settle(cx, handle))
    }

    #[gpui_kit::test]
    fn a_breadcrumb_title_folds_steadily_across_resizes_and_text_sizes(cx: &mut TestAppContext) {
        let (handle, _) = open_titled(cx, 1800., crate::ui::BASE_TEXT, true);
        let mut folds = 0;
        // Across the fold points and back: a title that truncated on a full
        // row keeps its full width, so each width settles where a fresh
        // header does.
        for width in [1800., 1100., 760., 520., 760., 1100., 1800.] {
            cx.simulate_window_resize(handle, size(px(width), px(560.)));
            let placed = settle(cx, handle);
            let settled = layout(&placed);
            folds += usize::from(placed.more.is_some());
            assert_eq!(settled, fresh(cx, width), "{width}");
        }
        assert!(folds >= 2, "the widths never crossed a fold point");
        // A new text size measures the title again, both ways.
        cx.simulate_window_resize(handle, size(px(1280.), px(560.)));
        settle(cx, handle);
        for text in [20., crate::ui::BASE_TEXT] {
            cx.update(|cx| crate::text_size::set(text, cx));
            let settled = layout(&settle(cx, handle));
            assert_eq!(settled, fresh(cx, 1280.), "{text}");
        }
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

    #[test]
    fn a_value_shows_after_its_label_and_a_long_one_is_cut() {
        assert_eq!(
            value_label("Namespace", "payments"),
            ("Namespace · payments".into(), false)
        );
        let exact = "a".repeat(VALUE_CHARS);
        assert_eq!(
            value_label("Node", &exact),
            (format!("Node · {exact}").into(), false)
        );
        // Characters, not bytes: 30 accented ones keep 23 and the "…".
        let long = "é".repeat(30);
        let (label, cut) = value_label("Namespace", &long);
        assert!(cut);
        assert_eq!(label, format!("Namespace · {}…", "é".repeat(23)));
    }

    #[test]
    fn hidden_columns_read_as_a_count() {
        assert_eq!(hidden_note(0), "no columns hidden");
        assert_eq!(hidden_note(1), "1 column hidden");
        assert_eq!(hidden_note(3), "3 columns hidden");
    }

    #[gpui_kit::test]
    fn a_folded_control_off_its_default_marks_the_menu_and_says_why(cx: &mut TestAppContext) {
        let (handle, _) = open(cx, 760., 20.);
        cx.update_window(handle, |_, window, _| window.remove_window())
            .unwrap();
        // At 1000 the chips go below and only the last controls fold.
        let changed = Rc::new(Cell::new(false));
        let (handle, _) = another_with(cx, 1000., false, changed.clone());
        let quiet = settle(cx, handle);
        assert!(quiet.more.is_some());
        cx.update_window(handle, |_, window, _| {
            assert!(window.try_find("app-more-dot").is_none());
            assert_eq!(window.find("app-more").label(), Some("More"));
        })
        .unwrap();
        // Every control has a note; only the folded ones' join the menu's.
        changed.set(true);
        let placed = settle(cx, handle);
        let notes: Vec<String> = (1..CONTROLS.len())
            .filter(|ix| placed.shown.iter().all(|(shown, _)| shown != ix))
            .map(|ix| format!("note {ix}"))
            .collect();
        // Control 1 shows on the row: its note stays out of the menu's.
        assert!(placed.shown.iter().any(|(shown, _)| *shown == 1));
        assert!(!notes.is_empty() && !notes.contains(&"note 1".to_string()));
        let tip = format!("More · {}", notes.join(" · "));
        cx.update_window(handle, |_, window, _| {
            let more = window.find("app-more").bounds();
            let dot = window.find("app-more-dot").bounds();
            // At the button's top right corner.
            assert!(dot.right() > more.right() - px(4.) && dot.top() < more.top() + px(4.));
            assert_eq!(window.find("app-more").label(), Some(tip.as_str()));
        })
        .unwrap();
        // Wide, nothing folds: no menu, so no dot.
        cx.simulate_window_resize(handle, size(px(1800.), px(560.)));
        let wide = settle(cx, handle);
        assert!(wide.more.is_none());
        cx.update_window(handle, |_, window, _| {
            assert!(window.try_find("app-more-dot").is_none());
        })
        .unwrap();
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

    /// A header with a long title, a filter and one wide foldable control,
    /// drawn with its title or without.
    struct Untitled {
        untitled: bool,
    }

    impl Render for Untitled {
        fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
            let header = PageHeader::new("un", "A title long enough to fold the row")
                .untitled(self.untitled);
            let control = div()
                .id(header.id("control"))
                .test_support()
                .w(dp(300.))
                .h(dp(20.));
            let handler: Handler = Rc::new(|_, _| {});
            header
                .filter(div().child(div().id("un-filter").test_support().size_full().h(dp(28.))))
                .foldable(control, Fold::from(item("Control", handler)))
                .render(window, cx)
                .w_full()
        }
    }

    #[gpui_kit::test]
    fn an_untitled_header_leads_with_its_filter_and_folds_without_the_title(
        cx: &mut TestAppContext,
    ) {
        cx.update(|cx| {
            gpui_kit::init(cx);
            crate::theme::install(cx);
            crate::text_size::install(None, cx);
            cx.set_reduce_motion(true);
        });
        // The filter and the control fit 560 wide; with the title before
        // them they don't.
        for untitled in [false, true] {
            let handle = cx.open_window(size(px(560.), px(400.)), |window, cx| {
                let view = cx.new(|_| Untitled { untitled });
                Root::new(view, window, cx)
            });
            for _ in 0..4 {
                cx.run_until_parked();
                let asked = cx
                    .update_window(handle.into(), |_, window, cx| {
                        window.render_frame(cx);
                        window.simulate_next_frame(cx)
                    })
                    .unwrap();
                if asked == 0 {
                    break;
                }
            }
            cx.update_window(handle.into(), |_, window, cx| {
                window.render_frame(cx);
                let row = window.find("un-toolbar").bounds();
                let filter = window.find("un-filter").bounds();
                assert_eq!(window.try_find("un-title").is_none(), untitled);
                assert_eq!(window.try_find("un-more").is_some(), !untitled);
                assert_eq!(window.try_find("un-slot-0").is_some(), untitled);
                if untitled {
                    assert_eq!(filter.left(), row.left(), "the filter leads the row");
                } else {
                    assert!(filter.left() > window.find("un-title").bounds().right());
                }
            })
            .unwrap();
        }
    }
}
