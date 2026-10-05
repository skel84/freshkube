//! Measures a page from the bounds its last frame painted, so a page that
//! drifts from DESIGN.md fails a test instead of a review.
//!
//! `assert_page_frame` checks a padded page's padding and toolbar,
//! `assert_edge_frame` an edge-to-edge page's inset and toolbar,
//! `assert_table` checks one table's header and rows (a page may hold
//! several), and `assert_table_page` checks a table page with the edge
//! frame and its table, and that the table sits in no card. A page names a few
//! elements by id; rows and group headers are found by their accessibility
//! roles inside the list, so the checks need no access to the page's state.

use gpui_kit::base::test_support::{ElementSnapshot, snapshots};
use gpui_kit::test::TestWindowExt;
use gpui_kit::{App, ElementId, Pixels, Role, SharedString, TextRun, Window, font, px};

use super::PAGE_PADDING;
use crate::ui::dp_px;
use freshkube_ui::page::PANE_PADDING;

/// DESIGN.md's table page, in dp.
pub(crate) const HEADER_HEIGHT: f32 = 26.;
pub(crate) const ROW_HEIGHT: f32 = 26.;
/// The header's toolbar: its row, the title as its label, and its controls.
pub(crate) const TOOLBAR_HEIGHT: f32 = 38.;
pub(crate) const LABEL_TEXT: f32 = 13.;
pub(crate) const CONTROL_HEIGHT: f32 = 24.;

/// The elements a page names for the frame check.
pub(crate) struct PageFrame {
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
pub(crate) struct Table {
    /// The table's frame, with the column header drawn at its top, or `None`
    /// for a list without column captions.
    pub table: Option<&'static str>,
    /// The list under the column header, holding rows (`Role::ListBoxOption`)
    /// and group headers (`Role::Heading`).
    pub list: &'static str,
}

/// The elements a table page names for the check: its frame and its table.
pub(crate) struct TablePage {
    pub page: &'static str,
    pub title: &'static str,
    pub title_text: &'static str,
    pub table: &'static str,
    pub list: &'static str,
}

/// What a page's frame drew, in pixels.
#[derive(Debug)]
pub(crate) struct FrameLayout {
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
pub(crate) struct TableRows {
    pub header: Option<Pixels>,
    pub row: Pixels,
    pub group: Option<Pixels>,
}

/// What a table page drew, in pixels, for a caller's own assertions and
/// failure messages.
#[derive(Debug)]
#[allow(dead_code, reason = "each caller reads the measurements it needs")]
pub(crate) struct TableLayout {
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
pub(crate) fn assert_table_page(
    window: &mut Window,
    cx: &mut App,
    page: &TablePage,
) -> TableLayout {
    let frame = assert_edge_frame(
        window,
        cx,
        &PageFrame {
            page: page.page,
            title: page.title,
            title_text: page.title_text,
            content: page.table,
        },
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
pub(crate) fn assert_bare(window: &Window, table: &'static str) {
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

/// Asserts DESIGN.md's padded frame, a page of cards': 26 dp side padding and
/// the header's toolbar (see [`assert_toolbar`]).
pub(crate) fn assert_page_frame(
    window: &mut Window,
    cx: &mut App,
    frame: &PageFrame,
) -> FrameLayout {
    window.render_frame(cx);
    let layout = measure_frame(window, frame);
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
pub(crate) fn assert_edge_frame(
    window: &mut Window,
    cx: &mut App,
    frame: &PageFrame,
) -> FrameLayout {
    window.render_frame(cx);
    let layout = measure_frame(window, frame);
    let root = window.find(frame.page).bounds();
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
    // The hairline: a quad bordered below, across the page, under the row.
    let scale = window.scale_factor();
    let row = toolbar_row(window, frame).scale(scale);
    let root = root.scale(scale);
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
    layout
}

/// Asserts DESIGN.md's toolbar: a 38 dp row with the title as its 13 dp
/// label, centred on the row, and every control 24 dp high.
pub(crate) fn assert_toolbar(window: &Window, frame: &PageFrame, layout: &FrameLayout) {
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
    let prefix = prefix(frame);
    // The controls left on the row, and the "…" holding the rest.
    let controls = (0..16)
        .map(|ix| format!("{prefix}-slot-{ix}"))
        .chain([format!("{prefix}-more")])
        .filter_map(|id| window.try_find(id))
        .map(|slot| slot.bounds().size.height)
        .collect();
    FrameLayout {
        padding_left: title.left() - root.left(),
        padding_right: root.right() - content.right(),
        toolbar: toolbar_row(window, frame).size.height,
        title_text: title_text(window, frame.title_text, title.size.width),
        controls,
    }
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
pub(crate) fn assert_table(window: &mut Window, cx: &mut App, table: &Table) -> TableRows {
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
