use gpui_kit::component::Root;
use gpui_kit::test::TestWindowExt;
use gpui_kit::{
    AnyView, AnyWindowHandle, AppContext, Background, Entity, InputEvent, IntoElement, MouseButton,
    MouseDownEvent, MouseMoveEvent, MouseUpEvent, Pixels, Point, Render, ScrollDelta,
    ScrollWheelEvent, StyleRefinement, TestAppContext, TouchPhase, point, px, size,
};

use super::*;
use crate::table::{CELL_PAD, FOOTER_HEIGHT, GLYPH_WIDTH, GroupRow, glyph_cell};
use crate::ui::Tone;

/// Lines 0 and 3 are group headers; the rest are rows.
fn lines(line: usize) -> Option<bool> {
    (line < 7).then_some(line != 0 && line != 3)
}

#[test]
fn steps_over_group_lines() {
    assert_eq!(step_line(7, Some(2), 1, lines), Some(4));
    assert_eq!(step_line(7, Some(4), -1, lines), Some(2));
    assert_eq!(step_line(7, Some(1), 3, lines), Some(5));
}

#[test]
fn stops_at_either_end() {
    assert_eq!(step_line(7, Some(6), 1, lines), Some(6));
    assert_eq!(step_line(7, Some(1), -1, lines), Some(1));
    assert_eq!(step_line(7, Some(5), 20, lines), Some(6));
}

#[test]
fn starts_at_the_first_or_last_row() {
    assert_eq!(step_line(7, None, 1, lines), Some(1));
    assert_eq!(step_line(7, None, -1, lines), Some(6));
    assert_eq!(step_line(7, Some(99), 1, lines), Some(1));
    assert_eq!(step_line(0, None, 1, lines), None);
    assert_eq!(step_line(1, None, 1, lines), None);
}

/// A label, and whether the column pins.
struct Column(SharedString, bool);

impl TableColumn for Column {
    fn label(&self) -> &SharedString {
        &self.0
    }

    fn width(&self) -> f32 {
        if self.0 == "Glyph" { GLYPH_WIDTH } else { 200. }
    }

    fn flexible(&self) -> bool {
        false
    }

    fn pinned(&self) -> bool {
        self.1
    }
}

/// Six 200 dp columns, wider than its window, so it scrolls sideways.
/// With `grouped`, line 0 is a group's header and the rows follow it.
struct Wide {
    table: TableState,
    columns: Vec<Column>,
    rows: usize,
    grouped: bool,
    selected: Option<usize>,
    /// Cells and headers that take clicks and record them, as
    /// Applications' report buttons do.
    interactive: bool,
    cell_clicks: Vec<SharedString>,
    hovered: Vec<SharedString>,
    sorts: Vec<SharedString>,
    carded: bool,
    /// The first column holds each row's glyph, as a page's does.
    glyphs: bool,
    /// How many rows are marked, for the footer's counts.
    marked: Option<usize>,
    /// The footer's legend line.
    legend: Option<SharedString>,
    /// The loading rows, until the first read answers.
    loading: Option<super::super::LoadingRows>,
    /// `DataTable::fit`'s most lines, for a table in a scrolling page.
    fit: Option<usize>,
}

impl Wide {
    fn new(pinned: usize, rows: usize, grouped: bool) -> Self {
        Self {
            table: TableState::new("wide"),
            columns: (0..6)
                .map(|ix| Column(format!("Column {ix}").into(), ix < pinned))
                .collect(),
            rows,
            grouped,
            selected: None,
            interactive: false,
            cell_clicks: Vec::new(),
            hovered: Vec::new(),
            sorts: Vec::new(),
            carded: false,
            glyphs: false,
            marked: None,
            legend: None,
            loading: None,
            fit: None,
        }
    }

    /// `marked` rows marked and four of nine showing, then `legend`.
    fn footer(self, marked: Option<usize>, legend: Option<&str>) -> Self {
        Self {
            marked,
            legend: legend.map(|legend| legend.to_owned().into()),
            ..self
        }
    }

