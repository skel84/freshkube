//! The Applications page: what core's `applications` derives from Kargo,
//! Argo CD and the `part-of` label, as a table page grouped by the rule that
//! found each application, with the selection in the Inspector.
//!
//! The shell owns the page and tells it when it shows. It reads only while
//! shown: showing it reads when it has nothing or what it has is older than
//! [`FRESH_FOR`], Refresh reads again, and hiding drops the read in flight.
//! A read is keyed by the connection's id, so another connection's answer
//! is never shown as this one's.
//!
//! Enter, a double-click or the row menu's Open shows the selected
//! application's own page (`page/`) in the list's place; its breadcrumb
//! comes back to the list with the selection kept. Each new read reaches the
//! open page, and one without its application closes it. A part opens in
//! Resources as any object link does (`links.rs`).
//!
//! On a Kargo application's page, a Stage leads to the change it carries
//! (`change/`), which shows in the application page's place; its
//! breadcrumb comes back. The change page reads the change itself, on the
//! open connection, while it shows; a live read records each Stage's
//! current Freight, which is what it follows.
//!
//! The column's Changes row shows the Changes list (`changes/`) in the
//! list's place: every Freight of the open cluster's Kargo projects, with
//! how sure it is that each Stage's pods run it. It reads itself, while it
//! shows, and opens the change page in its own place.
mod across;
mod change;
mod changes;
mod column;
mod display;
mod example;
mod links;
mod page;
mod table;
#[cfg(test)]
mod tests;
mod view;

use freshkube_core::delivery::change::live::Mapping;
use std::time::{Duration, Instant};

use chrono::Utc;

use freshkube_core::applications::across::{Open, read_workspace};
use freshkube_core::applications::{
    ArgoScope, Derived, Inputs, Override, SessionInputs, SessionKey, derive,
};
use freshkube_core::delivery::entry;
use freshkube_core::resources::kubeconfig_sources;
use freshkube_core::snapshot::{Request, Snapshot};
use freshkube_core::workspace::{Chosen, Part as PartKind, Sources};
use freshkube_ui::inspector::InspectorSplit;
use freshkube_ui::status::{Part, Segment};
use freshkube_ui::table::{self as kit, TableState};
use gpui_kit::component::input::{InputEvent, InputState};
use gpui_kit::prelude::*;
use gpui_kit::*;

use crate::backend::{self, OwnedJob};
use crate::resources::{KubeAccess, KubeSource, ResourceLink};
use across::{Across, Alone};
use change::{ChangeEvent, ChangePage, Changes};
use changes::ChangesEvent;
pub(crate) use changes::ChangesPage;
pub(crate) use column::Column;
#[cfg(test)]
pub(crate) use column::{ColumnApp, ColumnSection};
use display::{Body, Display, Labels, MARKS, Mark};
pub(crate) use example::Variant;
use links::Connections;
use page::{ApplicationEvent, ApplicationPage};

/// The page's id prefix: `applications-title`, `-list`, `-tally-…`.
const PREFIX: &str = "applications";
/// The list's key context, around the table, drawn in every state.
pub(crate) const CONTEXT: &str = "Applications";
/// A read this recent is shown again without reading.
const FRESH_FOR: Duration = Duration::from_secs(30);
/// How long one cluster's read may take: up to a few hundred lists.
const DEADLINE: Duration = Duration::from_secs(60);

gpui_kit::actions!(
    applications,
    [
        /// Selects the next application.
        NextApplication,
        /// Selects the previous application.
        PreviousApplication,
        /// Clears the selection, which closes the Inspector.
        ClearApplication,
        /// Opens the selected application's page.
        OpenApplication
    ]
);

pub(crate) fn key_bindings() -> Vec<KeyBinding> {
    let mut bindings = vec![
        KeyBinding::new("down", NextApplication, Some(CONTEXT)),
        KeyBinding::new("up", PreviousApplication, Some(CONTEXT)),
        KeyBinding::new("escape", ClearApplication, Some(CONTEXT)),
        KeyBinding::new("enter", OpenApplication, Some(CONTEXT)),
    ];
    bindings.extend(page::key_bindings());
    bindings.extend(change::key_bindings());
    bindings.extend(changes::key_bindings());
    bindings
}

/// One read: what was derived, what its clusters are called, and which
/// connection each is.
#[derive(Clone)]
struct Read {
    derived: Derived,
    labels: Labels,
    connections: Connections,
    changes: Changes,
    /// Argo CD's entry, when no destination is mapped and the workspace has
    /// other entries: labelled workloads were read there alone.
    unmapped: Option<String>,
    /// Where a live read found Argo CD's Applications, which the Changes
    /// list reads in.
    argocd_namespace: Option<String>,
}

