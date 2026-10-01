//! The Resources page: one kind's table as the API server prints it, kept
//! current by a watch while the page is visible. Rows are virtualized, sorted
//! and filtered locally, and selected by identity. Read-only: nothing here
//! changes the cluster.

use std::ops::Range;
use std::time::{Duration, SystemTime};

use freshkube_core::resources::{
    ResourceKind, WatchBatch, WatchEvent, builtin, list_table, watch_collection,
};
use gpui_kit::assets::IconName;
use gpui_kit::component::{
    Icon, IndexPath, Sizable,
    button::{Button, ButtonVariants},
    h_flex,
    input::{Input, InputEvent, InputState},
    resizable::{ResizableState, h_resizable, resizable_panel, v_resizable},
    select::{SearchableVec, Select, SelectEvent, SelectItem, SelectState},
    v_flex,
};
use gpui_kit::prelude::*;
use gpui_kit::*;
use tokio::runtime::Handle;
use tokio::sync::mpsc;

use super::detail::DetailTarget;
use super::direct::DirectAccess;
use super::model::{
    ColumnKind, ReadState, ResourceIdentity, SortDirection, SortKey, StatusTone, natural_cmp,
    status_tone,
};
use super::pane::{DetailEvent, DetailPane, KEYBOARD_PAUSE};
use super::projection::ResourceProjection;
use super::store::{ResourceBatch, ResourceEvent, ResourceStore};
use super::{example, live, navigation};
use crate::backend::{self, OwnedJob};
use crate::desktop::PAGE_PADDING;
use crate::palette::palette;
use crate::screens::{LiveSource, SCREEN_DEADLINE, content_width, mono, panel};
use crate::ui::{self, DISPLAY_FONT, MONO_FONT, clock};

const CONTEXT: &str = "KubeResources";
/// The key context around the filter input, which sits outside the list's.
const FILTER_CONTEXT: &str = "KubeResourcesFilter";
const ROW_HEIGHT: f32 = 28.;
const PAGE_ROWS: isize = 20;
/// Ages are redrawn this often while the page is visible.
const AGE_TICK: Duration = Duration::from_secs(5);
/// Advance of one character in the 12 px table font.
const CHAR_WIDTH: f32 = 7.2;
const CELL_PADDING: f32 = 24.;
const AGE_WIDTH: f32 = 76.;
const MIN_COLUMN: f32 = 64.;
const MAX_COLUMN: f32 = 280.;
const MAX_FLEXIBLE: f32 = 440.;
const LIST_MIN_HEIGHT: f32 = 200.;
/// Below this content width the detail pane stacks under the list.
const SPLIT_WIDTH: f32 = 900.;
const LIST_MIN_WIDTH: f32 = 320.;
const PANE_WIDTH: f32 = 460.;
const PANE_MIN_WIDTH: f32 = 320.;
const PANE_HEIGHT: f32 = 380.;
const PANE_MIN_HEIGHT: f32 = 220.;
/// Space between the list and the pane, where the resize handle sits.
const SPLIT_GAP: f32 = 14.;
const ALL_NAMESPACES: &str = "All namespaces";

actions!(
    kube_resources,
    [
        NextItem,
        PreviousItem,
        FirstItem,
        LastItem,
        NextPage,
        PreviousPage,
        FocusFilter,
        ClearFilter,
        LeaveFilter
    ]
);

/// Where the Resources page reads from. `id` is part of every row's
/// identity, so rows from one connection are never shown or selected as
/// another's; the same `id` keeps a running watch across source updates.
#[derive(Clone)]
pub(crate) struct KubeSource {
    pub(crate) id: String,
    /// The context the scope line names.
    pub(crate) context: String,
    pub(crate) access: KubeAccess,
}

#[derive(Clone)]
pub(crate) enum KubeAccess {
    /// Made-up objects for `--fixture`; nothing is contacted.
    Example,
    /// The Kubernetes client the Talos side sets up for its cluster.
    Talos(Box<LiveSource>),
    /// A kubeconfig context, without Talos.
    Direct(DirectAccess),
}

impl KubeAccess {
    pub(crate) async fn client(&self) -> Result<kube::Client, String> {
        match self {
            KubeAccess::Example => Err("Example data has no Kubernetes client".into()),
            KubeAccess::Talos(live) => live.kubernetes().await,
            KubeAccess::Direct(direct) => {
                direct.connect().await.map(|connection| connection.client)
            }
        }
    }

    /// Drops a reused client after a failure, so the next read rebuilds it.
    pub(crate) fn forget(&self) {
        match self {
            KubeAccess::Example => {}
            KubeAccess::Talos(live) => live.forget_kubernetes(),
            KubeAccess::Direct(direct) => direct.forget(),
        }
    }
}

/// One entry of the namespace picker; `None` lists every namespace.
#[derive(Clone, Debug, PartialEq)]
struct NamespaceChoice {
    label: SharedString,
    value: Option<String>,
}

impl SelectItem for NamespaceChoice {
    type Value = Option<String>;

    fn title(&self) -> SharedString {
        self.label.clone()
    }

    fn value(&self) -> &Self::Value {
        &self.value
    }
}