    /// The first column becomes a glyph column, as a page's is.
    fn glyphs(mut self) -> Self {
        self.columns[0].0 = "Glyph".into();
        Self {
            glyphs: true,
            ..self
        }
    }

    fn carded(self) -> Self {
        Self {
            carded: true,
            ..self
        }
    }

    fn interactive(self) -> Self {
        Self {
            interactive: true,
            ..self
        }
    }

    fn first_row(&self) -> usize {
        usize::from(self.grouped)
    }
}

impl TableSource for Wide {
    type Key = usize;
    type Sort = SharedString;
    type Column = Column;
    type Row<'a> = ();

    fn table_state(&self) -> &TableState {
        &self.table
    }

    fn loading(&self) -> Option<&super::super::LoadingRows> {
        self.loading.as_ref()
    }

    fn columns(&self) -> &[Column] {
        &self.columns
    }

    fn width(&self) -> f32 {
        self.columns.iter().map(TableColumn::width).sum()
    }

    fn list_label(&self) -> String {
        "Wide rows".into()
    }

    fn sorting(&self, column: &Column) -> Option<(SharedString, Option<SortOrder>)> {
        self.interactive.then(|| (column.0.clone(), None))
    }

    fn sort(&mut self, column: SharedString, _: &mut Context<Self>) {
        self.sorts.push(column);
    }

    fn line_count(&self) -> usize {
        self.rows + self.first_row()
    }

    fn line(&self, line: usize, _: &App) -> Option<Line<usize, ()>> {
        if self.grouped && line == 0 {
            return Some(Line::Group(0));
        }
        (line < self.line_count()).then(|| {
            Line::Row(TableRow {
                key: line,
                id: SharedString::from(format!("wide-row-{line}")).into(),
                label: format!("Row {line}").into(),
                tooltip: None,
                marked: false,
                muted: false,
                data: (),
            })
        })
    }

    fn cell(
        &self,
        row: &TableRow<usize, ()>,
        _: &RowStyle,
        column: &Column,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let label = SharedString::from(format!("{} {}", column.0, row.key));
        if self.glyphs && column.0 == "Glyph" {
            return glyph_cell(column)
                .id(("wide-glyph", row.key))
                .test_support()
                .children(ui::status_glyph(Tone::Good, cx))
                .into_any_element();
        }
        let (click, hover) = (label.clone(), label.clone());
        cell(column)
            .id(label.clone())
            .test_support()
            .child(label)
            .when(self.interactive, |this| {
                this.on_click(
                    cx.listener(move |wide, _, _, _| wide.cell_clicks.push(click.clone())),
                )
                .on_hover(cx.listener(move |wide, hovered: &bool, _, _| {
                    if *hovered {
                        wide.hovered.push(hover.clone())
                    }
                }))
            })
            .into_any_element()
    }

    fn group(&self, _: usize, cx: &mut Context<Self>) -> Option<AnyElement> {
        Some(
            GroupRow::new("wide-group", Tone::Crit, "Failing", ROW_HEIGHT)
                // Longer than the table is wide, as a node's group's can be.
                .detail(vec![
                    format!("{} rows", self.rows),
                    "a detail that runs on ".repeat(12),
                ])
                .chevron(
                    div()
                        .id("wide-group-action")
                        .test_support()
                        .child("Select all"),
                )
                .render(cx)
                .into_any_element(),
        )
    }

    fn empty(&self, _: &mut Context<Self>) -> Option<AnyElement> {
        (self.rows == 0).then(|| "No rows match this filter.".into_any_element())
    }

    fn selected_key(&self) -> Option<&usize> {
        self.selected.as_ref()
    }

    fn line_of(&self, key: &usize) -> Option<usize> {
        Some(*key)
    }

    fn click(&mut self, key: &usize, _: &ClickEvent, _: &mut Window, cx: &mut Context<Self>) {
        self.selected = Some(*key);
        cx.notify();
    }

    fn counts(&self, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let Some(marked) = self.marked else {
            return Vec::new();
        };
        vec![
            super::super::selection("wide-marks", marked, [], cx).into_any_element(),
            super::super::showing("wide-showing", 4, 9, div().child("Show all 9"), cx)
                .into_any_element(),
        ]
    }