/// The namespace Argo CD's Applications were first read in, on whichever
/// entry was asked for them; its default when none was scoped.
fn argocd_namespace(inputs: &Inputs) -> String {
    inputs
        .sessions
        .iter()
        .filter(|session| session.asked.argo_cd)
        .find_map(|session| match &session.argo_scope {
            ArgoScope::Namespace { namespaces, .. } => namespaces.first().cloned(),
            _ => None,
        })
        .unwrap_or_else(|| "argocd".into())
}

impl Read {
    fn of(inputs: &Inputs, labels: Labels, connections: Connections) -> Self {
        Self {
            derived: derive(inputs, &Override::default()),
            labels,
            connections,
            changes: Changes::None,
            unmapped: None,
            argocd_namespace: None,
        }
    }

    /// A live read: each cluster is a workspace entry, keyed and named by
    /// its id. `entry` is the workspace entry the open connection is: its
    /// session is keyed by the entry's id and maps to the open connection.
    /// The workspace of one has a session keyed by the connection's id,
    /// named by its context.
    fn live(inputs: &Inputs, source: &KubeSource, entry: Option<&str>) -> Self {
        let key = SessionKey::new(source.id.clone());
        let open: Vec<SessionKey> = [Some(key.clone()), entry.map(SessionKey::new)]
            .into_iter()
            .flatten()
            .collect();
        let labels = Labels::new(inputs.sessions.iter().map(|session| {
            let label = match session.key == key {
                true => source.context.clone(),
                false => session.key.0.clone(),
            };
            (session.key.clone(), label)
        }));
        // The change page reads each part from its entry: it follows a
        // change wherever Kargo was read.
        let changes = match inputs.sessions.iter().any(|session| session.asked.kargo) {
            true => Changes::live(inputs, source),
            false => Changes::None,
        };
        Self {
            changes,
            argocd_namespace: Some(argocd_namespace(inputs)),
            ..Self::of(
                inputs,
                labels,
                Connections::new(
                    source.id.clone(),
                    open.into_iter().map(|key| (key, source.id.clone())),
                ),
            )
        }
    }
}

/// Argo CD's entry, when no destination is mapped though the workspace has
/// other entries: the labelled workloads were read there alone.
fn unmapped(
    sources: &Sources,
    destinations: &[freshkube_core::workspace::Destination],
    entries: &[String],
) -> Option<String> {
    match sources.get(PartKind::ArgoCd) {
        Chosen::Entry { entry, .. }
            if destinations.is_empty() && entries.iter().any(|id| *id != entry.id) =>
        {
            Some(entry.id.clone())
        }
        _ => None,
    }
}

/// One line of the table: a rule's group header, or a row by index.
#[derive(Clone, Copy)]
enum Entry {
    Group(usize),
    Row(usize),
}

/// The workspace file's entries and the one the window is on, as the shell
/// says: a part in another entry's cluster opens there after a switch.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Clusters {
    pub(crate) entries: Vec<String>,
    pub(crate) active: Option<String>,
    /// What the workspace says about where Argo CD deploys
    /// (`Mapping::of`), which a change's live read resolves Stages through.
    pub(crate) mapping: Mapping,
    /// Where each part is read: a live read reads each from its entry.
    pub(crate) sources: Sources,
    /// The workspace's kubeconfig, `None` for the automatic one, which
    /// entries other than the open one are connected through.
    pub(crate) kubeconfig: Option<std::path::PathBuf>,
}

/// A part to open once the window has switched to the workspace entry its
/// cluster is: the shell asks first.
pub(crate) struct SwitchLink {
    pub(crate) entry: SharedString,
    pub(crate) link: ResourceLink,
}

/// Whose read it is: the connection, and the workspace entry it is.
type Identity = (String, Option<String>);

