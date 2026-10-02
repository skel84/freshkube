use std::cmp::Ordering;

use super::model::{
    ColumnKind, ResourceIdentity, ResourceRow, SortDirection, SortKey, natural_cmp,
};
use super::store::{ResourceEntry, ResourceStore};

/// The table's view of a `ResourceStore`: filter, sort order and selection.
/// Visible rows are store slots, valid only for the store revision the
/// projection was built from; row accessors return `None` for a stale
/// projection rather than read a slot that may now hold another object.
/// Selection is always an identity, never a position.
#[derive(Clone, Debug)]
pub(crate) struct ResourceProjection {
    revision: Option<u64>,
    visible: Vec<usize>,
    selected: Option<ResourceIdentity>,
    selected_ix: Option<usize>,
    query: String,
    sort: SortKey,
    direction: SortDirection,
}

impl ResourceProjection {
    pub(crate) fn new() -> Self {
        Self {
            revision: None,
            visible: Vec::new(),
            selected: None,
            selected_ix: None,
            query: String::new(),
            sort: SortKey::Column(0),
            direction: SortDirection::Default,
        }
    }

    pub(crate) fn is_current(&self, store: &ResourceStore) -> bool {
        self.revision == Some(store.revision())
    }

    /// Store slots in display order.
    #[cfg(test)]
    pub(crate) fn visible_rows(&self) -> &[usize] {
        &self.visible
    }

    pub(crate) fn len(&self) -> usize {
        self.visible.len()
    }

    pub(crate) fn sort_state(&self) -> (SortKey, SortDirection) {
        (self.sort, self.direction)
    }

    pub(crate) fn entry<'a>(
        &self,
        store: &'a ResourceStore,
        visible_ix: usize,
    ) -> Option<&'a ResourceEntry> {
        if !self.is_current(store) {
            return None;
        }
        self.visible
            .get(visible_ix)
            .and_then(|slot| store.entries().get(*slot))
    }

    pub(crate) fn row<'a>(
        &self,
        store: &'a ResourceStore,
        visible_ix: usize,
    ) -> Option<&'a ResourceRow> {
        self.entry(store, visible_ix).map(ResourceEntry::row)
    }

    pub(crate) fn selected(&self) -> Option<&ResourceIdentity> {
        self.selected.as_ref()
    }

    pub(crate) fn selected_index(&self) -> Option<usize> {
        self.selected_ix
    }

    /// Selects the row at a current visible position. A stale projection
    /// ignores the request rather than clear or retarget the selection.
    pub(crate) fn select(&mut self, store: &ResourceStore, visible_ix: Option<usize>) {
        if !self.is_current(store) {
            return;
        }
        let identity = visible_ix
            .and_then(|ix| self.row(store, ix))
            .map(|row| row.identity.clone());
        self.selected_ix = identity.as_ref().and(visible_ix);
        self.selected = identity;
    }

    /// Selects an object by identity, wherever it now sits. An identity that
    /// isn't visible clears the selection instead of selecting a neighbour.
    pub(crate) fn select_identity(&mut self, store: &ResourceStore, identity: &ResourceIdentity) {
        if !self.is_current(store) {
            return;
        }
        self.selected_ix = store
            .slot(identity)
            .and_then(|slot| self.visible.iter().position(|visible| *visible == slot));
        self.selected = self.selected_ix.map(|_| identity.clone());
    }

    pub(crate) fn filter(&mut self, store: &ResourceStore, query: &str) {
        let query = query.trim().to_lowercase();
        if self.query != query {
            self.query = query;
            self.rebuild(store);
        }
    }

    pub(crate) fn sort(&mut self, store: &ResourceStore, key: SortKey, direction: SortDirection) {
        if self.sort != key || self.direction != direction {
            self.sort = key;
            self.direction = direction;
            self.rebuild(store);
        }
    }

    /// Forgets a sort that belonged to another kind's columns.
    pub(crate) fn reset_sort(&mut self) {
        self.sort = SortKey::Column(0);
        self.direction = SortDirection::Default;
    }

    /// Recomputes visible slots and the selected position from the store's
    /// current revision. A selection that is no longer visible is cleared,
    /// never moved to whatever now occupies its old position.
    pub(crate) fn rebuild(&mut self, store: &ResourceStore) {
        let _span = crate::perf::span("table.rebuild");
        let entries = store.entries();
        let query = self.query.as_str();
        self.visible.clear();
        self.visible.extend(
            entries
                .iter()
                .enumerate()
                .filter(|(_, entry)| query.is_empty() || entry.search_key().contains(query))
                .map(|(slot, _)| slot),
        );
        match self.direction {
            SortDirection::Default => self
                .visible
                .sort_unstable_by_key(|slot| entries[*slot].seq()),
            direction => {
                let key = self.sort;
                let kind = match key {
                    SortKey::Column(ix) => store.columns().get(ix).map(|column| column.kind),
                    SortKey::Namespace => Some(ColumnKind::Text),
                };
                self.visible.sort_unstable_by(|left, right| {
                    let (left, right) = (entries[*left].row(), entries[*right].row());
                    let order = compare(left, right, key, kind)
                        .then_with(|| left.identity.cmp(&right.identity));
                    if direction == SortDirection::Descending {
                        order.reverse()
                    } else {
                        order
                    }
                });
            }
        }
        self.selected_ix = self
            .selected
            .as_ref()
            .and_then(|identity| store.slot(identity))
            .and_then(|slot| self.visible.iter().position(|visible| *visible == slot));
        if self.selected_ix.is_none() {
            self.selected = None;
        }
        self.revision = Some(store.revision());
    }
}