    fn legend(&self, _: &Window, cx: &mut Context<Self>) -> Option<AnyElement> {
        let legend = self.legend.clone()?;
        Some(
            super::super::legend_line("wide-legend", legend, "The legend", cx)
                .test_support()
                .into_any_element(),
        )
    }
}

impl Render for Wide {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let table = if self.carded {
            DataTable::new().carded()
        } else {
            DataTable::new()
        };
        let table = match self.fit {
            Some(max) => table.fit(max),
            None => table,
        };
        v_flex()
            .size_full()
            .child(table.render(self, window, cx).flex_1().min_h_0())
    }
}

/// The table in a window `width` wide and 400 tall, at the default
/// text size.
fn open(cx: &mut TestAppContext, wide: Wide, width: f32) -> (AnyWindowHandle, Entity<Wide>) {
    cx.update(|cx| {
        gpui_kit::init(cx);
        crate::theme::install(cx);
        crate::text_size::install(None, cx);
        cx.set_reduce_motion(true);
    });
    let mut view = None;
    let handle = cx.open_window(size(px(width), px(400.)), |window, cx| {
        let wide = cx.new(|_| wide);
        view = Some(wide.clone());
        Root::new(wide, window, cx)
    });
    (handle.into(), view.unwrap())
}

/// Scrolls the table `by` points to the right.
fn scroll_right(window: &mut Window, by: f32, cx: &mut App) {
    window.scroll(
        "wide-table-scroll",
        ScrollDelta::Pixels(point(px(-by), px(0.))),
        cx,
    );
}

/// How far right of the table's visible left edge an element starts.
fn inset(window: &Window, id: impl Into<ElementId>) -> f32 {
    let viewport = window.find("wide-table-scroll").bounds();
    f32::from(window.find(id).bounds().left() - viewport.left())
}

/// The same for a pinned cell on `line`, or a header cell with `line`
/// `None`, found inside the pinned part.
fn pinned_inset(window: &mut Window, line: Option<usize>, id: impl Into<ElementId>) -> f32 {
    let viewport = window.find("wide-table-scroll").bounds();
    let scope: ElementId = match line {
        Some(line) => ("wide-pinned", line).into(),
        None => "wide-pinned-header".into(),
    };
    f32::from(window.within(scope).find(id).bounds().left() - viewport.left())
}

/// Sends one wheel event over the middle of the table, as the start of a
/// gesture: a trackpad's gesture locks to the axis it starts on.
fn wheel(window: &mut Window, x: f32, y: f32, cx: &mut App) {
    window.render_frame(cx);
    let position = window.find("wide-table-scroll").bounds().center();
    window.dispatch_event(
        MouseMoveEvent {
            position,
            ..Default::default()
        }
        .to_platform_input(),
        cx,
    );
    window.dispatch_event(
        ScrollWheelEvent {
            position,
            delta: ScrollDelta::Pixels(point(px(x), px(y))),
            touch_phase: TouchPhase::Started,
            ..Default::default()
        }
        .to_platform_input(),
        cx,
    );
    window.render_frame(cx);
}

/// How far below the table's top edge an element starts.
fn top_of(window: &Window, id: impl Into<ElementId>) -> f32 {
    let viewport = window.find("wide-table-scroll").bounds();
    f32::from(window.find(id).bounds().top() - viewport.top())
}

/// Moves the pointer to `position` and presses and releases there,
/// through the window's own dispatch, without drawing a frame.
fn press(window: &mut Window, position: Point<Pixels>, cx: &mut App) {
    let events = [
        MouseMoveEvent {
            position,
            ..Default::default()
        }
        .to_platform_input(),
        MouseDownEvent {
            position,
            button: MouseButton::Left,
            click_count: 1,
            ..Default::default()
        }
        .to_platform_input(),
        MouseUpEvent {
            position,
            button: MouseButton::Left,
            click_count: 1,
            ..Default::default()
        }
        .to_platform_input(),
    ];
    for event in events {
        window.dispatch_event(event, cx);
    }
}

