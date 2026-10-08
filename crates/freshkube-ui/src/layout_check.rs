//! Measures a page from the bounds its last frame painted, so a page that
//! drifts from DESIGN.md fails a test instead of a review.
//!
//! `assert_page_frame` checks a padded page's padding and toolbar (and
//! `assert_page_frame_from` one whose header leads with a breadcrumb),
//! `assert_edge_frame` an edge-to-edge page's inset and toolbar (and
//! `assert_edge_frame_from` one whose header leads with a breadcrumb),
//! `assert_table` checks one table's header and rows (a page may hold
//! several), and `assert_table_page` checks a table page with the edge
//! frame and its table, and that the table sits in no card (and
//! `assert_table_page_from` one with a breadcrumb);
//! `assert_inspector` checks an inspector against its table, and
//! `assert_drawer` a drawer over its list; `settle_header` draws until a
//! page's header settles. A page names a few
//! elements by id; rows and group headers are found by their accessibility
//! roles inside the list, so the checks need no access to the page's state.

use gpui_kit::base::test_support::{ElementSnapshot, snapshots};
use gpui_kit::test::TestWindowExt;
use gpui_kit::{App, ElementId, Pixels, Role, SharedString, TextRun, Window, font, px};

use crate::page::{PAGE_PADDING, PANE_PADDING};
use crate::ui::dp_px;

/// DESIGN.md's table page, in dp.
pub const HEADER_HEIGHT: f32 = 26.;
pub const ROW_HEIGHT: f32 = 26.;
pub const FOOTER_HEIGHT: f32 = 26.;
/// The header's toolbar: its row, the title as its label, and its controls.
pub const TOOLBAR_HEIGHT: f32 = 38.;
pub const LABEL_TEXT: f32 = 13.;
pub const CONTROL_HEIGHT: f32 = 24.;

/// The elements a page names for the frame check.
pub struct PageFrame {
    /// The page's root; its edges are where the page padding starts.
    pub page: &'static str,
    /// The page title, `<prefix>-title` in a `PageHeader`, and the text it
    /// shows. The header's row is `<prefix>-toolbar` and its controls'
    /// boxes `<prefix>-slot-<n>`.
    pub title: &'static str,
    pub title_text: &'static str,
    /// The widest element under the header, whose right edge is where the
    /// right padding starts: a table page's table, a dashboard's grid.
    pub content: &'static str,
}

/// The elements a table names for the row check. A page may hold several,
/// such as a dashboard's table panels or a list beside a detail.
pub struct Table {
    /// The table's frame, with the column header drawn at its top, or `None`
    /// for a list without column captions.
    pub table: Option<&'static str>,
    /// The list under the column header, holding rows (`Role::ListBoxOption`)
    /// and group headers (`Role::Heading`).
    pub list: &'static str,
}

/// The elements a table page names for the check: its frame and its table.
pub struct TablePage {
    pub page: &'static str,
    pub title: &'static str,
    pub title_text: &'static str,
    pub table: &'static str,
    pub list: &'static str,
}

/// What a page's frame drew, in pixels.
#[derive(Debug)]
pub struct FrameLayout {
    pub padding_left: Pixels,
    pub padding_right: Pixels,
    /// The toolbar's row.
    pub toolbar: Pixels,
    pub title_text: Pixels,
    /// Each control's height, in order.
    pub controls: Vec<Pixels>,
}

/// What a table drew, in pixels: its header, if it has one, a row and,
/// when the list showed one, a group header.
#[derive(Debug)]
pub struct TableRows {
    pub header: Option<Pixels>,
    pub row: Pixels,
    pub group: Option<Pixels>,
}

/// What a table page drew, in pixels, for a caller's own assertions and
/// failure messages.
#[derive(Debug)]
#[allow(dead_code, reason = "each caller reads the measurements it needs")]
pub struct TableLayout {
    pub header: Pixels,
    pub row: Pixels,
    /// The group header, when the list showed a group.
    pub group: Option<Pixels>,
    pub padding_left: Pixels,
    pub padding_right: Pixels,
    pub toolbar: Pixels,
    pub title_text: Pixels,
}

/// Asserts DESIGN.md's table page: `assert_edge_frame` on its frame,
/// `assert_table` on its table, and `assert_bare` on the table.
pub fn assert_table_page(window: &mut Window, cx: &mut App, page: &TablePage) -> TableLayout {
    assert_table_page_from(window, cx, page, page.title)
}

