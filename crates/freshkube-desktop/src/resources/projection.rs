use std::cmp::Ordering;
use std::collections::BTreeSet;
use std::sync::Arc;

use super::model::{
    ColumnKind, ResourceIdentity, ResourceRow, SortDirection, SortKey, natural_cmp,
};
use super::rows::{PodRow, PodState};
use super::store::{ResourceEntry, ResourceStore};

/// Why a pod shows among the problems, in the order the groups appear.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum Cause {
    Failing,
    /// Its node's Kubernetes Ready condition isn't True, so nothing can
    /// confirm the pod's own state.
    NodeNotReady(String),
    NotReady,
    Pending,
    Terminating,
    /// A status this table doesn't know the meaning of.
    Unknown,
    Healthy,
}

impl Cause {
    fn of(row: &ResourceRow, not_ready: &BTreeSet<String>) -> Self {
        let Some(pod) = &row.pod else {
            return Self::Healthy;
        };
        match pod.state {
            PodState::Failing => Self::Failing,
            PodState::Completed => Self::Healthy,
            _ if not_ready.contains(&pod.node) => Self::NodeNotReady(pod.node.clone()),
            PodState::NotReady => Self::NotReady,
            PodState::Pending => Self::Pending,
            PodState::Terminating => Self::Terminating,
            PodState::Unknown => Self::Unknown,
            PodState::Running => Self::Healthy,
        }
    }
}

/// A glyph filter preserves the counts for every cause, so another status
/// stays one click away. It combines with the text filter and the sort.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PodFilter {
    Failing,
    Warning,
    Waiting,
    Healthy,
}
impl PodFilter {
    fn matches(self, cause: &Cause) -> bool {
        match self {
            Self::Failing => matches!(cause, Cause::Failing),
            Self::Warning => matches!(cause, Cause::NodeNotReady(_) | Cause::NotReady),
            Self::Waiting => matches!(cause, Cause::Pending | Cause::Terminating | Cause::Unknown),
            Self::Healthy => matches!(cause, Cause::Healthy),
        }
    }
}

/// A run of rows with one cause. `start` is its first row's visible
/// position; a collapsed group shows none of its `total` rows.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Group {
    pub(crate) cause: Cause,
    pub(crate) start: usize,
    pub(crate) shown: usize,
    pub(crate) total: usize,
    /// How many namespaces its rows are in.
    pub(crate) namespaces: usize,
}

/// One line of the list: a group's header or a row, by visible position.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Item {
    Group(usize),
    Row(usize),
}

/// Problems first: pods grouped by cause, healthy ones last and collapsed
/// while there are problems and no filter. Flat, the rows keep the sort
/// alone and only the counts are kept.
#[derive(Clone, Debug, Default)]
pub(crate) struct Grouping {
    /// Nodes whose Kubernetes Ready condition isn't True.
    pub(crate) not_ready: Arc<BTreeSet<String>>,
    pub(crate) healthy_open: bool,
    pub(crate) flat: bool,
}

/// Rows after the filter by the glyph their cause shows.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Tally {
    pub(crate) failing: usize,
    /// Not ready, or on a node that isn't.
    pub(crate) warning: usize,
    /// Pending, terminating or unknown.
    pub(crate) waiting: usize,
    pub(crate) healthy: usize,
}

impl Tally {
    pub(crate) fn total(&self) -> usize {
        self.failing + self.warning + self.waiting + self.healthy
    }

    pub(crate) fn problems(&self) -> usize {
        self.total() - self.healthy
    }

