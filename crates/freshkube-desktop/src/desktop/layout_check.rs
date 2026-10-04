//! Measures a table page from the bounds its last frame painted, so a page
//! that drifts from DESIGN.md's table page fails a test instead of a review.
//!
//! A page names a few elements by id; rows and group headers are found by
//! their accessibility roles inside the list, so the check needs no access to
//! the page's state and works the same on every table page.

use gpui_kit::base::test_support::{ElementSnapshot, snapshots};
use gpui_kit::test::TestWindowExt;
use gpui_kit::{App, ElementId, FontWeight, Pixels, Role, SharedString, TextRun, Window, font, px};

use super::PAGE_PADDING;
use crate::ui::dp_px;

/// DESIGN.md's table page, in dp.
pub(crate) const HEADER_HEIGHT: f32 = 30.;
pub(crate) const ROW_HEIGHT: f32 = 34.;
pub(crate) const COMPACT_ROW_HEIGHT: f32 = 26.;
pub(crate) const TITLE_TEXT: f32 = 20.;
pub(crate) const TITLE_LINE: f32 = 28.;

/// The elements a table page names for the check.
pub(crate) struct TablePage {
    /// The page's root; its edges are where the page padding starts.
    pub page: &'static str,
    /// The page title, drawn by `ui::page_title`, and the text it shows.
    pub title: &'static str,
    pub title_text: &'static str,
    /// The table's frame: the column header is drawn at its top.
    pub table: &'static str,
    /// The list under the column header, holding rows (`Role::ListBoxOption`)
    /// and group headers (`Role::Heading`).
    pub list: &'static str,
    /// The control that switches between comfortable and compact rows.
    pub density: &'static str,
}

/// What a table page drew, in pixels.
#[derive(Debug)]
pub(crate) struct TableLayout {
    pub header: Pixels,
    pub row: Pixels,
    pub compact_row: Pixels,
    /// Group headers at comfortable and compact density, when the list
    /// showed a group.
    pub group: Option<(Pixels, Pixels)>,
    pub padding_left: Pixels,
    pub padding_right: Pixels,
    pub title_line: Pixels,
    pub title_text: Pixels,
}

/// Measures `page` at both densities, leaves it at the density it had, and
/// asserts DESIGN.md's sizes: a 30 dp column header, 34 and 26 dp rows, group
/// headers at the row height (a `uniform_list` needs uniform lines), 26 dp
/// side padding and a 20 dp title on a 28 dp line.
pub(crate) fn assert_table_page(
    window: &mut Window,
    cx: &mut App,
    page: &TablePage,
) -> TableLayout {
    window.render_frame(cx);
    let first = measure(window, page);
    window.click(page.density, cx);
    window.render_frame(cx);
    let second = measure(window, page);
    window.click(page.density, cx);
    window.render_frame(cx);
    // Whichever density the page showed first, order them.
    let (comfortable, compact) = if first.row >= second.row {
        (first, second)
    } else {
        (second, first)
    };
    let layout = TableLayout {
        header: comfortable.header,
        row: comfortable.row,
        compact_row: compact.row,
        group: comfortable.group.zip(compact.group),
        padding_left: comfortable.padding_left,
        padding_right: comfortable.padding_right,
        title_line: comfortable.title_line,
        title_text: title_text(window, page, comfortable.title_width),
    };
    let dp = |n: f32| dp_px(n, window);
    let close = |what: &str, actual: Pixels, expected: f32| {
        assert!(
            (actual - dp(expected)).abs() < px(0.5),
            "{}: {what} is {actual:?}, DESIGN.md says {expected} dp ({:?}); {layout:#?}",
            page.page,
            dp(expected),
        );
    };
    close("column header", layout.header, HEADER_HEIGHT);
    close("comfortable row", layout.row, ROW_HEIGHT);
    close("compact row", layout.compact_row, COMPACT_ROW_HEIGHT);
    if let Some((group, compact_group)) = layout.group {
        close("comfortable group header", group, ROW_HEIGHT);
        close("compact group header", compact_group, COMPACT_ROW_HEIGHT);
    }
    close("left padding", layout.padding_left, PAGE_PADDING);
    // The table's panel draws a hairline border inside the padding.
    assert!(
        layout.padding_right >= dp(PAGE_PADDING) - px(0.5)
            && layout.padding_right <= dp(PAGE_PADDING) + px(1.5),
        "{}: right padding is {:?}, DESIGN.md says {PAGE_PADDING} dp; {layout:#?}",
        page.page,
        layout.padding_right,
    );
    close("title line", layout.title_line, TITLE_LINE);
    close("title text", layout.title_text, TITLE_TEXT);
    layout
}

/// One density's measurements.
struct Measured {
    header: Pixels,
    row: Pixels,
    group: Option<Pixels>,
    padding_left: Pixels,
    padding_right: Pixels,
    title_line: Pixels,
    title_width: Pixels,
}

fn measure(window: &Window, page: &TablePage) -> Measured {
    let root = window.find(page.page).bounds();
    let title = window.find(page.title).bounds();
    let table = window.find(page.table).bounds();
    let list = window.find(page.list).bounds();
    let lines = lines(window, page.list);
    let row = lines
        .iter()
        .find(|line| line.role() == Some(Role::ListBoxOption))
        .unwrap_or_else(|| panic!("{}: {} shows no rows", page.page, page.list));
    let group = lines
        .iter()
        .find(|line| line.role() == Some(Role::Heading))
        .map(|group| group.bounds().size.height);
    Measured {
        header: list.top() - table.top(),
        row: row.bounds().size.height,
        group,
        padding_left: title.left() - root.left(),
        padding_right: root.right() - table.right(),
        title_line: title.size.height,
        title_width: title.size.width,
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
fn title_text(window: &Window, page: &TablePage, width: Pixels) -> Pixels {
    let reference = px(100.);
    reference * (width / shaped_width(window, page.title_text, reference))
}

fn shaped_width(window: &Window, text: &str, size: Pixels) -> Pixels {
    let mut face = font(".SystemUIFont");
    face.weight = FontWeight::BLACK;
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