fn namespace_choices(names: &[String], current: Option<&str>) -> SearchableVec<NamespaceChoice> {
    let mut choices = vec![NamespaceChoice {
        label: ALL_NAMESPACES.into(),
        value: None,
    }];
    let choice = |name: &str| NamespaceChoice {
        label: name.to_owned().into(),
        value: Some(name.to_owned()),
    };
    choices.extend(names.iter().map(|name| choice(name)));
    // The chosen namespace stays pickable when the list lacks it, say
    // because listing namespaces isn't permitted.
    if let Some(current) = current.filter(|current| !names.iter().any(|name| name == current)) {
        choices.push(choice(current));
    }
    SearchableVec::new(choices)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ColumnSource {
    /// A printed column, by index into the store's columns.
    Cell(usize),
    /// The namespace, added when listing every namespace.
    Namespace,
}

#[derive(Clone, Debug)]
struct DisplayColumn {
    label: SharedString,
    source: ColumnSource,
    kind: ColumnKind,
    width: f32,
    /// Takes the room left over; at most one column does.
    flexible: bool,
    /// Printed statuses are coloured by what they mean.
    status: bool,
}

impl DisplayColumn {
    fn sort_key(&self) -> SortKey {
        match self.source {
            ColumnSource::Cell(ix) => SortKey::Column(ix),
            ColumnSource::Namespace => SortKey::Namespace,
        }
    }
}

/// The columns drawn for the current read and their widths. Derived when a
/// read resets, never while drawing; wide (`-o wide`) columns are left out.
#[derive(Clone, Debug, Default)]
struct TableLayout {
    columns: Vec<DisplayColumn>,
    width: f32,
}

impl TableLayout {
    fn new(store: &ResourceStore, namespace_column: bool) -> Self {
        let longest = |chars: &dyn Fn(&super::model::ResourceRow) -> usize| {
            store
                .entries()
                .iter()
                .map(|entry| chars(entry.row()))
                .max()
                .unwrap_or(0)
        };
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
                    longest(&|row| row.cells.get(ix).map_or(0, |cell| cell.chars().count()))
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
                    width: fit(
                        longest(&|row| row.identity.namespace.chars().count()).max(11),
                        MAX_COLUMN,
                    ),
                    flexible: false,
                    status: false,
                });
            }
        }
        let width = columns.iter().map(|column| column.width).sum();
        Self { columns, width }
    }
}

/// The element id of a row: derived from what it shows, so a press that
/// lands after the rows moved can't complete on another object.
pub(crate) fn row_id(identity: &ResourceIdentity) -> ElementId {
    SharedString::from(format!(
        "resource-row:{}/{}/{}",
        identity.namespace, identity.name, identity.uid
    ))
    .into()
}

/// A kind's name for titles: the navigation's label, or a custom kind's
/// own name.
pub(crate) fn title(kind: &ResourceKind) -> SharedString {
    navigation::label(&kind.key())
        .map(SharedString::from)
        .unwrap_or_else(|| kind.kind.clone().into())
}

/// The kind shown isn't served (404), so the sidebar's discovery of its
/// group may be out of date.
pub(crate) struct NotServed(pub(crate) ResourceKind);

pub(crate) struct ResourcesScreen {
    runtime: Handle,
    source: Option<KubeSource>,
    kind: ResourceKind,
    /// `None` lists every namespace. Kept across kinds and ignored for
    /// cluster-scoped ones.
    namespace: Option<String>,
    /// Namespaces for the picker, listed once per connection.
    namespaces: Vec<String>,
    namespaces_for: Option<String>,
    namespace_select: Entity<SelectState<SearchableVec<NamespaceChoice>>>,
    query: Entity<InputState>,
    store: ResourceStore,
    projection: ResourceProjection,
    layout: TableLayout,
    visible: bool,
    /// The selection to find again once a restarted read lists it.
    restore: Option<ResourceIdentity>,
    /// Unix seconds that ages are drawn against.
    now: i64,
    /// When the last batch landed.
    updated: Option<SystemTime>,
    focus: FocusHandle,
    scroll: UniformListScrollHandle,
    watch: Option<(OwnedJob, Task<()>)>,
    namespace_job: Option<(OwnedJob, Task<()>)>,
    tick: Option<Task<()>>,
    /// The selected object in full, beside the list or below it.
    detail: Entity<DetailPane>,
    split: Entity<ResizableState>,
    stacked: Entity<ResizableState>,
    _subscriptions: Vec<Subscription>,
}