    fn count(&mut self, cause: &Cause) {
        match cause {
            Cause::Failing => self.failing += 1,
            Cause::NodeNotReady(_) | Cause::NotReady => self.warning += 1,
            Cause::Pending | Cause::Terminating | Cause::Unknown => self.waiting += 1,
            Cause::Healthy => self.healthy += 1,
        }
    }
}

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
    grouping: Option<Grouping>,
    groups: Vec<Group>,
    items: Vec<Item>,
    tally: Tally,
    pod_filter: Option<PodFilter>,
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
            grouping: None,
            groups: Vec::new(),
            items: Vec::new(),
            tally: Tally::default(),
            pod_filter: None,
        }
    }

    pub(crate) fn grouping(&self) -> Option<&Grouping> {
        self.grouping.as_ref()
    }

    pub(crate) fn pod_filter(&self) -> Option<PodFilter> {
        self.pod_filter
    }
    pub(crate) fn set_pod_filter(&mut self, store: &ResourceStore, filter: Option<PodFilter>) {
        self.pod_filter = filter;
        self.rebuild(store);
    }

    /// Groups rows by cause, or with `None`, lists them flat.
    pub(crate) fn set_grouping(&mut self, store: &ResourceStore, grouping: Option<Grouping>) {
        self.grouping = grouping;
        self.rebuild(store);
    }

    pub(crate) fn groups(&self) -> &[Group] {
        &self.groups
    }

    /// Rows after the filter by cause; zero without a grouping.
    pub(crate) fn tally(&self) -> Tally {
        self.tally
    }

    /// Lines in the list: rows, and when grouped, group headers.
    pub(crate) fn items_len(&self) -> usize {
        self.items.len()
    }

    pub(crate) fn item(&self, ix: usize) -> Option<Item> {
        self.items.get(ix).copied()
    }

    /// The line a visible row is on, below its group's header.
    pub(crate) fn line_of(&self, row: usize) -> usize {
        row + self
            .groups
            .iter()
            .filter(|group| group.start <= row)
            .count()
    }

    /// Every row of `group` the filter keeps, shown or collapsed.
    pub(crate) fn group_rows(&self, store: &ResourceStore, group: &Group) -> Vec<ResourceIdentity> {
        let Some(grouping) = &self.grouping else {
            return Vec::new();
        };
        store
            .entries()
            .iter()
            .filter(|entry| entry.search_key().contains(&self.query))
            .map(ResourceEntry::row)
            .filter(|row| Cause::of(row, &grouping.not_ready) == group.cause)
            .map(|row| row.identity.clone())
            .collect()
    }

    /// Whether `identity` is among the healthy rows a collapsed group hides.
    pub(crate) fn hides(&self, store: &ResourceStore, identity: &ResourceIdentity) -> bool {
        let Some(grouping) = self.grouping.as_ref().filter(|grouping| !grouping.flat) else {
            return false;
        };
        self.groups
            .iter()
            .any(|group| group.cause == Cause::Healthy && group.shown < group.total)
            && store
                .get(identity)
                .is_some_and(|row| Cause::of(row, &grouping.not_ready) == Cause::Healthy)
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

    /// Where `identity` sits among the visible rows, if it does.
    pub(crate) fn index_of(
        &self,
        store: &ResourceStore,
        identity: &ResourceIdentity,
    ) -> Option<usize> {
        let slot = store.slot(identity)?;
        self.visible.iter().position(|visible| *visible == slot)
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
        self.selected_ix = self.index_of(store, identity);
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
                    _ => Some(ColumnKind::Text),
                };
                self.visible.sort_unstable_by(|left, right| {
                    let (left, right) = (&entries[*left], &entries[*right]);
                    let order = compare(left, right, key, kind)
                        .then_with(|| left.row().identity.cmp(&right.row().identity));
                    if direction == SortDirection::Descending {
                        order.reverse()
                    } else {
                        order
                    }
                });
            }
        }
        self.group(store);
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