/// [`assert_table_page`] for a header that leads with a breadcrumb: the
/// title inset is measured from `lead`, its parent.
pub fn assert_table_page_from(
    window: &mut Window,
    cx: &mut App,
    page: &TablePage,
    lead: &str,
) -> TableLayout {
    let frame = assert_edge_frame_from(
        window,
        cx,
        &PageFrame {
            page: page.page,
            title: page.title,
            title_text: page.title_text,
            content: page.table,
        },
        lead,
    );
    let rows = assert_table(
        window,
        cx,
        &Table {
            table: Some(page.table),
            list: page.list,
        },
    );
    let Some(header) = rows.header else {
        unreachable!("a table page's table has a header");
    };
    assert_bare(window, page.table);
    TableLayout {
        header,
        row: rows.row,
        group: rows.group,
        padding_left: frame.padding_left,
        padding_right: frame.padding_right,
        toolbar: frame.toolbar,
        title_text: frame.title_text,
    }
}

/// Asserts that no card, a rounded quad bordered on both sides, frames the
/// element `table`: DESIGN.md's table pages draw their table bare.
pub fn assert_bare(window: &Window, table: &'static str) {
    let view = window.find(table).bounds().scale(window.scale_factor());
    let card = window.painted_quads().into_iter().find(|quad| {
        let (b, w) = (quad.bounds, quad.border_widths);
        w.left.0 > 0.
            && w.right.0 > 0.
            && quad.corner_radii.top_left.0 > 0.
            && b.left() <= view.left()
            && b.right() >= view.right()
            && b.top() <= view.top()
            && b.bottom() >= view.bottom()
    });
    assert!(
        card.is_none(),
        "{table} sits in a card; DESIGN.md's table pages draw it bare: {card:#?}"
    );
}

/// Asserts DESIGN.md's inspector: beside its table with no gap and reaching
/// the split's right edge, or under it across the split's width; in no
/// card; and its heading's `title` `PANE_PADDING` in from its left, with
/// the heading at its top.
pub fn assert_inspector(
    window: &mut Window,
    cx: &mut App,
    split: &'static str,
    table: &'static str,
    inspector: &'static str,
    title: &'static str,
) {
    window.render_frame(cx);
    let split = window.find(split).bounds();
    let table = window.find(table).bounds();
    let pane = window.find(inspector).bounds();
    let heading = window
        .find(SharedString::from(format!("{inspector}-heading")))
        .bounds();
    let title = window.find(title).bounds();
    let check = |what: &str, actual: Pixels, expected: Pixels| {
        assert!(
            (actual - expected).abs() <= px(1.),
            "{inspector}: {what} is {actual:?}, expected {expected:?}"
        );
    };
    if pane.left() > table.left() {
        check(
            "its left against the table's right",
            pane.left(),
            table.right(),
        );
        check("its right against the split's", pane.right(), split.right());
        check("its top against the table's", pane.top(), table.top());
    } else {
        check(
            "its top against the table's bottom",
            pane.top(),
            table.bottom(),
        );
        check(
            "its width against the split's",
            pane.size.width,
            split.size.width,
        );
    }
    let pad = dp_px(PANE_PADDING, window);
    check("the title's inset", title.left() - pane.left(), pad);
    check("the heading's top", heading.top(), pane.top());
    assert!(
        title.top() - pane.top() >= pad - px(1.),
        "{inspector}: the title sits {:?} below the top, under {pad:?}",
        title.top() - pane.top()
    );
    assert_bare(window, inspector);
}

/// Asserts DESIGN.md's drawer: over the right edge of the `body` it covers,
/// top to bottom; the whole width on a page under `FULL_BELOW`, otherwise
/// at least `MIN_WIDTH` wide and leaving the list at least `LIST_KEEPS`; in
/// no card; and its inspector's `title` `PANE_PADDING` in from its left.
/// Returns the drawer's width in dp.
pub fn assert_drawer(
    window: &mut Window,
    cx: &mut App,
    body: &'static str,
    drawer: &'static str,
    inspector: &'static str,
    title: &'static str,
) -> f32 {
    use crate::drawer::{FULL_BELOW, LIST_KEEPS, MIN_WIDTH};
    window.render_frame(cx);
    let body = window.find(body).bounds();
    let pane = window.find(drawer).bounds();
    let title = window.find(title).bounds();
    let dp = |n: f32| dp_px(n, window);
    let check = |what: &str, actual: Pixels, expected: Pixels| {
        assert!(
            (actual - expected).abs() <= px(1.),
            "{drawer}: {what} is {actual:?}, expected {expected:?}"
        );
    };
    check("its right against the body's", pane.right(), body.right());
    check("its top against the body's", pane.top(), body.top());
    check(
        "its bottom against the body's",
        pane.bottom(),
        body.bottom(),
    );
    if body.size.width < dp(FULL_BELOW) {
        check(
            "its width on a narrow page",
            pane.size.width,
            body.size.width,
        );
    } else {
        assert!(
            pane.size.width >= dp(MIN_WIDTH) - px(1.),
            "{drawer}: {:?} wide, under {MIN_WIDTH} dp",
            pane.size.width
        );
        assert!(
            pane.left() - body.left() >= dp(LIST_KEEPS) - px(1.),
            "{drawer}: leaves the list {:?}, under {LIST_KEEPS} dp",
            pane.left() - body.left()
        );
    }
    let pad = dp(PANE_PADDING);
    check("the title's inset", title.left() - pane.left(), pad);
    assert_bare(window, inspector);
    pane.size.width / dp(1.)
}