pub(crate) struct ApplicationsPage {
    runtime: tokio::runtime::Handle,
    source: Option<KubeSource>,
    /// The workspace entry the source's connection is, handed with it, so
    /// a read never names one entry's cluster with another's.
    entry: Option<String>,
    visible: bool,
    /// The example's shape, in example data.
    variant: Variant,
    /// Example data never answers, for captures (`FRESHKUBE_FIXTURE_HOLD`).
    hold: bool,
    snapshot: Snapshot<Read, Identity>,
    /// Whether a read is in flight; dropping its job and task stops it.
    pending: bool,
    job: Option<OwnedJob>,
    task: Option<Task<()>>,
    /// Example data answers after this, in tests, so a read stays in
    /// flight with a real task; zero answers at once.
    example_delay: Duration,
    clusters: Clusters,
    /// When the shown read's answer arrived, on the executor's clock.
    read_at: Option<Instant>,
    display: Display,
    /// The banner for a refresh that failed over an earlier read, and its
    /// label, derived with the display.
    stale: Option<(SharedString, SharedString)>,
    /// The table's lines, derived when the rows or a filter change.
    lines: Vec<Entry>,
    /// Rows per rule under the filters, for the group headers.
    groups: [usize; 4],
    /// Rows per mark under the text filter, for the chips.
    counts: [usize; 4],
    mark: Option<Mark>,
    columns: Vec<table::Column>,
    width: f32,
    table: TableState,
    loading: kit::LoadingRows,
    loading_motion: Entity<kit::LoadingMotion>,
    filter: Entity<InputState>,
    focus: FocusHandle,
    split: InspectorSplit,
    /// The frame's scroll, used while the window is short.
    page_scroll: ScrollHandle,
    /// The selected application's id, kept while a read or filter shows it.
    selected: Option<SharedString>,
    /// Whether the Inspector shows the selection. A right-click selects
    /// without opening it, so the table keeps its place under the menu.
    inspect: bool,
    /// The application whose page shows in the list's place.
    open: Option<(Entity<ApplicationPage>, Subscription)>,
    /// The change shown in the application page's place.
    change: Option<(Entity<ChangePage>, Subscription)>,
    /// The Changes list, which shows in the list's place while `changes_shown`.
    changes: Entity<ChangesPage>,
    changes_shown: bool,
    /// The keyboard handle of a page that closed without a window at hand,
    /// kept until the next frame looks whether it had the keyboard.
    refocus: Option<FocusHandle>,
    /// `5 applications in 6 clusters`, in the status bar.
    pub(crate) status: Segment,
    /// What the shell's column lists, derived with the display.
    column: Column,
    /// Changes with the column's lines and the application shown, so the
    /// column draws again only then (`column.rs`).
    column_revision: usize,
    _subscriptions: [Subscription; 2],
}

/// A part to open in Resources, from the open application's page.
impl EventEmitter<ResourceLink> for ApplicationsPage {}
impl EventEmitter<SwitchLink> for ApplicationsPage {}
impl EventEmitter<crate::observability::RevisionLink> for ApplicationsPage {}

impl ApplicationsPage {
    pub(crate) fn new(
        runtime: tokio::runtime::Handle,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let filter = cx.new(|cx| InputState::new(window, cx).placeholder("Filter applications…"));
        let subscription = cx.subscribe(&filter, |this, _, event, cx| {
            if matches!(event, InputEvent::Change) {
                this.refilter(cx);
            }
        });
        let (columns, width) = table::columns(&[]);
        let loading = kit::LoadingRows::new(PREFIX);
        let loading_motion = cx.new(|_| loading.motion(kit::Look::Pulse));
        let split = InspectorSplit::new(PREFIX, cx);
        let changes = cx.new(|cx| ChangesPage::new(runtime.clone(), cx));
        // The change page's links, from a change the list opened.
        let links = cx.subscribe(&changes, |_, _, event, cx| match event {
            ChangesEvent::Open(link) => cx.emit(link.as_ref().clone()),
            ChangesEvent::Switch(entry, link) => cx.emit(SwitchLink {
                entry: entry.clone(),
                link: link.as_ref().clone(),
            }),
            ChangesEvent::Revision(link) => cx.emit(link.clone()),
        });
        Self {
            runtime,
            source: None,
            visible: false,
            variant: Variant::from_env(),
            hold: crate::fixture::hold().applications,
            entry: None,
            snapshot: Snapshot::default(),
            pending: false,
            job: None,
            task: None,
            example_delay: Duration::ZERO,
            clusters: Clusters::default(),
            read_at: None,
            display: Display::default(),
            stale: None,
            lines: Vec::new(),
            groups: [0; 4],
            counts: [0; 4],
            mark: None,
            columns,
            width,
            table: TableState::new(PREFIX),
            loading,
            loading_motion,
            filter,
            focus: cx.focus_handle(),
            split,
            page_scroll: ScrollHandle::new(),
            selected: None,
            inspect: false,
            open: None,
            change: None,
            changes,
            changes_shown: false,
            refocus: None,
            status: Segment::default(),
            column: Column::new(&Display::default(), false, false, false),
            column_revision: 0,
            _subscriptions: [subscription, links],
        }
    }

    #[cfg(test)]
    pub(crate) fn set_variant(&mut self, variant: Variant) {
        self.variant = variant;
    }

    #[cfg(test)]
    pub(crate) fn set_hold(&mut self, hold: bool) {
        self.hold = hold;
    }

    #[cfg(test)]
    pub(crate) fn set_example_delay(&mut self, delay: Duration) {
        self.example_delay = delay;
    }