impl ResourcesScreen {
    pub(crate) fn new(runtime: Handle, window: &mut Window, cx: &mut Context<Self>) -> Self {
        cx.bind_keys([
            KeyBinding::new("down", NextItem, Some(CONTEXT)),
            KeyBinding::new("up", PreviousItem, Some(CONTEXT)),
            KeyBinding::new("home", FirstItem, Some(CONTEXT)),
            KeyBinding::new("end", LastItem, Some(CONTEXT)),
            KeyBinding::new("pagedown", NextPage, Some(CONTEXT)),
            KeyBinding::new("pageup", PreviousPage, Some(CONTEXT)),
            KeyBinding::new("/", FocusFilter, Some(CONTEXT)),
            KeyBinding::new("escape", ClearFilter, Some(CONTEXT)),
            KeyBinding::new("escape", LeaveFilter, Some(FILTER_CONTEXT)),
        ]);
        let query = cx.new(|cx| InputState::new(window, cx).placeholder("Filter"));
        let detail = cx.new(|cx| DetailPane::new(runtime.clone(), window, cx));
        let namespace_select = cx.new(|cx| {
            SelectState::new(
                namespace_choices(&[], None),
                Some(IndexPath::new(0)),
                window,
                cx,
            )
            .searchable(true)
        });
        let subscriptions = vec![
            cx.subscribe_in(
                &query,
                window,
                |this, input, event, window, cx| match event {
                    InputEvent::Change => {
                        let text = input.read(cx).value().to_string();
                        this.projection.filter(&this.store, &text);
                        this.scroll.scroll_to_item(0, ScrollStrategy::Top);
                        cx.notify();
                    }
                    // Enter hands the keyboard back to the list.
                    InputEvent::PressEnter { .. } => window.focus(&this.focus, cx),
                    _ => {}
                },
            ),
            cx.subscribe_in(
                &namespace_select,
                window,
                |this, _, event: &SelectEvent<SearchableVec<NamespaceChoice>>, window, cx| {
                    let SelectEvent::Confirm(Some(namespace)) = event else {
                        return;
                    };
                    this.set_namespace(namespace.clone(), window, cx);
                },
            ),
            // Caret, selection and menu changes redraw the controls, and this
            // view is cached, so it has to hear about them.
            cx.observe(&query, |_, _, cx| cx.notify()),
            cx.observe(&namespace_select, |_, _, cx| cx.notify()),
            // The picker takes focus back when its menu closes by choice or
            // Escape; the keyboard belongs to the list then. A menu closed
            // by clicking elsewhere leaves focus where the click put it.
            cx.subscribe_in(
                &namespace_select,
                window,
                |this, select, _: &DismissEvent, window, cx| {
                    if select.focus_handle(cx).is_focused(window) {
                        window.focus(&this.focus, cx);
                    }
                },
            ),
            cx.subscribe_in(
                &detail,
                window,
                |this, _, event: &DetailEvent, window, cx| match event {
                    DetailEvent::Closed => {
                        this.close_detail(cx);
                        window.focus(&this.focus, cx);
                    }
                    DetailEvent::Open(identity) => {
                        this.select_identity(identity, window, cx);
                        this.open_detail(identity.clone(), Duration::ZERO, cx);
                    }
                },
            ),
        ];
        Self {
            runtime,
            source: None,
            kind: builtin(navigation::DEFAULT_KIND).expect("the default kind is built in"),
            namespace: None,
            namespaces: Vec::new(),
            namespaces_for: None,
            namespace_select,
            query,
            store: ResourceStore::new(),
            projection: ResourceProjection::new(),
            layout: TableLayout::default(),
            visible: false,
            restore: None,
            now: live::now(),
            updated: None,
            focus: cx.focus_handle(),
            scroll: UniformListScrollHandle::new(),
            watch: None,
            namespace_job: None,
            tick: None,
            detail,
            split: cx.new(|_| ResizableState::default()),
            stacked: cx.new(|_| ResizableState::default()),
            _subscriptions: subscriptions,
        }
    }

    /// A source with the same `id` only refreshes the handles; another one
    /// starts over, keeping nothing from the old connection.
    pub(crate) fn set_source(
        &mut self,
        source: Option<KubeSource>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let same = self.source.as_ref().map(|source| &source.id)
            == source.as_ref().map(|source| &source.id);
        self.source = source;
        if same {
            if let Some(source) = &self.source {
                let access = source.access.clone();
                self.detail
                    .update(cx, |detail, cx| detail.set_access(access, cx));
            }
            return;
        }
        self.close_detail(cx);
        self.projection.select(&self.store, None);
        self.restore = None;
        self.namespace = None;
        self.namespaces.clear();
        self.namespaces_for = None;
        self.namespace_job = None;
        self.sync_namespace_choices(window, cx);
        self.restart(window, cx);
    }

    /// Shows another kind; the same kind at another version reads again.
    pub(crate) fn set_kind(
        &mut self,
        kind: ResourceKind,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if kind == self.kind {
            return;
        }
        self.kind = kind;
        self.close_detail(cx);
        self.restore = None;
        self.projection.reset_sort();
        self.scroll.scroll_to_item(0, ScrollStrategy::Top);
        self.restart(window, cx);
    }

    /// The detail pane, which draws as a cached view of its own.
    pub(crate) fn detail_view(&self) -> EntityId {
        self.detail.entity_id()
    }

    /// Only a visible page reads: showing it lists again and watches,
    /// hiding it drops the watch.
    pub(crate) fn set_visible(
        &mut self,
        visible: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.visible == visible {
            return;
        }
        self.visible = visible;
        self.detail
            .update(cx, |detail, cx| detail.set_active(visible, cx));
        if visible {
            self.restart(window, cx);
            self.tick = Some(cx.spawn(async move |this, cx| {
                loop {
                    cx.background_executor().timer(AGE_TICK).await;
                    if this.update(cx, |view, cx| view.tick(cx)).is_err() {
                        break;
                    }
                }
            }));
        } else {
            self.watch = None;
            self.tick = None;
            if self.namespace_job.take().is_some() {
                self.namespaces_for = None;
            }
        }
    }

    /// Lists again from scratch, namespaces included.
    pub(crate) fn refresh(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.visible {
            return;
        }
        self.namespaces_for = None;
        self.restart(window, cx);
        self.detail.update(cx, |detail, cx| detail.refresh(cx));
    }

    pub(crate) fn focus(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        window.focus(&self.focus, cx);
    }

    fn lists_all_namespaces(&self) -> bool {
        self.kind.namespaced && self.namespace.is_none()
    }

    fn title(&self) -> SharedString {
        title(&self.kind)
    }

    /// The kind as a plural noun inside a sentence: the navigation's label,
    /// or a custom kind's resource name ("certificates").
    fn noun(&self) -> String {
        navigation::label(&self.kind.key())
            .map(str::to_lowercase)
            .unwrap_or_else(|| self.kind.plural.clone())
    }

