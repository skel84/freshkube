//! The table's columns for a read: which printed columns show, in what
//! order and how wide, derived when a read resets.

use gpui_kit::SharedString;

use super::super::model::{ColumnKind, SortKey};
use super::super::store::ResourceStore;

/// Advance of one character in the 12 px table font.
const CHAR_WIDTH: f32 = 7.2;
const CELL_PADDING: f32 = 24.;
const AGE_WIDTH: f32 = 76.;
const MIN_COLUMN: f32 = 64.;
const MAX_COLUMN: f32 = 280.;
const MAX_FLEXIBLE: f32 = 440.;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum ColumnSource {
    /// A printed column, by index into the store's columns.
    Cell(usize),
    /// The namespace, added when listing every namespace.
    Namespace,
}

#[derive(Clone, Debug)]
pub(super) struct DisplayColumn {
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
    pub(super) fn sort_key(&self) -> SortKey {
        match self.source {
            ColumnSource::Cell(ix) => SortKey::Column(ix),
            ColumnSource::Namespace => SortKey::Namespace,
        }
    }
}

/// The columns drawn for the current read and their widths. Derived when a
/// read resets, never while drawing; wide (`-o wide`) columns are left out.
#[derive(Clone, Debug, Default)]
pub(super) struct TableLayout {
    pub(super) columns: Vec<DisplayColumn>,
    pub(super) width: f32,
}

impl TableLayout {
    pub(super) fn new(store: &ResourceStore, namespace_column: bool) -> Self {
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
        let flexible = named("message")
            .or_else(|| named("name"))
            .or_else(|| printed.first().map(|(ix, _)| *ix));
        let mut columns = Vec::with_capacity(printed.len() + 1);
        for (ix, column) in printed {
            let is_flexible = flexible == Some(ix);
            let width = match column.kind {
                ColumnKind::Age => AGE_WIDTH,
                _ => fit(
                    widest
                        .cells
                        .get(ix)
                        .copied()
                        .unwrap_or(0)
                        // Room for the sort arrow beside the label.
                        .max(column.name.chars().count() + 2),
                    if is_flexible {
                        MAX_FLEXIBLE
                    } else {
                        MAX_COLUMN
                    },
                ),
            };
            columns.push(DisplayColumn {
                label: column.name.clone().into(),
                source: ColumnSource::Cell(ix),
                kind: column.kind,
                width,
                flexible: is_flexible,
                status: matches!(
                    column.name.to_ascii_lowercase().as_str(),
                    "status" | "phase"
                ),
            });
            if namespace_column && columns.len() == 1 {
                columns.push(DisplayColumn {
                    label: "Namespace".into(),
                    source: ColumnSource::Namespace,
                    kind: ColumnKind::Text,
                    width: fit(widest.namespace.max(11), MAX_COLUMN),
                    flexible: false,
                    status: false,
                });
            }
        }
        let width = columns.iter().map(|column| column.width).sum();
        Self { columns, width }
    }
}