    /// The connection the page reads, and the workspace entry it is.
    /// Another connection, or the same one as another entry, drops what was
    /// read and what is in flight; the page reads it again if shown.
    pub(crate) fn set_source(
        &mut self,
        source: Option<KubeSource>,
        entry: Option<String>,
        cx: &mut Context<Self>,
    ) {
        let same = self.source.as_ref().map(|s| &s.id) == source.as_ref().map(|s| &s.id)
            && self.entry == entry;
        self.source = source.clone();
        self.entry = entry;
        if same {
            return;
        }
        // The list has the new connections before it reads the new source.
        self.update_changes(cx);
        self.changes
            .update(cx, |changes, cx| changes.set_source(source, cx));
        self.stop();
        self.snapshot = Snapshot::default();
        self.read_at = None;
        self.selected = None;
        self.inspect = false;
        self.close_open(cx);
        self.show_read(cx);
        if self.visible {
            self.read(cx);
        }
    }

    /// The workspace's entries and the open one. The open application's
    /// parts say again where each opens.
    pub(crate) fn set_clusters(&mut self, clusters: Clusters, cx: &mut Context<Self>) {
        if clusters == self.clusters {
            return;
        }
        let reads_elsewhere = (
            &clusters.sources,
            &clusters.kubeconfig,
            &clusters.mapping.destinations,
        ) != (
            &self.clusters.sources,
            &self.clusters.kubeconfig,
            &self.clusters.mapping.destinations,
        );
        self.clusters = clusters;
        self.update_open(cx);
        self.update_change(cx);
        self.update_changes(cx);
        // A live read follows where the parts are read: shown, it reads
        // again; hidden, it reads again when next shown.
        let live = self
            .source
            .as_ref()
            .is_some_and(|source| !matches!(source.access, KubeAccess::Example));
        if reads_elsewhere && live {
            self.read_at = None;
            if self.visible {
                self.read(cx);
            }
        }
    }

    /// Tells the Changes list which connection each cluster is and where
    /// a live read reads each part.
    fn update_changes(&mut self, cx: &mut Context<Self>) {
        let connections = self.base_connections();
        let across = self.across();
        self.changes.update(cx, |changes, cx| {
            changes.set_clusters(connections, across, cx)
        });
    }

    /// The workspace as a live change read sees it.
    fn across(&self) -> Across {
        Across {
            mapping: self.clusters.mapping.clone(),
            sources: self.clusters.sources.clone(),
            kubeconfig: self.clusters.kubeconfig.clone(),
        }
    }

    /// Which connection each cluster is, without a read: the open
    /// connection's own, or in example data the acme cluster the open
    /// entry is; and the workspace's other entries.
    fn base_connections(&self) -> Connections {
        let Some(source) = &self.source else {
            return Connections::default();
        };
        let connections = match &source.access {
            KubeAccess::Example => {
                example::connections(&source.id, self.clusters.active.as_deref())
            }
            _ => Connections::new(
                source.id.clone(),
                [(SessionKey::new(source.id.clone()), source.id.clone())],
            ),
        };
        let active = self.clusters.active.as_ref();
        connections.with_entries(
            self.clusters
                .entries
                .iter()
                .filter(|entry| Some(*entry) != active)
                .cloned(),
        )
    }

    /// Tells the change page shown the entries and the mapping.
    fn update_change(&mut self, cx: &mut Context<Self>) {
        let (Some((change, _)), Some(read)) = (&self.change, self.snapshot.data()) else {
            return;
        };
        let connections = self.connections(read);
        let across = self.across();
        change.update(cx, |page, cx| page.set_clusters(across, connections, cx));
    }

    /// Which connection each cluster of a read is, and which workspace
    /// entries are a switch away. In example data the acme cluster the open
    /// entry is stands for the open example cluster, so it follows a switch
    /// whichever arrives first, the switch or the read.
    fn connections(&self, read: &Read) -> Connections {
        let connections = match &self.source {
            Some(source) if matches!(source.access, KubeAccess::Example) => {
                example::connections(&source.id, self.clusters.active.as_deref())
            }
            _ => read.connections.clone(),
        };
        let active = self.clusters.active.as_ref();
        connections.with_entries(
            self.clusters
                .entries
                .iter()
                .filter(|entry| Some(*entry) != active)
                .cloned(),
        )
    }

    /// Shown, the page reads unless what it has is fresh; hidden, it drops
    /// the read in flight.
    pub(crate) fn set_visible(&mut self, visible: bool, cx: &mut Context<Self>) {
        if visible == self.visible {
            return;
        }
        self.visible = visible;
        if let Some((change, _)) = &self.change {
            change.update(cx, |change, cx| change.set_visible(visible, cx));
        }
        let changes = visible && self.changes_shown;
        self.changes
            .update(cx, |page, cx| page.set_visible(changes, cx));
        if !visible {
            self.stop();
            self.derive_column();
            cx.notify();
            return;
        }
        // Fresh only by when an answer arrived: a read dropped on hide, or
        // a refresh that failed, doesn't make old data new.
        let now = cx.background_executor().now();
        let fresh = self.snapshot.data().is_some()
            && !self.snapshot.is_stale()
            && self
                .read_at
                .is_some_and(|at| now.saturating_duration_since(at) < FRESH_FOR);
        if !fresh {
            self.read(cx);
        }
    }