    /// Why the kind isn't served. A custom kind's definition can be removed;
    /// a built-in kind's version can be newer or older than the server.
    fn not_served(&self, context: &str) -> String {
        let why = if navigation::group_of(&self.kind.key()).is_some() {
            "The server may predate that version, or no longer serve it."
        } else {
            "Its definition may have been removed, or that version is no longer served."
        };
        format!(
            "The API server of {context} doesn't serve {} at {}. {why}",
            self.noun(),
            self.kind.api_version()
        )
    }

    /// Starts a new read session: forgets the rows, and when visible lists
    /// and watches the kind for the current connection and namespace.
    fn restart(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.watch = None;
        if let Some(selected) = self.projection.selected() {
            self.restore = Some(selected.clone());
        }
        let epoch = self.store.start_session();
        self.projection.rebuild(&self.store);
        self.layout = TableLayout::default();
        self.updated = None;
        self.now = live::now();
        cx.notify();
        if !self.visible {
            return;
        }
        let Some(source) = self.source.clone() else {
            return;
        };
        self.load_namespaces(window, cx);
        let kind = self.kind.clone();
        let namespace = self.namespace.clone().filter(|_| kind.namespaced);
        if let KubeAccess::Example = source.access {
            let events =
                match example::read(&source.context, &kind.key(), namespace.as_deref(), self.now) {
                    Some((columns, rows)) => vec![
                        ResourceEvent::Reset { columns, rows },
                        ResourceEvent::Read(ReadState::Loaded),
                    ],
                    None => vec![ResourceEvent::Read(ReadState::Loaded)],
                };
            self.apply(ResourceBatch { epoch, events }, cx);
            return;
        }
        let (sender, mut receiver) = mpsc::channel(8);
        let job = OwnedJob::new(self.runtime.spawn(watch(
            source.access,
            kind,
            namespace,
            source.id,
            sender,
        )));
        let task = cx.spawn(async move |this, cx| {
            while let Some(events) = receiver.recv().await {
                let applied = this.update(cx, |view, cx| {
                    view.apply(ResourceBatch { epoch, events }, cx)
                });
                if applied.is_err() {
                    break;
                }
            }
        });
        self.watch = Some((job, task));
    }

    fn apply(&mut self, batch: ResourceBatch, cx: &mut Context<Self>) {
        let reset = batch
            .events
            .iter()
            .any(|event| matches!(event, ResourceEvent::Reset { .. }));
        let served = !matches!(self.store.read_state(), ReadState::Missing(_));
        if !self.store.apply(batch) {
            return;
        }
        if served && let ReadState::Missing(_) = self.store.read_state() {
            // Nothing of the kind can be shown any more.
            self.close_detail(cx);
            cx.emit(NotServed(self.kind.clone()));
        }
        self.updated = Some(SystemTime::now());
        self.now = live::now();
        self.projection.rebuild(&self.store);
        if reset {
            self.layout = TableLayout::new(&self.store, self.lists_all_namespaces());
            // A restarted read selects the same object again if it still
            // exists; only its first list can tell.
            if let Some(identity) = self.restore.take() {
                self.projection.select_identity(&self.store, &identity);
                if let Some(ix) = self.projection.selected_index() {
                    self.scroll.scroll_to_item(ix, ScrollStrategy::Nearest);
                }
            }
        }
        self.follow_detail(cx);
        cx.notify();
    }

    /// Tells the pane what the list now shows of its object: a new version,
    /// or that it is gone. Only a list that shows rows can tell.
    fn follow_detail(&mut self, cx: &mut Context<Self>) {
        if !self.store.read_state().shows_rows() {
            return;
        }
        let Some(target) = self.detail.read(cx).target_identity().cloned() else {
            return;
        };
        match self.store.get(&target) {
            Some(row) => {
                let version = row.resource_version.clone();
                self.detail
                    .update(cx, |detail, cx| detail.observed(&version, cx));
            }
            None => {
                let successor = self
                    .store
                    .entries()
                    .iter()
                    .map(|entry| &entry.row().identity)
                    .find(|identity| {
                        identity.namespace == target.namespace && identity.name == target.name
                    })
                    .cloned();
                self.detail
                    .update(cx, |detail, cx| detail.gone(successor, cx));
            }
        }
    }

    /// Shows `identity` in the pane, reading it after `delay`.
    fn open_detail(&mut self, identity: ResourceIdentity, delay: Duration, cx: &mut Context<Self>) {
        let Some(source) = self.source.as_ref() else {
            return;
        };
        let version = self
            .store
            .get(&identity)
            .map(|row| row.resource_version.clone())
            .unwrap_or_default();
        let target = DetailTarget {
            identity,
            kind: self.kind.clone(),
        };
        let access = source.access.clone();
        self.detail.update(cx, |detail, cx| {
            detail.open(target, access, &version, delay, cx)
        });
        cx.notify();
    }

    /// Closes the pane and clears the selection it showed.
    fn close_detail(&mut self, cx: &mut Context<Self>) {
        self.detail.update(cx, |detail, cx| detail.close(cx));
        self.projection.select(&self.store, None);
        self.restore = None;
        cx.notify();
    }

    fn tick(&mut self, cx: &mut Context<Self>) {
        self.now = live::now();
        let ages = self
            .layout
            .columns
            .iter()
            .any(|column| column.kind == ColumnKind::Age);
        if ages && self.projection.len() > 0 {
            cx.notify();
        }
    }

