//! Nodes' G7 table. The shell retains the data, selection and pane.
use super::*;
use freshkube_ui::table::{
    self, Line, RowStyle, SortOrder, TableColumn, TableRow, TableSource, TableState,
};
use gpui_kit::prelude::*;
use std::collections::BTreeSet;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(super) enum Field {
    Glyph,
    Name,
    Role,
    Kubernetes,
    Talos,
    Load,
    Memory,
    Pods,
    Services,
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
        self.field == Field::Name
    }
}

impl Column {
    fn new(field: Field, rows: &[NodeRow]) -> Self {
        let label = match field {
            Field::Glyph => "",
            Field::Name => "Name",
            Field::Role => "Role",
            Field::Kubernetes => "Kubernetes",
            Field::Talos => "Talos",
            Field::Load => "Load",
            Field::Memory => "Memory",
            Field::Pods => "Pods",
            Field::Services => "System services",
        };
        let width = if field == Field::Glyph {
            34.
        } else if field == Field::Load {
            192.
        } else if field == Field::Memory {
            152.
        } else {
            let chars = rows
                .iter()
                .map(|row| field.value(row).chars().count())
                .max()
                .unwrap_or(0)
                .max(label.len());
            (chars as f32 * 7.5 + 24.).clamp(64., if field == Field::Name { 440. } else { 280. })
        };
        Self {
            field,
            label: label.into(),
            width,
        }
    }
}

impl Field {
    pub(super) fn value(self, row: &NodeRow) -> &str {
        match self {
            Self::Glyph => "",
            Self::Name => &row.name,
            Self::Role => row.role.label(),
            Self::Kubernetes => row.ready,
            Self::Talos => &row.talos_state,
            Self::Load => &row.load,
            Self::Memory => &row.memory,
            Self::Pods => &row.pods,
            Self::Services => &row.services,
        }
    }
}

impl Nodes {
    pub(super) fn rebuild_columns(&mut self, talos: bool) {
        self.all_columns = [
            Field::Glyph,
            Field::Name,
            Field::Role,
            Field::Kubernetes,
            Field::Talos,
            Field::Load,
            Field::Memory,
            Field::Pods,
            Field::Services,
        ]
        .into_iter()
        .filter(|field| {
            talos
                || !matches!(
                    field,
                    Field::Talos | Field::Load | Field::Memory | Field::Services
                )
        })
        .map(|field| Column::new(field, &self.rows))
        .collect();
        self.menu_columns = Arc::new(
            self.all_columns
                .iter()
                .filter(|column| !matches!(column.field, Field::Glyph | Field::Name))
                .map(|column| (column.field, column.label.clone()))
                .collect(),
        );
        self.show_columns();
    }

    pub(super) fn show_columns(&mut self) {
        self.columns = self
            .all_columns
            .iter()
            .filter(|column| !self.hidden_columns.contains(&column.field))
            .map(|column| Column {
                field: column.field,
                label: column.label.clone(),
                width: column.width,
            })
            .collect();
        self.table_width = self.columns.iter().map(|column| column.width).sum();
    }
}

impl Pilot {
    pub(in crate::desktop) fn sync_nodes_source_mode(&mut self) {
        self.node_workspace
            .rebuild_columns(self.kubernetes_only.is_none());
        self.rebuild_nodes_meta();
    }

    pub(super) fn nodes_columns_menu(&self, cx: &Context<Self>) -> AnyElement {
        use gpui_kit::component::{
            Sizable,
            button::Button,
            menu::{DropdownMenu, PopupMenuItem},
        };
        let owner = cx.entity().downgrade();
        let columns = self.node_workspace.menu_columns.clone();
        let hidden: BTreeSet<_> = self.node_workspace.hidden_columns.clone();
        Button::new(self.node_workspace.table.id("columns"))
            .outline()
            .small()
            .label("Columns")
            .dropdown_caret(true)
            .dropdown_menu(move |mut menu, _, _| {
                for (field, label) in columns.iter() {
                    let field = *field;
                    let owner = owner.clone();
                    menu = menu.item(
                        PopupMenuItem::new(label.clone())
                            .checked(!hidden.contains(&field))
                            .on_click(move |_, _, cx| {
                                _ = owner.update(cx, |view, cx| {
                                    if !view.node_workspace.hidden_columns.remove(&field) {
                                        view.node_workspace.hidden_columns.insert(field);
                                    }
                                    view.node_workspace.show_columns();
                                    cx.notify();
                                });
                            }),
                    );
                }
                menu
            })
            .into_any_element()
    }
}

impl TableSource for Pilot {
    type Key = NodeKey;
    type Sort = ();
    type Column = Column;
    type Row<'a> = &'a NodeRow;

