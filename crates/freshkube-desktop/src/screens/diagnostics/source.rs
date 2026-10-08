//! The checks as a `TableSource`: their rows, grouped by category, and the
//! tallies and meta parts, derived when the snapshot or the status filter
//! changes, and each row's cells.
use super::*;
use freshkube_core::pluralize;
use table::{Line, RowStyle, SortOrder, TableColumn, TableRow, TableSource, WIDEST_FLEXIBLE, fit};

/// A status as the header's chips count it; Checking counts as unknown.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Tally {
    Failing,
    Warnings,
    Unknown,
    Passing,
}

/// The chips' order: critical, warning, waiting, OK.
pub(super) const TALLIES: [Tally; 4] = [
    Tally::Failing,
    Tally::Warnings,
    Tally::Unknown,
    Tally::Passing,
];

impl Tally {
    pub(super) fn of(status: &CheckStatus) -> Self {
        match status {
            CheckStatus::Fail => Self::Failing,
            CheckStatus::Warn => Self::Warnings,
            CheckStatus::Unknown | CheckStatus::Checking => Self::Unknown,
            CheckStatus::Pass => Self::Passing,
        }
    }

    pub(super) fn tone(self) -> Tone {
        match self {
            Self::Failing => Tone::Crit,
            Self::Warnings => Tone::Warn,
            Self::Unknown => Tone::Unknown,
            Self::Passing => Tone::Good,
        }
    }

    /// The chip's word: `3 failing`, `2 warnings`.
    pub(super) fn what(self) -> &'static str {
        match self {
            Self::Failing => "failing",
            Self::Warnings => "warnings",
            Self::Unknown => "unknown",
            Self::Passing => "passing",
        }
    }

    fn index(self) -> usize {
        TALLIES.iter().position(|tally| *tally == self).unwrap_or(0)
    }
}

/// Which checks the table lists: all, the problems (`P`), or one chip's.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) enum Shown {
    #[default]
    All,
    Problems,
    Only(Tally),
}

impl Shown {
    pub(super) fn shows(self, status: &CheckStatus) -> bool {
        match self {
            Self::All => true,
            Self::Problems => matches!(status, CheckStatus::Warn | CheckStatus::Fail),
            Self::Only(tally) => Tally::of(status) == tally,
        }
    }

    /// Whether a chip shows as chosen: its own filter, or both problem
    /// chips while `P` shows the problems.
    pub(super) fn chosen(self, tally: Tally) -> bool {
        match self {
            Self::All => false,
            Self::Problems => matches!(tally, Tally::Failing | Tally::Warnings),
            Self::Only(only) => only == tally,
        }
    }

    /// Pressing a chip filters to it; pressing it again clears.
    pub(super) fn press(self, tally: Tally) -> Self {
        if self == Self::Only(tally) {
            Self::All
        } else {
            Self::Only(tally)
        }
    }

    pub(super) fn toggle_problems(self) -> Self {
        if self == Self::Problems {
            Self::All
        } else {
            Self::Problems
        }
    }

