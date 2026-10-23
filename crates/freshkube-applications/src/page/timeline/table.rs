//! The list as a `TableSource`: one row for each entry, newest first, with
//! a row where a source's entries stop and one for each stretch of quiet
//! hours. The rows are derived when an answer lands.
use freshkube_delivery::timeline::{Row, Severity, Source, Timeline, clock, span_words};
use freshkube_ui::table::{Line, RowStyle, SortOrder, TableColumn, TableRow, TableSource};
use freshkube_ui::ui::Tone;

use super::*;
use crate::ui;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Field {
    Glyph,
    Time,
    Source,
    What,
    Detail,
    ReadFrom,
}

pub struct Column {
    field: Field,
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
        self.field == Field::ReadFrom
    }

    /// The glyph and the time stay in view when the table scrolls sideways.
    fn pinned(&self) -> bool {
        matches!(self.field, Field::Glyph | Field::Time)
    }
}

/// DESIGN.md's widths: 7.5 a character plus 24, between 64 and `most`.
fn fit<'a>(label: &str, most: f32, texts: impl Iterator<Item = &'a SharedString>) -> f32 {
    let chars = texts
        .map(|text| text.chars().count())
        .chain([label.len()])
        .max()
        .unwrap_or_default();
    (chars as f32 * 7.5 + 24.).clamp(64., most)
}

/// The columns for these rows, and their total width.
pub fn columns(rows: &[TimelineRow]) -> (Vec<Column>, f32) {
    let column = |field, label: &str, width| Column {
        field,
        label: label.to_owned().into(),
        width,
    };
    let columns = vec![
        column(Field::Glyph, "", kit::GLYPH_WIDTH),
        column(
            Field::Time,
            "Time (UTC)",
            fit("Time (UTC)", 160., rows.iter().map(|row| &row.time)),
        ),
        column(
            Field::Source,
            "Source",
            fit("Source", 120., rows.iter().map(|row| &row.source)),
        ),
        column(
            Field::What,
            "What",
            fit("What", 300., rows.iter().map(|row| &row.what)),
        ),
        column(
            Field::Detail,
            "Detail",
            fit("Detail", 460., rows.iter().map(|row| &row.detail)),
        ),
        column(
            Field::ReadFrom,
            "Read from",
            fit("Read from", 360., rows.iter().map(|row| &row.read_from)),
        ),
    ];
    let width = columns.iter().map(|column| column.width).sum();
    (columns, width)
}

fn tone(severity: Severity) -> Tone {
    match severity {
        Severity::Info => Tone::Info,
        Severity::Warn => Tone::Warn,
        Severity::Bad => Tone::Crit,
    }
}

fn names(sources: &[Source]) -> String {
    sources
        .iter()
        .map(|source| source.label())
        .collect::<Vec<_>>()
        .join(", ")
}

/// The list's rows for a derived timeline.
pub fn rows(timeline: &Timeline) -> Vec<TimelineRow> {
    timeline
        .rows
        .iter()
        .map(|row| match row {
            Row::Entry(entry) => {
                let read_from = format!(
                    "{} · {} · read {} UTC",
                    entry.source.origin(),
                    entry.read_from,
                    clock(entry.read_at)
                );
                TimelineRow {
                    key: entry.id.clone().into(),
                    tone: tone(entry.severity),
                    time: clock(entry.at).into(),
                    source: entry.source.label().into(),
                    what: entry.title.clone().into(),
                    detail: entry.detail.clone().into(),
                    label: format!(
                        "{} {}: {}. {}. {read_from}",
                        clock(entry.at),
                        entry.source.label(),
                        entry.title,
                        entry.detail
                    )
                    .into(),
                    read_from: read_from.into(),
                    note: false,
                }
            }
            Row::NotKept {
                source,
                from,
                until,
            } => {
                let kept = span_words(timeline.now - until);
                let detail = format!(
                    "The cluster keeps {} for about {kept}; nothing is known of them from {} to {}.",
                    source.label(),
                    clock(*from),
                    clock(*until)
                );
                TimelineRow {
                    key: format!("not-kept-{}", source.slug()).into(),
                    tone: Tone::Unknown,
                    time: format!("before {}", clock(*until)).into(),
                    source: source.label().into(),
                    what: "Not kept by the cluster".into(),
                    label: format!("{} not kept by the cluster before {}. {detail}", source.label(), clock(*until)).into(),
                    detail: detail.into(),
                    read_from: source.origin().into(),
                    note: true,
                }
            }
            Row::Quiet {
                from,
                until,
                read,
                not_read,
            } => {
                let hours = (until - from + 1799) / 3600;
                let mut detail = format!("Read: {}", names(read));
                if !not_read.is_empty() {
                    detail.push_str(&format!(" · Not read: {}", names(not_read)));
                }
                TimelineRow {
                    key: format!("quiet-{from}").into(),
                    tone: if not_read.is_empty() {
                        Tone::Good
                    } else {
                        Tone::Unknown
                    },
                    time: format!("{}–{}", clock(*from), clock(*until)).into(),
                    source: SharedString::default(),
                    what: format!("Quiet for {hours} h").into(),
                    label: format!("Quiet {}–{}. {detail}", clock(*from), clock(*until)).into(),
                    detail: detail.into(),
                    read_from: SharedString::default(),
                    note: true,
                }
            }
        })
        .collect()
}

