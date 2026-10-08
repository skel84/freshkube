use std::collections::HashMap;

use freshkube_core::resources::Amounts;

use super::model::{ColumnKind, ReadState, ResourceColumn, ResourceIdentity, ResourceRow};
use super::rows;

/// One change to observed state, in the shape of a list/watch stream:
/// individual upserts and deletes, an atomic reset that replaces the columns
/// and every row, and changes to the collection's read state.
#[derive(Clone, Debug)]
pub(crate) enum ResourceEvent {
    Reset(Snapshot),
    Upsert(ResourceRow),
    Delete(ResourceIdentity),
    Read(ReadState),
}

impl ResourceEvent {
    /// A reset to these columns and rows. Build it where the list arrives,
    /// off the main thread: it does the work of a whole list.
    pub(crate) fn reset(columns: Vec<ResourceColumn>, rows: Vec<ResourceRow>) -> Self {
        Self::Reset(Snapshot::new(columns, rows))
    }
}

/// A list's rows made ready to replace a store's: search keys built,
/// duplicates merged, identities indexed and printed widths counted, so
/// applying it only swaps it in.
#[derive(Clone, Debug)]
pub(crate) struct Snapshot {
    columns: Vec<ResourceColumn>,
    entries: Vec<ResourceEntry>,
    index: HashMap<ResourceIdentity, usize>,
    widest: Widest,
}

impl Snapshot {
    // Duplicate identities keep the first occurrence's position with the last
    // occurrence's observation, as a relist that saw an object twice would.
    fn new(columns: Vec<ResourceColumn>, rows: Vec<ResourceRow>) -> Self {
        let mut entries: Vec<ResourceEntry> = Vec::with_capacity(rows.len());
        let mut index: HashMap<ResourceIdentity, usize> = HashMap::with_capacity(rows.len());
        for row in rows {
            if let Some(&slot) = index.get(&row.identity) {
                let seq = entries[slot].seq;
                entries[slot] = ResourceEntry::new(row, seq);
            } else {
                index.insert(row.identity.clone(), entries.len());
                entries.push(ResourceEntry::new(row, entries.len() as u64));
            }
        }
        let mut widest = Widest::default();
        for entry in &entries {
            widest.include(&entry.row);
        }
        Self {
            columns,
            entries,
            index,
            widest,
        }
    }

    #[cfg(test)]
    pub(crate) fn len(&self) -> usize {
        self.entries.len()
    }
}

/// The longest printed text of a store's rows, in characters: per cell
/// column, and of the namespace. Column widths come from it without a pass
/// over every row.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct Widest {
    pub(crate) cells: Vec<usize>,
    pub(crate) namespace: usize,
    /// The Owner column's `deploy/worker`.
    pub(crate) owner: usize,
    /// A pod's node, in full.
    pub(crate) node: usize,
}

impl Widest {
    fn include(&mut self, row: &ResourceRow) {
        if self.cells.len() < row.cells.len() {
            self.cells.resize(row.cells.len(), 0);
        }
        for (widest, cell) in self.cells.iter_mut().zip(&row.cells) {
            *widest = (*widest).max(cell.chars().count());
        }
        self.namespace = self.namespace.max(row.identity.namespace.chars().count());
        if let Some(owner) = &row.owner {
            let label = owner.short.chars().count() + 1 + owner.name.chars().count();
            self.owner = self.owner.max(label);
        }
        if let Some(pod) = &row.pod {
            self.node = self.node.max(pod.node.chars().count());
        }
    }
}

/// The printed columns that tell how an object is doing, for kinds other
/// than pods: a change to one of them flashes the row.
const STATUS_COLUMNS: &[&str] = &[
    "Status",
    "Ready",
    "Phase",
    "Up-to-date",
    "Available",
    "Completions",
    "Conditions",
];

/// Whether `new` changes how `old` is doing: a pod's state, reason or
/// restarts, or another kind's [`STATUS_COLUMNS`].
fn state_changed(old: &ResourceRow, new: &ResourceRow, columns: &[ResourceColumn]) -> bool {
    if let (Some(old), Some(new)) = (&old.pod, &new.pod) {
        return old.state != new.state || old.reason != new.reason || old.restarts != new.restarts;
    }
    columns
        .iter()
        .enumerate()
        .filter(|(_, column)| STATUS_COLUMNS.contains(&column.name.as_str()))
        .any(|(ix, _)| old.cells.get(ix) != new.cells.get(ix))
}