    fn empty(self) -> &'static str {
        match self {
            Self::All => "The diagnostics reported nothing.",
            Self::Problems => "No warnings or failures.",
            Self::Only(Tally::Failing) => "No checks are failing.",
            Self::Only(Tally::Warnings) => "No checks have warnings.",
            Self::Only(Tally::Unknown) => "No checks are unknown.",
            Self::Only(Tally::Passing) => "No checks pass.",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Field {
    Glyph,
    Check,
    Result,
}

#[derive(Debug)]
pub(crate) struct Column {
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
        self.field == Field::Result
    }

    /// The glyph and the check stay in view when the table scrolls sideways.
    fn pinned(&self) -> bool {
        matches!(self.field, Field::Glyph | Field::Check)
    }
}

/// The Check column's widest; a longer name truncates into the row's
/// tooltip.
const CHECK_WIDTH: f32 = 200.;
/// The Result column's least width. It truncates there rather than push
/// the table wider than the list, and the row's tooltip and the details
/// hold the whole result.
const RESULT_WIDTH: f32 = 160.;

/// A check and what its row shows.
#[derive(Debug)]
pub(crate) struct CheckRow {
    pub(super) key: String,
    /// The row's place among the listed checks: `(diagnostic-check, n)`.
    index: usize,
    label: SharedString,
    name: SharedString,
    message: SharedString,
    tone: Tone,
    status: &'static str,
}

impl CheckRow {
    fn new(index: usize, check: &DiagnosticCheck) -> Self {
        let (tone, status) = status_tone(&check.status);
        Self {
            key: key(check),
            index,
            label: format!("{} · {status} · {}", check.name, check.message).into(),
            name: check.name.clone().into(),
            message: check.message.clone().into(),
            tone,
            status,
        }
    }
}

/// A category's header line.
#[derive(Debug)]
struct Group {
    category: CheckCategory,
    title: String,
    tone: Tone,
    detail: Vec<String>,
}

#[derive(Debug)]
enum Entry {
    Group(usize),
    Row(usize),
}

/// What the table, the chips and the meta line show for one snapshot and
/// one filter.
pub(super) struct Derived {
    revision: u64,
    shown: Shown,
    rows: Vec<CheckRow>,
    lines: Vec<Entry>,
    groups: Vec<Group>,
    columns: Vec<Column>,
    width: f32,
    /// Every check's tally, whatever the filter, in [`TALLIES`] order.
    pub(super) counts: [usize; 4],
    /// The CNI and the addons, for the meta line.
    pub(super) meta: Vec<SharedString>,
}

impl Derived {
    fn new(revision: u64, snapshot: &DiagnosticSnapshot, shown: Shown) -> Self {
        crate::desktop::probe::hit("diagnostics.rows");
        let mut counts = [0; 4];
        for check in &snapshot.checks {
            counts[Tally::of(&check.status).index()] += 1;
        }
        let checks = visible_checks(snapshot, shown);
        let rows: Vec<CheckRow> = checks
            .iter()
            .enumerate()
            .map(|(index, check)| CheckRow::new(index, check))
            .collect();
        let mut lines = Vec::new();
        let mut groups: Vec<Group> = Vec::new();
        for (row, check) in checks.iter().enumerate() {
            if groups.last().map(|group| group.category) != Some(check.category) {
                let category = check.category;
                let members: Vec<&&DiagnosticCheck> = checks
                    .iter()
                    .filter(|check| check.category == category)
                    .collect();
                lines.push(Entry::Group(groups.len()));
                groups.push(Group {
                    category,
                    title: section_title(category, snapshot),
                    tone: worst(members.iter().map(|check| &check.status)),
                    detail: group_detail(members.iter().map(|check| &check.status)),
                });
            }
            lines.push(Entry::Row(row));
        }
        let columns = columns(&rows);
        let width = columns.iter().map(|column| column.width).sum();
        let cni = snapshot
            .cni
            .cni_type
            .as_ref()
            .map_or("unknown", CniType::name);
        let meta = vec![
            format!("CNI {cni}").into(),
            format!("addons {}", addons_label(&snapshot.addons)).into(),
        ];
        Self {
            revision,
            shown,
            rows,
            lines,
            groups,
            columns,
            width,
            counts,
            meta,
        }
    }
}

/// A group's tone: its worst check's.
fn worst<'a>(statuses: impl Iterator<Item = &'a CheckStatus>) -> Tone {
    let tallies: Vec<Tally> = statuses.map(Tally::of).collect();
    TALLIES
        .iter()
        .find(|tally| tallies.contains(tally))
        .map_or(Tone::Unknown, |tally| tally.tone())
}

/// `5 checks · 1 failing · 2 warnings`: the count, then any problems.
fn group_detail<'a>(statuses: impl Iterator<Item = &'a CheckStatus>) -> Vec<String> {
    let tallies: Vec<Tally> = statuses.map(Tally::of).collect();
    let count = |tally| tallies.iter().filter(|each| **each == tally).count();
    let mut detail = vec![pluralize(tallies.len(), "check", "checks")];
    let (failing, warnings) = (count(Tally::Failing), count(Tally::Warnings));
    if failing > 0 {
        detail.push(format!("{failing} failing"));
    }
    if warnings > 0 {
        detail.push(pluralize(warnings, "warning", "warnings"));
    }
    detail
}

fn columns(rows: &[CheckRow]) -> Vec<Column> {
    let column = |field, label: &str, width| Column {
        field,
        label: label.to_owned().into(),
        width,
    };
    vec![
        column(Field::Glyph, "", table::GLYPH_WIDTH),
        column(
            Field::Check,
            "Check",
            fit("Check", rows.iter().map(|row| &row.name), CHECK_WIDTH),
        ),
        column(
            Field::Result,
            "Result",
            fit(
                "Result",
                rows.iter().map(|row| &row.message),
                WIDEST_FLEXIBLE,
            )
            .min(RESULT_WIDTH),
        ),
    ]
}

