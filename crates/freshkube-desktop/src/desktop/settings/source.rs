//! The clusters table as a `TableSource`.
use super::*;
use table::{Line, RowStyle, SortOrder, TableColumn, TableRow, TableSource, TableState};

#[derive(Clone, Copy, PartialEq, Eq)]
enum Field {
    Glyph,
    Name,
    Role,
    Protection,
    Context,
    Talosconfig,
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
        matches!(self.field, Field::Glyph | Field::Name)
    }
}

/// Derives widths using the shared table's DESIGN.md sizing rules.
pub(super) fn columns(rows: &[ClusterRow]) -> (Vec<Column>, f32) {
    let column = |field, label: &str, width| Column {
        field,
        label: label.to_owned().into(),
        width,
    };
    let columns = vec![
        column(Field::Glyph, "", table::GLYPH_WIDTH),
        column(
            Field::Name,
            "Cluster",
            table::fit("Cluster", rows.iter().map(|row| &row.id), table::WIDEST),
        ),
        column(Field::Role, "Role", 120.),
        column(Field::Protection, "Protection", 170.),
        column(
            Field::Context,
            "Context",
            table::fit(
                "Context",
                rows.iter().map(|row| &row.context),
                table::WIDEST,
            ),
        ),
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
            tooltip: Some(row.tooltip.clone()),
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
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let row = line.data;
        let text_cell = |field: &str, text: &SharedString| {
            table::word_cell(
                column,
                format!("settings-cluster-{}-{field}", row.id),
                text.clone(),
            )
            .test_support()
        };
        match column.field {
            Field::Glyph => table::glyph_cell(column)
                .id(SharedString::from(format!(
                    "settings-cluster-{}-glyph",
                    row.id
                )))
                .test_support()
                .when_some(row.note.clone(), |this, note| {
                    this.child(ui::status_mark(
                        SharedString::from(format!("settings-cluster-{}-mark", row.id)),
                        ui::Tone::Warn,
                        note,
                        cx,
                    ))
                })
                .into_any_element(),
            Field::Name => text_cell("name", &row.id).into_any_element(),
            Field::Role => text_cell("role", &row.role).into_any_element(),
            Field::Protection => {
                // An unset level is guarded, and says it is the default.
                let level = row.protection.unwrap_or_default();
                let id = format!("settings-cluster-{}-protection", row.id);
                let words = if row.protection.is_none() {
                    format!("{} · default", level.label())
                } else {
                    level.label().to_owned()
                };
                table::cell(column)
                    .id(SharedString::from(format!("{id}-cell")))
                    .flex()
                    .items_center()
                    .gap(dp(6.))
                    .child(
                        freshkube_ui::level::tag(
                            crate::desktop::shell::mark(level),
                            freshkube_ui::level::Form::Cell,
                            cx,
                        )
                        .id(SharedString::from(id.clone()))
                        .test_support()
                        .aria_label(words.clone()),
                    )
                    .when(row.protection.is_none(), |this| {
                        this.child(
                            div()
                                .id(SharedString::from(format!("{id}-default")))
                                .test_support()
                                .aria_label("· default")
                                .text_size(dp(12.))
                                .text_color(style.p.muted)
                                .whitespace_nowrap()
                                .child("· default"),
                        )
                    })
                    .into_any_element()
            }
            Field::Context => text_cell("context", &row.context).into_any_element(),
            Field::Talosconfig => text_cell("talosconfig", &row.talosconfig)
                .text_color(style.p.muted)
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
        self.request_select(Some(key.clone()), window, cx);
        self.focus(window, cx);
    }

    fn menu_focus(&self, _: &App) -> Option<FocusHandle> {
        Some(self.focus.clone())
    }

    /// A right-click selects the row as a click does; its menu is the
    /// toolbar's actions on it, with their keys.
    fn row_menu(
        &mut self,
        key: &SharedString,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Vec<menu::MenuAction> {
        let inspecting = self.entry.is_some() && !self.menu_holds_inspector;
        self.request_select(Some(key.clone()), window, cx);
        self.focus(window, cx);
        self.menu_holds_inspector = !inspecting;
        self.menu_had_focus = false;
        // Unsaved changes hold the selection on its row until the prompt is
        // answered; the actions are then not for this row.
        let editable = self.editable() && !self.selection_pending(key);
        let at = self.rows.iter().position(|row| &row.id == key);
        let last = self.rows.len().saturating_sub(1);
        vec![
            menu::MenuAction::new("Edit", EditCluster).enabled(editable),
            menu::MenuAction::new("Remove", RemoveCluster).enabled(editable),
            menu::MenuAction::Separator,
            menu::MenuAction::new("Move up", MoveClusterUp)
                .enabled(editable && at.is_some_and(|at| at > 0)),
            menu::MenuAction::new("Move down", MoveClusterDown)
                .enabled(editable && at.is_some_and(|at| at < last)),
        ]
    }

    fn empty(&self, _: &mut Context<Self>) -> Option<AnyElement> {
        if !self.rows.is_empty() {
            return None;
        }
        let text = if self.source_recovery.all_clusters_skipped {
            "All configured clusters were skipped. Fix the skipped rows in freshkube.yaml and Reload to use them."
        } else {
            "Nothing is listed. The one cluster counted in the status bar is implicit: the one this window opened."
        };
        Some(
            div()
                .id("settings-empty-explanation")
                .test_support()
                .aria_label(text)
                .child(text)
                .into_any_element(),
        )
    }
}
