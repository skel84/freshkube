//! A report's table, drawn by the shared `DataTable`. Each table is an
//! entity of its own: a report can show several, and the table reads its
//! source again while its rows lay out, after the page's render returns.
use super::super::*;
use freshkube_core::coroot as api;
use freshkube_ui::table::{
    self as shared, DataTable, Line, RowStyle, SortOrder, TableColumn, TableRow, TableSource,
    TableState,
};

/// The most rows a table is tall; longer ones scroll inside it.
const MOST_LINES: usize = 16;

/// A table as Coroot sent it, measured and worded when it arrives.
pub(in crate::observability) struct Prepared {
    columns: Vec<Column>,
    width: f32,
    rows: Vec<Row>,
}

#[derive(Clone)]
pub(in crate::observability) struct Column {
    /// Its place in Coroot's rows.
    index: usize,
    label: SharedString,
    width: f32,
    flexible: bool,
}

impl TableColumn for Column {
    fn label(&self) -> &SharedString {
        &self.label
    }
    fn width(&self) -> f32 {
        self.width
    }
    fn flexible(&self) -> bool {
        self.flexible
    }
}

pub(in crate::observability) struct Row {
    label: SharedString,
    cells: Vec<Cell>,
}

struct Cell {
    text: SharedString,
    /// The unit and tags, muted after the text.
    note: SharedString,
    status: Option<Status>,
    /// An application this cell opens.
    link: Option<api::AppId>,
    progress: Option<(f32, Tone)>,
    /// The whole cell, when its column is too narrow for it.
    tooltip: Option<SharedString>,
}

fn cell(raw: &api::Cell) -> Cell {
    let text = if !raw.deployments.is_empty() {
        raw.deployments
            .iter()
            .map(|d| format!("{}: {}", d.report, d.message))
            .collect::<Vec<_>>()
            .join(" · ")
    } else if !raw.values.is_empty() {
        raw.values.join(" · ")
    } else if let Some((rx, tx)) = &raw.bandwidth {
        format!("↓ {rx} ↑ {tx}")
    } else if let Some((percent, _)) = &raw.progress {
        format!("{percent}%")
    } else {
        raw.value.clone()
    };
    // A missing figure ("—" for a refused connection) has no unit to read.
    let unit = if is_missing(&text) {
        ""
    } else {
        raw.unit.as_str()
    };
    let note = [unit]
        .into_iter()
        .chain(raw.tags.iter().map(String::as_str))
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join(" ");
    let status = raw.status.map(Status::from).or_else(|| {
        (!raw.deployments.is_empty()).then(|| {
            if raw.deployments.iter().all(|d| d.ok) {
                Status::Ok
            } else {
                Status::Warning
            }
        })
    });
    Cell {
        link: raw
            .link
            .as_ref()
            .filter(|l| l.view == "applications" && !l.id.is_empty())
            .map(|l| api::AppId::new(l.id.clone())),
        progress: raw.progress.as_ref().map(|(percent, color)| {
            let tone = match color.as_str() {
                "red" => Tone::Crit,
                "orange" | "yellow" => Tone::Warn,
                _ => Tone::Info,
            };
            (f32::from(*percent) / 100., tone)
        }),
        text: text.into(),
        note: note.into(),
        status,
        tooltip: None,
    }
}

/// Whether a cell's text stands for no figure at all.
fn is_missing(text: &str) -> bool {
    matches!(text.trim(), "" | "—" | "-" | "–")
}

/// The characters a cell shows, as the column's width counts them.
fn shown(cell: &Cell) -> SharedString {
    let glyph = if cell.status.is_some() { "  " } else { "" };
    if cell.note.is_empty() {
        format!("{glyph}{}", cell.text).into()
    } else {
        format!("{glyph}{} {}", cell.text, cell.note).into()
    }
}

impl Prepared {
    pub(in crate::observability) fn new(table: &api::Table) -> Self {
        let mut rows: Vec<Row> = table
            .rows
            .iter()
            .map(|raw| {
                let cells: Vec<Cell> = raw.iter().map(cell).collect();
                Row {
                    label: cells.first().map(|c| c.text.clone()).unwrap_or_default(),
                    cells,
                }
            })
            .collect();
        let count = table
            .header
            .len()
            .max(rows.iter().map(|r| r.cells.len()).max().unwrap_or(0));
        let columns: Vec<Column> = (0..count)
            .map(|ix| {
                let label: SharedString = table.header.get(ix).cloned().unwrap_or_default().into();
                let texts: Vec<SharedString> = rows
                    .iter()
                    .filter_map(|r| r.cells.get(ix))
                    .map(shown)
                    .collect();
                let flexible = ix == 0;
                let most = if flexible {
                    shared::WIDEST_FLEXIBLE
                } else {
                    shared::WIDEST
                };
                Column {
                    index: ix,
                    width: shared::fit(&label, texts.iter(), most),
                    label,
                    flexible,
                }
            })
            .collect();
        for row in &mut rows {
            for (cell, column) in row.cells.iter_mut().zip(&columns) {
                let text = shown(cell);
                if text.chars().count() as f32 * 7.5 + 24. > column.width {
                    cell.tooltip = Some(text.trim_start().to_string().into());
                }
            }
        }
        Self {
            width: columns.iter().map(|c| c.width).sum(),
            columns,
            rows,
        }
    }
}