/// A page that keeps its table in a cached view, as the Resources pane
/// keeps its own.
struct CachedTable(Entity<Wide>);

impl Render for CachedTable {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        AnyView::from(self.0.clone()).cached(StyleRefinement::default().size_full())
    }
}

/// A page that keeps its table cached and draws the loading rows'
/// motion over it, as a page adopting them would.
struct LoadingPage {
    table: Entity<Wide>,
    motion: Entity<super::super::LoadingMotion>,
}

impl Render for LoadingPage {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        freshkube_probe::probe::hit("test.loading-page");
        div()
            .relative()
            .size_full()
            .child(AnyView::from(self.table.clone()).cached(StyleRefinement::default().size_full()))
            .child(self.motion.clone())
    }
}

/// The loading motion asks frames while the table draws its loading
/// rows, and none from the frame its rows replace them: it forgets the
/// bars, so it veils nothing over the rows either.
#[gpui_kit::test]
fn the_loading_motion_stops_when_the_rows_arrive(cx: &mut TestAppContext) {
    use super::super::{LoadingRows, Look};
    cx.update(|cx| {
        gpui_kit::init(cx);
        crate::theme::install(cx);
        crate::text_size::install(None, cx);
        cx.set_reduce_motion(false);
    });
    let mut wide = None;
    let handle: AnyWindowHandle = cx
        .open_window(size(px(900.), px(400.)), |window, cx| {
            let rows = LoadingRows::new("wide");
            let motion = cx.new(|_| rows.motion(Look::Shimmer));
            let table = cx.new(|_| Wide {
                loading: Some(rows),
                ..Wide::new(0, 4, false)
            });
            wide = Some(table.clone());
            let page = cx.new(|_| LoadingPage { table, motion });
            Root::new(page, window, cx)
        })
        .into();
    let wide = wide.unwrap();
    let frame = |cx: &mut TestAppContext| {
        cx.background_executor
            .advance_clock(std::time::Duration::from_millis(16));
        let asked = cx
            .update_window(handle, |_, window, cx| window.simulate_next_frame(cx))
            .unwrap();
        cx.run_until_parked();
        asked
    };
    cx.run_until_parked();
    for _ in 0..3 {
        assert!(frame(cx) > 0, "the bars move while the table loads");
    }
    cx.update(|cx| {
        wide.update(cx, |wide, cx| {
            wide.loading = None;
            cx.notify();
        })
    });
    cx.run_until_parked();
    // At most the frame already asked, which draws the rows.
    frame(cx);
    let pages = freshkube_probe::probe::count("test.loading-page");
    for _ in 0..3 {
        assert_eq!(frame(cx), 0, "no frames once the rows arrive");
    }
    assert_eq!(freshkube_probe::probe::count("test.loading-page"), pages);
    cx.update_window(handle, |_, window, _| {
        assert!(window.try_find("wide-loading").is_none());
        assert!(window.find("wide-row-0").visible());
    })
    .unwrap();
}

/// A fitted table's loading rows take its most lines, as its rows
/// would, rather than collapsing with no lines to fit; the rows that
/// arrive then take their own height.
#[gpui_kit::test]
fn a_fitted_table_loads_at_its_most_lines(cx: &mut TestAppContext) {
    let wide = Wide {
        loading: Some(super::super::LoadingRows::new("wide")),
        fit: Some(3),
        ..Wide::new(0, 2, false)
    };
    let (handle, wide) = open(cx, wide, 900.);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let loading = window.find("wide-loading");
        assert!(loading.visible());
        let height = loading.bounds().size.height;
        assert!(
            (height - ui::dp_px(3. * ROW_HEIGHT, window)).abs() < px(0.5),
            "{height:?}"
        );
        wide.update(cx, |wide, cx| {
            wide.loading = None;
            cx.notify();
        });
        window.render_frame(cx);
        let list = window.find("wide-list").bounds().size.height;
        assert!(
            (list - ui::dp_px(2. * ROW_HEIGHT, window)).abs() < px(0.5),
            "{list:?}"
        );
    })
    .unwrap();
}

mod appearance;
mod pinned;
mod scrolling;
