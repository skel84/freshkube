//! The clusters table as a `TableSource`.
use super::*;
use table::{Line, RowStyle, SortOrder, TableColumn, TableRow, TableSource, TableState};

#[derive(Clone, Copy, PartialEq, Eq)]
enum Field {
    Name,
    Role,
    Context,
    Talosconfig,
    Starts,
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
        self.field == Field::Talosconfig
    }

    fn pinned(&self) -> bool {
        self.field == Field::Name
    }
}

/// DESIGN.md's widths: 7.5 a character plus 24, between 64 and 240.
fn fit<'a>(label: &str, texts: impl Iterator<Item = &'a SharedString>) -> f32 {
    let chars = texts
        .map(|text| text.chars().count())
        .chain([label.len()])
        .max()
        .unwrap_or_default();
    (chars as f32 * 7.5 + 24.).clamp(64., 240.)
}

pub(super) fn columns(rows: &[ClusterRow]) -> (Vec<Column>, f32) {
    let column = |field, label: &str, width| Column {
        field,
        label: label.to_owned().into(),
        width,
    };
    let columns = vec![
        column(
            Field::Name,
            "Cluster",
            fit("Cluster", rows.iter().map(|row| &row.id)),
        ),
        column(Field::Role, "Role", 120.),
        column(
            Field::Context,
            "Context",
            fit("Context", rows.iter().map(|row| &row.context)),
        ),
        column(Field::Starts, "Opens at launch", 130.),
        column(Field::Talosconfig, "Talosconfig", 240.),
    ];
    let width = columns.iter().map(|column| column.width).sum();
    (columns, width)
}

impl TableSource for SettingsPage {
    type Key = SharedString;
    type Sort = ();
    type Column = Column;
    type Row<'a> = &'a ClusterRow;

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
        "The clusters of the workspace".into()
    }

    fn sorting(&self, _: &Column) -> Option<((), Option<SortOrder>)> {
        None
    }

    fn sort(&mut self, _: (), _: &mut Context<Self>) {}

    fn line_count(&self) -> usize {
        self.rows.len()
    }

    fn line(&self, line: usize, _: &App) -> Option<Line<SharedString, &ClusterRow>> {
        let row = self.rows.get(line)?;
        Some(Line::Row(TableRow {
            key: row.id.clone(),
            id: format!("settings-cluster-{}", row.id).into(),
            label: format!("{} · {} · {}", row.id, row.role, row.context).into(),
            tooltip: None,
            marked: false,
            muted: false,
            data: row,
        }))
    }

    fn cell(
        &self,
        line: &TableRow<SharedString, &ClusterRow>,
        style: &RowStyle,
        column: &Column,
        _: &mut Context<Self>,
    ) -> AnyElement {
        let row = line.data;
        let cell = table::cell(column);
        match column.field {
            Field::Name => cell.child(row.id.clone()).into_any_element(),
            Field::Role => cell.child(row.role.clone()).into_any_element(),
            Field::Context => cell.child(row.context.clone()).into_any_element(),
            Field::Starts => cell
                .child(if row.starts { "Yes" } else { "" })
                .into_any_element(),
            Field::Talosconfig => cell
                .text_color(style.p.muted)
                .child(row.talosconfig.clone())
                .into_any_element(),
        }
    }

    fn group(&self, _: usize, _: &mut Context<Self>) -> Option<AnyElement> {
        None
    }

    fn selected_key(&self) -> Option<&SharedString> {
        self.selected.as_ref()
    }

    fn line_of(&self, key: &SharedString) -> Option<usize> {
        self.rows.iter().position(|row| &row.id == key)
    }

    fn click(
        &mut self,
        key: &SharedString,
        _: &ClickEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.select(key.clone(), cx);
        self.focus(window, cx);
    }

    fn empty(&self, _: &mut Context<Self>) -> Option<AnyElement> {
        if !self.rows.is_empty() {
            return None;
        }
        Some(
            "No clusters are listed: this window works with the one cluster it opened."
                .into_any_element(),
        )
    }
}
