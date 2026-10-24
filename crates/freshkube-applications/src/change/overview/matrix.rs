//! The matrix: one row per Stage in promotion order, a step cell per
//! column, as a `TableSource` of its own (the page's table is All hops').
//! The Stage cell is pinned, so a matrix wider than the room scrolls
//! sideways with its Stages in view. Its rows are 40 dp, two lines of
//! 16 dp (DESIGN.md, "The Overview").
use std::rc::Rc;

use freshkube_ui::table::{
    Line, RowStyle, SortOrder, Step, TableColumn, TableRow, TableSource, TableState,
};
use gpui_kit::component::{h_flex, v_flex};

use super::cells::{Column, StageRow};
use super::*;
use crate::ui::dp;

/// The id prefix of the matrix: `change-matrix-list`, `change-matrix-row-<stage>`.
pub const MATRIX: &str = "change-matrix";
/// A step column's width, in dp: the widest word and its inset.
const STEP_WIDTH: f32 = 112.;
/// How many frames a selection's row is revealed in.
const REVEAL_FRAMES: usize = 6;
/// The pinned Stage column's width, in dp.
const STAGE_WIDTH: f32 = 220.;

pub struct MatrixColumn {
    kind: Option<Column>,
    label: SharedString,
    width: f32,
}

impl TableColumn for MatrixColumn {
    fn label(&self) -> &SharedString {
        &self.label
    }

    fn width(&self) -> f32 {
        self.width
    }

    fn flexible(&self) -> bool {
        false
    }

    fn pinned(&self) -> bool {
        self.kind.is_none()
    }
}

pub fn columns() -> Vec<MatrixColumn> {
    std::iter::once(MatrixColumn {
        kind: None,
        label: "Stage".into(),
        width: STAGE_WIDTH,
    })
    .chain(Column::ALL.map(|kind| MatrixColumn {
        kind: Some(kind),
        label: kind.label().into(),
        width: STEP_WIDTH,
    }))
    .collect()
}

/// The matrix, an entity beside the page, which hands it the rows when
/// the change changes and the selection when it moves.
pub struct Matrix {
    /// The page, which a pressed cell selects its hop on.
    page: WeakEntity<ChangePage>,
    freight: SharedString,
    rows: Vec<Rc<StageRow>>,
    columns: Vec<MatrixColumn>,
    table: TableState,
    /// The selected Stage's row and, when a cell stands for the selected
    /// hop, its column.
    selected: Option<(SharedString, Option<Column>)>,
    loading: Option<kit::LoadingRows>,
    /// The selected cell's column, to scroll into view once the table has
    /// laid out.
    reveal: Option<Column>,
    /// Frames left to reveal the selected Stage's row in: a stacked
    /// Inspector shrinks the matrix over a few.
    reveal_rows: usize,
}

impl Matrix {
    pub fn new(page: WeakEntity<ChangePage>) -> Self {
        Self {
            page,
            freight: SharedString::default(),
            rows: Vec::new(),
            columns: columns(),
            table: TableState::new(MATRIX),
            selected: None,
            loading: None,
            reveal: None,
            reveal_rows: 0,
        }
    }

    pub fn set(
        &mut self,
        freight: &str,
        rows: &[&StageRow],
        selected: Option<(SharedString, Option<Column>)>,
        loading: Option<kit::LoadingRows>,
    ) {
        self.freight = freight.to_owned().into();
        if self.rows.len() != rows.len()
            || self.rows.iter().zip(rows).any(|(old, new)| **old != **new)
        {
            self.rows = rows.iter().map(|row| Rc::new((*row).clone())).collect();
        }
        if self.selected != selected {
            self.reveal = selected.as_ref().and_then(|(_, column)| *column);
            self.reveal_rows = if selected.is_some() { REVEAL_FRAMES } else { 0 };
        }
        self.selected = selected;
        self.loading = loading;
    }

    /// The cell ids, for tests and the page's key handling.
    pub fn cell_id(stage: &str, column: Column) -> SharedString {
        format!("change-cell-{stage}-{}", column.id()).into()
    }

    /// Selects the hop a pressed cell stands for, on the page.
    fn pick(&mut self, hop: Option<SharedString>, window: &mut Window, cx: &mut Context<Self>) {
        let Some(hop) = hop else { return };
        _ = self.page.update(cx, |page, cx| {
            page.select(&hop, window, cx);
            page.focus(window, cx);
        });
    }

    /// The Stage cell: the rail's node, the name and where it deploys on
    /// the first line, since when or what it still runs on the second.
    fn stage_cell(
        &self,
        row: &Rc<StageRow>,
        column: &MatrixColumn,
        style: &RowStyle,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let p = &style.p;
        let hop = row.cells[0].hop.clone();
        let rail = kit::rail(
            format!("change-rail-{}", row.name),
            row.at,
            row.tone,
            row.rail[0],
            row.rail[1],
            cx,
        );
        let first = h_flex()
            .h(dp(kit::LINE))
            .items_center()
            .gap(dp(8.))
            .min_w_0()
            .child(
                div()
                    .flex_none()
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(p.ink)
                    .child(row.name.clone()),
            )
            .child(
                div()
                    .min_w_0()
                    .truncate()
                    .text_color(p.muted)
                    .child(row.cluster.clone()),
            );
        let second = div()
            .id(SharedString::from(format!(
                "change-stage-{}-since",
                row.name
            )))
            .test_support()
            .h(dp(kit::LINE))
            .flex()
            .items_center()
            .truncate()
            .text_size(dp(11.))
            .text_color(p.muted)
            .child(row.line2.clone());
        kit::cell(column)
            .id(SharedString::from(format!("change-stage-{}", row.name)))
            .test_support()
            .px_0()
            .h_full()
            .flex()
            .items_center()
            .cursor_pointer()
            .on_click(cx.listener(move |this, _, window, cx| this.pick(hop.clone(), window, cx)))
            .child(rail)
            .child(
                v_flex()
                    .min_w_0()
                    .flex_1()
                    .justify_center()
                    .pr(dp(kit::CELL_PAD))
                    .child(first)
                    .child(second),
            )
            .into_any_element()
    }
}