impl ResourceProjection {
    /// Orders the sorted rows by cause, keeping the sort within each, and
    /// leaves out the healthy ones while they are collapsed.
    fn group(&mut self, store: &ResourceStore) {
        self.groups.clear();
        self.items.clear();
        self.tally = Tally::default();
        let Some(grouping) = self.grouping.as_ref().filter(|grouping| !grouping.flat) else {
            if let Some(grouping) = &self.grouping {
                for slot in &self.visible {
                    let cause = Cause::of(store.entries()[*slot].row(), &grouping.not_ready);
                    self.tally.count(&cause);
                }
                if let Some(filter) = self.pod_filter {
                    self.visible.retain(|slot| {
                        filter.matches(&Cause::of(
                            store.entries()[*slot].row(),
                            &grouping.not_ready,
                        ))
                    });
                }
            }
            self.items.extend((0..self.visible.len()).map(Item::Row));
            return;
        };
        let entries = store.entries();
        let mut caused: Vec<(Cause, usize)> = self
            .visible
            .drain(..)
            .map(|slot| (Cause::of(entries[slot].row(), &grouping.not_ready), slot))
            .collect();
        // Stable, so each group keeps the sort.
        caused.sort_by(|left, right| left.0.cmp(&right.0));
        for (cause, _) in &caused {
            self.tally.count(cause);
        }
        if let Some(filter) = self.pod_filter {
            caused.retain(|(cause, _)| filter.matches(cause));
        }
        let collapse = self.pod_filter.is_none()
            && !grouping.healthy_open
            && self.tally.problems() > 0
            && self.query.is_empty();
        let mut rest = caused.as_slice();
        while let Some((cause, _)) = rest.first() {
            let total = rest.iter().take_while(|(other, _)| other == cause).count();
            let (run, after) = rest.split_at(total);
            let shown = if collapse && *cause == Cause::Healthy {
                0
            } else {
                total
            };
            let namespaces: BTreeSet<&str> = run
                .iter()
                .map(|(_, slot)| entries[*slot].row().identity.namespace.as_str())
                .collect();
            self.items.push(Item::Group(self.groups.len()));
            let start = self.visible.len();
            self.items.extend((start..start + shown).map(Item::Row));
            self.visible
                .extend(run[..shown].iter().map(|(_, slot)| *slot));
            self.groups.push(Group {
                cause: cause.clone(),
                start,
                shown,
                total,
                namespaces: namespaces.len(),
            });
            rest = after;
        }
    }
}

