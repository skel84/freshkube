//! Measures a page from the bounds its last frame painted, so a page that
//! drifts from DESIGN.md fails a test instead of a review.
//!
//! `assert_page_frame` checks a padded page's padding and title (and
//! `assert_page_frame_from` one whose header leads with a breadcrumb),
//! `assert_edge_frame` an edge-to-edge page's inset and title,
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
pub(crate) const TITLE_TEXT: f32 = 20.;
pub(crate) const TITLE_LINE: f32 = 28.;

/// The elements a page names for the frame check.
pub(crate) struct PageFrame {
    /// The page's root; its edges are where the page padding starts.
    pub page: &'static str,
    /// The page title, drawn by `ui::page_title`, and the text it shows.
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
    pub title_line: Pixels,
    pub title_text: Pixels,
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
    pub title_line: Pixels,
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
        title_line: frame.title_line,
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
/// a 20 dp title on a 28 dp line.
pub(crate) fn assert_page_frame(
    window: &mut Window,
    cx: &mut App,
    frame: &PageFrame,
) -> FrameLayout {
    assert_page_frame_from(window, cx, frame, frame.title)
}

/// [`assert_page_frame`] for a header that leads with something before the
/// title, such as a breadcrumb's parent: the left padding is measured from
/// `lead`, and the title keeps its size and line checks.
pub(crate) fn assert_page_frame_from(
    window: &mut Window,
    cx: &mut App,
    frame: &PageFrame,
    lead: &str,
) -> FrameLayout {
    window.render_frame(cx);
    let root = window.find(frame.page).bounds();
    let lead = window.find(SharedString::from(lead.to_owned())).bounds();
    let title = window.find(frame.title).bounds();
    let content = window.find(frame.content).bounds();
    let layout = FrameLayout {
        padding_left: lead.left() - root.left(),
        padding_right: root.right() - content.right(),
        title_line: title.size.height,
        title_text: title_text(window, frame.title_text, title.size.width),
    };
    let dp = |n: f32| dp_px(n, window);
    let close = |what: &str, actual: Pixels, expected: f32| {
        assert!(
            (actual - dp(expected)).abs() < px(0.5),
            "{}: {what} is {actual:?}, DESIGN.md says {expected} dp ({:?}); {layout:#?}",
            frame.page,
            dp(expected),
        );
    };
    close("left padding", layout.padding_left, PAGE_PADDING);
    // A panel draws a hairline border inside the padding.
    assert!(
        layout.padding_right >= dp(PAGE_PADDING) - px(0.5)
            && layout.padding_right <= dp(PAGE_PADDING) + px(1.5),
        "{}: right padding is {:?}, DESIGN.md says {PAGE_PADDING} dp; {layout:#?}",
        frame.page,
        layout.padding_right,
    );
    close("title line", layout.title_line, TITLE_LINE);
    close("title text", layout.title_text, TITLE_TEXT);
    layout
}

/// Asserts DESIGN.md's frame without margins: the content, a table page's
/// table, runs from edge to edge of the page, and the title sits
/// `PANE_PADDING` in from its left edge, 20 dp text on a 28 dp line. The
/// layout's paddings are the title's inset and the space right of the
/// content.
pub(crate) fn assert_edge_frame(
    window: &mut Window,
    cx: &mut App,
    frame: &PageFrame,
) -> FrameLayout {
    window.render_frame(cx);
    let root = window.find(frame.page).bounds();
    let title = window.find(frame.title).bounds();
    let content = window.find(frame.content).bounds();
    let layout = FrameLayout {
        padding_left: title.left() - root.left(),
        padding_right: root.right() - content.right(),
        title_line: title.size.height,
        title_text: title_text(window, frame.title_text, title.size.width),
    };
    let dp = |n: f32| dp_px(n, window);
    let close = |what: &str, actual: Pixels, expected: f32| {
        assert!(
            (actual - dp(expected)).abs() < px(0.5),
            "{}: {what} is {actual:?}, DESIGN.md says {expected} dp ({:?}); {layout:#?}",
            frame.page,
            dp(expected),
        );
    };
    close("title inset", layout.padding_left, PANE_PADDING);
    close(
        "space left of the content",
        content.left() - root.left(),
        0.,
    );
    close("space right of the content", layout.padding_right, 0.);
    close("title line", layout.title_line, TITLE_LINE);
    close("title text", layout.title_text, TITLE_TEXT);
    layout
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
    face.weight = crate::ui::TITLE_WEIGHT;
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
