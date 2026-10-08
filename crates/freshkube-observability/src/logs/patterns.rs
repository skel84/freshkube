//! The patterns Coroot found in an application's messages, drawn by the
//! shared `DataTable`: each pattern's level, how many messages it matched in
//! the window and one of them. Selecting a pattern shows its messages over
//! time under the table.
use super::super::*;
use freshkube_core::coroot::{self as api, ChartPanel};
use freshkube_core::group_digits;
use freshkube_core::types::LogLevel;
use freshkube_ui::table::{
    self as shared, DataTable, Line, RowStyle, SortOrder, TableColumn, TableRow, TableSource,
    TableState,
};

/// The most rows the table is tall; more scroll inside it.
const MOST_LINES: usize = 12;

/// A pattern, worded when Coroot answers.
pub(crate) struct PatternRow {
    tone: Tone,
    level: SharedString,
    count: SharedString,
    /// The sample's first line; the table shows one line a pattern.
    sample: SharedString,
    /// The whole sample, when it has more than its first line.
    tooltip: Option<SharedString>,
    pub(super) chart: Option<Rc<ChartPanel>>,
}

impl PatternRow {
    pub(super) fn new(pattern: &api::LogPattern) -> Self {
        let first = pattern.sample.lines().next().unwrap_or_default();
        let (tone, level) = match pattern.level {
            LogLevel::Error => (Tone::Crit, "Error"),
            LogLevel::Warning => (Tone::Warn, "Warning"),
            LogLevel::Info => (Tone::Info, "Info"),
            LogLevel::Debug => (Tone::Unknown, "Debug"),
            LogLevel::Unknown => (Tone::Unknown, "Unknown"),
        };
        Self {
            tone,
            level: level.into(),
            count: group_digits(pattern.count).into(),
            tooltip: (first.len() < pattern.sample.len()).then(|| pattern.sample.clone().into()),
            sample: first.to_owned().into(),
            chart: pattern
                .chart
                .as_ref()
                .and_then(ChartPanel::severities)
                .map(Rc::new),
        }
    }
}

#[derive(Clone, Copy, PartialEq)]
enum Kind {
    Glyph,
    Level,
    Count,
    Sample,
}

#[derive(Clone)]
pub(crate) struct Column {
    kind: Kind,
    label: SharedString,
    width: f32,
}

impl TableColumn for Column {
    fn label(&self) -> &SharedString {
        &self.label
    }
    fn width(&self) -> f32 {
        self.width
    }
    fn flexible(&self) -> bool {
        self.kind == Kind::Sample
    }
}

pub(crate) struct PatternTable {
    state: TableState,
    /// The loading rows while the patterns asked for are still to come.
    loading: freshkube_ui::table::TableLoading,
    waiting: bool,
    columns: Vec<Column>,
    width: f32,
    rows: Rc<Vec<PatternRow>>,
    selected: Option<usize>,
    page: WeakEntity<ObservabilityPage>,
}

impl PatternTable {
    pub(super) fn new(page: WeakEntity<ObservabilityPage>, cx: &mut App) -> Self {
        let columns = columns(&[]);
        Self {
            state: TableState::new("obs-log-patterns"),
            loading: freshkube_ui::table::TableLoading::new("obs-log-patterns", cx),
            waiting: false,
            width: columns.iter().map(|c| c.width).sum(),
            columns,
            rows: Rc::default(),
            selected: None,
            page,
        }
    }

    /// Coroot's patterns, in its order: the most frequent first.
    pub(super) fn set(&mut self, rows: Rc<Vec<PatternRow>>, cx: &mut Context<Self>) {
        self.columns = columns(&rows);
        self.width = self.columns.iter().map(|c| c.width).sum();
        self.selected = self.selected.filter(|&ix| ix < rows.len());
        self.rows = rows;
        cx.notify();
    }

    /// Whether the table waits for the patterns the query asked for, and
    /// shows its loading rows meanwhile.
    pub(super) fn set_waiting(&mut self, waiting: bool, cx: &mut Context<Self>) {
        if self.waiting != waiting {
            self.waiting = waiting;
            cx.notify();
        }
    }

    /// The motion over the loading rows, while the table waits.
    pub(super) fn loading_motion(&self) -> Option<Entity<freshkube_ui::table::LoadingMotion>> {
        self.loading.motion(self.waiting)
    }

    pub(super) fn rows(&self) -> &Rc<Vec<PatternRow>> {
        &self.rows
    }

