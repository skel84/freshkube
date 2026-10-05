//! The table's columns for a read: which printed columns show, in what
//! order and how wide, derived when a read resets.

use gpui_kit::SharedString;

use super::super::model::{ColumnKind, SortKey};
use super::super::store::ResourceStore;

/// Advance of one character in the 12.5 px table font.
const CHAR_WIDTH: f32 = 7.5;
const CELL_PADDING: f32 = 24.;
const AGE_WIDTH: f32 = 64.;
const MIN_COLUMN: f32 = 64.;
const MAX_COLUMN: f32 = 280.;
const MAX_FLEXIBLE: f32 = 440.;
/// The status glyph's column: 16 for the glyph and its padding.
pub(super) use freshkube_ui::table::GLYPH_WIDTH;
/// A pod's `0/1 ↻14`.
const READY_WIDTH: f32 = 88.;
/// A use figure and its 44-wide bullet.
const USAGE_WIDTH: f32 = 116.;
const MAX_OWNER: f32 = 132.;
/// A pod's name and namespace take what is left, but no less than this.
const MIN_POD_NAME: f32 = 224.;
const MAX_NODE: f32 = 140.;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(super) enum ColumnSource {
    /// A printed column, by index into the store's columns.
    Cell(usize),
    /// The printed name, after its namespace when listing every namespace,
    /// and for a pod, the reason it isn't healthy.
    Name(usize),
    /// The namespace, for kinds that print no name, such as events.
    Namespace,
    /// The status glyph, or a check where the row is marked.
    Glyph,
    Owner,
    /// A pod's ready containers and restarts.
    Ready,
    Cpu,
    Memory,
    Node,
}

#[derive(Clone, Debug)]
pub(crate) struct DisplayColumn {
    pub(super) label: SharedString,
    pub(super) source: ColumnSource,
    pub(super) kind: ColumnKind,
    pub(super) width: f32,
    /// Takes the room left over; at most one column does.
    pub(super) flexible: bool,
    /// Printed statuses are coloured by what they mean.
    pub(super) status: bool,
}

impl DisplayColumn {
    /// The order a header click sorts by; the glyph's column has none. A
    /// name shown after its namespace sorts by both.
    pub(super) fn sort_key(&self, namespaced: bool) -> Option<SortKey> {
        Some(match self.source {
            ColumnSource::Name(_) if namespaced => SortKey::Namespace,
            ColumnSource::Cell(ix) | ColumnSource::Name(ix) => SortKey::Column(ix),
            ColumnSource::Namespace => SortKey::Namespace,
            ColumnSource::Glyph => return None,
            ColumnSource::Owner => SortKey::Owner,
            ColumnSource::Ready => SortKey::Restarts,
            ColumnSource::Cpu => SortKey::Cpu,
            ColumnSource::Memory => SortKey::Memory,
            ColumnSource::Node => SortKey::Node,
        })
    }

    fn new(label: &str, source: ColumnSource, width: f32) -> Self {
        Self {
            label: label.to_owned().into(),
            source,
            kind: ColumnKind::Text,
            width,
            flexible: false,
            status: false,
        }
    }
}

impl freshkube_ui::table::TableColumn for DisplayColumn {
    fn label(&self) -> &SharedString {
        &self.label
    }

    fn width(&self) -> f32 {
        self.width
    }

    fn flexible(&self) -> bool {
        self.flexible
    }

    /// The glyph and the name stay in view when the table scrolls sideways.
    fn pinned(&self) -> bool {
        matches!(self.source, ColumnSource::Glyph | ColumnSource::Name(_))
    }
}

/// Where a row's glyph comes from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum ToneSource {
    /// A printed status or phase.
    Status(usize),
    /// A workload's ready replicas of all, `2/3`.
    Ready(usize),
}

/// The columns drawn for the current read and their widths. Derived when a
/// read resets, never while drawing; wide (`-o wide`) columns are left out.
#[derive(Clone, Debug, Default)]
pub(super) struct TableLayout {
    pub(super) columns: Vec<DisplayColumn>,
    pub(super) all_columns: Vec<DisplayColumn>,
    pub(super) width: f32,
    /// The printed column that gives a row its glyph, for kinds other than
    /// pods, which have their own state.
    pub(super) tone_from: Option<ToneSource>,
    /// The name shows its namespace before it.
    pub(super) namespaced: bool,
}