    /// Refresh: reads again, whatever is in flight, while shown.
    pub(crate) fn refresh(&mut self, cx: &mut Context<Self>) {
        if !self.visible {
            return;
        }
        if self.changes_shown {
            self.changes.update(cx, |changes, cx| changes.refresh(cx));
            return;
        }
        // A change shown reads itself; the applications read on below it.
        if let Some((change, _)) = &self.change {
            change.update(cx, |change, cx| change.refresh(cx));
        }
        self.read(cx);
    }

    /// Drops the read in flight; what was read stays.
    fn stop(&mut self) {
        self.pending = false;
        self.job = None;
        self.task = None;
    }

    #[cfg(test)]
    pub(crate) fn is_reading(&self) -> bool {
        self.pending
    }

    fn read(&mut self, cx: &mut Context<Self>) {
        let Some(source) = self.source.clone() else {
            return;
        };
        self.stop();
        let request = self.snapshot.begin((source.id.clone(), self.entry.clone()));
        match &source.access {
            KubeAccess::Example => {
                // Held, it stays in flight, so the page keeps its loading
                // rows.
                self.pending = true;
                if self.hold {
                    self.derive_column();
                    cx.notify();
                    return;
                }
                let inputs = example::inputs(self.variant);
                // The Changes list follows the same shape.
                let shape = self.variant.shape();
                self.changes
                    .update(cx, |changes, _| changes.set_shape(shape));
                let read = Read {
                    changes: Changes::Example(Utc::now(), shape),
                    ..Read::of(
                        &inputs,
                        example::labels(&inputs),
                        example::connections(&source.id, self.clusters.active.as_deref()),
                    )
                };
                if self.example_delay.is_zero() {
                    self.answer(&request, Ok(read), cx);
                } else {
                    let delay = self.example_delay;
                    self.task = Some(cx.spawn(async move |this, cx| {
                        cx.background_executor().timer(delay).await;
                        _ = this.update(cx, |page, cx| page.answer(&request, Ok(read), cx));
                    }));
                }
            }
            access => {
                let access = access.clone();
                let sources = self.clusters.sources.clone();
                let kubeconfig = self.clusters.kubeconfig.clone();
                let destinations = self.clusters.mapping.destinations.clone();
                let unmapped = unmapped(&sources, &destinations, &self.clusters.entries);
                let entry = self.entry.clone();
                let (job, receiver) = backend::spawn_job(
                    &self.runtime,
                    DEADLINE,
                    "Reading the applications timed out".into(),
                    async move {
                        // The workspace of one reads the open connection
                        // alone, under the read's deadline, and fails as it
                        // does; beside other entries, the open one has
                        // their deadlines.
                        let alone = matches!(sources.get(PartKind::Kargo), Chosen::Open);
                        let open =
                            across::open_reader(&access, alone.then_some(Alone::Within(DEADLINE)))
                                .await?;
                        let kubeconfigs = kubeconfig_sources(kubeconfig.as_deref());
                        let sessions = read_workspace(
                            &sources,
                            Some(Open {
                                entry: entry.as_deref(),
                                key: SessionKey::new(source.id.clone()),
                                reader: open.as_ref().map_err(Clone::clone),
                            }),
                            |chosen| entry::connect_entry(kubeconfigs.clone(), chosen),
                        )
                        .await;
                        let inputs = Inputs {
                            sessions,
                            stage_naming: None,
                            destinations,
                        };
                        Ok(Read {
                            unmapped,
                            ..Read::live(&inputs, &source, entry.as_deref())
                        })
                    },
                );
                self.job = Some(job);
                self.pending = true;
                self.task = Some(cx.spawn(async move |this, cx| {
                    let result = receiver
                        .await
                        .unwrap_or_else(|_| Err("Reading the applications stopped".into()));
                    _ = this.update(cx, |page, cx| page.answer(&request, result, cx));
                }));
            }
        }
        self.derive_column();
        cx.notify();
    }

    /// A read's answer, published only while it is the one in flight.
    fn answer(
        &mut self,
        request: &Request<Identity>,
        result: Result<Read, String>,
        cx: &mut Context<Self>,
    ) {
        let answered = result.is_ok();
        if !self.pending || !self.snapshot.apply(request, result) {
            return;
        }
        self.pending = false;
        self.job = None;
        self.task = None;
        if answered {
            self.read_at = Some(cx.background_executor().now());
        }
        self.show_read(cx);
    }

