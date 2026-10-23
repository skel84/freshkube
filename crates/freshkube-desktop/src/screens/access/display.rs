//! What the table draws, derived from the board whenever it changes.

use freshkube_core::access::{Board, COLUMNS, Cell, Cells, Need, RowKey, RowView};
use gpui_kit::SharedString;

use super::table::{self, Column};
use crate::ui::Tone;

/// A cell of the table: a word, never colour alone.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct CellView {
    pub word: &'static str,
    pub tip: SharedString,
}

/// What the Inspector says of a row.
#[derive(Clone, Debug, Default)]
pub(crate) struct Detail {
    pub headline: SharedString,
    /// Why the row could not be checked, in the server's words.
    pub failure: Option<SharedString>,
    /// Each column with something missing: its label, and one line per need
    /// in the short form.
    pub lacking: Vec<(&'static str, bool, Vec<SharedString>)>,
    pub note: Option<SharedString>,
    /// The plain no's, which an access request asks for.
    pub denied: Vec<Need>,
    pub checking: bool,
}

#[derive(Clone, Debug)]
pub(crate) struct Row {
    pub key: SharedString,
    pub id: String,
    pub row_key: RowKey,
    pub label: SharedString,
    pub namespace: Option<String>,
    pub tone: Tone,
    pub state: SharedString,
    pub cells: Vec<CellView>,
    pub detail: Detail,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Entry {
    Group(usize),
    Row(usize),
}

#[derive(Clone, Debug)]
pub(crate) struct GroupLine {
    pub slug: &'static str,
    pub label: &'static str,
    pub count: usize,
}

pub(crate) struct Display {
    pub rows: Vec<Row>,
    pub lines: Vec<Entry>,
    pub groups: Vec<GroupLine>,
    pub columns: (Vec<Column>, f32),
    /// Rows with an answer, and rows that could not be checked.
    pub checked: usize,
    pub failed: usize,
    /// Some answer came from an incomplete rules review.
    pub incomplete: bool,
}

impl Default for Display {
    fn default() -> Self {
        Self::new(&[], &Board::default())
    }
}

fn cell_view(cell: &Cell, namespace: Option<&str>) -> CellView {
    match cell {
        Cell::Yes => CellView {
            word: "yes",
            tip: "Allowed".into(),
        },
        Cell::No(needs) => CellView {
            word: "no",
            tip: needs
                .iter()
                .map(|need| need.phrase(namespace))
                .collect::<Vec<_>>()
                .join("; ")
                .into(),
        },
        Cell::Ask(needs) => CellView {
            word: "ask",
            tip: format!(
                "Not confirmed: the rules review was incomplete ({}). The check before the action decides.",
                needs
                    .iter()
                    .map(|need| need.phrase(namespace))
                    .collect::<Vec<_>>()
                    .join("; ")
            )
            .into(),
        },
        Cell::NotApplicable => CellView {
            word: "–",
            tip: "Does not apply to nodes".into(),
        },
    }
}

fn placeholder(word: &'static str, tip: &str) -> Vec<CellView> {
    COLUMNS
        .iter()
        .map(|_| CellView {
            word,
            tip: tip.to_owned().into(),
        })
        .collect()
}

fn detail_of(cells: &Cells, namespace: Option<&str>) -> Detail {
    let lacking = cells
        .missing()
        .into_iter()
        .map(|(column, needs)| {
            let ask = matches!(cells.cell(column), Cell::Ask(_));
            (
                column.label(),
                ask,
                needs
                    .iter()
                    .map(|need| need.phrase(namespace).into())
                    .collect(),
            )
        })
        .collect();
    let headline = if cells
        .cells
        .iter()
        .all(|c| matches!(c, Cell::Yes | Cell::NotApplicable))
    {
        "Everything here is allowed"
    } else if cells.incomplete {
        "Rules incomplete: what is not shown allowed may still be"
    } else {
        "Some actions are not allowed"
    };
    Detail {
        headline: headline.into(),
        failure: None,
        lacking,
        note: cells.note.clone().map(Into::into),
        denied: cells.denied(),
        checking: false,
    }
}

fn tone_of(cells: &Cells) -> (Tone, &'static str) {
    if cells.cells.iter().any(|c| matches!(c, Cell::Ask(_))) {
        (Tone::Unknown, "Some answers are not confirmed")
    } else if cells.cells.iter().any(|c| matches!(c, Cell::No(_))) {
        (Tone::Info, "Some actions are not allowed")
    } else {
        (Tone::Good, "Everything here is allowed")
    }
}

fn row_of(key: RowKey, board: &Board) -> Row {
    let (id, label, namespace) = match &key {
        RowKey::Nodes => (
            "access-row-cluster-nodes".to_owned(),
            "Nodes".to_owned(),
            None,
        ),
        RowKey::Namespace(name) => (
            format!("access-row-ns-{name}"),
            name.clone(),
            Some(name.clone()),
        ),
    };
    let namespace_ref = namespace.as_deref();
    let (tone, state, cells, detail) = match board.view(&key) {
        RowView::Checking { earlier: None } => (
            Tone::Waiting,
            "Checking".to_owned(),
            placeholder("…", "Checking"),
            Detail {
                headline: "Checking".into(),
                checking: true,
                ..Detail::default()
            },
        ),
        RowView::Checking {
            earlier: Some(cells),
        } => {
            let mut detail = detail_of(cells, namespace_ref);
            detail.checking = true;
            (
                Tone::Waiting,
                "Checking again".to_owned(),
                cells
                    .cells
                    .iter()
                    .map(|cell| cell_view(cell, namespace_ref))
                    .collect(),
                detail,
            )
        }
        RowView::Known(cells) => {
            let (tone, state) = tone_of(cells);
            (
                tone,
                state.to_owned(),
                cells
                    .cells
                    .iter()
                    .map(|cell| cell_view(cell, namespace_ref))
                    .collect(),
                detail_of(cells, namespace_ref),
            )
        }
        RowView::Failed(message) => (
            Tone::Warn,
            "Couldn't check".to_owned(),
            placeholder("?", &format!("Couldn't check: {message}")),
            Detail {
                headline: "Couldn't check".into(),
                failure: Some(message.to_owned().into()),
                ..Detail::default()
            },
        ),
    };
    Row {
        key: id.clone().into(),
        id,
        row_key: key,
        label: label.into(),
        namespace,
        tone,
        state: state.into(),
        cells,
        detail,
    }
}

impl Display {
    pub(super) fn new(names: &[String], board: &Board) -> Self {
        let mut rows = vec![row_of(RowKey::Nodes, board)];
        rows.extend(
            names
                .iter()
                .map(|name| row_of(RowKey::Namespace(name.clone()), board)),
        );
        let mut lines = vec![Entry::Group(0), Entry::Row(0)];
        let mut groups = vec![GroupLine {
            slug: "cluster",
            label: "Cluster-wide",
            count: 1,
        }];
        if !names.is_empty() {
            groups.push(GroupLine {
                slug: "namespaces",
                label: "Namespaces",
                count: names.len(),
            });
            lines.push(Entry::Group(1));
            lines.extend((1..rows.len()).map(Entry::Row));
        }
        let checked = rows
            .iter()
            .filter(|row| matches!(board.view(&row.row_key), RowView::Known(_)))
            .count();
        let failed = rows
            .iter()
            .filter(|row| matches!(board.view(&row.row_key), RowView::Failed(_)))
            .count();
        let incomplete = rows.iter().any(
            |row| matches!(board.view(&row.row_key), RowView::Known(cells) if cells.incomplete),
        );
        let columns = table::columns(&rows);
        Self {
            rows,
            lines,
            groups,
            columns,
            checked,
            failed,
            incomplete,
        }
    }
}
