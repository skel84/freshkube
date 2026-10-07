//! The roster as a `TableSource`: the nodes an operation can run on, with
//! the run's order marked on the nodes it holds. The table's selected row
//! is the cursor that Space and Enter add to or take from the run.
use super::*;
use freshkube_ui::table::{
    self, Line, RowStyle, SortOrder, TableColumn, TableRow, TableSource, TableState,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Field {
    Order,
    Node,
    Address,
    Role,
}

#[derive(Clone, Debug)]
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
        self.field == Field::Node
    }

    /// The run order and the name stay in view when the table scrolls
    /// sideways.
    fn pinned(&self) -> bool {
        matches!(self.field, Field::Order | Field::Node)
    }
}

/// The Order column holds a check and a run position.
const ORDER_WIDTH: f32 = 72.;

/// The columns for this roster, and their total width.
pub(super) fn columns(roster: &[RosterNode]) -> (Vec<Column>, f32) {
    let column = |field, label: &str, width| Column {
        field,
        label: label.to_owned().into(),
        width,
    };
    let widest = |label: &str, text: &dyn Fn(&RosterNode) -> SharedString| {
        let texts: Vec<SharedString> = roster.iter().map(text).collect();
        table::fit(label, texts.iter(), table::WIDEST)
    };
    let widest_flexible = |label: &str, text: &dyn Fn(&RosterNode) -> SharedString| {
        let texts: Vec<SharedString> = roster.iter().map(text).collect();
        table::fit(label, texts.iter(), table::WIDEST_FLEXIBLE)
    };
    let columns = vec![
        column(Field::Order, "Order", ORDER_WIDTH),
        column(
            Field::Node,
            "Node",
            widest_flexible("Node", &|node| node.target.name.clone().into()),
        ),
        column(
            Field::Address,
            "Address",
            widest("Address", &|node| node.target.address.clone().into()),
        ),
        column(
            Field::Role,
            "Role",
            widest("Role", &|node| node.role.label().into()),
        ),
    ];
    let width = columns.iter().map(|column| column.width).sum();
    (columns, width)
}

impl RosterNode {
    pub(super) fn new(target: NodeTarget, role: NodeRole, responding: bool) -> Self {
        let element_id = format!("ops-node-{}", target.name).into();
        let label = format!("{} · {} · {}", target.name, target.address, role.label()).into();
        Self {
            target,
            role,
            responding,
            element_id,
            label,
        }
    }
}

impl OperationsScreen {
    /// Where this node is in the run, if it is in it.
    fn order_of(&self, node: &RosterNode) -> Option<usize> {
        self.order.get(&node.target.name).copied()
    }

    /// Derives each selected node's place in the run; whatever changes the
    /// selection calls it.
    pub(super) fn derive_order(&mut self) {
        self.order = (self.selected.iter().enumerate())
            .map(|(k, target)| (target.name.clone(), k))
            .collect();
    }
}

impl TableSource for OperationsScreen {
    type Key = String;
    type Sort = ();
    type Column = Column;
    type Row<'a> = &'a RosterNode;

    fn table_state(&self) -> &TableState {
        &self.table
    }

    fn columns(&self) -> &[Column] {
        &self.columns.0
    }

    fn width(&self) -> f32 {
        self.columns.1
    }

    fn list_label(&self) -> String {
        "Nodes; arrows move, Space selects, Alt+arrows reorder the run, Escape clears".into()
    }

    fn sorting(&self, _: &Column) -> Option<((), Option<SortOrder>)> {
        None
    }

    fn sort(&mut self, _: (), _: &mut Context<Self>) {}

    fn line_count(&self) -> usize {
        self.roster.len()
    }

    fn line(&self, line: usize, _: &App) -> Option<Line<String, &RosterNode>> {
        let node = self.roster.get(line)?;
        let order = self.order_of(node);
        let label = format!(
            "{} · {}{}",
            node.label,
            match order {
                Some(k) => format!("selected, run order {}", k + 1),
                None => "not selected".to_owned(),
            },
            if node.responding {
                ""
            } else {
                " · not responding to the Talos API"
            }
        );
        Some(Line::Row(TableRow {
            key: node.target.name.clone(),
            id: node.element_id.clone().into(),
            label: label.into(),
            tooltip: Some(node.target.name.clone().into()),
            marked: order.is_some(),
            muted: false,
            data: node,
        }))
    }

    fn cell(
        &self,
        line: &TableRow<String, &RosterNode>,
        style: &RowStyle,
        column: &Column,
        _: &mut Context<Self>,
    ) -> AnyElement {
        let node = line.data;
        let p = &style.p;
        let cell = table::cell(column);
        match column.field {
            Field::Order => match self.order_of(node) {
                Some(k) => cell
                    .flex()
                    .items_center()
                    .gap(dp(4.))
                    .text_color(p.accent)
                    .child(Icon::new(IconName::Check).size(dp(14.)))
                    .child((k + 1).to_string())
                    .into_any_element(),
                None => cell.text_color(p.muted).child("·").into_any_element(),
            },
            Field::Node => cell
                .font_weight(FontWeight::MEDIUM)
                .child(node.target.name.clone())
                .into_any_element(),
            Field::Address => cell.child(node.target.address.clone()).into_any_element(),
            Field::Role => cell.child(node.role.label()).into_any_element(),
        }
    }

    fn group(&self, _: usize, _: &mut Context<Self>) -> Option<AnyElement> {
        None
    }

    fn selected_key(&self) -> Option<&String> {
        self.roster.get(self.cursor).map(|node| &node.target.name)
    }

    fn line_of(&self, key: &String) -> Option<usize> {
        self.roster.iter().position(|node| &node.target.name == key)
    }

    /// A click puts the cursor on the node and adds it to the run, or takes
    /// it out, as Space does.
    fn click(&mut self, key: &String, _: &ClickEvent, window: &mut Window, cx: &mut Context<Self>) {
        let Some(ix) = self.line_of(key) else {
            return;
        };
        self.cursor = ix;
        window.focus(&self.focus, cx);
        let target = self.roster[ix].target.clone();
        self.toggle(target, window, cx);
    }

    fn empty(&self, _: &mut Context<Self>) -> Option<AnyElement> {
        self.roster.is_empty().then(|| {
            "No node roster is available, so there is nothing to select.".into_any_element()
        })
    }
}
