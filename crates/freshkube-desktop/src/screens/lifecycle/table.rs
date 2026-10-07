//! The node roster as a `TableSource`: its columns, sized from the rows when
//! they change, and the cells of each row.
use super::*;
use freshkube_ui::table::{
    self, Line, RowStyle, SortOrder, TableColumn, TableRow, TableSource, TableState,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Field {
    Node,
    Role,
    Talos,
    Kubelet,
    Config,
    Discovery,
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

    /// The name stays in view when the table scrolls sideways.
    fn pinned(&self) -> bool {
        self.field == Field::Node
    }
}

/// The Node column's widest: a longer name truncates, and the row's tooltip
/// holds it, so the figures stay in view beside the details.
const NODE_WIDTH: f32 = 200.;
/// The presence column holds two glyphs and a dot, whatever the rows say.
const PRESENCE_CHARS: usize = 15;

/// DESIGN.md's widths: 7.5 a character plus 24, between 64 and 280.
fn fit(label: &str, texts: impl Iterator<Item = usize>) -> f32 {
    let chars = texts
        .chain([label.chars().count()])
        .max()
        .unwrap_or_default();
    (chars as f32 * 7.5 + 24.).clamp(64., 280.)
}

impl NodeRow {
    const NOT_REPORTED: &'static str = "not reported";

    fn talos_text(&self) -> &str {
        self.talos.as_deref().unwrap_or(Self::NOT_REPORTED)
    }

    fn kubelet_text(&self) -> &str {
        self.kubelet.as_deref().unwrap_or(Self::NOT_REPORTED)
    }

    /// The row's id and accessibility label, once its drift is known.
    pub(super) fn derive(&mut self) {
        let not_reported = || Self::NOT_REPORTED.to_owned();
        let config = match (&self.config, self.drift) {
            (Ok(hash), Drift::Differs) => format!("{hash}, differs from other nodes"),
            (Ok(hash), _) => hash.clone(),
            (Err(_), _) => not_reported(),
        };
        self.element_id = format!("lifecycle-node-{}", self.name).into();
        self.label = format!(
            "{} · {} · Talos {} · kubelet {} · config {config} · discovery {} · Kubernetes {}",
            self.name,
            self.role_label(),
            self.talos.clone().unwrap_or_else(|_| not_reported()),
            self.kubelet.clone().unwrap_or_else(|_| not_reported()),
            presence_text(self.in_discovery, "discovery"),
            presence_text(self.in_kubernetes, "Kubernetes"),
        )
        .into();
    }

    /// The config hash, and how it compares with the other nodes'.
    fn config_text(&self) -> String {
        match (&self.config, self.drift) {
            (Ok(hash), Drift::Differs) => format!("{hash} · differs"),
            (Ok(hash), _) => hash.clone(),
            (Err(_), _) => "—".to_owned(),
        }
    }
}

/// The columns for these rows, and their total width.
pub(super) fn columns(rows: &[NodeRow]) -> (Vec<Column>, f32) {
    let column = |field, label: &str, width| Column {
        field,
        label: label.to_owned().into(),
        width,
    };
    let widest = |label: &str, text: &dyn Fn(&NodeRow) -> usize| fit(label, rows.iter().map(text));
    let columns = vec![
        column(
            Field::Node,
            "Node",
            widest("Node", &|row| row.name.chars().count()).min(NODE_WIDTH),
        ),
        column(
            Field::Role,
            "Role",
            widest("Role", &|row| row.role_label().chars().count()),
        ),
        column(
            Field::Talos,
            "Talos",
            widest("Talos", &|row| row.talos_text().chars().count()),
        ),
        column(
            Field::Kubelet,
            "Kubelet",
            widest("Kubelet", &|row| row.kubelet_text().chars().count()),
        ),
        column(
            Field::Config,
            "Config",
            widest("Config", &|row| row.config_text().chars().count()),
        ),
        column(
            Field::Discovery,
            "Discovery · K8s",
            fit("Discovery · K8s", [PRESENCE_CHARS].into_iter()),
        ),
    ];
    let width = columns.iter().map(|column| column.width).sum();
    (columns, width)
}

impl TableSource for LifecycleScreen {
    type Key = String;
    type Sort = ();
    type Column = Column;
    type Row<'a> = &'a NodeRow;

    fn table_state(&self) -> &TableState {
        &self.table
    }

    fn columns(&self) -> &[Column] {
        self.loader.data().map_or(&self.loading_columns.0, |view| {
            view.display.columns.as_slice()
        })
    }

    fn width(&self) -> f32 {
        self.loader
            .data()
            .map_or(self.loading_columns.1, |view| view.display.width)
    }

    fn list_label(&self) -> String {
        "Nodes; arrows select a node or alert, Escape clears the selection".into()
    }

    fn sorting(&self, _: &Column) -> Option<((), Option<SortOrder>)> {
        None
    }

    fn sort(&mut self, _: (), _: &mut Context<Self>) {}

    fn line_count(&self) -> usize {
        self.loader.data().map_or(0, |view| view.display.rows.len())
    }

    fn line(&self, line: usize, _: &App) -> Option<Line<String, &NodeRow>> {
        let row = self.loader.data()?.display.rows.get(line)?;
        Some(Line::Row(TableRow {
            key: row.name.clone(),
            id: row.element_id.clone().into(),
            label: row.label.clone(),
            tooltip: Some(row.name.clone().into()),
            marked: false,
            muted: false,
            data: row,
        }))
    }

    fn cell(
        &self,
        line: &TableRow<String, &NodeRow>,
        style: &RowStyle,
        column: &Column,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let row = line.data;
        let p = &style.p;
        let cell = table::cell(column);
        // A value the node didn't report, or that differs, takes its tone
        // only while the row isn't selected.
        let toned = |cell: Div, unknown: bool, warn: bool| {
            cell.when(!style.selected && unknown, |this| {
                this.text_color(p.unk_ink)
            })
            .when(!style.selected && warn, |this| this.text_color(p.warn_ink))
        };
        match column.field {
            Field::Node => cell
                .font_weight(FontWeight::MEDIUM)
                .child(row.name.clone())
                .into_any_element(),
            Field::Role => cell.child(row.role_label()).into_any_element(),
            Field::Talos => toned(cell, row.talos.is_err(), false)
                .child(row.talos_text().to_owned())
                .into_any_element(),
            Field::Kubelet => toned(
                cell,
                row.kubelet.is_err(),
                row.kubelet.is_ok() && row.kubelet_behind,
            )
            .child(row.kubelet_text().to_owned())
            .into_any_element(),
            Field::Config => toned(cell, row.config.is_err(), row.drift == Drift::Differs)
                .child(row.config_text())
                .into_any_element(),
            Field::Discovery => cell
                .flex()
                .items_center()
                .gap(dp(6.))
                .children(ui::status_glyph(presence(row.in_discovery), cx))
                .child(div().text_color(p.muted).child("·"))
                .children(ui::status_glyph(presence(row.in_kubernetes), cx))
                .into_any_element(),
        }
    }

    fn group(&self, _: usize, _: &mut Context<Self>) -> Option<AnyElement> {
        None
    }

    fn selected_key(&self) -> Option<&String> {
        match &self.selected {
            Some(Item::Node(name)) => Some(name),
            _ => None,
        }
    }

    fn line_of(&self, key: &String) -> Option<usize> {
        self.loader
            .data()?
            .display
            .rows
            .iter()
            .position(|row| &row.name == key)
    }

    fn click(&mut self, key: &String, _: &ClickEvent, window: &mut Window, cx: &mut Context<Self>) {
        self.select(Item::Node(key.clone()), window, cx);
    }

    fn loading(&self) -> Option<&table::LoadingRows> {
        self.waiting().then_some(&self.loading)
    }

    fn empty(&self, _: &mut Context<Self>) -> Option<AnyElement> {
        (self.line_count() == 0).then(|| {
            "No node roster is available, so versions, time and configuration are unknown."
                .into_any_element()
        })
    }
}
