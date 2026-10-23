//! The list as a `TableSource`: one row for each entry, newest first, with
//! a row where a source's entries stop and one for each stretch of quiet
//! hours. The rows are derived when an answer lands.
use freshkube_delivery::timeline::{Row, Severity, Source, Timeline, clock, clock_at, span_words};
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
    let now = timeline.now;
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
                    time: clock_at(entry.at, now).into(),
                    source: entry.source.label().into(),
                    what: entry.title.clone().into(),
                    detail: entry.detail.clone().into(),
                    label: format!(
                        "{} {}: {}. {}. {read_from}",
                        clock_at(entry.at, now),
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
                    clock_at(*from, now),
                    clock_at(*until, now)
                );
                TimelineRow {
                    key: format!("not-kept-{}", source.slug()).into(),
                    tone: Tone::Unknown,
                    time: format!("before {}", clock_at(*until, now)).into(),
                    source: source.label().into(),
                    what: "Not kept by the cluster".into(),
                    label: format!("{} not kept by the cluster before {}. {detail}", source.label(), clock_at(*until, now)).into(),
                    detail: detail.into(),
                    read_from: source.origin().into(),
                    note: true,
                }
            }
            Row::Quiet {
                from,
                until,
                read,
                state_only,
                not_read,
            } => {
                let hours = (until - from + 1799) / 3600;
                let mut detail = format!("Read: {}", names(read));
                if !state_only.is_empty() {
                    detail.push_str(&format!(" · Latest state only: {}", names(state_only)));
                }
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
                    time: format!("{}–{}", clock_at(*from, now), clock(*until)).into(),
                    source: SharedString::default(),
                    // "Quiet" only where every source was read; otherwise the
                    // words say what was looked at, not that nothing happened.
                    what: if not_read.is_empty() {
                        format!("Quiet for {hours} h")
                    } else {
                        "No events found in what was read".to_owned()
                    }
                    .into(),
                    label: format!(
                        "{} {}–{}. {detail}",
                        if not_read.is_empty() { "Quiet" } else { "No events found in what was read" },
                        clock_at(*from, now),
                        clock(*until)
                    )
                    .into(),
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
        self.rows
            .is_empty()
            .then(|| empty_words(&self.chips).into_any_element())
    }
}

/// What an empty list says, from what the chips say was read.
fn empty_words(chips: &[TimelineChip]) -> &'static str {
    if chips.is_empty() {
        "Nothing read yet."
    } else if chips.iter().any(|chip| chip.read) {
        // Some source answered and found nothing it can vouch for.
        "Nothing was reported in this window. The chips say what was read."
    } else {
        "No source could be read for this window; the chips say why."
    }
}

#[cfg(test)]
mod tests {
    use super::{TimelineChip, empty_words};
    use freshkube_ui::ui::Tone;

    fn chip(read: bool) -> TimelineChip {
        TimelineChip {
            text: "Events".into(),
            tone: Tone::Outline,
            label: "Events".into(),
            read,
            id: "timeline-chip-events".into(),
        }
    }

    #[test]
    fn a_partly_read_hour_is_not_called_quiet() {
        use freshkube_delivery::timeline::{self, example};
        let now = 1_800_000_000;
        let words = |shape| -> Vec<String> {
            let timeline = timeline::derive(&example::reads(now, shape), now, 3);
            super::rows(&timeline)
                .into_iter()
                .filter(|row| {
                    row.note && row.what.contains("Quiet") || row.what.starts_with("No events")
                })
                .map(|row| row.what.to_string())
                .collect()
        };
        // Syncs and Incidents weren't read: nothing may say quiet.
        let partial = words(example::Shape::Partial);
        assert!(!partial.is_empty());
        assert!(
            partial
                .iter()
                .all(|what| what == "No events found in what was read"),
            "{partial:?}"
        );
        // The older hours of `quiet` lack Events, which the cluster no longer keeps.
        let quiet = words(example::Shape::Quiet);
        assert_eq!(quiet, ["Quiet for 1 h", "No events found in what was read"]);
    }

    #[test]
    fn a_row_across_midnight_names_the_day() {
        use freshkube_delivery::timeline::{self, example};
        // 03:00 UTC: the 3 h window starts at 00:00, the quiet hours before
        // the older retention are on the day before when the window is 24 h.
        let now = 1_800_000_000 / 86_400 * 86_400 + 3 * 3600;
        let timeline = timeline::derive(&example::reads(now, example::Shape::Quiet), now, 24);
        let times: Vec<_> = super::rows(&timeline)
            .into_iter()
            .map(|row| row.time.to_string())
            .collect();
        assert!(
            times.iter().any(|time| time.starts_with("yesterday ")),
            "{times:?}"
        );
    }

    #[test]
    fn an_empty_list_is_worded_from_what_was_read() {
        assert_eq!(empty_words(&[]), "Nothing read yet.");
        assert!(empty_words(&[chip(false), chip(false)]).starts_with("No source could be read"));
        assert!(empty_words(&[chip(false), chip(true)]).starts_with("Nothing was reported"));
    }
}