impl TableSource for TimelineView {
    type Key = SharedString;
    type Sort = ();
    type Column = Column;
    type Row<'a> = &'a TimelineRow;

    fn table_state(&self) -> &TableState {
        &self.table
    }

    fn columns(&self) -> &[Column] {
        &self.columns
    }

    fn width(&self) -> f32 {
        self.width
    }

    fn list_label(&self) -> String {
        format!(
            "What happened to {} in the last {} h, newest first",
            self.app, self.hours
        )
    }

    fn sorting(&self, _: &Column) -> Option<((), Option<SortOrder>)> {
        None
    }

    fn sort(&mut self, _: (), _: &mut Context<Self>) {}

    fn line_count(&self) -> usize {
        self.rows.len()
    }

    fn line(&self, line: usize, _: &App) -> Option<Line<SharedString, &TimelineRow>> {
        let row = self.rows.get(line)?;
        Some(Line::Row(TableRow {
            key: row.key.clone(),
            id: SharedString::from(format!("timeline-row-{}", row.key)).into(),
            label: row.label.clone(),
            tooltip: Some(row.label.clone()),
            marked: false,
            muted: row.note,
            data: row,
        }))
    }

    fn cell(
        &self,
        line: &TableRow<SharedString, &TimelineRow>,
        style: &RowStyle,
        column: &Column,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let row = line.data;
        let cell = kit::cell(column);
        match column.field {
            Field::Glyph => kit::glyph_cell(column)
                .child(ui::status_mark(
                    SharedString::from(format!("timeline-row-{}-mark", row.key)),
                    row.tone,
                    if row.note {
                        row.what.clone()
                    } else {
                        row.source.clone()
                    },
                    cx,
                ))
                .into_any_element(),
            Field::Time => cell.child(row.time.clone()).into_any_element(),
            Field::Source => cell.child(row.source.clone()).into_any_element(),
            Field::What => cell.child(row.what.clone()).into_any_element(),
            Field::Detail => cell
                .text_color(style.p.muted)
                .child(row.detail.clone())
                .into_any_element(),
            Field::ReadFrom => cell
                .text_color(style.p.muted)
                .child(row.read_from.clone())
                .into_any_element(),
        }
    }

    fn group(&self, _: usize, _: &mut Context<Self>) -> Option<AnyElement> {
        None
    }

    /// Rows only read: there is nothing to select, open or act on.
    fn clickable(&self) -> bool {
        false
    }

    fn loading(&self) -> Option<&kit::LoadingRows> {
        (self.loader.is_loading() && self.loader.data().is_none()).then_some(&self.loading)
    }

    fn empty(&self, _: &mut Context<Self>) -> Option<AnyElement> {
        (self.rows.is_empty()).then(|| {
            if self.loader.data().is_some() {
                "No source could be read for this window; the chips say why.".into_any_element()
            } else {
                "Nothing read yet.".into_any_element()
            }
        })
    }
}