fn compare(
    left_entry: &ResourceEntry,
    right_entry: &ResourceEntry,
    key: SortKey,
    kind: Option<ColumnKind>,
) -> Ordering {
    fn cell(row: &ResourceRow, ix: usize) -> &str {
        row.cells.get(ix).map(String::as_str).unwrap_or("")
    }
    let (left, right) = (left_entry.row(), right_entry.row());
    let usage = |entry: &ResourceEntry, cpu: bool| {
        entry.usage().and_then(|usage| {
            if cpu {
                usage.cpu_millis
            } else {
                usage.memory_bytes
            }
        })
    };
    fn pod_text(row: &ResourceRow, pick: fn(&PodRow) -> &str) -> &str {
        row.pod.as_deref().map(pick).unwrap_or("")
    }
    match (key, kind) {
        (SortKey::Namespace, _) => natural_cmp(&left.identity.namespace, &right.identity.namespace)
            .then_with(|| natural_cmp(&left.identity.name, &right.identity.name)),
        (SortKey::Owner, _) => {
            fn label(row: &ResourceRow) -> Option<(&str, &str)> {
                row.owner
                    .as_ref()
                    .map(|owner| (owner.short.as_str(), owner.name.as_str()))
            }
            label(left).cmp(&label(right))
        }
        (SortKey::Restarts, _) => {
            let restarts = |row: &ResourceRow| row.pod.as_ref().map(|pod| pod.restarts);
            restarts(left).cmp(&restarts(right)).then_with(|| {
                natural_cmp(
                    pod_text(left, |pod| &pod.ready),
                    pod_text(right, |pod| &pod.ready),
                )
            })
        }
        (SortKey::Cpu | SortKey::Memory, _) => {
            let cpu = key == SortKey::Cpu;
            match (usage(left_entry, cpu), usage(right_entry, cpu)) {
                (Some(a), Some(b)) => a.total_cmp(&b),
                // Unknown use sorts below any known.
                (a, b) => a.is_some().cmp(&b.is_some()),
            }
        }
        (SortKey::Node, _) => natural_cmp(
            pod_text(left, |pod| &pod.node),
            pod_text(right, |pod| &pod.node),
        ),
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
                ResourceEvent::reset(columns, rows),
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
        let reset = |rows: Vec<ResourceRow>| ResourceEvent::reset(pod_columns(), rows);
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
            vec![ResourceEvent::reset(pod_columns(), Vec::new())],
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

    fn grouped(not_ready: &[&str]) -> Grouping {
        Grouping {
            not_ready: Arc::new(not_ready.iter().map(|node| (*node).to_owned()).collect()),
            healthy_open: false,
            flat: false,
        }
    }

    #[test]
    fn problems_come_first_by_cause_with_healthy_pods_folded() {
        // Pods 5 and 28 crash and one waits to start; node-b runs every
        // third pod.
        let (store, mut view) = loaded(pod_rows(40));
        view.set_grouping(&store, Some(grouped(&["node-b"])));
        let causes: Vec<Cause> = view
            .groups()
            .iter()
            .map(|group| group.cause.clone())
            .collect();
        assert_eq!(
            causes,
            [
                Cause::Failing,
                Cause::NodeNotReady("node-b".into()),
                Cause::Pending,
                Cause::Healthy
            ]
        );
        let tally = view.tally();
        assert_eq!((tally.failing, tally.waiting), (2, 1));
        assert_eq!(tally.total(), 40);
        // A crashing pod on that node still counts as failing.
        assert!(tally.warning > 0 && tally.warning < 13);
        // Healthy pods are folded: only problems are rows.
        let healthy = view.groups().last().unwrap().clone();
        assert_eq!((healthy.shown, healthy.total), (0, tally.healthy));
        assert_eq!(view.len(), tally.problems());
        assert_eq!(view.items_len(), view.len() + 4);
        assert_eq!(view.item(0), Some(Item::Group(0)));
        assert_eq!(view.item(1), Some(Item::Row(0)));
        // A group's first row sits below its header and those before it.
        let pending = &view.groups()[2];
        assert_eq!(view.line_of(pending.start), pending.start + 3);
        assert_eq!(view.group_rows(&store, &view.groups()[0].clone()).len(), 2);
        let folded = (0..store.len())
            .find(|slot| !view.visible_rows().contains(slot))
            .map(|slot| store.entries()[slot].row().identity.clone())
            .unwrap();
        assert!(view.hides(&store, &folded));

        // Opened, healthy pods follow the problems.
        let mut open = grouped(&["node-b"]);
        open.healthy_open = true;
        view.set_grouping(&store, Some(open));
        assert_eq!(view.len(), 40);
        assert!(!view.hides(&store, &folded));

        // A filter shows every match, healthy or not.
        view.set_grouping(&store, Some(grouped(&["node-b"])));
        view.filter(&store, "worker");
        let shown = view.len();
        assert!(view.groups().iter().all(|group| group.shown == group.total));
        assert_eq!(shown, view.tally().total());

        // Flat keeps the sort alone and still counts.
        view.filter(&store, "");
        let mut flat = grouped(&["node-b"]);
        flat.flat = true;
        view.set_grouping(&store, Some(flat));
        assert!(view.groups().is_empty());
        assert_eq!((view.len(), view.items_len()), (40, 40));
        assert_eq!(view.tally(), tally);
    }

    #[test]
    fn grouping_keeps_the_sort_within_each_cause() {
        let (store, mut view) = loaded(pod_rows(40));
        view.set_grouping(&store, Some(grouped(&[])));
        view.sort(&store, NAME, SortDirection::Descending);
        let names: Vec<&str> = (0..view.len())
            .map(|ix| view.row(&store, ix).unwrap().identity.name.as_str())
            .collect();
        let pending = &view.groups()[1];
        assert_eq!(pending.cause, Cause::Pending);
        let run = &names[pending.start..pending.start + pending.shown];
        assert!(
            run.windows(2)
                .all(|pair| natural_cmp(pair[0], pair[1]).is_ge())
        );
    }
}