fn compare(
    left: &ResourceRow,
    right: &ResourceRow,
    key: SortKey,
    kind: Option<ColumnKind>,
) -> Ordering {
    fn cell(row: &ResourceRow, ix: usize) -> &str {
        row.cells.get(ix).map(String::as_str).unwrap_or("")
    }
    match (key, kind) {
        (SortKey::Namespace, _) => natural_cmp(&left.identity.namespace, &right.identity.namespace),
        // Ascending age is youngest first, as the column reads.
        (SortKey::Column(_), Some(ColumnKind::Age)) => right.created.cmp(&left.created),
        (SortKey::Column(ix), Some(ColumnKind::Number)) => {
            let number = |row: &ResourceRow| cell(row, ix).trim().parse::<f64>().ok();
            match (number(left), number(right)) {
                (Some(a), Some(b)) => a.total_cmp(&b),
                _ => natural_cmp(cell(left, ix), cell(right, ix)),
            }
        }
        (SortKey::Column(ix), _) => natural_cmp(cell(left, ix), cell(right, ix)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::resources::example::{
        deployment_columns, deployment_rows, inserted_pod, pod_columns, pod_rows, recreated,
    };
    use crate::resources::model::{ReadState, ResourceColumn};
    use crate::resources::store::{ResourceBatch, ResourceEvent};

    const NAME: SortKey = SortKey::Column(0);
    const STATUS: SortKey = SortKey::Column(2);
    const RESTARTS: SortKey = SortKey::Column(3);
    const AGE: SortKey = SortKey::Column(4);

    fn loaded_with(
        columns: Vec<ResourceColumn>,
        rows: Vec<ResourceRow>,
    ) -> (ResourceStore, ResourceProjection) {
        let mut store = ResourceStore::new();
        let epoch = store.start_session();
        store.apply(ResourceBatch {
            epoch,
            events: vec![
                ResourceEvent::Reset { columns, rows },
                ResourceEvent::Read(ReadState::Loaded),
            ],
        });
        let mut view = ResourceProjection::new();
        view.rebuild(&store);
        (store, view)
    }

    fn loaded(rows: Vec<ResourceRow>) -> (ResourceStore, ResourceProjection) {
        loaded_with(pod_columns(), rows)
    }

    fn apply(store: &mut ResourceStore, view: &mut ResourceProjection, events: Vec<ResourceEvent>) {
        let epoch = store.epoch();
        assert!(store.apply(ResourceBatch { epoch, events }));
        view.rebuild(store);
    }

    fn identities(store: &ResourceStore, view: &ResourceProjection) -> Vec<ResourceIdentity> {
        (0..view.len())
            .map(|ix| view.row(store, ix).unwrap().identity.clone())
            .collect()
    }

    #[test]
    fn sorting_filtering_and_updates_resolve_the_selected_identity() {
        let (mut store, mut view) = loaded(pod_rows(100));
        view.select(&store, Some(2));
        let identity = view.selected().unwrap().clone();
        for key in [
            SortKey::Namespace,
            NAME,
            SortKey::Column(1),
            STATUS,
            RESTARTS,
            AGE,
            SortKey::Column(5),
        ] {
            for direction in [SortDirection::Ascending, SortDirection::Descending] {
                view.sort(&store, key, direction);
                assert_eq!(view.selected(), Some(&identity));
                let ix = view.selected_index().unwrap();
                assert_eq!(view.row(&store, ix).unwrap().identity, identity);
            }
        }
        view.filter(&store, &identity.name.to_uppercase());
        assert_eq!(view.selected(), Some(&identity));
        assert_eq!(view.selected_index(), Some(0));
        let mut updated = store.get(&identity).unwrap().clone();
        updated.cells[3] = "7 (1m ago)".into();
        apply(
            &mut store,
            &mut view,
            vec![ResourceEvent::Upsert(updated.clone())],
        );
        assert_eq!(view.selected(), Some(&identity));
        assert_eq!(store.get(&identity), Some(&updated));
        apply(
            &mut store,
            &mut view,
            vec![ResourceEvent::Delete(identity.clone())],
        );
        assert!(view.selected().is_none());
        assert_eq!(store.len(), 99);
    }

    #[test]
    fn numbers_sort_by_value_and_age_by_creation_time() {
        let (store, mut view) = loaded_with(deployment_columns(), deployment_rows(40));
        let up_to_date = SortKey::Column(2);
        view.sort(&store, up_to_date, SortDirection::Ascending);
        let values: Vec<f64> = (0..view.len())
            .map(|ix| view.row(&store, ix).unwrap().cells[2].parse().unwrap())
            .collect();
        assert!(values.windows(2).all(|pair| pair[0] <= pair[1]));
        assert!(values.iter().any(|value| *value >= 10.), "{values:?}");
        let (store, mut view) = loaded(pod_rows(100));
        view.sort(&store, AGE, SortDirection::Ascending);
        let created: Vec<_> = (0..view.len())
            .map(|ix| view.row(&store, ix).unwrap().created.unwrap())
            .collect();
        assert!(
            created.windows(2).all(|pair| pair[0] >= pair[1]),
            "youngest first"
        );
    }

    #[test]
    fn filtering_away_a_selection_never_selects_the_same_position() {
        let (store, mut view) = loaded(pod_rows(100));
        view.select(&store, Some(0));
        let selected = view.selected().unwrap().clone();
        let other_name = view.row(&store, 1).unwrap().identity.name.clone();
        view.filter(&store, &other_name);
        assert!(view.selected().is_none());
        view.filter(&store, "");
        assert!(view.selected().is_none());
        assert_eq!(view.row(&store, 0).unwrap().identity, selected);
        view.select(&store, Some(usize::MAX));
        assert!(view.selected().is_none());
    }

    #[test]
    fn reset_reorder_preserves_uid_but_a_new_incarnation_clears_it() {
        let mut rows = pod_rows(100);
        let (mut store, mut view) = loaded(rows.clone());
        view.select(&store, Some(4));
        let selected = view.selected().unwrap().clone();
        rows.reverse();
        let reset = |rows: Vec<ResourceRow>| ResourceEvent::Reset {
            columns: pod_columns(),
            rows,
        };
        apply(&mut store, &mut view, vec![reset(rows.clone())]);
        assert_eq!(view.selected(), Some(&selected));
        assert_eq!(view.selected_index(), Some(95));
        let row = rows
            .iter_mut()
            .find(|row| row.identity == selected)
            .unwrap();
        row.identity.uid.push_str("-replacement");
        apply(&mut store, &mut view, vec![reset(rows)]);
        assert!(view.selected().is_none());
    }

    #[test]
    fn filtered_recreation_does_not_follow_a_position() {
        let (mut store, mut view) = loaded(pod_rows(100));
        view.sort(&store, NAME, SortDirection::Descending);
        view.select(&store, Some(17));
        let old = view.selected().unwrap().clone();
        view.filter(&store, &old.name);
        let replacement = recreated(store.get(&old).unwrap());
        apply(
            &mut store,
            &mut view,
            vec![
                ResourceEvent::Delete(old.clone()),
                ResourceEvent::Upsert(replacement.clone()),
            ],
        );
        assert!(view.selected().is_none());
        assert_eq!(
            identities(&store, &view),
            std::slice::from_ref(&replacement.identity)
        );
    }

    #[test]
    fn upserting_the_selected_observation_or_a_new_row_preserves_selection() {
        let (mut store, mut view) = loaded(pod_rows(100));
        view.select(&store, Some(3));
        let identity = view.selected().unwrap().clone();
        let mut replacement = store.get(&identity).unwrap().clone();
        replacement.cells[3] = "42".into();
        apply(
            &mut store,
            &mut view,
            vec![ResourceEvent::Upsert(replacement.clone())],
        );
        assert_eq!(view.selected(), Some(&identity));
        apply(
            &mut store,
            &mut view,
            vec![ResourceEvent::Upsert(inserted_pod(1))],
        );
        assert_eq!(store.len(), 101);
        assert_eq!(view.selected_index(), Some(3));
    }

    #[test]
    fn default_sort_restores_observation_order_and_ties_use_identity() {
        let rows = pod_rows(100);
        let source: Vec<_> = rows.iter().map(|row| row.identity.clone()).collect();
        let (store, mut view) = loaded(rows);
        view.select(&store, Some(7));
        let selected = view.selected().unwrap().clone();
        view.sort(&store, SortKey::Column(1), SortDirection::Ascending);
        view.sort(&store, NAME, SortDirection::Default);
        assert_eq!(identities(&store, &view), source);
        assert_eq!(view.selected(), Some(&selected));
        assert_eq!(view.selected_index(), Some(7));
    }

    #[test]
    fn filter_matches_namespace_name_and_printed_cells() {
        let mut row = pod_rows(1).remove(0);
        row.identity.namespace = "équipe-services".into();
        row.identity.name = "unique-workload".into();
        row.cells[2] = "CrashLoopBackOff".into();
        let (mut store, mut view) = loaded(vec![row]);
        for query in ["ÉQUIPE", "WORKLOAD", "CRASHLOOP", " crashloopbackoff "] {
            view.filter(&store, query);
            assert_eq!(view.visible_rows(), &[0], "{query}");
        }
        view.filter(&store, "no such pod");
        assert!(view.visible_rows().is_empty());
        apply(
            &mut store,
            &mut view,
            vec![ResourceEvent::Reset {
                columns: pod_columns(),
                rows: Vec::new(),
            }],
        );
        assert!(store.is_empty());
    }

    #[test]
    fn a_stale_projection_reads_no_rows_and_ignores_selection_until_rebuilt() {
        let rows = pod_rows(10);
        let (mut store, mut view) = loaded(rows.clone());
        view.select(&store, Some(3));
        let selected = view.selected().unwrap().clone();
        let epoch = store.epoch();
        store.apply(ResourceBatch {
            epoch,
            events: vec![ResourceEvent::Delete(rows[0].identity.clone())],
        });
        assert!(!view.is_current(&store));
        assert!(view.row(&store, 0).is_none());
        view.select(&store, None);
        view.select_identity(&store, &rows[5].identity);
        assert_eq!(view.selected(), Some(&selected));
        view.rebuild(&store);
        assert_eq!(view.len(), 9);
        assert_eq!(view.selected(), Some(&selected));
    }
}