pub(in crate::observability) struct ReportTable {
    state: TableState,
    table: Rc<Prepared>,
    page: WeakEntity<ObservabilityPage>,
}

impl ReportTable {
    pub(in crate::observability) fn new(
        prefix: &str,
        table: Rc<Prepared>,
        page: WeakEntity<ObservabilityPage>,
    ) -> Self {
        Self {
            state: TableState::new(prefix),
            table,
            page,
        }
    }

    pub(in crate::observability) fn table(&self) -> &Rc<Prepared> {
        &self.table
    }

    pub(in crate::observability) fn set(&mut self, table: Rc<Prepared>, cx: &mut Context<Self>) {
        self.table = table;
        cx.notify();
    }
}

impl Render for ReportTable {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        DataTable::new()
            .carded()
            .fit(self.table.rows.len().min(MOST_LINES))
            .render(self, window, cx)
    }
}

impl TableSource for ReportTable {
    type Key = usize;
    type Sort = ();
    type Column = Column;
    type Row<'a> = &'a Row;

    fn table_state(&self) -> &TableState {
        &self.state
    }
    fn columns(&self) -> &[Column] {
        &self.table.columns
    }
    fn width(&self) -> f32 {
        self.table.width
    }
    fn list_label(&self) -> String {
        let first = self.table.columns.first().map_or("", |c| c.label.as_ref());
        format!("{first} table")
    }
    fn sorting(&self, _: &Column) -> Option<((), Option<SortOrder>)> {
        None
    }
    fn sort(&mut self, _: (), _: &mut Context<Self>) {}
    fn line_count(&self) -> usize {
        self.table.rows.len()
    }
    fn line(&self, line: usize, _: &App) -> Option<Line<usize, &Row>> {
        let row = self.table.rows.get(line)?;
        Some(Line::Row(TableRow {
            key: line,
            // Rows don't select, so their position names them.
            id: self.state.id(&format!("row-{line}")).into(),
            label: row.label.clone(),
            tooltip: None,
            marked: false,
            muted: false,
            data: row,
        }))
    }
    fn clickable(&self) -> bool {
        false
    }
    fn cell(
        &self,
        row: &TableRow<usize, &Row>,
        style: &RowStyle,
        column: &Column,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let p = &style.p;
        let ix = column.index;
        let frame = shared::cell(column)
            .h_full()
            .flex()
            .items_center()
            .gap(dp(6.));
        let Some(cell) = row.data.cells.get(ix) else {
            return frame.into_any_element();
        };
        let text = div()
            .min_w_0()
            .truncate()
            .font_family(MONO_FONT)
            .text_size(dp(12.))
            .child(cell.text.clone());
        let text = match &cell.link {
            Some(app) => {
                let app = app.clone();
                let page = self.page.clone();
                text.id(self.state.id(&format!("link-{}-{ix}", row.key)))
                    .test_support()
                    .text_color(p.accent)
                    .cursor_pointer()
                    .on_click(cx.listener(move |_, _, _, cx| {
                        let app = app.clone();
                        _ = page.update(cx, |page, cx| page.open_linked_app(app, cx));
                    }))
                    .into_any_element()
            }
            None => text.into_any_element(),
        };
        let frame = frame
            .children(cell.status.map(|s| status(s, cx)))
            .children(cell.progress.map(|(fraction, tone)| {
                div()
                    .w(dp(44.))
                    .flex_none()
                    .child(freshkube_ui::card::gauge(fraction, Some(tone), cx))
            }))
            .child(text)
            .when(!cell.note.is_empty(), |this| {
                this.child(
                    div()
                        .flex_none()
                        .text_size(dp(11.5))
                        .text_color(p.muted)
                        .child(cell.note.clone()),
                )
            });
        match cell.tooltip.clone() {
            Some(tip) => frame
                .id(self.state.id(&format!("cell-{}-{ix}", row.key)))
                .tooltip(move |window, cx| Tooltip::new(tip.clone()).build(window, cx))
                .into_any_element(),
            None => frame.into_any_element(),
        }
    }
    fn group(&self, _: usize, _: &mut Context<Self>) -> Option<AnyElement> {
        None
    }
    fn empty(&self, _: &mut Context<Self>) -> Option<AnyElement> {
        self.table
            .rows
            .is_empty()
            .then(|| "Coroot sent this table without rows.".into_any_element())
    }
}

#[cfg(test)]
mod tests {
    use super::{api, cell};

    fn rtt(value: &str) -> api::Cell {
        api::Cell {
            value: value.into(),
            unit: "ms".into(),
            ..Default::default()
        }
    }

    #[test]
    fn a_missing_figure_reads_alone_without_its_unit() {
        for missing in ["—", "-", ""] {
            let cell = cell(&rtt(missing));
            assert!(cell.note.is_empty(), "{missing:?} kept {:?}", cell.note);
        }
        let measured = cell(&rtt("0.4"));
        assert_eq!(measured.text, "0.4");
        assert_eq!(measured.note, "ms");
        let tagged = cell(&api::Cell {
            tags: vec!["refused".into()],
            ..rtt("—")
        });
        assert_eq!(tagged.note, "refused");
    }
}