/// Asserts DESIGN.md's padded frame, a page of cards': 26 dp side padding and
/// the header's toolbar (see [`assert_toolbar`]).
pub fn assert_page_frame(window: &mut Window, cx: &mut App, frame: &PageFrame) -> FrameLayout {
    assert_page_frame_from(window, cx, frame, frame.title)
}

/// [`assert_page_frame`] for a header that leads with something before the
/// title, such as a breadcrumb's parent: the left padding is measured from
/// `lead`, and the toolbar checks still go by the title.
pub fn assert_page_frame_from(
    window: &mut Window,
    cx: &mut App,
    frame: &PageFrame,
    lead: &str,
) -> FrameLayout {
    window.render_frame(cx);
    let mut layout = measure_frame(window, frame);
    let root = window.find(frame.page).bounds();
    let lead = window.find(SharedString::from(lead.to_owned())).bounds();
    layout.padding_left = lead.left() - root.left();
    let dp = |n: f32| dp_px(n, window);
    close(
        window,
        frame,
        &layout,
        "left padding",
        layout.padding_left,
        PAGE_PADDING,
    );
    // A panel draws a hairline border inside the padding.
    assert!(
        layout.padding_right >= dp(PAGE_PADDING) - px(0.5)
            && layout.padding_right <= dp(PAGE_PADDING) + px(1.5),
        "{}: right padding is {:?}, DESIGN.md says {PAGE_PADDING} dp; {layout:#?}",
        frame.page,
        layout.padding_right,
    );
    assert_toolbar(window, frame, &layout);
    layout
}

/// Asserts DESIGN.md's frame without margins: the content, a table page's
/// table, runs from edge to edge of the page, and the header is a toolbar
/// (see [`assert_toolbar`]) whose title sits `PANE_PADDING` in from the
/// page's left edge, with a hairline under it across the page. The layout's
/// paddings are the title's inset and the space right of the content.
pub fn assert_edge_frame(window: &mut Window, cx: &mut App, frame: &PageFrame) -> FrameLayout {
    assert_edge_frame_from(window, cx, frame, frame.title)
}

/// [`assert_edge_frame`] for a header that leads with something before the
/// title, such as a breadcrumb's parent: the title inset is measured from
/// `lead`, and the toolbar checks still go by the title.
pub fn assert_edge_frame_from(
    window: &mut Window,
    cx: &mut App,
    frame: &PageFrame,
    lead: &str,
) -> FrameLayout {
    window.render_frame(cx);
    let mut layout = measure_frame(window, frame);
    let root = window.find(frame.page).bounds();
    let lead = window.find(SharedString::from(lead.to_owned())).bounds();
    layout.padding_left = lead.left() - root.left();
    let content = window.find(frame.content).bounds();
    let check = |what: &str, actual: Pixels, expected: f32| {
        close(window, frame, &layout, what, actual, expected)
    };
    check("title inset", layout.padding_left, PANE_PADDING);
    check(
        "space left of the content",
        content.left() - root.left(),
        0.,
    );
    check("space right of the content", layout.padding_right, 0.);
    assert_toolbar(window, frame, &layout);
    assert_hairline(window, frame, &layout);
    layout
}

