//! The Resources page: one kind's table as the API server prints it, kept
//! current by a watch while the page is visible. Rows are virtualized, sorted
//! and filtered locally, and selected by identity. Read-only: nothing here
//! changes the cluster.

use std::collections::BTreeSet;
use std::time::{Duration, SystemTime};

use freshkube_core::resources::{
    ResourceKind, WatchBatch, WatchEvent, builtin, list_table, watch_collection,
};
use freshkube_ui::drawer;
use gpui_kit::assets::IconName;
use gpui_kit::component::{
    Icon, IndexPath, Sizable,
    button::{Button, ButtonGroup, ButtonVariants},
    h_flex,
    input::{Input, InputEvent, InputState},
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
use super::pane::{DetailEvent, DetailPane, KEYBOARD_PAUSE, NextTab, PreviousTab};
use super::projection::ResourceProjection;
use super::store::{Changed, ResourceEvent, ResourceStore};
use super::{ResourceLink, example, live, navigation};
use crate::backend::{self, OwnedJob};
use crate::palette::palette;
use crate::screens::{SCREEN_DEADLINE, page_width};
use crate::ui::{self, clock, dp};
use freshkube_ui::{page, table};
use layout::TableLayout;
use pods::{ListView, NotReady, UsageState};

const CONTEXT: &str = "KubeResources";
/// The key context around the filter input, which sits outside the list's.
const EMBEDDED_CONTEXT: &str = "NodePods";
const FILTER_CONTEXT: &str = "KubeResourcesFilter";
const PAGE_ROWS: isize = 20;
/// Ages are redrawn this often while the page is visible.
const AGE_TICK: Duration = Duration::from_secs(5);
/// The least time between two watch batches applied during a burst. A change
/// after a quiet spell shows at once; while changes keep coming, they are
/// gathered and applied together, so a churning list re-sorts and redraws at
/// most ten times a second instead of once per batch core sends.
const WATCH_COALESCE: Duration = Duration::from_millis(100);
/// How long a drag of the drawer's edge pauses before its width is saved.
const DRAWER_SAVE_DELAY: Duration = Duration::from_millis(300);
/// The namespace picker's width in the toolbar.
const NAMESPACE_WIDTH: f32 = 132.;
/// The table's header and a couple of rows.
const LIST_MIN_HEIGHT: f32 = 96.;
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
        ChooseNamespace,
        ToggleMark,
        OpenLogs,
        SelectGroup,
        OpenNode,
        ToggleHealthy
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
    Talos(Box<super::talos::TalosAccess>),
    /// A kubeconfig context, without Talos.
    Direct(DirectAccess),
}

impl KubeAccess {
    pub(crate) async fn client(&self) -> Result<kube::Client, String> {
        match self {
            KubeAccess::Example => Err("Example data has no Kubernetes client".into()),
            KubeAccess::Talos(live) => live.client().await,
            KubeAccess::Direct(direct) => {
                direct.connect().await.map(|connection| connection.client)
            }
        }
    }

    /// Drops a reused client after a failure, so the next read rebuilds it.
    pub(crate) fn forget(&self) {
        match self {
            KubeAccess::Example => {}
            KubeAccess::Talos(live) => live.forget(),
            KubeAccess::Direct(direct) => direct.forget(),
        }
    }
}

/// Pages outside Resources reach a live cluster through this, so a clone
/// shares the client the Talos or kubeconfig access caches.
impl freshkube_core::cluster_source::KubeClientSource for KubeAccess {
    fn client(&self) -> futures::future::BoxFuture<'_, Result<kube::Client, String>> {
        Box::pin(KubeAccess::client(self))
    }
}

