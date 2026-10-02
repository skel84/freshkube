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
use super::pane::shell::unless_shell;
use super::pane::{DetailEvent, DetailPane, KEYBOARD_PAUSE, NextTab, PreviousTab};
use super::projection::ResourceProjection;
use super::store::{ResourceBatch, ResourceEvent, ResourceStore};
use super::{example, live, navigation};
use crate::backend::{self, OwnedJob};
use crate::desktop::PAGE_PADDING;
use crate::palette::palette;
use crate::screens::{LiveSource, SCREEN_DEADLINE, content_width, mono, panel};
use crate::ui::{self, DISPLAY_FONT, MONO_FONT, clock, dp, dp_px};

const CONTEXT: &str = "KubeResources";
/// The key context around the filter input, which sits outside the list's.
const EMBEDDED_CONTEXT: &str = "NodePods";
const FILTER_CONTEXT: &str = "KubeResourcesFilter";
const ROW_HEIGHT: f32 = 28.;
const PAGE_ROWS: isize = 20;
/// Ages are redrawn this often while the page is visible.
const AGE_TICK: Duration = Duration::from_secs(5);
/// The least time between two watch batches applied during a burst. A change
/// after a quiet spell shows at once; while changes keep coming, they are
/// gathered and applied together, so a churning list re-sorts and redraws at
/// most ten times a second instead of once per batch core sends.
const WATCH_COALESCE: Duration = Duration::from_millis(100);
/// Advance of one character in the 12 px table font.
const CHAR_WIDTH: f32 = 7.2;
/// The namespace picker's width in the toolbar.
const NAMESPACE_WIDTH: f32 = 200.;
/// The narrowest one-row toolbar without the namespace picker: the
/// filter at its narrowest, the Refresh button and the gap between them.
const CONTROLS_MIN_WIDTH: f32 = 120. + 8. + 96.;
const CELL_PADDING: f32 = 24.;
const AGE_WIDTH: f32 = 76.;
const MIN_COLUMN: f32 = 64.;
const MAX_COLUMN: f32 = 280.;
const MAX_FLEXIBLE: f32 = 440.;
/// The table's header and a couple of rows.
const LIST_MIN_HEIGHT: f32 = 96.;
/// Below this content width the detail pane stacks under the list.
const SPLIT_WIDTH: f32 = 900.;
const LIST_MIN_WIDTH: f32 = 320.;
const PANE_WIDTH: f32 = 460.;
const PANE_MIN_WIDTH: f32 = 320.;
/// Stacked, the list and the pane share the height one to two, so a short
/// window still leaves the pane room for a few lines of YAML or logs.
const STACKED_LIST_HEIGHT: f32 = 190.;
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
        LeaveFilter,
        OpenSelected,
        ChooseNamespace
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
    field_selector: Option<String>,
    embedded: bool,
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
            KeyBinding::new("secondary-f", FocusFilter, Some(CONTEXT)),
            KeyBinding::new("escape", ClearFilter, Some(CONTEXT)),
            KeyBinding::new("enter", OpenSelected, Some(CONTEXT)),
            KeyBinding::new("n", ChooseNamespace, Some(CONTEXT)),
            // Command-Shift-] and [, as macOS reports them.
            KeyBinding::new("secondary-}", NextTab, Some(CONTEXT)),
            KeyBinding::new("secondary-{", PreviousTab, Some(CONTEXT)),
            KeyBinding::new("escape", LeaveFilter, Some(FILTER_CONTEXT)),
        ]);
        cx.bind_keys([
            KeyBinding::new("down", NextItem, Some(EMBEDDED_CONTEXT)),
            KeyBinding::new("up", PreviousItem, Some(EMBEDDED_CONTEXT)),
            KeyBinding::new("home", FirstItem, Some(EMBEDDED_CONTEXT)),
            KeyBinding::new("end", LastItem, Some(EMBEDDED_CONTEXT)),
            KeyBinding::new("pagedown", NextPage, Some(EMBEDDED_CONTEXT)),
            KeyBinding::new("pageup", PreviousPage, Some(EMBEDDED_CONTEXT)),
            KeyBinding::new("/", FocusFilter, Some(EMBEDDED_CONTEXT)),
            KeyBinding::new("secondary-f", FocusFilter, Some(EMBEDDED_CONTEXT)),
            KeyBinding::new("escape", ClearFilter, Some(EMBEDDED_CONTEXT)),
            KeyBinding::new("enter", OpenSelected, Some(EMBEDDED_CONTEXT)),
        ]);
        let query = cx.new(|cx| InputState::new(window, cx).placeholder("Filter  /"));
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
                    DetailEvent::Closed => this.close_pane(window, cx),
                    DetailEvent::Leave => window.focus(&this.focus, cx),
                    DetailEvent::Open(identity) => {
                        this.select_identity(identity, window, cx);
                        this.open_now(identity.clone(), window, |_, _, _| {}, cx);
                    }
                },
            ),
        ];
        Self {
            field_selector: None,
            embedded: false,
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

    pub(crate) fn set_node(
        &mut self,
        name: Option<&str>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.embedded = true;
        let selector = name.map(|name| format!("spec.nodeName={name}"));
        if self.field_selector == selector {
            return;
        }
        self.field_selector = selector;
        self.namespace = None;
        self.restore = None;
        self.projection.select(&self.store, None);
        self.restart(window, cx);
    }

    #[cfg(test)]
    pub(crate) fn filter_value(&self, cx: &App) -> String {
        self.query.read(cx).value().to_string()
    }
    #[cfg(test)]
    pub(crate) fn detail_tab(&self, cx: &App) -> crate::resources::Tab {
        self.detail.read(cx).tab()
    }

    /// Called only after the shell's navigation question has been accepted.
    pub(crate) fn close_for_link(&mut self, cx: &mut Context<Self>) {
        self.close_detail(cx);
    }

    pub(crate) fn identity_named(&self, namespace: &str, name: &str) -> Option<ResourceIdentity> {
        self.store
            .entries()
            .iter()
            .find(|entry| {
                entry.row().identity.namespace == namespace && entry.row().identity.name == name
            })
            .map(|entry| entry.row().identity.clone())
    }
    pub(crate) fn showing(&self, identity: &ResourceIdentity, cx: &App) -> bool {
        self.detail.read(cx).target_identity() == Some(identity)
    }
    pub(crate) fn set_filter(&mut self, text: &str, window: &mut Window, cx: &mut Context<Self>) {
        self.query
            .update(cx, |input, cx| input.set_value(text.to_owned(), window, cx));
        self.projection.filter(&self.store, text);
        self.scroll.scroll_to_item(0, ScrollStrategy::Top);
        window.focus(&self.focus, cx);
        cx.notify();
    }
    pub(crate) fn open_identity_on(
        &mut self,
        identity: ResourceIdentity,
        tab: crate::resources::Tab,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self
            .namespace
            .as_ref()
            .is_some_and(|namespace| namespace != &identity.namespace)
        {
            self.set_namespace(None, window, cx);
        }
        let query = self.query.read(cx).value().to_lowercase();
        let shown = self
            .store
            .entries()
            .iter()
            .find(|entry| entry.row().identity == identity)
            .is_some_and(|entry| entry.search_key().contains(&query));
        if !shown {
            self.clear_filter(window, cx);
        }
        self.select_identity(&identity, window, cx);
        self.restore = Some(identity.clone());
        self.open_now(
            identity,
            window,
            move |this, window, cx| {
                this.detail.update(cx, |detail, cx| {
                    detail.set_tab(tab, cx);
                    detail.focus(window, cx);
                })
            },
            cx,
        );
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
        self.leave_detail(cx);
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
        if !self.embedded {
            self.load_namespaces(window, cx);
        }
        let kind = self.kind.clone();
        let namespace = self.namespace.clone().filter(|_| kind.namespaced);
        if let KubeAccess::Example = source.access {
            let events = match example::read_filtered(
                &source.context,
                &kind.key(),
                namespace.as_deref(),
                self.field_selector.as_deref(),
                self.now,
            ) {
                Some((columns, rows)) => vec![
                    ResourceEvent::reset(columns, rows),
                    ResourceEvent::Read(ReadState::Loaded),
                ],
                None => vec![ResourceEvent::Read(ReadState::Loaded)],
            };
            self.apply(ResourceBatch { epoch, events }, cx);
            return;
        }
        let (sender, receiver) = mpsc::channel(8);
        let job = OwnedJob::new(self.runtime.spawn(watch(
            source.access,
            kind,
            namespace,
            self.field_selector.clone(),
            source.id,
            sender,
        )));
        let task = self.receive(epoch, receiver, cx);
        self.watch = Some((job, task));
    }

    /// Applies what a read sends until it stops, at most once per
    /// `WATCH_COALESCE`: a batch arriving sooner waits, and everything sent
    /// meanwhile is applied with it.
    fn receive(
        &self,
        epoch: u64,
        mut receiver: mpsc::Receiver<Vec<ResourceEvent>>,
        cx: &mut Context<Self>,
    ) -> Task<()> {
        cx.spawn(async move |this, cx| {
            let mut last_applied = None;
            while let Some(mut events) = receiver.recv().await {
                if let Some(due) = last_applied.map(|at| at + WATCH_COALESCE) {
                    let now = cx.background_executor().now();
                    if now < due {
                        cx.background_executor().timer(due - now).await;
                    }
                }
                while let Ok(more) = receiver.try_recv() {
                    events.extend(more);
                }
                let applied = this.update(cx, |view, cx| {
                    view.apply(ResourceBatch { epoch, events }, cx)
                });
                if applied.is_err() {
                    break;
                }
                last_applied = Some(cx.background_executor().now());
            }
        })
    }

    fn apply(&mut self, batch: ResourceBatch, cx: &mut Context<Self>) {
        let _span = crate::perf::span("table.apply");
        crate::perf::value("table.batch", batch.events.len() as f64);
        let reset = batch
            .events
            .iter()
            .any(|event| matches!(event, ResourceEvent::Reset(_)));
        let served = !matches!(self.store.read_state(), ReadState::Missing(_));
        let stored = {
            let _span = crate::perf::span("table.store");
            self.store.apply(batch)
        };
        if !stored {
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
            let _span = crate::perf::span("table.layout");
            self.layout = TableLayout::new(&self.store, self.lists_all_namespaces());
            drop(_span);
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
        let Some(target) = self.detail.read(cx).target().cloned() else {
            return;
        };
        // A pane pinned by its shell may show what this list doesn't cover.
        let covered = target.kind == self.kind
            && self
                .namespace
                .as_ref()
                .is_none_or(|namespace| *namespace == target.identity.namespace);
        if !covered {
            return;
        }
        let target = target.identity;
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
        if self.embedded {
            return;
        }
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
        let context = source.context.clone();
        self.detail.update(cx, |detail, cx| {
            detail.set_context(context, cx);
            detail.open(target, access, &version, delay, cx)
        });
        cx.notify();
    }

    /// The pod whose shell runs in the pane, which keeps the pane on it:
    /// the selection moves freely, and only an explicit open asks first.
    fn pinned(&self, cx: &App) -> Option<SharedString> {
        self.detail.read(cx).running_shell(cx)
    }

    /// Opens `identity` in the pane at once, then runs `then`. A shell
    /// running there for another object asks first.
    fn open_now(
        &mut self,
        identity: ResourceIdentity,
        window: &mut Window,
        then: impl FnOnce(&mut Self, &mut Window, &mut Context<Self>) + 'static,
        cx: &mut Context<Self>,
    ) {
        let pinned = self
            .pinned(cx)
            .filter(|_| self.detail.read(cx).target_identity() != Some(&identity));
        unless_shell(self, pinned, window, cx, move |this, window, cx| {
            this.open_detail(identity, Duration::ZERO, cx);
            then(this, window, cx);
        });
    }

    /// Closes the pane, once a shell running there may end, and hands the
    /// keyboard to the list.
    fn close_pane(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let pinned = self.pinned(cx);
        unless_shell(self, pinned, window, cx, |this, window, cx| {
            this.close_detail(cx);
            window.focus(&this.focus, cx);
        });
    }

    /// The list moves on to rows the pane's object isn't among: the pane
    /// closes, unless a shell pins it.
    fn leave_detail(&mut self, cx: &mut Context<Self>) {
        if self.pinned(cx).is_some() {
            self.projection.select(&self.store, None);
            self.restore = None;
        } else {
            self.close_detail(cx);
        }
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
            self.leave_detail(cx);
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
                let table = list_table(&client, &kind, None, None)
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
        if self.embedded {
            cx.emit(NodePodsEvent::Open(identity.clone()));
            return;
        }
        self.open_now(identity.clone(), window, |_, _, _| {}, cx);
    }

    /// Enter on the list opens the selected row at once and hands the
    /// keyboard to it. Anywhere else, such as on a focused button, Enter
    /// keeps its own meaning.
    fn open_selected(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let selected = self.projection.selected().cloned();
        let Some(identity) = selected.filter(|_| self.focus.is_focused(window)) else {
            cx.propagate();
            return;
        };
        if self.embedded {
            cx.emit(NodePodsEvent::Open(identity));
            return;
        }
        self.open_now(
            identity,
            window,
            |this, window, cx| {
                this.detail
                    .update(cx, |detail, cx| detail.focus(window, cx))
            },
            cx,
        );
    }

    /// N on the list opens the namespace picker, as Enter on it would.
    /// Choosing or cancelling hands the keyboard back to the list.
    fn choose_namespace(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.kind.namespaced || self.embedded {
            return;
        }
        let focus = self.namespace_select.focus_handle(cx);
        window.focus(&focus, cx);
        focus.dispatch_action(&base::actions::Confirm { secondary: false }, window, cx);
    }

    /// Command-Shift-] and [ on the list switch the pane's tab and leave
    /// the keyboard where it is.
    fn step_tab(&mut self, delta: isize, cx: &mut Context<Self>) {
        self.detail
            .update(cx, |detail, cx| detail.turn_tab(delta, cx));
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
        if let Some(identity) = self.projection.selected().cloned()
            && self.pinned(cx).is_none()
        {
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
        } else if self.embedded {
            cx.emit(NodePodsEvent::Back);
        } else if self.detail.read(cx).target_identity().is_some() {
            self.close_pane(window, cx);
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
}

/// Lists and watches one collection on Tokio, converting batches there so
/// the UI thread only applies them. Stops when the receiver goes away.
async fn watch(
    access: KubeAccess,
    kind: ResourceKind,
    namespace: Option<String>,
    field_selector: Option<String>,
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
        watch_collection(client, kind, namespace, field_selector, raw_sender),
        forward
    );
}

impl EventEmitter<NotServed> for ResourcesScreen {}

pub(crate) enum NodePodsEvent {
    Open(ResourceIdentity),
    Back,
}
impl EventEmitter<NodePodsEvent> for ResourcesScreen {}

mod view;

#[cfg(test)]
mod tests;