/// [`assert_edge_frame`] for a screen embedded where a tab names it, such
/// as a node's tab in the node inspector: its header draws no title, and
/// `lead`, the toolbar's first part, sits `PANE_PADDING` in from the page's
/// left edge, centred on the 38 dp row. A toolbar of controls alone, which
/// sit at its right, passes no lead. The layout's `title_text` is zero, and
/// its `padding_left` too without a lead.
pub fn assert_untitled_edge_frame(
    window: &mut Window,
    cx: &mut App,
    frame: &PageFrame,
    lead: Option<&str>,
) -> FrameLayout {
    window.render_frame(cx);
    assert!(
        window.try_find(frame.title).is_none(),
        "{}: an embedded header draws no title",
        frame.page
    );
    let root = window.find(frame.page).bounds();
    let content = window.find(frame.content).bounds();
    let lead = lead.map(|lead| window.find(SharedString::from(lead.to_owned())).bounds());
    let row = toolbar_row(window, frame);
    let layout = FrameLayout {
        padding_left: lead.map_or(px(0.), |lead| lead.left() - root.left()),
        padding_right: root.right() - content.right(),
        toolbar: row.size.height,
        title_text: px(0.),
        controls: controls(window, frame),
    };
    let check = |what: &str, actual: Pixels, expected: f32| {
        close(window, frame, &layout, what, actual, expected)
    };
    if let Some(lead) = lead {
        check("lead inset", layout.padding_left, PANE_PADDING);
        assert!(
            (lead.center().y - row.center().y).abs() < px(0.5),
            "{}: the toolbar's lead isn't centred on its row: {lead:?} in {row:?}",
            frame.page
        );
    }
    check(
        "space left of the content",
        content.left() - root.left(),
        0.,
    );
    check("space right of the content", layout.padding_right, 0.);
    check("toolbar row", layout.toolbar, TOOLBAR_HEIGHT);
    for (ix, control) in layout.controls.iter().enumerate() {
        check(&format!("control {ix}"), *control, CONTROL_HEIGHT);
    }
    assert_hairline(window, frame, &layout);
    layout
}

/// The hairline: a quad bordered below, across the page, under the row.
fn assert_hairline(window: &Window, frame: &PageFrame, layout: &FrameLayout) {
    let scale = window.scale_factor();
    let row = toolbar_row(window, frame).scale(scale);
    let root = window.find(frame.page).bounds().scale(scale);
    let hairline = window.painted_quads().into_iter().any(|quad| {
        let b = quad.bounds;
        quad.border_widths.bottom.0 > 0.
            && (b.left() - root.left()).0.abs() < 1.
            && (b.right() - root.right()).0.abs() < 1.
            && b.top() <= row.top()
            && b.bottom() >= row.bottom()
    });
    assert!(
        hairline,
        "{}: no hairline under the toolbar across the page; {layout:#?}",
        frame.page
    );
}

/// Asserts DESIGN.md's toolbar: a 38 dp row with the title as its 13 dp
/// label, centred on the row, and every control 24 dp high.
pub fn assert_toolbar(window: &Window, frame: &PageFrame, layout: &FrameLayout) {
    let check = |what: &str, actual: Pixels, expected: f32| {
        close(window, frame, layout, what, actual, expected)
    };
    check("toolbar row", layout.toolbar, TOOLBAR_HEIGHT);
    check("title text", layout.title_text, LABEL_TEXT);
    let (title, row) = (
        window.find(frame.title).bounds(),
        toolbar_row(window, frame),
    );
    assert!(
        (title.center().y - row.center().y).abs() < px(0.5),
        "{}: the title isn't centred on the toolbar's row: {title:?} in {row:?}",
        frame.page
    );
    for (ix, control) in layout.controls.iter().enumerate() {
        check(&format!("control {ix}"), *control, CONTROL_HEIGHT);
    }
}

fn measure_frame(window: &Window, frame: &PageFrame) -> FrameLayout {
    let root = window.find(frame.page).bounds();
    let title = window.find(frame.title).bounds();
    let content = window.find(frame.content).bounds();
    FrameLayout {
        padding_left: title.left() - root.left(),
        padding_right: root.right() - content.right(),
        toolbar: toolbar_row(window, frame).size.height,
        title_text: title_text(window, frame.title_text, title.size.width),
        controls: controls(window, frame),
    }
}

/// Each control's height: those left on the row, and the "…" holding the
/// rest.
fn controls(window: &Window, frame: &PageFrame) -> Vec<Pixels> {
    let prefix = prefix(frame);
    (0..16)
        .map(|ix| format!("{prefix}-slot-{ix}"))
        .chain([format!("{prefix}-more")])
        .filter_map(|id| window.try_find(id))
        .map(|slot| slot.bounds().size.height)
        .collect()
}

/// The header's prefix, from its title's id.
fn prefix(frame: &PageFrame) -> &'static str {
    frame
        .title
        .strip_suffix("-title")
        .unwrap_or_else(|| panic!("{}: a PageHeader's title is <prefix>-title", frame.title))
}