impl KubeSource {
    /// This connection as Monitoring and Observability take it.
    pub(crate) fn cluster(&self) -> freshkube_core::cluster_source::ClusterSource {
        use freshkube_core::cluster_source::{ClusterAccess, ClusterSource};
        let access = match &self.access {
            KubeAccess::Example => ClusterAccess::Example,
            live => ClusterAccess::Live(std::sync::Arc::new(live.clone())),
        };
        ClusterSource {
            id: self.id.clone(),
            context: self.context.clone(),
            access,
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
    SearchableVec::new(
        namespace_entries(names, current)
            .into_iter()
            .map(|(label, value)| NamespaceChoice { label, value })
            .collect::<Vec<_>>(),
    )
}

/// The namespaces to pick from, with All namespaces first: the picker's
/// choices and its folded form's items.
fn namespace_entries(
    names: &[String],
    current: Option<&str>,
) -> Vec<(SharedString, Option<String>)> {
    let mut entries = vec![(SharedString::from(ALL_NAMESPACES), None)];
    let entry = |name: &str| (name.to_owned().into(), Some(name.to_owned()));
    entries.extend(names.iter().map(|name| entry(name)));
    // The chosen namespace stays pickable when the list lacks it, say
    // because listing namespaces isn't permitted.
    if let Some(current) = current.filter(|current| !names.iter().any(|name| name == current)) {
        entries.push(entry(current));
    }
    entries
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
    /// Example lists hold their answers (`fixture::hold`), so the page
    /// stays on its loading rows.
    pub(crate) hold: bool,
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
    namespace_generation: u64,
    namespace_select: Entity<SelectState<SearchableVec<NamespaceChoice>>>,
    query: Entity<InputState>,
    store: ResourceStore,
    projection: ResourceProjection,
    layout: TableLayout,
    /// The kind and namespace scope `layout` was built from by a list, so
    /// a relist of the same keeps its header over the loading rows.
    layout_for: Option<(ResourceKind, bool)>,
    hidden_columns: BTreeSet<layout::ColumnSource>,
    visible: bool,
    /// The selection to find again once a restarted read lists it.
    restore: Option<ResourceIdentity>,
    /// Unix seconds that ages are drawn against.
    now: i64,
    /// When the last batch landed.
    updated: Option<SystemTime>,
    /// The status bar's line, and what it was derived from.
    status: (controls::StatusKey, freshkube_ui::status::Segment),
    focus: FocusHandle,
    /// The table's scrolls.
    table: table::TableState,
    /// What the table shows until the first list answers, and their motion,
    /// drawn over the table so its frames don't redraw it.
    loading: table::LoadingRows,
    loading_motion: Entity<table::LoadingMotion>,
    /// The rows a watch changed, tinted over the list and fading, drawn
    /// by the shell beside the cached page (`flash_layer`).
    flash: Entity<table::FlashLayer<ResourceIdentity>>,
    /// The projection's generation the flash last looked its lines up in.
    flash_rows: Option<u64>,
    page_scroll: ScrollHandle,
    /// The dock's height under the page when it last drew, in dp: when the
    /// dock takes more, the shorter list keeps its selected row in sight.
    below: f32,
    watch: Option<(OwnedJob, Task<()>)>,
    namespace_job: Option<(OwnedJob, Task<()>)>,
    tick: Option<Task<()>>,
    /// The selected object in full, in the drawer over the list.
    detail: Entity<DetailPane>,
    /// The drawer's width in dp, one for every kind, as the user left it;
    /// remembered in `navigation.json` under `drawer`.
    drawer_width: f32,
    /// Saves a dragged width once the drag pauses.
    drawer_save: Option<Task<()>>,
    /// The width in dp of the room the drawer lays out in, as last drawn.
    body_width: std::rc::Rc<std::cell::Cell<Option<f32>>>,
    /// Pods: problems first, or all in one list.
    list_view: ListView,
    /// The healthy pods show under the problems.
    healthy_open: bool,
    not_ready: NotReady,
    /// Rows marked with X or a group's Select all, by identity.
    marked: BTreeSet<ResourceIdentity>,
    /// Reads pods' use while the pods list shows.
    usage: Option<Task<()>>,
    usage_generation: u64,
    usage_state: UsageState,
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
            KeyBinding::new("x", ToggleMark, Some(CONTEXT)),
            KeyBinding::new("l", OpenLogs, Some(CONTEXT)),
            KeyBinding::new("shift-x", SelectGroup, Some(CONTEXT)),
            KeyBinding::new("o", OpenNode, Some(CONTEXT)),
            KeyBinding::new("h", ToggleHealthy, Some(CONTEXT)),
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
            // Every change to the rows notifies the screen; the flash hears
            // of it before the next frame draws, so a fade never paints on
            // a row's old line.
            cx.observe_self(|screen, cx| screen.flash_rows_changed(cx)),
            cx.subscribe_in(
                &query,
                window,
                |this, input, event, window, cx| match event {
                    InputEvent::Change => {
                        let text = input.read(cx).value().to_string();
                        this.projection.filter(&this.store, &text);
                        this.table.scroll.scroll_to_item(0, ScrollStrategy::Top);
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
                    DetailEvent::Leave => {
                        // A short page's frame returns to the list's top.
                        this.page_scroll.set_offset(point(px(0.), px(0.)));
                        window.focus(&this.focus, cx);
                        cx.notify();
                    }
                    DetailEvent::Link(intent) => {
                        // Logs and shells open in the dock, under the
                        // drawer: it steps aside, keeping the row.
                        if matches!(intent, ResourceLink::Logs(_) | ResourceLink::Shell(_)) {
                            this.hide_detail(cx);
                        }
                        cx.emit(intent.clone());
                    }
                    DetailEvent::Open(identity) => {
                        this.select_identity(identity, window, cx);
                        this.open_now(identity.clone(), window, |_, _, _| {}, cx);
                    }
                },
            ),
        ];
        let loading = table::LoadingRows::new("resource");
        let loading_motion = cx.new(|_| loading.motion(table::Look::Pulse));
        let table = table::TableState::new("resource");
        let screen = cx.entity().downgrade();
        let flash = cx.new(|_| {
            table::FlashLayer::new("resource", table.rows_at(), move |key, cx| {
                table::TableSource::line_of(screen.upgrade()?.read(cx), key)
            })
        });
        Self {
            field_selector: None,
            embedded: false,
            runtime,
            source: None,
            kind: builtin(navigation::DEFAULT_KIND).expect("the default kind is built in"),
            namespace: None,
            namespaces: Vec::new(),
            namespaces_for: None,
            namespace_generation: 0,
            namespace_select,
            query,
            store: ResourceStore::new(),
            projection: ResourceProjection::new(),
            layout: TableLayout::default(),
            layout_for: None,
            hold: crate::fixture::hold().lists,
            hidden_columns: BTreeSet::new(),
            visible: false,
            restore: None,
            now: live::now(),
            updated: None,
            status: Default::default(),
            focus: cx.focus_handle(),
            table,
            loading,
            loading_motion,
            flash,
            flash_rows: None,
            page_scroll: ScrollHandle::new(),
            below: 0.,
            watch: None,
            namespace_job: None,
            tick: None,
            detail,
            drawer_width: crate::ui::start_width(
                crate::navigation_file::NavigationFile::global(cx).drawer_width("resources"),
                drawer::WIDTH,
                drawer::MIN_WIDTH,
            ),
            drawer_save: None,
            body_width: Default::default(),
            list_view: ListView::default(),
            healthy_open: false,
            not_ready: NotReady::new(),
            marked: BTreeSet::new(),
            usage: None,
            usage_generation: 0,
            usage_state: UsageState::default(),
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
    pub(crate) fn selected_row(&self) -> Option<&ResourceIdentity> {
        self.projection.selected()
    }
    #[cfg(test)]
    pub(crate) fn field_selector_value(&self) -> Option<&str> {
        self.field_selector.as_deref()
    }
    #[cfg(test)]
    pub(crate) fn detail_tab(&self, cx: &App) -> crate::resources::Tab {
        self.detail.read(cx).tab()
    }

    /// Applies events as one watch batch of the current read.
    #[cfg(test)]
    pub(crate) fn deliver(&mut self, events: Vec<ResourceEvent>, cx: &mut Context<Self>) {
        let epoch = self.store.epoch();
        self.apply(epoch, vec![events], cx);
    }

    /// The `ix`th row the list shows as a watch would send it once its
    /// pod restarted again.
    #[cfg(test)]
    pub(crate) fn restarted(&self, ix: usize) -> super::model::ResourceRow {
        let mut row = self
            .projection
            .row(&self.store, ix)
            .expect("a shown row")
            .clone();
        let mut pod = (**row.pod.as_ref().expect("a pod row")).clone();
        pod.restarts += 1;
        row.pod = Some(std::sync::Arc::new(pod));
        row
    }

    /// The rows flashing now.
    #[cfg(test)]
    pub(crate) fn flashing(&self, cx: &App) -> Vec<ResourceIdentity> {
        self.flash.read(cx).flashing(cx)
    }

    /// Applies a read state as if the current read delivered it.
    #[cfg(test)]
    pub(crate) fn deliver_read(&mut self, state: ReadState, cx: &mut Context<Self>) {
        let epoch = self.store.epoch();
        self.apply(epoch, vec![vec![ResourceEvent::Read(state)]], cx);
    }

    /// Called only after the shell's navigation question has been accepted.
    pub(crate) fn set_node_rows(
        &mut self,
        rows: std::sync::Arc<Vec<crate::desktop::nodes::NodeRow>>,
        cx: &mut Context<Self>,
    ) {
        self.set_not_ready(&rows, cx);
        self.detail
            .update(cx, |pane, cx| pane.set_node_rows(rows, cx));
    }
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
    pub(crate) fn detail_identity<'a>(&'a self, cx: &'a App) -> Option<&'a ResourceIdentity> {
        self.detail.read(cx).target_identity()
    }
    pub(crate) fn showing(&self, identity: &ResourceIdentity, cx: &App) -> bool {
        self.detail.read(cx).target_identity() == Some(identity)
    }
    pub(crate) fn set_filter(&mut self, text: &str, window: &mut Window, cx: &mut Context<Self>) {
        self.query
            .update(cx, |input, cx| input.set_value(text.to_owned(), window, cx));
        self.projection.filter(&self.store, text);
        self.table.scroll.scroll_to_item(0, ScrollStrategy::Top);
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
        // Logs open in the dock and close the drawer at once: its read
        // waits as for a keyboard pause, which the close cancels.
        let delay = if tab == crate::resources::Tab::Logs {
            KEYBOARD_PAUSE
        } else {
            Duration::ZERO
        };
        self.open_detail(identity, delay, cx);
        self.detail.update(cx, |detail, cx| {
            detail.set_tab(tab, cx);
            // Logs open in the dock, which takes the keyboard and hands it
            // back to the list.
            if tab != crate::resources::Tab::Logs {
                detail.focus(window, cx);
            }
        });
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
        self.namespace_generation = self.namespace_generation.wrapping_add(1);
        self.namespace_job = None;
        self.sync_namespace_choices(window, cx);
        self.restart(window, cx);
    }

    /// Where a pod's CPU and memory history reads, from the Monitoring page.
    pub(crate) fn set_history(
        &mut self,
        history: Option<crate::monitoring::history::HistorySource>,
        cx: &mut Context<Self>,
    ) {
        self.detail
            .update(cx, |detail, cx| detail.set_history(history, cx));
    }

    /// Debug fixture checks: answers the open pod's history now.
    #[cfg(any(debug_assertions, feature = "stress"))]
    pub(crate) fn answer_history_now(&mut self, cx: &mut Context<Self>) {
        self.detail
            .update(cx, |detail, cx| detail.answer_history_now(cx));
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
        self.hidden_columns.clear();
        self.projection.set_pod_filter(&self.store, None);
        self.leave_detail(cx);
        self.restore = None;
        self.projection.reset_sort();
        self.table.scroll.scroll_to_item(0, ScrollStrategy::Top);
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
            self.clear_flash(cx);
            self.usage = None;
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

    /// The motion over the page's loading rows, which the shell mounts
    /// beside the cached page while the page shows, so its frames redraw
    /// neither the page nor its table. Only while the table draws those
    /// rows: a refusal or a failure replaces the table, and the motion
    /// goes with it. An embedded list keeps its own.
    pub(crate) fn loading_motion(&self) -> Option<Entity<table::LoadingMotion>> {
        (!self.embedded && table::TableSource::loading(self).is_some())
            .then(|| self.loading_motion.clone())
    }

    /// The flash over the page's rows, which the shell mounts beside the
    /// cached page, as it does the loading motion, so its frames redraw
    /// neither the page nor its table. Only while the table draws rows.
    pub(crate) fn flash_layer(&self) -> Option<Entity<table::FlashLayer<ResourceIdentity>>> {
        (!self.embedded && table::TableSource::loading(self).is_none()).then(|| self.flash.clone())
    }

    /// Whether changes flash: not in an embedded list, nor with reduced
    /// motion.
    fn flashes(&self, cx: &App) -> bool {
        !self.embedded && !cx.reduce_motion()
    }

    /// Flashes the rows a watch changed, one part per watch batch, so a
    /// burst holds back only the batch that brought it. Reported once
    /// this update ends, since the layer looks the rows' lines up in the
    /// list.
    fn flash(&self, changes: Vec<Changed>, cx: &mut Context<Self>) {
        let quiet = |part: &Changed| matches!(part, Changed::Rows(rows) if rows.is_empty());
        if !self.flashes(cx) || changes.iter().all(quiet) {
            return;
        }
        let layer = self.flash.clone();
        cx.defer(move |cx| {
            layer.update(cx, |layer, cx| {
                for part in changes {
                    match part {
                        Changed::Rows(rows) => {
                            layer.changed(rows, cx);
                        }
                        Changed::Many(count) => layer.held_back(count, cx),
                    }
                }
            })
        });
    }

    /// Tells the flash its rows' lines may have moved, when the projection
    /// was rebuilt since it last heard. The screen calls it whenever it
    /// notifies; the layer looks the lines up once this update ends,
    /// before the frame draws.
    fn flash_rows_changed(&mut self, cx: &mut Context<Self>) {
        let generation = self.projection.generation();
        if self.embedded || self.flash_rows == Some(generation) {
            return;
        }
        self.flash_rows = Some(generation);
        let layer = self.flash.clone();
        cx.defer(move |cx| layer.update(cx, |layer, cx| layer.rows_changed(generation, cx)));
    }

    fn clear_flash(&self, cx: &mut Context<Self>) {
        self.flash.update(cx, |layer, cx| layer.clear(cx));
    }

    /// Starts a new read session: forgets the rows, and when visible lists
    /// and watches the kind for the current connection and namespace.
    fn restart(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.watch = None;
        self.clear_flash(cx);
        self.usage = None;
        self.usage_generation = self.usage_generation.wrapping_add(1);
        if let Some(selected) = self.projection.selected() {
            self.restore = Some(selected.clone());
        }
        let epoch = self.store.start_session();
        self.marked.clear();
        self.usage_state = UsageState::Unknown;
        self.regroup();
        // The list's header over its loading rows until it answers: the
        // last list's for the same kind and scope, else Name and Age.
        let scope = (self.kind.clone(), self.lists_all_namespaces());
        if self.layout_for.as_ref() != Some(&scope) {
            self.layout_for = None;
            self.layout = TableLayout::new(
                &ResourceStore::provisional(),
                scope.1,
                self.lists_pods(),
                !self.embedded,
            );
            self.layout.hide(&self.hidden_columns);
        }
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
            if self.hold {
                return;
            }
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
            self.apply(epoch, vec![events], cx);
            self.poll_usage(window, cx);
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
        self.poll_usage(window, cx);
    }

    /// Applies what a read sends until it stops, at most once per
    /// `WATCH_COALESCE`: a batch arriving sooner waits, and everything sent
    /// meanwhile is applied with it, each watch batch kept as its own part
    /// so the flash counts bursts as the watch sent them.
    fn receive(
        &self,
        epoch: u64,
        mut receiver: mpsc::Receiver<Vec<ResourceEvent>>,
        cx: &mut Context<Self>,
    ) -> Task<()> {
        cx.spawn(async move |this, cx| {
            let mut last_applied = None;
            while let Some(events) = receiver.recv().await {
                if let Some(due) = last_applied.map(|at| at + WATCH_COALESCE) {
                    let now = cx.background_executor().now();
                    if now < due {
                        cx.background_executor().timer(due - now).await;
                    }
                }
                let mut parts = vec![events];
                while let Ok(more) = receiver.try_recv() {
                    parts.push(more);
                }
                let applied = this.update(cx, |view, cx| view.apply(epoch, parts, cx));
                if applied.is_err() {
                    break;
                }
                last_applied = Some(cx.background_executor().now());
            }
        })
    }

    fn apply(&mut self, epoch: u64, parts: Vec<Vec<ResourceEvent>>, cx: &mut Context<Self>) {
        let _span = crate::perf::span("table.apply");
        let events: usize = parts.iter().map(Vec::len).sum();
        crate::perf::value("table.batch", events as f64);
        let reset = parts
            .iter()
            .flatten()
            .any(|event| matches!(event, ResourceEvent::Reset(_)));
        let served = !matches!(self.store.read_state(), ReadState::Missing(_));
        // Rows only for a part that could flash.
        let keep = if self.flashes(cx) {
            self.flash.read(cx).burst()
        } else {
            0
        };
        let changes = {
            let _span = crate::perf::span("table.store");
            self.store.apply(epoch, parts, keep)
        };
        let Some(changes) = changes else {
            return;
        };
        if served && let ReadState::Missing(_) = self.store.read_state() {
            // Nothing of the kind can be shown any more.
            self.close_detail(cx);
            cx.emit(NotServed(self.kind.clone()));
        }
        self.updated = Some(SystemTime::now());
        self.now = live::now();
        self.projection.rebuild(&self.store);
        self.prune_marks();
        if reset {
            // The list was read again: nothing it shows is a change.
            self.clear_flash(cx);
            let _span = crate::perf::span("table.layout");
            self.layout = TableLayout::new(
                &self.store,
                self.lists_all_namespaces(),
                self.lists_pods(),
                !self.embedded,
            );
            self.layout.hide(&self.hidden_columns);
            self.layout_for = (!self.store.columns().is_empty())
                .then(|| (self.kind.clone(), self.lists_all_namespaces()));
            drop(_span);
            // A restarted read selects the same object again if it still
            // exists; only its first list can tell.
            if let Some(identity) = self.restore.take() {
                self.show_identity(&identity);
                self.scroll_to_selection(ScrollStrategy::Nearest);
            }
        }
        self.flash(changes, cx);
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
        // A pane opened from a link may show what this list doesn't cover.
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
        // A short page scrolls its frame: reveal the drawer.
        self.page_scroll.scroll_to_bottom();
        cx.notify();
    }

    /// Opens `identity` in the pane at once, then runs `then`.
    fn open_now(
        &mut self,
        identity: ResourceIdentity,
        window: &mut Window,
        then: impl FnOnce(&mut Self, &mut Window, &mut Context<Self>) + 'static,
        cx: &mut Context<Self>,
    ) {
        self.open_detail(identity, Duration::ZERO, cx);
        then(self, window, cx);
    }

    /// Closes the pane and hands the keyboard to the list.
    fn close_pane(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.close_detail(cx);
        window.focus(&self.focus, cx);
    }

    /// The list moves on to rows the pane's object isn't among: the pane
    /// closes.
    fn leave_detail(&mut self, cx: &mut Context<Self>) {
        self.close_detail(cx);
    }

    /// Closes the pane and clears the selection it showed.
    fn close_detail(&mut self, cx: &mut Context<Self>) {
        self.hide_detail(cx);
        self.projection.select(&self.store, None);
        self.restore = None;
    }

    /// Closes the pane, keeping the row it showed selected, so Enter
    /// opens it again.
    fn hide_detail(&mut self, cx: &mut Context<Self>) {
        self.detail.update(cx, |detail, cx| detail.close(cx));
        self.page_scroll.set_offset(point(px(0.), px(0.)));
        cx.notify();
    }

    /// The width in dp the drawer lays out in: the list's, as last drawn,
    /// or the page's before it has been.
    fn drawer_room(&self, window: &Window) -> f32 {
        self.body_width.get().unwrap_or_else(|| page_width(window))
    }

    /// A drag of the drawer's edge, in dp: kept within what the page
    /// allows, and saved once the drag pauses.
    fn resize_drawer(&mut self, width: f32, window: &mut Window, cx: &mut Context<Self>) {
        let fit = drawer::fit(width, self.drawer_room(window));
        if fit.full || (fit.width - self.drawer_width).abs() < 0.5 {
            return;
        }
        self.drawer_width = fit.width;
        self.drawer_save = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(DRAWER_SAVE_DELAY).await;
            _ = this.update(cx, |screen, cx| {
                screen.drawer_save = None;
                crate::navigation_file::NavigationFile::global(cx).set_drawer_width(
                    "resources",
                    screen.drawer_width,
                    cx,
                );
            });
        }));
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

    pub(crate) fn namespace(&self) -> Option<&str> {
        self.namespace.as_deref()
    }

    pub(crate) fn set_namespace(
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
        self.table.scroll.scroll_to_item(0, ScrollStrategy::Top);
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
        self.namespace_generation = self.namespace_generation.wrapping_add(1);
        let generation = self.namespace_generation;
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
            let result = receiver
                .await
                .unwrap_or_else(|_| Err("Listing namespaces stopped".into()));
            _ = this.update_in(cx, |view, window, cx| {
                view.finish_namespaces(generation, result, window, cx);
            });
        });
        self.namespace_job = Some((job, task));
    }