impl Render for Matrix {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if self.reveal_rows > 0 {
            self.reveal_rows -= 1;
            kit::reveal(self, ScrollStrategy::Nearest);
            window.on_next_frame(|window, _| window.refresh());
        }
        if let Some(column) = self.reveal {
            let at = Column::ALL
                .iter()
                .position(|c| *c == column)
                .unwrap_or_default();
            let left = STAGE_WIDTH + at as f32 * STEP_WIDTH;
            if self
                .table
                .reveal_column(left, STEP_WIDTH, STAGE_WIDTH, window)
            {
                self.reveal = None;
            } else {
                // The table hasn't laid out: try again on the next frame.
                window.on_next_frame(|window, _| window.refresh());
            }
        }
        v_flex()
            .id(MATRIX)
            .test_support()
            .size_full()
            .min_w_0()
            .min_h_0()
            .child(kit::data_table(self, window, cx).flex_1().min_h_0())
    }
}

impl TableSource for Matrix {
    type Key = SharedString;
    type Sort = ();
    type Column = MatrixColumn;
    type Row<'a> = Rc<StageRow>;

    fn table_state(&self) -> &TableState {
        &self.table
    }

    fn columns(&self) -> &[MatrixColumn] {
        &self.columns
    }

    fn width(&self) -> f32 {
        self.columns.iter().map(TableColumn::width).sum()
    }

    fn row_height(&self) -> f32 {
        kit::STEP_ROW
    }

    fn list_label(&self) -> String {
        format!(
            "The Stages of Freight {}, in promotion order, and how far it got in each",
            self.freight
        )
    }

    fn sorting(&self, _: &MatrixColumn) -> Option<((), Option<SortOrder>)> {
        None
    }

    fn sort(&mut self, _: (), _: &mut Context<Self>) {}

    fn line_count(&self) -> usize {
        self.rows.len()
    }

    fn line(&self, line: usize, _: &App) -> Option<Line<SharedString, Rc<StageRow>>> {
        let row = self.rows.get(line)?;
        Some(Line::Row(TableRow {
            key: row.name.clone(),
            id: SharedString::from(format!("{MATRIX}-row-{}", row.name)).into(),
            label: format!("Stage {}: {}", row.name, row.line2).into(),
            tooltip: None,
            marked: false,
            muted: false,
            data: row.clone(),
        }))
    }

    fn cell(
        &self,
        line: &TableRow<SharedString, Rc<StageRow>>,
        style: &RowStyle,
        column: &MatrixColumn,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let row = &line.data;
        let Some(kind) = column.kind else {
            return self.stage_cell(row, column, style, cx);
        };
        let at = Column::ALL
            .iter()
            .position(|c| *c == kind)
            .unwrap_or_default();
        let cell = &row.cells[at];
        let next = row.cells.get(at + 1);
        let hop = cell.hop.clone();
        let selected = self
            .selected
            .as_ref()
            .is_some_and(|(stage, column)| *column == Some(kind) && *stage == row.name);
        kit::stepper_cell(
            column,
            Step {
                id: Self::cell_id(&row.name, kind),
                tone: cell.tone,
                word: cell.word.clone(),
                into: cell.track(),
                onward: next.map_or(kit::Track::None, |next| next.track()),
                onward_tone: next.map_or(Tone::Outline, |next| next.tone),
                tooltip: cell.tooltip.clone(),
                selected,
            },
            cx,
        )
        .cursor_pointer()
        .on_click(cx.listener(move |this, _, window, cx| this.pick(hop.clone(), window, cx)))
        .into_any_element()
    }

    fn group(&self, _: usize, _: &mut Context<Self>) -> Option<AnyElement> {
        None
    }

    fn selected_key(&self) -> Option<&SharedString> {
        let (stage, _) = self.selected.as_ref()?;
        self.rows
            .iter()
            .map(|row| &row.name)
            .find(|name| *name == stage)
    }

    fn line_of(&self, key: &SharedString) -> Option<usize> {
        self.rows.iter().position(|row| &row.name == key)
    }

    /// The cells are the buttons; a row on its own does nothing.
    fn clickable(&self) -> bool {
        false
    }

    /// A stacked Inspector leaves the matrix a row or two: the selected
    /// Stage ends whole under the header, never half.
    fn whole_rows(&self) -> bool {
        true
    }

    fn loading(&self) -> Option<&kit::LoadingRows> {
        self.loading.as_ref()
    }

    fn empty(&self, _: &mut Context<Self>) -> Option<AnyElement> {
        self.rows
            .is_empty()
            .then(|| "The project has no Stages.".into_any_element())
    }
}
