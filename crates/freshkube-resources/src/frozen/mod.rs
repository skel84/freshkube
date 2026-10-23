//! Frozen rows (#877): what a Resources tab keeps of its list when it stops
//! showing, so coming back shows it at once, muted and marked last known,
//! while the list is read again.
//!
//! The bounds are fixed: at most [`MAX_TABS`] tabs keep rows, a list of more
//! than [`MAX_ROWS`] rows keeps none, and every tab's rows together are
//! kept within [`MAX_BYTES`]. A tab gets an even share of that budget, so a
//! list too big for its share keeps its first rows and says how many.

use std::collections::HashMap;
use std::time::SystemTime;

use super::model::{ResourceColumn, ResourceIdentity, ResourceRow};

/// How many tabs keep their rows.
pub const MAX_TABS: usize = 8;
/// A list with more rows than this keeps none.
pub const MAX_ROWS: usize = 5_000;
/// What every tab's kept rows may add up to, estimated.
pub const MAX_BYTES: usize = 64 * 1024 * 1024;
/// One tab's share of [`MAX_BYTES`].
const TAB_BYTES: usize = MAX_BYTES / MAX_TABS;

/// A list as one tab left it.
#[derive(Clone, Debug)]
pub struct FrozenRows {
    /// The kind's kubectl key.
    pub(crate) kind: String,
    /// The namespace listed, if the kind has them and one was chosen.
    pub(crate) namespace: Option<String>,
    /// The connection the rows were read from.
    pub(crate) connection: String,
    pub(crate) columns: Vec<ResourceColumn>,
    pub(crate) rows: Vec<ResourceRow>,
    /// How many rows the list had.
    pub(crate) total: usize,
    /// When the list was last current.
    pub(crate) captured: SystemTime,
    bytes: usize,
}

impl FrozenRows {
    /// Freezes `rows` as listed, or nothing for a list over [`MAX_ROWS`] or
    /// without rows. Rows past the tab's share of the budget are left out.
    pub(crate) fn capture(
        (kind, namespace, connection): (String, Option<String>, String),
        columns: Vec<ResourceColumn>,
        rows: impl IntoIterator<Item = ResourceRow>,
        total: usize,
        captured: SystemTime,
    ) -> Option<Self> {
        if total == 0 || total > MAX_ROWS {
            return None;
        }
        let mut bytes = 0;
        let kept: Vec<_> = rows
            .into_iter()
            .take_while(|row| {
                bytes += estimate(row);
                bytes <= TAB_BYTES
            })
            .collect();
        let bytes = kept.iter().map(estimate).sum();
        (!kept.is_empty()).then_some(Self {
            kind,
            namespace,
            connection,
            columns,
            rows: kept,
            total,
            captured,
            bytes,
        })
    }

    /// How many rows are kept.
    pub fn kept(&self) -> usize {
        self.rows.len()
    }

    /// How many rows the list had.
    pub fn total(&self) -> usize {
        self.total
    }

    /// The estimated size of the kept rows.
    pub fn bytes(&self) -> usize {
        self.bytes
    }

    /// When the list was last current.
    pub fn captured(&self) -> SystemTime {
        self.captured
    }

    pub fn kind(&self) -> &str {
        &self.kind
    }

    /// Each kept row's version, for telling which rows changed meanwhile.
    pub(crate) fn versions(&self) -> HashMap<ResourceIdentity, String> {
        self.rows
            .iter()
            .map(|row| (row.identity.clone(), row.resource_version.clone()))
            .collect()
    }
}

/// What a row takes in memory, about: its strings and a fixed part for the
/// rest. An estimate, to bound the total, not to account for it.
pub(crate) fn estimate(row: &ResourceRow) -> usize {
    let identity = &row.identity;
    let strings = identity.connection.len()
        + identity.resource.len()
        + identity.namespace.len()
        + identity.name.len()
        + identity.uid.len()
        + row.resource_version.len()
        + row.cells.iter().map(|cell| cell.len() + 24).sum::<usize>();
    let owner = row.owner.as_ref().map_or(0, |owner| {
        owner.short.len() + owner.kind.len() + owner.name.len() + 72
    });
    let pod = row.pod.as_ref().map_or(0, |_| 512);
    // Search key and layout widths add about the printed text again.
    512 + 2 * strings + owner + pod
}

/// The tabs' frozen rows, the tab that froze last last. Over a bound, the
/// tab that froze longest ago loses its rows first.
#[derive(Debug)]
pub struct FrozenSet<K> {
    tabs: Vec<(K, FrozenRows)>,
}

impl<K> Default for FrozenSet<K> {
    fn default() -> Self {
        Self { tabs: Vec::new() }
    }
}

impl<K: PartialEq> FrozenSet<K> {
    /// Keeps `rows` for `tab`, in place of what it kept before.
    pub fn insert(&mut self, tab: K, rows: FrozenRows) {
        self.tabs.retain(|(key, _)| *key != tab);
        self.tabs.push((tab, rows));
        while self.tabs.len() > MAX_TABS || (self.bytes() > MAX_BYTES && self.tabs.len() > 1) {
            self.tabs.remove(0);
        }
    }

    /// Takes the rows `tab` kept: they are shown now, and freeze again when
    /// it leaves.
    pub fn take(&mut self, tab: &K) -> Option<FrozenRows> {
        let at = self.tabs.iter().position(|(key, _)| key == tab)?;
        Some(self.tabs.remove(at).1)
    }

    pub fn forget(&mut self, tab: &K) {
        self.tabs.retain(|(key, _)| key != tab);
    }

    /// Keeps the rows of the tabs `keep` names.
    pub fn retain(&mut self, mut keep: impl FnMut(&K) -> bool) {
        self.tabs.retain(|(key, _)| keep(key));
    }

    pub fn clear(&mut self) {
        self.tabs.clear();
    }

    pub fn contains(&self, tab: &K) -> bool {
        self.tabs.iter().any(|(key, _)| key == tab)
    }

    pub fn len(&self) -> usize {
        self.tabs.len()
    }

    pub fn is_empty(&self) -> bool {
        self.tabs.is_empty()
    }

    pub fn bytes(&self) -> usize {
        self.tabs.iter().map(|(_, rows)| rows.bytes).sum()
    }
}

#[cfg(test)]
mod tests;