    fn set_namespace(
        &mut self,
        namespace: Option<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.namespace == namespace {
            return;
        }
        self.namespace = namespace;
        self.sync_namespace_choices(window, cx);
        self.scroll.scroll_to_item(0, ScrollStrategy::Top);
        if self.kind.namespaced {
            self.close_detail(cx);
            self.restart(window, cx);
        }
    }

    fn load_namespaces(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(source) = self.source.clone() else {
            return;
        };
        if !self.kind.namespaced || self.namespaces_for.as_ref() == Some(&source.id) {
            return;
        }
        self.namespaces_for = Some(source.id);
        let access = match source.access {
            KubeAccess::Example => {
                self.namespaces = example::namespaces();
                self.sync_namespace_choices(window, cx);
                return;
            }
            access => access,
        };
        let (job, receiver) = backend::spawn_job(
            &self.runtime,
            SCREEN_DEADLINE,
            "Listing namespaces timed out".into(),
            async move {
                let client = access.client().await?;
                let kind = builtin("namespaces").ok_or("Namespaces aren't a known kind")?;
                let table = list_table(&client, &kind, None)
                    .await
                    .map_err(|failure| failure.to_string())?;
                let mut names: Vec<String> = table
                    .rows
                    .iter()
                    .filter_map(|row| row.metadata())
                    .map(|metadata| metadata.name.clone())
                    .filter(|name| !name.is_empty())
                    .collect();
                names.sort_by(|left, right| natural_cmp(left, right));
                Ok(names)
            },
        );
        let task = cx.spawn_in(window, async move |this, cx| {
            let result = receiver.await;
            _ = this.update_in(cx, |view, window, cx| {
                view.namespace_job = None;
                // A refused or failed list leaves the picker with every
                // namespace and the current one.
                if let Ok(Ok(names)) = result {
                    view.namespaces = names;
                    view.sync_namespace_choices(window, cx);
                }
            });
        });
        self.namespace_job = Some((job, task));
    }

    fn sync_namespace_choices(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let choices = namespace_choices(&self.namespaces, self.namespace.as_deref());
        let value = self.namespace.clone();
        self.namespace_select.update(cx, |state, cx| {
            state.set_items(choices, window, cx);
            state.set_selected_value(&value, window, cx);
        });
        cx.notify();
    }

    fn select_identity(
        &mut self,
        identity: &ResourceIdentity,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.restore = None;
        self.projection.select_identity(&self.store, identity);
        window.focus(&self.focus, cx);
        cx.notify();
    }

    /// A click opens the row at once.
    fn click_row(
        &mut self,
        identity: &ResourceIdentity,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.select_identity(identity, window, cx);
        self.open_detail(identity.clone(), Duration::ZERO, cx);
    }

    fn step(&mut self, delta: isize, cx: &mut Context<Self>) {
        let count = self.projection.len();
        if count == 0 {
            return;
        }
        let next = match self.projection.selected_index() {
            Some(ix) => ix.saturating_add_signed(delta).min(count - 1),
            None if delta < 0 => count - 1,
            None => 0,
        };
        self.restore = None;
        self.projection.select(&self.store, Some(next));
        self.scroll.scroll_to_item(next, ScrollStrategy::Nearest);
        if let Some(identity) = self.projection.selected().cloned() {
            self.open_detail(identity, KEYBOARD_PAUSE, cx);
        }
        cx.notify();
    }

    /// Header clicks cycle ascending, descending, then the order rows
    /// arrived in.
    fn sort_by(&mut self, key: SortKey, cx: &mut Context<Self>) {
        let (current, direction) = self.projection.sort_state();
        let direction = if current == key {
            direction.next()
        } else {
            SortDirection::Ascending
        };
        self.projection.sort(&self.store, key, direction);
        // The selection follows its object to wherever the sort put it.
        if let Some(ix) = self.projection.selected_index() {
            self.scroll.scroll_to_item(ix, ScrollStrategy::Nearest);
        }
        cx.notify();
    }

    /// Escape clears the filter, or with none, closes the pane.
    fn escape(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.query.read(cx).value().is_empty() {
            self.clear_filter(window, cx);
        } else if self.detail.read(cx).target_identity().is_some() {
            self.close_detail(cx);
        }
    }

    /// Escape in the filter clears it; in an empty filter it hands the
    /// keyboard back to the list.
    fn leave_filter(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.query.read(cx).value().is_empty() {
            window.focus(&self.focus, cx);
        } else {
            self.clear_filter(window, cx);
        }
    }

    fn clear_filter(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        // Setting the value from code emits no change event.
        self.query
            .update(cx, |input, cx| input.set_value("", window, cx));
        self.projection.filter(&self.store, "");
        cx.notify();
    }

