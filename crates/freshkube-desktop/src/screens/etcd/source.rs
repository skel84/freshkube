//! The members table as a `TableSource`: its columns, sized from the rows
//! when they change, and the cells of each row.
use super::*;
use freshkube_ui::table::{
    self, Line, RowStyle, SortOrder, TableColumn, TableRow, TableSource, TableState,
};

#[derive(Clone, Copy, PartialEq, Eq)]
enum Field {
    Glyph,
    Member,
    Role,
    Endpoint,
    Db,
    Raft,
    Issues,
}

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
        self.field == Field::Member
    }

    /// The glyph and name stay in view when the table scrolls sideways.
    fn pinned(&self) -> bool {
        matches!(self.field, Field::Glyph | Field::Member)
    }
}

/// The Member column's widest: a longer name truncates, and the row's
/// tooltip holds it, so the figures stay in view beside the details.
const MEMBER_WIDTH: f32 = 200.;
/// The widest role tag, "Not reported", with its glyph and padding.
const ROLE_WIDTH: f32 = 124.;

/// The columns for these rows, and their total width.
pub(super) fn columns(rows: &[MemberRow]) -> (Vec<Column>, f32) {
    let column = |field, label: &str, width| Column {
        field,
        label: label.to_owned().into(),
        width,
    };
    let columns = vec![
        column(Field::Glyph, "", table::GLYPH_WIDTH),
        column(
            Field::Member,
            "Member",
            table::fit("Member", rows.iter().map(|row| &row.name), table::WIDEST).min(MEMBER_WIDTH),
        ),
        column(Field::Role, "Role", ROLE_WIDTH),
        column(
            Field::Endpoint,
            "Endpoint",
            table::fit(
                "Endpoint",
                rows.iter().map(|row| &row.endpoint),
                table::WIDEST,
            ),
        ),
        column(
            Field::Db,
            "DB size",
            table::fit("DB size", rows.iter().map(|row| &row.db), table::WIDEST),
        ),
        column(
            Field::Raft,
            "Raft index",
            table::fit(
                "Raft index",
                rows.iter().map(|row| &row.raft),
                table::WIDEST,
            ),
        ),
        column(
            Field::Issues,
            "Issues",
            table::fit("Issues", rows.iter().map(|row| &row.issues), table::WIDEST),
        ),
    ];
    let width = columns.iter().map(|column| column.width).sum();
    (columns, width)
}

impl TableSource for EtcdScreen {
    type Key = u64;
    type Sort = ();
    type Column = Column;
    type Row<'a> = &'a MemberRow;

    fn table_state(&self) -> &TableState {
        &self.table
    }

    fn columns(&self) -> &[Column] {
        &self.derived.columns
    }

    fn width(&self) -> f32 {
        self.derived.width
    }

    fn list_label(&self) -> String {
        "etcd members; arrows select a member".into()
    }

    fn sorting(&self, _: &Column) -> Option<((), Option<SortOrder>)> {
        None
    }

    fn sort(&mut self, _: (), _: &mut Context<Self>) {}

    fn line_count(&self) -> usize {
        self.derived.lines.len()
    }

    fn line(&self, line: usize, _: &App) -> Option<Line<u64, &MemberRow>> {
        let row = self.derived.rows.get(*self.derived.lines.get(line)?)?;
        Some(Line::Row(TableRow {
            key: row.id,
            id: row.element_id.clone().into(),
            label: row.label.clone(),
            tooltip: Some(row.name.clone()),
            marked: false,
            muted: false,
            data: row,
        }))
    }

    fn cell(
        &self,
        line: &TableRow<u64, &MemberRow>,
        style: &RowStyle,
        column: &Column,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let row = line.data;
        let cell = table::cell(column);
        match column.field {
            Field::Glyph => cell
                .flex()
                .items_center()
                .child(ui::status_mark(
                    SharedString::from(format!("etcd-member-health-{:x}", row.id)),
                    row.state.tone(),
                    row.state.label(),
                    cx,
                ))
                .into_any_element(),
            Field::Member => cell
                .font_weight(FontWeight::MEDIUM)
                .child(row.name.clone())
                .into_any_element(),
            Field::Role => cell
                .flex()
                .items_center()
                .child(
                    div()
                        .id(SharedString::from(format!("etcd-role-{:x}", row.id)))
                        .test_support()
                        .aria_label(row.role.label())
                        .child(role_tag(row.role, cx)),
                )
                .into_any_element(),
            Field::Endpoint => cell.child(row.endpoint.clone()).into_any_element(),
            Field::Db => cell.child(row.db.clone()).into_any_element(),
            Field::Raft => cell.child(row.raft.clone()).into_any_element(),
            Field::Issues => cell
                .when(row.issue_count > 0 && !style.selected, |this| {
                    this.text_color(style.p.crit_ink)
                })
                .child(row.issues.clone())
                .into_any_element(),
        }
    }

    fn group(&self, _: usize, _: &mut Context<Self>) -> Option<AnyElement> {
        None
    }

    fn selected_key(&self) -> Option<&u64> {
        self.selected.as_ref()
    }

    fn line_of(&self, key: &u64) -> Option<usize> {
        self.derived
            .lines
            .iter()
            .position(|&ix| self.derived.rows[ix].id == *key)
    }

    fn click(&mut self, key: &u64, _: &ClickEvent, window: &mut Window, cx: &mut Context<Self>) {
        self.selected = Some(*key);
        window.focus(&self.focus, cx);
        cx.notify();
    }

    fn empty(&self, _: &mut Context<Self>) -> Option<AnyElement> {
        if !self.derived.lines.is_empty() {
            return None;
        }
        Some(
            if self.derived.rows.is_empty() {
                "etcd reported no members."
            } else {
                "No members match this filter."
            }
            .into_any_element(),
        )
    }
}