    /// Derives the display from the snapshot: what was read, or, when the
    /// cluster couldn't be reached before anything was read, every source
    /// as unread, which is never "no applications".
    fn show_read(&mut self, cx: &mut Context<Self>) {
        let unread = || -> Option<Read> {
            let source = self.source.as_ref()?;
            let why = self.snapshot.error()?;
            let key = SessionKey::new(source.id.clone());
            let inputs = Inputs {
                sessions: vec![SessionInputs::unread(key.clone(), why)],
                stage_naming: None,
                destinations: Vec::new(),
            };
            Some(Read::live(&inputs, source, self.entry.as_deref()))
        };
        if let Some(namespace) = self
            .snapshot
            .data()
            .and_then(|read| read.argocd_namespace.clone())
        {
            self.changes.update(cx, |changes, cx| {
                changes.set_argocd_namespace(namespace, cx)
            });
        }
        self.display = match self.snapshot.data().cloned().or_else(unread) {
            Some(read) => {
                let mut display = Display::new(&read.derived, &read.labels);
                if let Some(entry) = &read.unmapped {
                    display.add_legend(format!(
                        "No Argo CD destination is mapped; labelled workloads were read on {} \
                         only. Settings › Workspace maps them",
                        read.labels.of(&SessionKey::new(entry.clone()))
                    ));
                }
                display
            }
            None => Display::default(),
        };
        self.stale = self.snapshot.data().and(self.snapshot.error()).map(|why| {
            let text = format!("Showing the last read. {why}");
            let label = format!("Couldn't read again: {text}");
            (text.into(), label.into())
        });
        (self.columns, self.width) = table::columns(&self.display.rows);
        self.status = self.segment();
        self.derive_column();
        self.update_open(cx);
        self.rebuild(cx);
    }

    /// Hands the open page its application's new read; without it there,
    /// the page closes.
    fn update_open(&mut self, cx: &mut Context<Self>) {
        let Some((open, _)) = &self.open else {
            return;
        };
        let id = open.read(cx).id().clone();
        let read = self.snapshot.data();
        match read.and_then(|read| Some((read.derived.find(&id)?, read))) {
            Some((app, read)) => {
                let stale = self.stale.clone();
                let connections = self.connections(read);
                open.update(cx, |page, cx| {
                    page.update(app, read, &connections, stale, cx)
                });
            }
            None => self.close_open(cx),
        }
    }

    /// Drops the open page without a window at hand, as a read or another
    /// connection does: the next frame gives the keyboard back to the list
    /// if the page had it.
    fn close_open(&mut self, cx: &App) {
        let change = self
            .change
            .take()
            .map(|(page, _)| page.read(cx).focus_handle());
        if let Some((page, _)) = self.open.take() {
            self.column_revision += 1;
            self.refocus = change.or(Some(page.read(cx).focus_handle()));
        }
    }

    /// Hands the keyboard to the list when the page closed under it.
    fn refocus(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(closed) = self.refocus.take() else {
            return;
        };
        if closed.is_focused(window) {
            let list = self.focus.clone();
            window.defer(cx, move |window, cx| window.focus(&list, cx));
        }
    }

    /// Shows the application's own page in the list's place, with the
    /// keyboard on it.
    fn open_application(
        &mut self,
        key: &SharedString,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(read) = self.snapshot.data() else {
            return;
        };
        let Some(app) = read
            .derived
            .applications
            .iter()
            .find(|app| app.id.as_str() == key.as_ref())
        else {
            return;
        };
        // Its page shows, not a change followed from another.
        self.change = None;
        let stale = self.stale.clone();
        let connections = self.connections(read);
        let page = cx.new(|cx| ApplicationPage::new(app, read, &connections, stale, cx));
        let subscription =
            cx.subscribe_in(&page, window, |this, _, event, window, cx| match event {
                ApplicationEvent::Back => this.close_application(window, cx),
                ApplicationEvent::Open(link) => cx.emit(link.as_ref().clone()),
                ApplicationEvent::Follow(stage) => this.follow(stage, window, cx),
                ApplicationEvent::Switch(entry, link) => cx.emit(SwitchLink {
                    entry: entry.clone(),
                    link: link.as_ref().clone(),
                }),
            });
        window.focus(&page.read(cx).focus_handle(), cx);
        self.open = Some((page, subscription));
        self.column_revision += 1;
        cx.notify();
    }