/// What one part of a batch changed: the rows, or, past the most the
/// caller keeps, only how many, so nothing is cloned for a batch the
/// flash would hold back anyway.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Changed {
    Rows(Vec<ResourceIdentity>),
    Many(usize),
}

/// A retained observation plus a search key derived once when it arrives, so
/// filtering never allocates per row.
#[derive(Clone, Debug)]
pub(crate) struct ResourceEntry {
    row: ResourceRow,
    search_key: String,
    /// Observation order: position in the last reset, then arrival order.
    seq: u64,
    /// A pod's use, as metrics-server last reported it.
    usage: Option<Amounts>,
}

impl ResourceEntry {
    fn new(row: ResourceRow, seq: u64) -> Self {
        // Fields are joined by a separator a single-line filter cannot contain,
        // so a query never matches across two fields.
        let mut search_key = format!("{}\n{}", row.identity.namespace, row.identity.name);
        for cell in &row.cells {
            search_key.push('\n');
            search_key.push_str(cell);
        }
        Self {
            search_key: search_key.to_lowercase(),
            row,
            seq,
            usage: None,
        }
    }

    pub(crate) fn row(&self) -> &ResourceRow {
        &self.row
    }

    /// Lowercased namespace, name and printed cells.
    pub(crate) fn search_key(&self) -> &str {
        &self.search_key
    }

    pub(crate) fn seq(&self) -> u64 {
        self.seq
    }

    pub(crate) fn usage(&self) -> Option<&Amounts> {
        self.usage.as_ref()
    }
}

/// Pods' use by namespace, then name, so an entry finds its own without
/// building a key.
pub(crate) type Usage = HashMap<String, HashMap<String, Amounts>>;

/// Observations for one session of one kind. Only batches change it, and
/// every applied batch advances `revision`, which tells a projection that its
/// slot indices are out of date.
#[derive(Debug)]
pub(crate) struct ResourceStore {
    epoch: u64,
    revision: u64,
    read_state: ReadState,
    columns: Vec<ResourceColumn>,
    entries: Vec<ResourceEntry>,
    index: HashMap<ResourceIdentity, usize>,
    next_seq: u64,
    /// Over every row since the last reset; deletes don't narrow it.
    widest: Widest,
    /// The last use metrics-server reported, kept for pods that arrive later.
    usage: Usage,
    /// How much of every pod's node name all of them share, such as
    /// `talos-`, which the Node column leaves out.
    node_prefix: usize,
}

impl ResourceStore {
    pub(crate) fn new() -> Self {
        Self {
            epoch: 0,
            revision: 0,
            read_state: ReadState::Loading,
            columns: Vec::new(),
            entries: Vec::new(),
            index: HashMap::new(),
            next_seq: 0,
            widest: Widest::default(),
            usage: Usage::new(),
            node_prefix: 0,
        }
    }

    /// An empty store printing the columns every kind prints, Name and
    /// Age: the header a list shows over its loading rows until its first
    /// answer brings the kind's own.
    pub(crate) fn provisional() -> Self {
        Self {
            columns: vec![
                ResourceColumn::new("Name", ColumnKind::Text, false),
                ResourceColumn::new("Age", ColumnKind::Age, false),
            ],
            ..Self::new()
        }
    }

    /// Replaces the session: clears observations and returns the epoch that
    /// batches for the new session must carry.
    pub(crate) fn start_session(&mut self) -> u64 {
        self.epoch += 1;
        self.revision += 1;
        self.read_state = ReadState::Loading;
        self.columns.clear();
        self.entries.clear();
        self.index.clear();
        self.next_seq = 0;
        self.widest = Widest::default();
        self.usage.clear();
        self.node_prefix = 0;
        self.epoch
    }

    #[cfg(test)]
    pub(crate) fn epoch(&self) -> u64 {
        self.epoch
    }