    pub(super) fn selected(&self) -> Option<usize> {
        self.selected
    }
}

/// The columns for these patterns: fixed, fitted to their texts.
fn columns(rows: &[PatternRow]) -> Vec<Column> {
    let column = |kind, label: &str, texts: Vec<&SharedString>, most| Column {
        kind,
        width: shared::fit(label, texts.into_iter(), most),
        label: label.to_owned().into(),
    };
    vec![
        Column {
            kind: Kind::Glyph,
            label: SharedString::default(),
            width: shared::GLYPH_WIDTH,
        },
        column(
            Kind::Level,
            "Level",
            rows.iter().map(|r| &r.level).collect(),
            shared::WIDEST,
        ),
        column(
            Kind::Count,
            "Messages",
            rows.iter().map(|r| &r.count).collect(),
            shared::WIDEST,
        ),
        column(
            Kind::Sample,
            "Pattern",
            rows.iter().map(|r| &r.sample).collect(),
            shared::WIDEST_FLEXIBLE,
        ),
    ]
}

/// The loading rows a table of patterns shows.
const LOADING_LINES: usize = 6;

impl Render for PatternTable {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.loading.show(self.waiting);
        let lines = if self.waiting {
            LOADING_LINES
        } else {
            self.rows.len().clamp(1, MOST_LINES)
        };
        DataTable::new()
            .carded()
            .fit(lines)
            .render(self, window, cx)
    }
}

impl TableSource for PatternTable {
    type Key = usize;
    type Sort = ();
    type Column = Column;
    type Row<'a> = &'a PatternRow;

    fn table_state(&self) -> &TableState {
        &self.state
    }
    fn columns(&self) -> &[Column] {
        &self.columns
    }
    fn width(&self) -> f32 {
        self.width
    }
    fn list_label(&self) -> String {
        "Log patterns".into()
    }
    fn sorting(&self, _: &Column) -> Option<((), Option<SortOrder>)> {
        None
    }
    fn sort(&mut self, _: (), _: &mut Context<Self>) {}
    fn line_count(&self) -> usize {
        self.rows.len()
    }
    fn line(&self, line: usize, _: &App) -> Option<Line<usize, &PatternRow>> {
        let row = self.rows.get(line)?;
        Some(Line::Row(TableRow {
            key: line,
            // Coroot's order names a pattern within one answer.
            id: self.state.id(&format!("row-{line}")).into(),
            label: row.sample.clone(),
            tooltip: row.tooltip.clone(),
            marked: false,
            muted: false,
            data: row,
        }))
    }
    fn selected_key(&self) -> Option<&usize> {
        self.selected.as_ref()
    }
    fn line_of(&self, key: &usize) -> Option<usize> {
        (*key < self.rows.len()).then_some(*key)
    }
    fn click(&mut self, key: &usize, _: &ClickEvent, _: &mut Window, cx: &mut Context<Self>) {
        self.selected = (self.selected != Some(*key)).then_some(*key);
        let selected = self.selected;
        cx.notify();
        let page = self.page.clone();
        cx.defer(move |cx| {
            _ = page.update(cx, |page, cx| page.show_pattern(selected, cx));
        });
    }
    fn cell(
        &self,
        row: &TableRow<usize, &PatternRow>,
        _: &RowStyle,
        column: &Column,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let frame = shared::cell(column).h_full().flex().items_center();
        let pattern = row.data;
        match column.kind {
            Kind::Glyph => shared::glyph_cell(column)
                .children(ui::status_glyph(pattern.tone, cx))
                .into_any_element(),
            Kind::Level => frame.child(text(pattern.level.clone())).into_any_element(),
            Kind::Count => frame.child(mono(pattern.count.clone())).into_any_element(),
            Kind::Sample => frame
                .child(
                    div()
                        .min_w_0()
                        .truncate()
                        .font_family(MONO_FONT)
                        .text_size(dp(12.))
                        .child(pattern.sample.clone()),
                )
                .into_any_element(),
        }
    }
    fn group(&self, _: usize, _: &mut Context<Self>) -> Option<AnyElement> {
        None
    }
    fn loading(&self) -> Option<&freshkube_ui::table::LoadingRows> {
        self.loading.rows()
    }
    fn empty(&self, _: &mut Context<Self>) -> Option<AnyElement> {
        self.rows
            .is_empty()
            .then(|| "Coroot found no patterns in this window.".into_any_element())
    }
}