impl TableLayout {
    /// Pods show a glyph, their name with its namespace, owner, readiness
    /// and restarts together, use, node and age. Other kinds show a glyph
    /// where they print a status, their name, an owner where any row has
    /// one, and their printed columns.
    pub(super) fn new(store: &ResourceStore, namespace_column: bool, pods: bool) -> Self {
        let widest = store.widest();
        let fit = |chars: usize, max: f32| {
            (chars as f32 * CHAR_WIDTH + CELL_PADDING).clamp(MIN_COLUMN, max)
        };
        let printed: Vec<_> = store
            .columns()
            .iter()
            .enumerate()
            .filter(|(_, column)| !column.wide)
            .collect();
        let named = |name: &str| {
            printed
                .iter()
                .find(|(_, column)| column.name.eq_ignore_ascii_case(name))
                .map(|(ix, _)| *ix)
        };
        let name = named("name");
        let status = named("status").or_else(|| named("phase"));
        let tone_from = status
            .map(ToneSource::Status)
            .or_else(|| named("ready").map(ToneSource::Ready));
        let flexible = named("message")
            .or(name)
            .or_else(|| printed.first().map(|(ix, _)| *ix));
        let namespaced = namespace_column && name.is_some();
        let mut columns = Vec::with_capacity(printed.len() + 4);
        if pods || tone_from.is_some() {
            columns.push(DisplayColumn::new("", ColumnSource::Glyph, GLYPH_WIDTH));
        }
        let name_chars = |ix: usize| {
            widest.cells.get(ix).copied().unwrap_or(0)
                + if namespaced { widest.namespace + 1 } else { 0 }
        };
        for (position, (ix, column)) in printed.iter().enumerate() {
            let (ix, first) = (*ix, position == 0);
            let is_flexible = flexible == Some(ix);
            let is_name = name == Some(ix);
            // A pod's status follows its name, and its readiness and
            // restarts share a column.
            if pods && !is_name && column.kind != ColumnKind::Age {
                continue;
            }
            let width = match column.kind {
                ColumnKind::Age => AGE_WIDTH,
                // Long names truncate, their tooltip holding them whole.
                _ if pods && is_name => MIN_POD_NAME,
                _ => fit(
                    if is_name {
                        name_chars(ix)
                    } else {
                        widest.cells.get(ix).copied().unwrap_or(0)
                    }
                    // Room for the sort arrow beside the label.
                    .max(column.name.chars().count() + 2),
                    if is_flexible {
                        MAX_FLEXIBLE
                    } else {
                        MAX_COLUMN
                    },
                ),
            };
            if column.kind == ColumnKind::Age && pods {
                columns.extend(pod_columns(store));
            }
            columns.push(DisplayColumn {
                label: column.name.clone().into(),
                source: if is_name {
                    ColumnSource::Name(ix)
                } else {
                    ColumnSource::Cell(ix)
                },
                kind: column.kind,
                width,
                flexible: is_flexible,
                status: status == Some(ix),
            });
            if is_name && !pods && widest.owner > 0 {
                columns.push(DisplayColumn::new(
                    "Owner",
                    ColumnSource::Owner,
                    fit(widest.owner.max(7), MAX_OWNER),
                ));
            }
            // A kind that prints no name shows the namespace second.
            if namespace_column && !namespaced && first {
                columns.push(DisplayColumn::new(
                    "Namespace",
                    ColumnSource::Namespace,
                    fit(widest.namespace.max(11), MAX_COLUMN),
                ));
            }
        }
        let width = columns.iter().map(|column| column.width).sum();
        Self {
            all_columns: columns.clone(),
            columns,
            width,
            tone_from: tone_from.filter(|_| !pods),
            namespaced,
        }
    }

    pub(super) fn hide(&mut self, hidden: &std::collections::BTreeSet<ColumnSource>) {
        self.columns = self
            .all_columns
            .iter()
            .filter(|column| !hidden.contains(&column.source))
            .cloned()
            .collect();
        self.width = self.columns.iter().map(|column| column.width).sum();
    }
}

/// A pod's columns between its name and its age.
fn pod_columns(store: &ResourceStore) -> [DisplayColumn; 5] {
    let widest = store.widest();
    let fit =
        |chars: usize, max: f32| (chars as f32 * CHAR_WIDTH + CELL_PADDING).clamp(MIN_COLUMN, max);
    let node = widest.node.saturating_sub(store.node_prefix());
    [
        DisplayColumn::new(
            "Owner",
            ColumnSource::Owner,
            fit(widest.owner.max(7), MAX_OWNER),
        ),
        DisplayColumn::new("Ready", ColumnSource::Ready, READY_WIDTH),
        DisplayColumn::new("CPU", ColumnSource::Cpu, USAGE_WIDTH),
        DisplayColumn::new("Memory", ColumnSource::Memory, USAGE_WIDTH),
        // Room for the NotReady mark before a name.
        DisplayColumn::new("Node", ColumnSource::Node, fit(node.max(6) + 2, MAX_NODE)),
    ]
}