    /// Shows the change a Stage of the open Kargo application carries, in
    /// the application page's place, on the Stage's gates.
    fn follow(&mut self, stage: &str, window: &mut Window, cx: &mut Context<Self>) {
        let (Some((open, _)), Some(read)) = (&self.open, self.snapshot.data()) else {
            return;
        };
        let Some(source) = &self.source else {
            return;
        };
        let project = open.read(cx).name().to_string();
        let Some(fetch) = read.changes.fetch(
            &project,
            stage,
            &self.runtime,
            &source.access,
            self.across(),
            self.example_delay,
        ) else {
            return;
        };
        // As the application page's: in example data, the acme cluster the
        // open entry is stands for the open example cluster.
        let connections = self.connections(read);
        let visible = self.visible;
        let page =
            cx.new(|cx| ChangePage::new(fetch, Some(stage), connections, visible, window, cx));
        let subscription =
            cx.subscribe_in(&page, window, |this, _, event, window, cx| match event {
                ChangeEvent::Back => this.close_change(window, cx),
                ChangeEvent::Open(link) => cx.emit(link.as_ref().clone()),
                ChangeEvent::Switch(entry, link) => cx.emit(SwitchLink {
                    entry: entry.clone(),
                    link: link.as_ref().clone(),
                }),
                ChangeEvent::Revision(link) => cx.emit(link.clone()),
            });
        window.focus(&page.read(cx).focus_handle(), cx);
        self.change = Some((page, subscription));
        cx.notify();
    }

    /// Back to the application's page, with the selection it had.
    pub(super) fn close_change(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.change.take().is_some() {
            self.focus_shown(window, cx);
            cx.notify();
        }
    }

    /// The change shown, if one is.
    pub(crate) fn change_page(&self) -> Option<&Entity<ChangePage>> {
        self.change.as_ref().map(|(page, _)| page)
    }

    /// Follows a Stage of the open application, for `FRESHKUBE_PAGE=change`.
    pub(crate) fn follow_named(
        &mut self,
        stage: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.follow(stage, window, cx);
    }

    /// Back to the list, with the selection it had.
    fn close_application(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.change = None;
        if self.open.take().is_some() {
            self.column_revision += 1;
            self.focus(window, cx);
            cx.notify();
        }
    }

    /// Enter on the list: the selection's page.
    fn open_selected(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(key) = self.selected.clone() {
            self.open_application(&key, window, cx);
        }
    }

    /// Opens an application by its name, selecting it on the list first,
    /// for `FRESHKUBE_PAGE=application`.
    pub(crate) fn open_named(&mut self, name: &str, window: &mut Window, cx: &mut Context<Self>) {
        let Some(key) = self
            .display
            .rows
            .iter()
            .find(|row| row.name.as_ref() == name)
            .map(|row| row.key.clone())
        else {
            return;
        };
        self.select(key.clone(), cx);
        self.open_application(&key, window, cx);
    }

    /// The open application's page, if one shows.
    pub(crate) fn open_page(&self) -> Option<&Entity<ApplicationPage>> {
        self.open.as_ref().map(|(page, _)| page)
    }

    /// Puts the keyboard on what shows: the open application's page, or
    /// the list.
    pub(crate) fn focus_shown(&self, window: &mut Window, cx: &mut App) {
        if self.changes_shown {
            let focus = self.changes.read(cx).shown_focus(cx);
            window.focus(&focus, cx);
            return;
        }
        if let Some(change) = self.change_page() {
            let focus = change.read(cx).focus_handle();
            window.focus(&focus, cx);
            return;
        }
        match self.open_page() {
            Some(page) => {
                let focus = page.read(cx).focus_handle();
                window.focus(&focus, cx);
            }
            None => self.focus(window, cx),
        }
    }

    /// The status bar's segment: the Changes list's while it shows.
    pub(crate) fn status(&self, cx: &App) -> Segment {
        if self.changes_shown {
            return self.changes.read(cx).status.clone();
        }
        self.status.clone()
    }

    fn segment(&self) -> Segment {
        use freshkube_ui::ui::Tone;
        let context = self.source.as_ref().map(|s| s.context.clone());
        // A refused or failed read says so, never a count of nothing.
        match &self.display.body {
            Body::Refused { .. } => {
                let part = Part::new("applications not permitted").tone(Tone::Unknown);
                return Segment::new(context, vec![part]);
            }
            Body::Failed { .. } => {
                let part = Part::new("couldn't read applications").tone(Tone::Warn);
                return Segment::new(context, vec![part]);
            }
            Body::Table | Body::Empty { .. } => {}
        }
        let Some(read) = self.snapshot.data() else {
            return Segment::default();
        };
        let plural = |count: usize, one: &str, many: &str| {
            format!("{count} {}", if count == 1 { one } else { many })
        };
        let clusters = read
            .derived
            .coverage
            .iter()
            .map(|c| &c.session)
            .collect::<std::collections::BTreeSet<_>>()
            .len();
        let mut parts = vec![Part::new(format!(
            "{} in {}",
            plural(self.display.rows.len(), "application", "applications"),
            plural(clusters, "cluster", "clusters")
        ))];
        let incomplete = self.display.marks[Mark::Incomplete.index()];
        if incomplete > 0 {
            parts.push(Part::new(format!("{incomplete} may be incomplete")).tone(Tone::Unknown));
        }
        if self.snapshot.is_stale() {
            parts.push(Part::new("last read").tone(Tone::Warn));
        }
        Segment::new(context, parts)
    }