    pub(crate) fn revision(&self) -> u64 {
        self.revision
    }

    pub(crate) fn read_state(&self) -> &ReadState {
        &self.read_state
    }

    pub(crate) fn columns(&self) -> &[ResourceColumn] {
        &self.columns
    }

    pub(crate) fn len(&self) -> usize {
        self.entries.len()
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub(crate) fn entries(&self) -> &[ResourceEntry] {
        &self.entries
    }

    pub(crate) fn widest(&self) -> &Widest {
        &self.widest
    }

    pub(crate) fn node_prefix(&self) -> usize {
        self.node_prefix
    }

    /// Replaces pods' use as one revision, so a sort by it is redone.
    pub(crate) fn set_usage(&mut self, usage: Usage) {
        self.usage = usage;
        for entry in &mut self.entries {
            entry.usage = Self::usage_of(&self.usage, &entry.row);
        }
        self.revision += 1;
    }

    fn usage_of(usage: &Usage, row: &ResourceRow) -> Option<Amounts> {
        usage
            .get(&row.identity.namespace)
            .and_then(|names| names.get(&row.identity.name))
            .copied()
    }

    pub(crate) fn slot(&self, identity: &ResourceIdentity) -> Option<usize> {
        self.index.get(identity).copied()
    }

    pub(crate) fn get(&self, identity: &ResourceIdentity) -> Option<&ResourceRow> {
        self.slot(identity).map(|slot| &self.entries[slot].row)
    }

    /// Applies the parts' events in order as one revision. Returns `None`,
    /// changing nothing, when they belong to another session: a producer
    /// captures the epoch [`start_session`](Self::start_session) returned
    /// when its work began, so a late result from a replaced connection,
    /// kind or namespace cannot populate the current view. Epoch 0 means
    /// no session has started, so nothing is accepted.
    ///
    /// For each part it returns the rows whose state changed in it, or
    /// their count when there are more than `keep`: a
    /// pod's state, reason or restarts, or another kind's status cells
    /// ([`STATUS_COLUMNS`]). A part is what one watch batch sent, so a list
    /// can flash each as it would have flashed alone. A reset reports
    /// nothing before it, in its part or earlier ones: the list was read
    /// again. A row that arrives or goes isn't a change, nor is one that
    /// changed only its other cells.
    pub(crate) fn apply(
        &mut self,
        epoch: u64,
        parts: Vec<Vec<ResourceEvent>>,
        keep: usize,
    ) -> Option<Vec<Changed>> {
        if self.epoch == 0 || epoch != self.epoch {
            return None;
        }
        // Changed rows by slot, kept current as deletes move rows, so
        // nothing is cloned until the end.
        let mut changes: Vec<Vec<usize>> = Vec::with_capacity(parts.len());
        for events in parts {
            changes.push(Vec::new());
            for event in events {
                match event {
                    ResourceEvent::Reset(snapshot) => {
                        changes.iter_mut().for_each(Vec::clear);
                        self.reset(snapshot)
                    }
                    ResourceEvent::Upsert(row) => {
                        if let Some(slot) = self.upsert(row) {
                            changes.last_mut().expect("a part").push(slot);
                        }
                    }
                    ResourceEvent::Delete(identity) => {
                        if let Some((gone, moved)) = self.delete(&identity) {
                            for slot in changes.iter_mut().flatten() {
                                if *slot == gone {
                                    *slot = usize::MAX;
                                } else if *slot == moved {
                                    *slot = gone;
                                }
                            }
                        }
                    }
                    // A failure with rows on screen leaves them up, marked stale.
                    ResourceEvent::Read(ReadState::Failed(reason)) if !self.entries.is_empty() => {
                        self.read_state = ReadState::Stale(reason)
                    }
                    ResourceEvent::Read(state) => self.read_state = state,
                }
            }
        }
        self.node_prefix = rows::shared_prefix(
            self.entries
                .iter()
                .filter_map(|entry| entry.row.pod.as_ref())
                .map(|pod| pod.node.as_str()),
        );
        self.revision += 1;
        // A part that changed a row twice reports it once; a deleted row
        // not at all.
        let changes = changes
            .into_iter()
            .map(|mut slots| {
                slots.sort_unstable();
                slots.dedup();
                // Gone rows sort last.
                let count = slots.partition_point(|slot| *slot != usize::MAX);
                if count > keep {
                    return Changed::Many(count);
                }
                Changed::Rows(
                    slots[..count]
                        .iter()
                        .map(|slot| self.entries[*slot].row.identity.clone())
                        .collect(),
                )
            })
            .collect();
        Some(changes)
    }

    /// Stores the row, and returns its slot when it replaced one whose
    /// state it changes.
    fn upsert(&mut self, row: ResourceRow) -> Option<usize> {
        self.widest.include(&row);
        let usage = Self::usage_of(&self.usage, &row);
        let (slot, changed) = if let Some(slot) = self.slot(&row.identity) {
            let old = &self.entries[slot];
            let changed = state_changed(&old.row, &row, &self.columns);
            let seq = old.seq;
            self.entries[slot] = ResourceEntry::new(row, seq);
            (slot, changed)
        } else {
            self.index.insert(row.identity.clone(), self.entries.len());
            self.entries.push(ResourceEntry::new(row, self.next_seq));
            self.next_seq += 1;
            (self.entries.len() - 1, false)
        };
        self.entries[slot].usage = usage;
        changed.then_some(slot)
    }

    /// Removes the row, and returns its slot and the slot of the row that
    /// moved into it, which was the last.
    fn delete(&mut self, identity: &ResourceIdentity) -> Option<(usize, usize)> {
        let slot = self.index.remove(identity)?;
        self.entries.swap_remove(slot);
        if let Some(moved) = self.entries.get(slot) {
            self.index.insert(moved.row.identity.clone(), slot);
        }
        Some((slot, self.entries.len()))
    }

    fn reset(&mut self, snapshot: Snapshot) {
        self.next_seq = snapshot.entries.len() as u64;
        self.columns = snapshot.columns;
        self.entries = snapshot.entries;
        self.index = snapshot.index;
        self.widest = snapshot.widest;
        if !self.usage.is_empty() {
            for entry in &mut self.entries {
                entry.usage = Self::usage_of(&self.usage, &entry.row);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::resources::example::{inserted_pod, pod_columns, pod_rows};

    fn loaded(rows: Vec<ResourceRow>) -> ResourceStore {
        let mut store = ResourceStore::new();
        let epoch = store.start_session();
        assert!(
            store
                .apply(
                    epoch,
                    vec![vec![
                        ResourceEvent::reset(pod_columns(), rows),
                        ResourceEvent::Read(ReadState::Loaded)
                    ]],
                    usize::MAX,
                )
                .is_some()
        );
        store
    }

    fn apply(store: &mut ResourceStore, events: Vec<ResourceEvent>) {
        let epoch = store.epoch();
        assert!(store.apply(epoch, vec![events], usize::MAX).is_some());
    }

    fn reset(rows: Vec<ResourceRow>) -> ResourceEvent {
        ResourceEvent::reset(pod_columns(), rows)
    }

    fn assert_index_consistent(store: &ResourceStore) {
        assert_eq!(store.index.len(), store.entries.len());
        for (slot, entry) in store.entries.iter().enumerate() {
            assert_eq!(store.slot(&entry.row.identity), Some(slot));
        }
    }

    #[test]
    fn nothing_is_accepted_before_a_session_starts() {
        let mut store = ResourceStore::new();
        assert!(
            !store
                .apply(0, vec![vec![reset(pod_rows(3))]], usize::MAX)
                .is_some()
        );
        assert!(store.is_empty());
    }

    #[test]
    fn a_batch_from_a_replaced_session_changes_nothing() {
        let mut store = loaded(pod_rows(10));
        let old_epoch = store.epoch();
        let new_epoch = store.start_session();
        assert_ne!(old_epoch, new_epoch);
        assert!(store.is_empty());
        assert!(store.columns().is_empty());
        assert_eq!(store.read_state(), &ReadState::Loading);
        let revision = store.revision();
        assert!(
            !store
                .apply(
                    old_epoch,
                    vec![vec![
                        reset(pod_rows(10)),
                        ResourceEvent::Read(ReadState::Loaded)
                    ]],
                    usize::MAX,
                )
                .is_some()
        );
        assert!(store.is_empty());
        assert_eq!(store.revision(), revision);
        assert_eq!(store.read_state(), &ReadState::Loading);
    }

    #[test]
    fn upsert_replaces_in_place_and_new_identities_append() {
        let mut store = loaded(pod_rows(10));
        let mut updated = store.entries()[3].row().clone();
        updated.cells[3] = "99".into();
        let seq = store.entries()[3].seq();
        let revision = store.revision();
        apply(&mut store, vec![ResourceEvent::Upsert(updated.clone())]);
        assert_eq!(store.len(), 10);
        assert_eq!(store.entries()[3].row(), &updated);
        assert_eq!(store.entries()[3].seq(), seq);
        assert_eq!(store.revision(), revision + 1);
        apply(&mut store, vec![ResourceEvent::Upsert(inserted_pod(1))]);
        assert_eq!(store.len(), 11);
        assert_eq!(store.entries()[10].seq(), 10);
        assert_index_consistent(&store);
    }

    #[test]
    fn deletes_keep_the_identity_index_consistent() {
        let rows = pod_rows(20);
        let mut store = loaded(rows.clone());
        let doomed: Vec<_> = [0, 7, 19, 3]
            .iter()
            .map(|ix| rows[*ix].identity.clone())
            .collect();
        apply(
            &mut store,
            doomed.iter().cloned().map(ResourceEvent::Delete).collect(),
        );
        assert_eq!(store.len(), 16);
        assert_index_consistent(&store);
        for row in &rows {
            assert_eq!(
                store.get(&row.identity).is_some(),
                !doomed.contains(&row.identity)
            );
        }
        apply(&mut store, vec![ResourceEvent::Delete(doomed[0].clone())]);
        assert_eq!(store.len(), 16);
    }

    #[test]
    fn reset_replaces_columns_and_rows_and_deduplicates_by_identity() {
        let mut store = loaded(pod_rows(10));
        let mut rows = pod_rows(3);
        let mut duplicate = rows[0].clone();
        duplicate.cells[1] = "duplicate".into();
        rows.push(duplicate);
        let columns = vec![pod_columns()[0].clone()];
        apply(
            &mut store,
            vec![ResourceEvent::reset(columns.clone(), rows)],
        );
        assert_eq!(store.columns(), columns.as_slice());
        assert_eq!(store.len(), 3);
        assert_eq!(store.entries()[0].row().cells[1], "duplicate");
        assert_eq!(
            store
                .entries()
                .iter()
                .map(ResourceEntry::seq)
                .collect::<Vec<_>>(),
            [0, 1, 2]
        );
        assert_index_consistent(&store);
    }

    #[test]
    fn every_identity_field_participates() {
        let original = pod_rows(1).remove(0);
        let mut rows = vec![original.clone()];
        for field in 0..5 {
            let mut row = original.clone();
            match field {
                0 => row.identity.connection.push_str("-other"),
                1 => row.identity.resource.push_str("-other"),
                2 => row.identity.namespace.push_str("-other"),
                3 => row.identity.name.push_str("-other"),
                _ => row.identity.uid.push_str("-other"),
            }
            rows.push(row);
        }
        let mut store = loaded(rows);
        assert_eq!(store.len(), 6);
        apply(
            &mut store,
            vec![ResourceEvent::Delete(original.identity.clone())],
        );
        assert_eq!(store.len(), 5);
        assert!(store.get(&original.identity).is_none());
    }

    #[test]
    fn widest_counts_characters_from_the_reset_on() {
        let mut rows = pod_rows(3);
        rows[1].cells[0] = format!("pod-{}", "δ".repeat(36));
        rows[2].identity.namespace = "a-namespace-longer-than-test".into();
        let mut store = loaded(rows);
        let name = |widest: &Widest| widest.cells[0];
        // Characters, not bytes.
        assert_eq!(name(store.widest()), 40);
        assert_eq!(store.widest().namespace, 28);
        // A wider row widens it; deleting that row doesn't narrow it.
        let mut wide = inserted_pod(1);
        wide.cells[0] = "x".repeat(50);
        apply(&mut store, vec![ResourceEvent::Upsert(wide.clone())]);
        assert_eq!(name(store.widest()), 50);
        apply(&mut store, vec![ResourceEvent::Delete(wide.identity)]);
        assert_eq!(name(store.widest()), 50);
        // A reset counts only its own rows.
        apply(&mut store, vec![reset(pod_rows(2))]);
        let expected = store
            .entries()
            .iter()
            .map(|entry| entry.row().cells[0].chars().count())
            .max();
        assert_eq!(Some(name(store.widest())), expected);
        store.start_session();
        assert_eq!(store.widest(), &Widest::default());
    }

    #[test]
    fn the_search_key_covers_address_and_printed_cells() {
        let row = pod_rows(3).remove(2);
        let store = loaded(vec![row.clone()]);
        let key = store.entries()[0].search_key();
        assert!(key.starts_with(&format!(
            "{}\n{}",
            row.identity.namespace, row.identity.name
        )));
        assert!(key.contains(&row.cells[2].to_lowercase()));
        assert_eq!(key, key.to_lowercase());
    }

    #[test]
    fn a_failure_keeps_rows_as_stale_but_fails_an_empty_read() {
        let mut store = loaded(pod_rows(3));
        apply(
            &mut store,
            vec![ResourceEvent::Read(ReadState::Failed("gone".into()))],
        );
        assert_eq!(store.read_state(), &ReadState::Stale("gone".into()));
        assert_eq!(store.len(), 3);
        let mut empty = ResourceStore::new();
        let epoch = empty.start_session();
        empty.apply(
            epoch,
            vec![vec![ResourceEvent::Read(ReadState::Failed("gone".into()))]],
            usize::MAX,
        );
        assert_eq!(empty.read_state(), &ReadState::Failed("gone".into()));
    }

    /// The row at `ix` with its pod's restarts one higher.
    fn restarted(store: &ResourceStore, ix: usize) -> ResourceRow {
        let mut row = store.entries()[ix].row().clone();
        let mut pod = (**row.pod.as_ref().expect("a pod row")).clone();
        pod.restarts += 1;
        row.pod = Some(std::sync::Arc::new(pod));
        row
    }

    fn parts(
        store: &mut ResourceStore,
        parts: Vec<Vec<ResourceEvent>>,
    ) -> Vec<Vec<ResourceIdentity>> {
        let epoch = store.epoch();
        store
            .apply(epoch, parts, usize::MAX)
            .expect("the session's batch")
            .into_iter()
            .map(|changed| match changed {
                Changed::Rows(rows) => rows,
                Changed::Many(count) => panic!("{count} rows kept"),
            })
            .collect()
    }

    #[test]
    fn a_pods_new_state_is_a_change_and_its_other_cells_are_not() {
        let mut store = loaded(pod_rows(10));
        let restarted = restarted(&store, 3);
        let id = restarted.identity.clone();
        let mut moved = store.entries()[4].row().clone();
        moved.resource_version.push('1');
        moved.cells[0].push('x');
        let changes = parts(
            &mut store,
            vec![vec![
                ResourceEvent::Upsert(restarted.clone()),
                ResourceEvent::Upsert(moved),
                ResourceEvent::Upsert(restarted),
            ]],
        );
        // Once, though it came twice.
        assert_eq!(changes, vec![vec![id]]);
    }

    #[test]
    fn another_kinds_status_cells_are_its_state() {
        let rows: Vec<ResourceRow> = pod_rows(4)
            .into_iter()
            .map(|row| ResourceRow { pod: None, ..row })
            .collect();
        let mut store = loaded(rows);
        let mut status = store.entries()[1].row().clone();
        status.cells[2] = "Failed".into();
        let mut restarts = store.entries()[2].row().clone();
        restarts.cells[3] = "99".into();
        let changes = parts(
            &mut store,
            vec![
                vec![ResourceEvent::Upsert(status.clone())],
                vec![ResourceEvent::Upsert(restarts)],
            ],
        );
        assert_eq!(changes, vec![vec![status.identity], vec![]]);
    }

    #[test]
    fn a_changed_row_keeps_its_change_while_deletes_move_it() {
        let mut store = loaded(pod_rows(10));
        // Row 9 is last: deleting row 2 moves it into slot 2.
        let moved = restarted(&store, 9);
        let last_gone = restarted(&store, 8);
        let gone = store.entries()[2].row().identity.clone();
        let changes = parts(
            &mut store,
            vec![
                vec![
                    ResourceEvent::Upsert(moved.clone()),
                    ResourceEvent::Upsert(last_gone.clone()),
                ],
                vec![
                    ResourceEvent::Delete(gone),
                    ResourceEvent::Delete(last_gone.identity),
                ],
            ],
        );
        assert_eq!(changes, vec![vec![moved.identity], vec![]]);
    }

    #[test]
    fn a_long_batch_of_changes_and_deletes_reports_the_rows_still_changed() {
        let mut store = loaded(pod_rows(60));
        let rows: Vec<ResourceRow> = (0..60).map(|ix| restarted(&store, ix)).collect();
        // Every row changes, across two parts, then every third goes: some
        // changed earlier in the batch, some the last entry as it moves.
        let first = rows[..30]
            .iter()
            .cloned()
            .map(ResourceEvent::Upsert)
            .collect();
        let mut second: Vec<ResourceEvent> = rows[30..]
            .iter()
            .cloned()
            .map(ResourceEvent::Upsert)
            .collect();
        let doomed: Vec<ResourceIdentity> = (0..60)
            .rev()
            .step_by(3)
            .map(|ix| rows[ix].identity.clone())
            .collect();
        second.extend(doomed.iter().cloned().map(ResourceEvent::Delete));
        // A row of the first part changed again after the deletes.
        let mut again = rows[1].clone();
        let mut pod = (**again.pod.as_ref().expect("a pod row")).clone();
        pod.restarts += 1;
        again.pod = Some(std::sync::Arc::new(pod));
        second.push(ResourceEvent::Upsert(again));
        let changes = parts(&mut store, vec![first, second]);
        assert_index_consistent(&store);
        let expect = |range: std::ops::Range<usize>| -> Vec<ResourceIdentity> {
            let mut ids: Vec<ResourceIdentity> = rows[range]
                .iter()
                .map(|row| row.identity.clone())
                .filter(|id| !doomed.contains(id))
                .collect();
            ids.sort_by_key(|id| store.slot(id));
            ids
        };
        let mut second_expected = expect(30..60);
        second_expected.push(rows[1].identity.clone());
        second_expected.sort_by_key(|id| store.slot(id));
        assert_eq!(changes, vec![expect(0..30), second_expected]);
    }

    #[test]
    fn a_part_past_what_is_kept_is_only_counted() {
        let mut store = loaded(pod_rows(10));
        let few = vec![ResourceEvent::Upsert(restarted(&store, 1))];
        let many = (2..5)
            .map(|ix| ResourceEvent::Upsert(restarted(&store, ix)))
            .collect();
        let id = store.entries()[1].row().identity.clone();
        let epoch = store.epoch();
        let changes = store.apply(epoch, vec![few, many], 2).unwrap();
        assert_eq!(changes, vec![Changed::Rows(vec![id]), Changed::Many(3)]);
    }

    #[test]
    fn lists_arrivals_and_deletes_are_no_change() {
        let mut store = ResourceStore::new();
        let epoch = store.start_session();
        let changes = store
            .apply(epoch, vec![vec![reset(pod_rows(10))]], usize::MAX)
            .unwrap();
        assert_eq!(changes, vec![Changed::Rows(Vec::new())]);
        let gone = store.entries()[0].row().identity.clone();
        let changes = parts(
            &mut store,
            vec![vec![
                ResourceEvent::Upsert(inserted_pod(99)),
                ResourceEvent::Delete(gone),
            ]],
        );
        assert_eq!(changes, vec![Vec::<ResourceIdentity>::new()]);
    }

    #[test]
    fn a_relist_reports_nothing_before_it_and_what_follows_it() {
        let mut store = loaded(pod_rows(10));
        let before = restarted(&store, 1);
        let after = restarted(&store, 2);
        let relisted = restarted(&store, 5);
        let changes = parts(
            &mut store,
            vec![
                vec![ResourceEvent::Upsert(before)],
                vec![ResourceEvent::Upsert(relisted), reset(pod_rows(10))],
                vec![ResourceEvent::Upsert(after.clone())],
            ],
        );
        assert_eq!(changes, vec![vec![], vec![], vec![after.identity]]);
    }

    /// What flashes when the watch's batches arrive at `times` (ms) and
    /// are applied either one by one or coalesced as the screen does: a
    /// batch within `WATCH_COALESCE` of the last apply waits and joins the
    /// next. A batch is how many pods it restarts, or `None` for a relist.
    fn flashed(batches: &[(u64, Option<usize>)], coalesce: bool) -> Vec<ResourceIdentity> {
        use freshkube_ui::motion::Flashes;
        use std::time::{Duration, Instant};
        let start = Instant::now();
        let mut store = loaded(pod_rows(40));
        let mut flashes = Flashes::new();
        let mut next = 0;
        let mut pending: Vec<Vec<ResourceEvent>> = Vec::new();
        let mut last: Option<u64> = None;
        // As the screen reports them: rows up to the burst, else a count.
        let apply = |store: &mut ResourceStore,
                     flashes: &mut Flashes<ResourceIdentity>,
                     pending: Vec<Vec<ResourceEvent>>,
                     at: u64| {
            let at = start + Duration::from_millis(at);
            let relist = pending
                .iter()
                .flatten()
                .any(|event| matches!(event, ResourceEvent::Reset(_)));
            let epoch = store.epoch();
            let changes = store.apply(epoch, pending, flashes.burst()).unwrap();
            if relist {
                flashes.clear();
            }
            for part in changes {
                match part {
                    Changed::Rows(rows) => {
                        flashes.changed(rows, at);
                    }
                    Changed::Many(count) => flashes.held_back(count, at),
                }
            }
        };
        let mut end = 0;
        for &(at, batch) in batches {
            let events = match batch {
                Some(count) => (0..count)
                    .map(|_| {
                        next = (next + 1) % 40;
                        ResourceEvent::Upsert(restarted(&store, next))
                    })
                    .collect(),
                None => vec![reset(pod_rows(40))],
            };
            if !coalesce {
                apply(&mut store, &mut flashes, vec![events], at);
                end = at;
                continue;
            }
            // Applied when due: at once if the last apply is old enough.
            let due = last.map_or(at, |last| (last + 100).max(at));
            if due > at {
                pending.push(events);
            } else {
                if !pending.is_empty() {
                    let earlier = std::mem::take(&mut pending);
                    apply(&mut store, &mut flashes, earlier, at);
                }
                apply(&mut store, &mut flashes, vec![events], at);
                last = Some(at);
            }
            end = at;
        }
        if !pending.is_empty() {
            end = last.map_or(end, |last| (last + 100).max(end));
            apply(&mut store, &mut flashes, pending, end);
        }
        let mut live: Vec<ResourceIdentity> = flashes
            .live(start + Duration::from_millis(end))
            .map(|flash| flash.key.clone())
            .collect();
        live.sort();
        live
    }

    #[test]
    fn coalesced_batches_flash_as_the_watch_sent_them() {
        let cases: [&[(u64, Option<usize>)]; 3] = [
            // Three small batches flash; the burst after them doesn't.
            &[(0, Some(2)), (20, Some(2)), (40, Some(2)), (60, Some(20))],
            // After a quiet fade, a small batch flashes again.
            &[(0, Some(20)), (1_600, Some(2)), (1_620, Some(2))],
            // A relist forgets what came before it; what follows flashes.
            &[(0, Some(3)), (30, None), (60, Some(2))],
        ];
        for (ix, batches) in cases.into_iter().enumerate() {
            let alone = flashed(batches, false);
            assert!(!alone.is_empty(), "case {ix} flashes something");
            assert_eq!(alone, flashed(batches, true), "case {ix}");
        }
        assert_eq!(flashed(cases[0], true).len(), 6);
        assert_eq!(flashed(cases[2], true).len(), 2);
    }
}