    fn table_state(&self) -> &TableState {
        &self.node_workspace.table
    }
    fn columns(&self) -> &[Column] {
        &self.node_workspace.columns
    }
    fn width(&self) -> f32 {
        self.node_workspace.table_width
    }
    fn list_label(&self) -> String {
        "Nodes; arrows select, Enter opens the selected node".into()
    }
    fn sorting(&self, _: &Column) -> Option<((), Option<SortOrder>)> {
        None
    }
    fn sort(&mut self, _: (), _: &mut Context<Self>) {}
    fn line_count(&self) -> usize {
        if self.node_workspace.open {
            // The compact pane switcher deliberately keeps the full roster. Main-list
            // filters remain retained and resume when the pane closes.
            self.node_workspace.rows.len()
        } else if self.node_workspace.view == NodeView::Cards {
            self.node_workspace.lines.len()
        } else {
            self.node_workspace.items.len()
        }
    }
    fn line(&self, line: usize, _: &App) -> Option<Line<NodeKey, &NodeRow>> {
        let ix = if self.node_workspace.open {
            line
        } else if self.node_workspace.view == NodeView::Cards {
            *self.node_workspace.lines.get(line)?
        } else {
            match self.node_workspace.items.get(line)? {
                projection::Item::Group(status) => return Some(Line::Group(status.index())),
                projection::Item::Row(ix) => *ix,
            }
        };
        let row = self.node_workspace.rows.get(ix)?;
        Some(Line::Row(TableRow {
            key: row.key.clone(),
            id: row.id.clone().into(),
            label: row.name.clone(),
            tooltip: Some(row.name.clone()),
            marked: false,
            muted: false,
            data: row,
        }))
    }
    fn cell(
        &self,
        line: &TableRow<NodeKey, &NodeRow>,
        style: &RowStyle,
        column: &Column,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let row = line.data;
        match column.field {
            Field::Glyph => table::cell(column)
                .children(ui::status_glyph(row.tone, cx))
                .into_any_element(),
            Field::Name => table::cell(column)
                .id("node-row-name")
                .test_support()
                .child(row.name.clone())
                .into_any_element(),
            Field::Kubernetes => table::cell(column).child(row.ready).into_any_element(),
            Field::Services => table::cell(column)
                .text_color(style.p.muted)
                .child(row.services.clone())
                .into_any_element(),
            field => {
                let value: SharedString = match field {
                    Field::Role => row.role.label().into(),
                    Field::Talos => row.talos_state.clone(),
                    Field::Load => row.load.clone(),
                    Field::Memory => row.memory.clone(),
                    Field::Pods => row.pods.clone(),
                    _ => unreachable!("the other columns have dedicated cells"),
                };
                table::cell(column)
                    .text_color(style.p.muted)
                    .child(value)
                    .into_any_element()
            }
        }
    }
    fn group(&self, group: usize, cx: &mut Context<Self>) -> Option<AnyElement> {
        let status = *projection::Status::ALL.get(group)?;
        let count = self.node_workspace.group_counts[status.index()];
        let collapsed =
            status == projection::Status::Healthy && self.node_workspace.healthy_collapsed();
        let mut detail = vec![format!("{count} nodes")];
        if collapsed {
            detail.push("collapsed".into());
        }
        let row = table::GroupRow::new(
            status.group_id(),
            status.tone(),
            status.label(),
            self.node_workspace.table.row_height(),
        )
        .detail(detail);
        let row = if status == projection::Status::Healthy
            && self.node_workspace.counts[..status.index()]
                .iter()
                .any(|count| *count > 0)
        {
            use gpui_kit::component::{
                Sizable,
                button::{Button, ButtonVariants},
            };
            row.action(
                Button::new("nodes-healthy-toggle")
                    .ghost()
                    .xsmall()
                    .label(if collapsed { "Expand" } else { "Collapse" })
                    .on_click(cx.listener(|view, _, _, cx| {
                        view.node_workspace.healthy_open = !view.node_workspace.healthy_open;
                        view.node_workspace.rebuild_lines();
                        cx.notify();
                    })),
            )
        } else {
            row
        };
        Some(row.render(cx).into_any_element())
    }
    fn selected_key(&self) -> Option<&NodeKey> {
        self.node_workspace.selected.as_ref()
    }
    fn line_of(&self, key: &NodeKey) -> Option<usize> {
        if self.node_workspace.open {
            self.node_workspace
                .rows
                .iter()
                .position(|row| &row.key == key)
        } else if self.node_workspace.view == NodeView::Cards {
            self.node_workspace
                .lines
                .iter()
                .position(|ix| &self.node_workspace.rows[*ix].key == key)
        } else {
            self.node_workspace.items.iter().position(|item| {
                matches!(item, projection::Item::Row(ix) if &self.node_workspace.rows[*ix].key == key)
            })
        }
    }
    fn click(
        &mut self,
        key: &NodeKey,
        _: &ClickEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.open_node(key.clone(), window, cx);
    }
    fn empty(&self, _: &mut Context<Self>) -> Option<AnyElement> {
        if self.node_workspace.lines.is_empty() {
            Some(
                div()
                    .id("nodes-empty-message")
                    .test_support()
                    .role(gpui_kit::Role::Status)
                    .aria_label("No matching nodes")
                    .child("No nodes match these filters")
                    .into_any_element(),
            )
        } else {
            None
        }
    }
}