    fn header(&self, cx: &mut Context<Self>) -> Div {
        let p = palette(cx);
        let example = self
            .source
            .as_ref()
            .is_some_and(|source| matches!(source.access, KubeAccess::Example));
        let read = self.store.read_state();
        let state = match read {
            ReadState::Loading => "loading",
            ReadState::Loaded if example => "example data",
            ReadState::Loaded => "watching",
            ReadState::Stale(_) => "reconnecting",
            ReadState::Refused(_) => "not permitted",
            ReadState::Failed(_) => "failed",
            ReadState::Missing(_) => "not served",
        };
        let (total, shown) = (self.store.len(), self.projection.len());
        let scope = h_flex()
            .id("resource-scope")
            .test_support()
            .gap_1p5()
            .flex_wrap()
            .text_size(px(12.5))
            .text_color(p.muted)
            .map(|this| match &self.source {
                Some(source) => this.child("in").child(mono(source.context.clone())),
                None => this.child("not connected"),
            })
            .when(read.shows_rows(), |this| {
                this.child("·").child(if shown == total {
                    total.to_string()
                } else {
                    format!("{shown} of {total}")
                })
            })
            .when(self.source.is_some(), |this| this.child("·").child(state))
            .when_some(self.updated, |this, time| {
                this.child("·").child(clock(time))
            });
        // The controls keep to one row: beside the title when they fit,
        // below it otherwise, where a narrow page shrinks the filter. (A
        // wrapping row would be measured without its gaps and wrap early.)
        let controls = h_flex()
            .max_w_full()
            .gap_2()
            .when(self.kind.namespaced, |this| {
                this.child(
                    div().flex_none().child(
                        Select::new(&self.namespace_select)
                            .id("resource-namespace")
                            .small()
                            .w(px(200.))
                            .menu_width(px(260.))
                            .search_placeholder("Find a namespace")
                            .accessibility_label("Namespace"),
                    ),
                )
            })
            .child(
                div()
                    .w(px(240.))
                    .min_w(px(120.))
                    .key_context(FILTER_CONTEXT)
                    .on_action(cx.listener(|view, _: &LeaveFilter, window, cx| {
                        view.leave_filter(window, cx)
                    }))
                    .child(
                        Input::new(&self.query)
                            .id("resource-filter")
                            .aria_label("Filter by name, namespace or any column")
                            .small()
                            .cleanable(true)
                            .prefix(Icon::new(IconName::Search).with_size(px(14.))),
                    ),
            )
            .child(
                div().flex_none().child(
                    Button::new("resource-refresh")
                        .outline()
                        .small()
                        .icon(IconName::RefreshCw)
                        .label("Refresh")
                        .on_click(cx.listener(|view, _, window, cx| view.refresh(window, cx))),
                ),
            );
        h_flex()
            .items_end()
            .gap_3()
            .flex_wrap()
            .child(
                v_flex()
                    .flex_1()
                    .min_w(px(240.))
                    .gap(px(7.))
                    .child(
                        div()
                            .id("resource-title")
                            .test_support()
                            .font_family(DISPLAY_FONT)
                            .text_size(px(28.))
                            .line_height(px(32.))
                            .child(self.title()),
                    )
                    .child(scope),
            )
            .child(controls)
    }

    fn head(&self, cx: &mut Context<Self>) -> Div {
        let p = palette(cx);
        let (key, direction) = self.projection.sort_state();
        h_flex()
            .w_full()
            .py(px(7.))
            .border_b_1()
            .border_color(p.line)
            .children(self.layout.columns.iter().enumerate().map(|(ix, column)| {
                let sort = column.sort_key();
                let order =
                    (key == sort)
                        .then_some(direction)
                        .and_then(|direction| match direction {
                            SortDirection::Ascending => Some(("ascending", IconName::ArrowUp)),
                            SortDirection::Descending => Some(("descending", IconName::ArrowDown)),
                            SortDirection::Default => None,
                        });
                cell(column)
                    .id(("resource-sort", ix))
                    .test_support()
                    .role(Role::ColumnHeader)
                    .aria_label(match order {
                        Some((order, _)) => format!("{}, sorted {order}", column.label),
                        None => column.label.to_string(),
                    })
                    .flex()
                    .items_center()
                    .gap_1()
                    .cursor_pointer()
                    .child(ui::caption(&column.label, cx))
                    .children(
                        order.map(|(_, icon)| {
                            Icon::new(icon).with_size(px(12.)).text_color(p.muted)
                        }),
                    )
                    .on_click(cx.listener(move |view, _, _, cx| view.sort_by(sort, cx)))
            }))
    }

    fn render_row(&self, ix: usize, cx: &mut Context<Self>) -> Option<AnyElement> {
        let row = self.projection.row(&self.store, ix)?;
        let p = palette(cx);
        let selected = self.projection.selected_index() == Some(ix);
        let identity = row.identity.clone();
        let mut element = h_flex()
            .id(row_id(&identity))
            .test_support()
            .role(Role::ListBoxOption)
            .aria_selected(selected)
            .aria_label(format!(
                "{} · {}",
                identity.address(),
                row.cells.join(" · ")
            ))
            .w_full()
            .h(px(ROW_HEIGHT))
            .font_family(MONO_FONT)
            .text_size(px(12.))
            .cursor_pointer()
            .when(row.terminating, |this| this.text_color(p.muted))
            .when(selected, |this| this.bg(p.accent_soft))
            .when(!selected, |this| this.hover(|style| style.bg(p.hover)));
        for column in &self.layout.columns {
            let text: SharedString = match column.source {
                ColumnSource::Namespace => identity.namespace.clone().into(),
                ColumnSource::Cell(cell_ix) => {
                    let printed = row.cells.get(cell_ix).map(String::as_str).unwrap_or("");
                    match column.kind {
                        ColumnKind::Age => row
                            .age(self.now)
                            .map(SharedString::from)
                            .unwrap_or_else(|| printed.to_owned().into()),
                        _ => printed.to_owned().into(),
                    }
                }
            };
            let tone = column
                .status
                .then(|| match status_tone(&text) {
                    StatusTone::Success => Some(p.good_ink),
                    StatusTone::Warning => Some(p.warn_ink),
                    StatusTone::Danger => Some(p.crit_ink),
                    StatusTone::Neutral => None,
                })
                .flatten();
            element = element.child(
                cell(column)
                    .when_some(tone, |this, color| this.text_color(color))
                    .child(text),
            );
        }
        Some(
            element
                .on_click(
                    cx.listener(move |view, _, window, cx| view.click_row(&identity, window, cx)),
                )
                .into_any_element(),
        )
    }