fn toolbar_row(window: &Window, frame: &PageFrame) -> gpui_kit::Bounds<Pixels> {
    window.find(format!("{}-toolbar", prefix(frame))).bounds()
}

/// Asserts that `actual` is `expected` dp, to half a pixel.
fn close(
    window: &Window,
    frame: &PageFrame,
    layout: &FrameLayout,
    what: &str,
    actual: Pixels,
    expected: f32,
) {
    let expected_px = dp_px(expected, window);
    assert!(
        (actual - expected_px).abs() < px(0.5),
        "{}: {what} is {actual:?}, DESIGN.md says {expected} dp ({expected_px:?}); {layout:#?}",
        frame.page,
    );
}

/// Asserts DESIGN.md's table: a 26 dp column header, and 26 dp rows with
/// group headers at the row height (a `uniform_list` needs uniform lines).
pub fn assert_table(window: &mut Window, cx: &mut App, table: &Table) -> TableRows {
    window.render_frame(cx);
    let header = table
        .table
        .map(|frame| window.find(table.list).bounds().top() - window.find(frame).bounds().top());
    let rows = measure(window, table.list, header);
    let dp = |n: f32| dp_px(n, window);
    let close = |what: &str, actual: Pixels, expected: f32| {
        assert!(
            (actual - dp(expected)).abs() < px(0.5),
            "{}: {what} is {actual:?}, DESIGN.md says {expected} dp ({:?}); {rows:#?}",
            table.list,
            dp(expected),
        );
    };
    if let Some(header) = rows.header {
        close("column header", header, HEADER_HEIGHT);
    }
    close("row", rows.row, ROW_HEIGHT);
    if let Some(group) = rows.group {
        close("group header", group, ROW_HEIGHT);
    }
    // The footer, when the table has counts or a legend: one row under
    // the rows, never a strip above them.
    let footer = table
        .table
        .and_then(|frame| frame.strip_suffix("-table-scroll"))
        .and_then(|prefix| window.try_find(format!("{prefix}-footer")));
    if let (Some(footer), Some(frame)) = (footer, table.table) {
        close("footer", footer.bounds().size.height, FOOTER_HEIGHT);
        assert!(
            footer.bounds().top() >= window.find(frame).bounds().bottom(),
            "{}: the footer sits above the rows",
            table.list
        );
    }
    rows
}

fn measure(window: &Window, list: &'static str, header: Option<Pixels>) -> TableRows {
    let lines = lines(window, list);
    let row = lines
        .iter()
        .find(|line| line.role() == Some(Role::ListBoxOption))
        .unwrap_or_else(|| panic!("{list} shows no rows"));
    let group = lines
        .iter()
        .find(|line| line.role() == Some(Role::Heading))
        .map(|group| group.bounds().size.height);
    TableRows {
        header,
        row: row.bounds().size.height,
        group,
    }
}

/// The list's visible rows and group headers, top to bottom.
fn lines(window: &Window, list: &str) -> Vec<ElementSnapshot> {
    let list = ElementId::from(SharedString::from(list.to_owned()));
    let mut lines: Vec<_> = snapshots(window)
        .into_iter()
        .filter(|line| {
            line.visible()
                && matches!(line.role(), Some(Role::ListBoxOption | Role::Heading))
                && line.path().contains(&list)
        })
        .collect();
    lines.sort_by(|a, b| f32::from(a.bounds().top()).total_cmp(&f32::from(b.bounds().top())));
    lines
}

/// The title's font size, from its drawn width and the width its text shapes
/// to at a known size. Headless text advances in proportion to the size.
fn title_text(window: &Window, text: &str, width: Pixels) -> Pixels {
    let reference = px(100.);
    reference * (width / shaped_width(window, text, reference))
}

fn shaped_width(window: &Window, text: &str, size: Pixels) -> Pixels {
    let mut face = font(".SystemUIFont");
    face.weight = crate::ui::LABEL_WEIGHT;
    let run = TextRun {
        len: text.len(),
        font: face,
        color: Default::default(),
        background_color: None,
        underline: None,
        strikethrough: None,
    };
    window
        .text_system()
        .shape_line(SharedString::from(text.to_owned()), size, &[run], None)
        .width
}

/// Draws until the page's header stops asking for frames: it folds its
/// controls from what its parts measured on the frame before.
pub fn settle_header(window: &mut Window, cx: &mut App) {
    for _ in 0..4 {
        window.render_frame(cx);
        if window.simulate_next_frame(cx) == 0 {
            return;
        }
    }
    panic!("the header keeps moving");
}
