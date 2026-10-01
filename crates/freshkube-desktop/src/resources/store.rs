use std::collections::HashMap;

use super::model::{ReadState, ResourceColumn, ResourceIdentity, ResourceRow};

/// One change to observed state, in the shape of a list/watch stream:
/// individual upserts and deletes, an atomic reset that replaces the columns
/// and every row, and changes to the collection's read state.
#[derive(Clone, Debug)]
pub(crate) enum ResourceEvent {
    Reset {
        columns: Vec<ResourceColumn>,
        rows: Vec<ResourceRow>,
    },
    Upsert(ResourceRow),
    Delete(ResourceIdentity),
    Read(ReadState),
}

/// Events observed within one session. The store applies a batch as one
/// revision and rejects a batch from any other session, so a late result from
/// a replaced connection, kind or namespace cannot populate the current view.
/// A producer captures the epoch `ResourceStore::start_session` returned when
/// its work began; it never reads the store's epoch when results arrive.
#[derive(Clone, Debug)]
pub(crate) struct ResourceBatch {
    pub(crate) epoch: u64,
    pub(crate) events: Vec<ResourceEvent>,
}

/// A retained observation plus a search key derived once when it arrives, so
/// filtering never allocates per row.
#[derive(Clone, Debug)]
pub(crate) struct ResourceEntry {
    row: ResourceRow,
    search_key: String,
    /// Observation order: position in the last reset, then arrival order.
    seq: u64,
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
}

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

    pub(crate) fn slot(&self, identity: &ResourceIdentity) -> Option<usize> {
        self.index.get(identity).copied()
    }

    pub(crate) fn get(&self, identity: &ResourceIdentity) -> Option<&ResourceRow> {
        self.slot(identity).map(|slot| &self.entries[slot].row)
    }

    /// Applies every event in order as one revision. Returns false, changing
    /// nothing, when the batch belongs to another session. Epoch 0 means no
    /// session has started, so nothing is accepted.
    pub(crate) fn apply(&mut self, batch: ResourceBatch) -> bool {
        if self.epoch == 0 || batch.epoch != self.epoch {
            return false;
        }
        for event in batch.events {
            match event {
                ResourceEvent::Reset { columns, rows } => self.reset(columns, rows),
                ResourceEvent::Upsert(row) => self.upsert(row),
                ResourceEvent::Delete(identity) => self.delete(&identity),
                // A failure with rows on screen leaves them up, marked stale.
                ResourceEvent::Read(ReadState::Failed(reason)) if !self.entries.is_empty() => {
                    self.read_state = ReadState::Stale(reason)
                }
                ResourceEvent::Read(state) => self.read_state = state,
            }
        }
        self.revision += 1;
        true
    }

    fn upsert(&mut self, row: ResourceRow) {
        if let Some(slot) = self.slot(&row.identity) {
            let seq = self.entries[slot].seq;
            self.entries[slot] = ResourceEntry::new(row, seq);
        } else {
            self.index.insert(row.identity.clone(), self.entries.len());
            self.entries.push(ResourceEntry::new(row, self.next_seq));
            self.next_seq += 1;
        }
    }

    fn delete(&mut self, identity: &ResourceIdentity) {
        let Some(slot) = self.index.remove(identity) else {
            return;
        };
        self.entries.swap_remove(slot);
        if let Some(moved) = self.entries.get(slot) {
            self.index.insert(moved.row.identity.clone(), slot);
        }
    }

    // Duplicate identities keep the first occurrence's position with the last
    // occurrence's observation, as a relist that saw an object twice would.
    fn reset(&mut self, columns: Vec<ResourceColumn>, rows: Vec<ResourceRow>) {
        self.columns = columns;
        self.entries.clear();
        self.index.clear();
        self.next_seq = 0;
        for row in rows {
            self.upsert(row);
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
        assert!(store.apply(ResourceBatch {
            epoch,
            events: vec![
                ResourceEvent::Reset {
                    columns: pod_columns(),
                    rows
                },
                ResourceEvent::Read(ReadState::Loaded)
            ],
        }));
        store
    }

    fn apply(store: &mut ResourceStore, events: Vec<ResourceEvent>) {
        let epoch = store.epoch();
        assert!(store.apply(ResourceBatch { epoch, events }));
    }

    fn reset(rows: Vec<ResourceRow>) -> ResourceEvent {
        ResourceEvent::Reset {
            columns: pod_columns(),
            rows,
        }
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
        assert!(!store.apply(ResourceBatch {
            epoch: 0,
            events: vec![reset(pod_rows(3))],
        }));
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
        assert!(!store.apply(ResourceBatch {
            epoch: old_epoch,
            events: vec![reset(pod_rows(10)), ResourceEvent::Read(ReadState::Loaded)],
        }));
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
            vec![ResourceEvent::Reset {
                columns: columns.clone(),
                rows,
            }],
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
        empty.apply(ResourceBatch {
            epoch,
            events: vec![ResourceEvent::Read(ReadState::Failed("gone".into()))],
        });
        assert_eq!(empty.read_state(), &ReadState::Failed("gone".into()));
    }
}