impl DiagnosticsScreen {
    /// Derives the rows again when the snapshot or the filter changed since
    /// they last were; render and the keys call it before reading them.
    pub(super) fn sync(&mut self) {
        let Some(snapshot) = self.loader.data() else {
            self.derived = None;
            return;
        };
        let revision = self.loader.revision();
        let current = self
            .derived
            .as_ref()
            .is_some_and(|derived| derived.revision == revision && derived.shown == self.shown);
        if !current {
            self.derived = Some(Derived::new(revision, snapshot, self.shown));
        }
    }

    fn rows(&self) -> &[CheckRow] {
        self.derived
            .as_ref()
            .map_or(&[], |derived| derived.rows.as_slice())
    }

    /// The selected check: the one chosen while it's listed, else the first.
    pub(super) fn selected_check(&self) -> Option<&DiagnosticCheck> {
        let key = self.selected_key()?;
        self.loader
            .data()?
            .checks
            .iter()
            .find(|check| &self::key(check) == key)
    }
}

impl TableSource for DiagnosticsScreen {
    type Key = String;
    type Sort = ();
    type Column = Column;
    type Row<'a> = &'a CheckRow;

    fn table_state(&self) -> &table::TableState {
        &self.table
    }

    fn columns(&self) -> &[Column] {
        self.derived
            .as_ref()
            .map_or(&[], |derived| derived.columns.as_slice())
    }

    fn width(&self) -> f32 {
        self.derived.as_ref().map_or(0., |derived| derived.width)
    }

    fn list_label(&self) -> String {
        "Diagnostic checks; arrows select a check, P shows only problems".into()
    }

    fn sorting(&self, _: &Column) -> Option<((), Option<SortOrder>)> {
        None
    }

    fn sort(&mut self, _: (), _: &mut Context<Self>) {}

    fn line_count(&self) -> usize {
        self.derived
            .as_ref()
            .map_or(0, |derived| derived.lines.len())
    }

    fn line(&self, line: usize, _: &App) -> Option<Line<String, &CheckRow>> {
        let derived = self.derived.as_ref()?;
        let row = match *derived.lines.get(line)? {
            Entry::Group(group) => return Some(Line::Group(group)),
            Entry::Row(row) => derived.rows.get(row)?,
        };
        Some(Line::Row(TableRow {
            key: row.key.clone(),
            id: ("diagnostic-check", row.index).into(),
            label: row.label.clone(),
            tooltip: Some(row.message.clone()),
            marked: false,
            muted: false,
            data: row,
        }))
    }

    fn cell(
        &self,
        line: &TableRow<String, &CheckRow>,
        style: &RowStyle,
        column: &Column,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let row = line.data;
        let cell = table::cell(column);
        match column.field {
            Field::Glyph => table::glyph_cell(column)
                .child(ui::status_mark(
                    SharedString::from(format!("diagnostic-{}-status", row.key)),
                    row.tone,
                    row.status,
                    cx,
                ))
                .into_any_element(),
            Field::Check => cell.child(row.name.clone()).into_any_element(),
            Field::Result => cell
                .when(!style.selected, |this| this.text_color(style.p.muted))
                .child(row.message.clone())
                .into_any_element(),
        }
    }

    fn group(&self, group: usize, cx: &mut Context<Self>) -> Option<AnyElement> {
        let group = self.derived.as_ref()?.groups.get(group)?;
        Some(
            table::GroupRow::new(
                SharedString::from(format!(
                    "diagnostic-section-{}",
                    section_slug(group.category)
                )),
                group.tone,
                group.title.clone(),
                table::ROW_HEIGHT,
            )
            .detail(group.detail.clone())
            .render(cx)
            .into_any_element(),
        )
    }

    /// The chosen check while it's listed, else the first, so the details
    /// always show one.
    fn selected_key(&self) -> Option<&String> {
        let rows = self.rows();
        self.selected
            .as_ref()
            .filter(|key| rows.iter().any(|row| &row.key == *key))
            .or_else(|| rows.first().map(|row| &row.key))
    }

    fn line_of(&self, key: &String) -> Option<usize> {
        let derived = self.derived.as_ref()?;
        derived.lines.iter().position(|line| match line {
            Entry::Row(row) => derived.rows.get(*row).is_some_and(|row| &row.key == key),
            Entry::Group(_) => false,
        })
    }

    fn click(&mut self, key: &String, _: &ClickEvent, window: &mut Window, cx: &mut Context<Self>) {
        self.selected = Some(key.clone());
        window.focus(&self.focus, cx);
        cx.notify();
    }

    fn loading(&self) -> Option<&freshkube_ui::table::LoadingRows> {
        self.loading.rows()
    }

    fn empty(&self, _: &mut Context<Self>) -> Option<AnyElement> {
        if !self.rows().is_empty() {
            return None;
        }
        Some(self.shown.empty().into_any_element())
    }
}