    fn table(&self, cx: &mut Context<Self>) -> AnyElement {
        let p = palette(cx);
        let count = self.projection.len();
        let title = self.noun();
        let empty = (count == 0).then(|| {
            if !self.store.is_empty() {
                format!("No {title} match this filter.")
            } else if let Some(namespace) = self.namespace.as_ref().filter(|_| self.kind.namespaced)
            {
                format!("No {title} in {namespace}.")
            } else {
                format!("No {title} found.")
            }
        });
        let list = div()
            .id("resource-list")
            .test_support()
            .role(Role::ListBox)
            .aria_label(format!(
                "{}; arrows select and show details, slash filters, Escape clears the filter or closes the details",
                self.title()
            ))
            .flex_1()
            .min_h_0()
            .map(|this| match empty {
                Some(text) => this.child(
                    div()
                        .id("resource-empty")
                        .test_support()
                        .px_3()
                        .py_3p5()
                        .text_size(px(12.5))
                        .text_color(p.muted)
                        .child(text),
                ),
                None => this.child(
                    uniform_list(
                        "resource-rows",
                        count,
                        cx.processor(|view, range: Range<usize>, _, cx| {
                            range
                                .filter_map(|ix| view.render_row(ix, cx))
                                .collect::<Vec<_>>()
                        }),
                    )
                    .track_scroll(&self.scroll)
                    .size_full(),
                ),
            });
        panel(cx)
            .flex_1()
            .min_h(px(LIST_MIN_HEIGHT))
            .overflow_hidden()
            .child(
                div()
                    .id("resource-table-scroll")
                    .test_support()
                    .size_full()
                    .overflow_x_scroll()
                    .child(
                        v_flex()
                            .h_full()
                            .w_full()
                            .min_w(px(self.layout.width))
                            .child(self.head(cx))
                            .child(list),
                    ),
            )
            .into_any_element()
    }

    /// The list's keys and the page's focus, around the table or whatever
    /// replaces it, so the keyboard keeps working while no rows show.
    fn keyed(&self, body: AnyElement, cx: &mut Context<Self>) -> AnyElement {
        v_flex()
            .id("resource-body")
            .test_support()
            .key_context(CONTEXT)
            .track_focus(&self.focus)
            .on_action(cx.listener(|view, _: &NextItem, _, cx| view.step(1, cx)))
            .on_action(cx.listener(|view, _: &PreviousItem, _, cx| view.step(-1, cx)))
            .on_action(cx.listener(|view, _: &FirstItem, _, cx| view.step(isize::MIN, cx)))
            .on_action(cx.listener(|view, _: &LastItem, _, cx| view.step(isize::MAX, cx)))
            .on_action(cx.listener(|view, _: &NextPage, _, cx| view.step(PAGE_ROWS, cx)))
            .on_action(cx.listener(|view, _: &PreviousPage, _, cx| view.step(-PAGE_ROWS, cx)))
            .on_action(cx.listener(|view, _: &FocusFilter, window, cx| {
                let focus = view.query.read(cx).focus_handle(cx);
                window.focus(&focus, cx);
            }))
            .on_action(cx.listener(|view, _: &ClearFilter, window, cx| view.escape(window, cx)))
            .flex_1()
            .min_h_0()
            .min_w_0()
            .w_full()
            .child(body)
            .into_any_element()
    }

    fn retry(&self, id: &'static str, cx: &mut Context<Self>) -> Button {
        Button::new(id)
            .icon(IconName::RefreshCw)
            .label("Retry now")
            .on_click(cx.listener(|view, _, window, cx| view.refresh(window, cx)))
    }