    /// The motion over the table's loading rows, which the shell mounts
    /// beside the cached page.
    pub(crate) fn loading_motion(&self, cx: &App) -> Option<Entity<kit::LoadingMotion>> {
        if self.changes_shown {
            return self.changes.read(cx).loading_motion(cx);
        }
        if let Some(change) = self.change_page() {
            return change.read(cx).loading_motion();
        }
        if self.open.is_some() {
            return None;
        }
        kit::TableSource::loading(self)
            .is_some()
            .then(|| self.loading_motion.clone())
    }

    /// Puts the keyboard on the list, where its keys are bound.
    pub(crate) fn focus(&self, window: &mut Window, cx: &mut App) {
        window.focus(&self.focus, cx);
    }

    fn selected_row(&self) -> Option<&display::AppRow> {
        let key = self.selected.as_ref()?;
        self.display.rows.iter().find(|row| &row.key == key)
    }

    fn select(&mut self, key: SharedString, cx: &mut Context<Self>) {
        self.selected = Some(key);
        self.inspect = true;
        kit::reveal(self, ScrollStrategy::Nearest);
        cx.notify();
    }

    fn step(&mut self, delta: isize, cx: &mut Context<Self>) {
        if let Some(key) = kit::step(self, delta, cx) {
            self.select(key, cx);
        }
    }

    fn clear_selection(&mut self, cx: &mut Context<Self>) {
        self.inspect = false;
        if self.selected.take().is_some() {
            cx.notify();
        } else {
            cx.propagate();
        }
    }

    fn toggle_mark(&mut self, mark: Mark, cx: &mut Context<Self>) {
        self.mark = (self.mark != Some(mark)).then_some(mark);
        self.refilter(cx);
    }

    /// A filter changed: the lines again, from the top.
    fn refilter(&mut self, cx: &mut Context<Self>) {
        self.rebuild(cx);
        self.table.reveal(0, ScrollStrategy::Top);
    }

    /// Derives the lines, keeping the filters and the scroll: rows matching
    /// the text, counted per mark, then those of the chosen mark, under a
    /// header per rule.
    fn rebuild(&mut self, cx: &mut Context<Self>) {
        let query = self.filter.read(cx).value().to_lowercase();
        self.counts = [0; 4];
        self.groups = [0; 4];
        let mut shown: Vec<usize> = Vec::new();
        for (ix, row) in self.display.rows.iter().enumerate() {
            if !row.query.contains(&query) {
                continue;
            }
            self.counts[row.mark.index()] += 1;
            if self.mark.is_none_or(|mark| mark == row.mark) {
                self.groups[display::rule_index(row.rule)] += 1;
                shown.push(ix);
            }
        }
        // Rows are sorted by rule, so a rule's rows are consecutive.
        self.lines.clear();
        let mut current = None;
        for ix in shown {
            let rule = display::rule_index(self.display.rows[ix].rule);
            if current != Some(rule) {
                self.lines.push(Entry::Group(rule));
                current = Some(rule);
            }
            self.lines.push(Entry::Row(ix));
        }
        // A row the filters hide, or that went, is no longer selected.
        if self.selected.as_ref().is_some_and(|key| {
            !self
                .lines
                .iter()
                .any(|entry| matches!(entry, Entry::Row(ix) if &self.display.rows[*ix].key == key))
        }) {
            self.selected = None;
            self.inspect = false;
        }
        cx.notify();
    }
}

#[cfg(test)]
impl ApplicationsPage {
    pub(crate) fn names(&self) -> Vec<String> {
        self.lines
            .iter()
            .filter_map(|entry| match entry {
                Entry::Row(ix) => Some(self.display.rows[*ix].name.to_string()),
                Entry::Group(_) => None,
            })
            .collect()
    }

    pub(crate) fn body(&self) -> &Body {
        &self.display.body
    }

    pub(crate) fn has_read(&self) -> bool {
        self.snapshot.data().is_some()
    }

    /// A read again that fails, as a cluster that stops answering does.
    pub(crate) fn fail_read(&mut self, why: &str, cx: &mut Context<Self>) {
        let Some(source) = self.source.clone() else {
            return;
        };
        self.stop();
        let request = self.snapshot.begin((source.id, self.entry.clone()));
        self.pending = true;
        self.answer(&request, Err(why.into()), cx);
    }
}

/// Every mark, for the header's chips in order.
fn marks() -> impl Iterator<Item = Mark> {
    MARKS.into_iter()
}