    fn finish_namespaces(
        &mut self,
        generation: u64,
        result: Result<Vec<String>, String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        if generation != self.namespace_generation || !self.visible {
            return false;
        }
        self.namespace_job = None;
        // Refusal leaves the all-namespaces and current-namespace choices.
        if let Ok(names) = result {
            self.namespaces = names;
            self.sync_namespace_choices(window, cx);
        }
        true
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
        self.show_identity(identity);
        self.scroll_to_selection(ScrollStrategy::Nearest);
        window.focus(&self.focus, cx);
        cx.notify();
    }

    /// Selects `identity`, unfolding the healthy pods when it is among them.
    fn show_identity(&mut self, identity: &ResourceIdentity) {
        if self.projection.hides(&self.store, identity) {
            self.healthy_open = true;
            self.regroup();
        }
        self.projection.select_identity(&self.store, identity);
    }

    /// Scrolls the list to the selected row's line, below its group's
    /// header.
    fn scroll_to_selection(&self, strategy: ScrollStrategy) {
        match self.projection.selected_index() {
            Some(ix) => self
                .table
                .scroll
                .scroll_to_item(self.projection.line_of(ix), strategy),
            None if strategy == ScrollStrategy::Top => {
                self.table.scroll.scroll_to_item(0, strategy)
            }
            None => {}
        }
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
        self.page_scroll.set_offset(point(px(0.), px(0.)));
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
        self.scroll_to_selection(ScrollStrategy::Nearest);
        // An open drawer follows the selection; a closed one stays closed
        // until Enter or a click.
        if self.detail.read(cx).target_identity().is_some()
            && let Some(identity) = self.projection.selected().cloned()
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
        self.scroll_to_selection(ScrollStrategy::Nearest);
        cx.notify();
    }

    /// Escape clears the filter, or with none, closes the drawer and
    /// clears the selection; with the drawer already closed, it clears
    /// the selection it kept.
    fn escape(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.query.read(cx).value().is_empty() {
            self.clear_filter(window, cx);
        } else if self.embedded {
            cx.emit(NodePodsEvent::Back);
        } else if self.detail.read(cx).target_identity().is_some()
            || self.projection.selected().is_some()
        {
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
        let mut columns = Vec::new();
        while let Some(batch) = raw.recv().await {
            let transient = batch.events.iter().any(|event| {
                matches!(event, WatchEvent::Failed { failure, .. } if !failure.kind.is_permanent())
            });
            if transient {
                access.forget();
            }
            if sender
                .send(live::convert(&connection, &key, &mut columns, batch.events))
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

mod cells;
mod controls;
mod layout;
mod menu;
mod pods;
mod source;
mod view;

#[cfg(test)]
mod tests;

impl EventEmitter<super::ResourceLink> for ResourcesScreen {}