    /// What replaces the table: no connection, the first list, a refusal,
    /// a failure with nothing to show, or a kind the example data lacks.
    fn placeholder(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let title = self.title();
        let scope = match self.namespace.as_ref().filter(|_| self.kind.namespaced) {
            Some(namespace) => format!("in {namespace}"),
            None if self.kind.namespaced => "across all namespaces".to_owned(),
            None => "in this cluster".to_owned(),
        };
        let state = |id: &'static str, element: Div| {
            Some(
                element
                    .id(id)
                    .test_support()
                    .role(Role::Status)
                    .into_any_element(),
            )
        };
        let Some(source) = self.source.as_ref() else {
            return state(
                "resource-disconnected",
                ui::empty_state(
                    IconName::Unplug,
                    "Not connected to Kubernetes",
                    "Resources are read through the cluster's Kubernetes API once a context is connected. If it doesn't connect, check the kubeconfig in Settings.",
                    None,
                    Vec::new(),
                    cx,
                ),
            );
        };
        match self.store.read_state() {
            ReadState::Loading => state(
                "resource-loading",
                panel(cx)
                    .p_3()
                    .gap_3()
                    .children((0..9).map(|_| ui::skeleton(relative(0.7), px(12.)))),
            ),
            ReadState::Refused(reason) => state(
                "resource-refused",
                ui::empty_state(
                    IconName::ShieldX,
                    format!("Not permitted to list {title}"),
                    format!(
                        "The identity of {} may not list {} {scope}. That says nothing about whether any exist.",
                        source.context,
                        self.noun()
                    ),
                    Some(reason.clone()),
                    Vec::new(),
                    cx,
                ),
            ),
            ReadState::Missing(reason) => state(
                "resource-not-served",
                ui::empty_state(
                    IconName::SearchX,
                    format!("{title} isn't served here"),
                    self.not_served(&source.context),
                    Some(reason.clone()),
                    vec![self.retry("resource-missing-retry", cx).into_any_element()],
                    cx,
                ),
            ),
            ReadState::Failed(reason) => state(
                "resource-failed",
                ui::empty_state(
                    IconName::CircleDashed,
                    format!("Couldn't list {title}"),
                    "Nothing is known yet, so nothing is shown as missing. A failed list retries by itself; Retry now starts over.",
                    Some(reason.clone()),
                    vec![
                        self.retry("resource-retry", cx)
                            .primary()
                            .into_any_element(),
                    ],
                    cx,
                ),
            ),
            ReadState::Loaded
                if self.store.columns().is_empty()
                    && matches!(source.access, KubeAccess::Example) =>
            {
                let kinds: Vec<SharedString> = example::KINDS
                    .iter()
                    .filter_map(|key| example::kind(key))
                    .map(|kind| self::title(&kind))
                    .collect();
                state(
                    "resource-not-in-example",
                    ui::empty_state(
                        IconName::Inbox,
                        format!("Example data doesn't include {title}"),
                        format!(
                            "It includes {}. Connect to a cluster to list every kind.",
                            kinds.join(", ")
                        ),
                        None,
                        Vec::new(),
                        cx,
                    ),
                )
            }
            _ => None,
        }
    }

    fn stale_banner(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let ReadState::Stale(reason) = self.store.read_state() else {
            return None;
        };
        let seen = self
            .updated
            .map(|time| format!(" at {}", clock(time)))
            .unwrap_or_default();
        Some(
            div()
                .id("resource-stale")
                .test_support()
                .role(Role::Status)
                .child(ui::warning_banner(
                    Some("The watch was interrupted; reconnecting.".into()),
                    format!("Showing {} as last seen{seen}. {reason}", self.noun()),
                    Some(
                        self.retry("resource-stale-retry", cx)
                            .small()
                            .into_any_element(),
                    ),
                    cx,
                ))
                .into_any_element(),
        )
    }
}

fn cell(column: &DisplayColumn) -> Div {
    let cell = div().px_3().min_w_0().whitespace_nowrap().truncate();
    if column.flexible {
        cell.flex_1().min_w(px(column.width))
    } else {
        cell.flex_none().w(px(column.width))
    }
}

/// Lists and watches one collection on Tokio, converting batches there so
/// the UI thread only applies them. Stops when the receiver goes away.
async fn watch(
    access: KubeAccess,
    kind: ResourceKind,
    namespace: Option<String>,
    connection: String,
    sender: mpsc::Sender<Vec<ResourceEvent>>,
) {
    let key = kind.key();
    let client = match access.client().await {
        Ok(client) => client,
        Err(error) => {
            access.forget();
            let _ = sender
                .send(vec![ResourceEvent::Read(ReadState::Failed(error))])
                .await;
            return;
        }
    };
    let (raw_sender, mut raw) = mpsc::channel::<WatchBatch>(4);
    let forward = async move {
        while let Some(batch) = raw.recv().await {
            let transient = batch.events.iter().any(|event| {
                matches!(event, WatchEvent::Failed { failure, .. } if !failure.kind.is_permanent())
            });
            if transient {
                access.forget();
            }
            if sender
                .send(live::convert(&connection, &key, batch.events))
                .await
                .is_err()
            {
                break;
            }
        }
    };
    tokio::join!(
        watch_collection(client, kind, namespace, raw_sender),
        forward
    );
}

impl EventEmitter<NotServed> for ResourcesScreen {}

impl Render for ResourcesScreen {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        crate::desktop::probe::hit("resources");
        let list = self.placeholder(cx).unwrap_or_else(|| self.table(cx));
        let list = self.keyed(list, cx);
        let body = if self.detail.read(cx).target_identity().is_some() {
            // Cached: list updates and age ticks don't redraw the pane.
            let pane =
                AnyView::from(self.detail.clone()).cached(StyleRefinement::default().size_full());
            let split = if content_width(window) >= px(SPLIT_WIDTH) {
                h_resizable("resource-split")
                    .with_state(&self.split)
                    .child(
                        resizable_panel()
                            .size_range(px(LIST_MIN_WIDTH)..Pixels::MAX)
                            .child(list),
                    )
                    .child(
                        resizable_panel()
                            .size(px(PANE_WIDTH))
                            .size_range(px(PANE_MIN_WIDTH)..Pixels::MAX)
                            .flex_none()
                            .pl(px(SPLIT_GAP))
                            .child(pane),
                    )
            } else {
                v_resizable("resource-split-stacked")
                    .with_state(&self.stacked)
                    .child(
                        resizable_panel()
                            .size_range(px(LIST_MIN_HEIGHT)..Pixels::MAX)
                            .child(list),
                    )
                    .child(
                        resizable_panel()
                            .size(px(PANE_HEIGHT))
                            .size_range(px(PANE_MIN_HEIGHT)..Pixels::MAX)
                            .flex_none()
                            .pt(px(SPLIT_GAP))
                            .child(pane),
                    )
            };
            v_flex().flex_1().min_h_0().child(split).into_any_element()
        } else {
            list
        };
        v_flex()
            .id("resources-page")
            .size_full()
            .min_h_0()
            .px(px(PAGE_PADDING))
            .pt(px(22.))
            .pb(px(18.))
            .gap(px(14.))
            .child(self.header(cx))
            .children(self.stale_banner(cx))
            .child(body)
    }
}

#[cfg(test)]
mod tests;
