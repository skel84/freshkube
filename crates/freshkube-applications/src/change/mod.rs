//! The change page (docs/DESIGN.md, "The change page"): one change, from
//! its pull request to the pods that run it, as a trail page. Its rows are
//! hops in the order the change travels, grouped by phase and by Stage,
//! never problems first; the chips count the hops by state and show only
//! theirs, and a Stage whose every row is fine, and whose pods are
//! confirmed to run it, folds while anything else needs a look. Link
//! confidence has its own column, beside the hop's state. The selection's
//! Inspector shows the hop, or for a gate row its Stage with the three
//! gates apart.
//!
//! It is opened from a Kargo application's page, on a Stage, and reads the
//! change itself (`read.rs`): live, through core's `delivery::change::live`
//! across the workspace, while it shows; example data answers at once.
//! Its breadcrumb, and Escape with nothing selected, go back to the
//! application's page.
//! An action that leads into Resources goes through the shell, as every
//! object link does; the page never approves, promotes or syncs.
mod changes;
mod detail;
mod outcome;
pub(crate) mod read;
pub(crate) mod table;
mod view;

use std::collections::BTreeSet;

use chrono::{DateTime, Local, Utc};
use freshkube_core::indicators::HealthIndicator;
use freshkube_core::resources::ResourceKind;
use freshkube_delivery::applications::SessionKey;
use freshkube_delivery::change::{Change, Destination, Object, Phase, Reach, Shows, Stage, Target};
use freshkube_delivery::join::Confidence;
use freshkube_reads::Loader;
use freshkube_ui::inspector::{self, InspectorSplit, Pane, Stacked};
use freshkube_ui::table::{self as kit, TableState};
use freshkube_ui::ui::Tone;
use gpui_kit::prelude::*;
use gpui_kit::*;

use super::links::Connections;
use crate::resources::detail::DetailTarget;
use crate::resources::model::ResourceIdentity;
use crate::resources::{LogsAt, LogsRequest, ResourceLink, Tab, model::ObjectRef};
pub use changes::Changes;
#[cfg(any(test, feature = "testing"))]
pub use changes::LiveChanges;
pub use outcome::{Coroot, Kind as OutcomeKind, Links, Live, Mark};
pub(crate) use outcome::{Identity, Key, SessionMemo, Summary, session_memo};
pub(super) use read::Fetch;

/// The page's id prefix: `change-title`, `change-hop-<key>`, `change-detail`.
const PREFIX: &str = "change";
/// The page's key context, drawn in every state.
pub(crate) const CONTEXT: &str = "Change";

gpui_kit::actions!(
    change,
    [
        /// Selects the next hop.
        NextHop,
        /// Selects the previous hop.
        PreviousHop,
        /// Clears the selection; with none, goes back.
        Back,
        /// Opens the selected hop's object in Resources.
        OpenHop
    ]
);

pub(crate) fn key_bindings() -> [KeyBinding; 4] {
    [
        KeyBinding::new("down", NextHop, Some(CONTEXT)),
        KeyBinding::new("up", PreviousHop, Some(CONTEXT)),
        KeyBinding::new("escape", Back, Some(CONTEXT)),
        KeyBinding::new("o", OpenHop, Some(CONTEXT)),
    ]
}

/// What the page asks of the page that owns it.
pub enum ChangeEvent {
    /// Back to the application's page.
    Back,
    /// An object to open in Resources.
    Open(Box<ResourceLink>),
    /// An object on another workspace entry's cluster, to open there once
    /// the window has switched to that entry; the shell asks first.
    Switch(SharedString, Box<ResourceLink>),
    /// A Deployment's revision to show in Observability.
    Revision(crate::observability::RevisionLink),
}

/// Stacked, the trail and the Inspector share the height evenly: the
/// trail is what the page is for, and a short window keeps rows of it.
const STACKED: Stacked = Stacked {
    lead: Pane::new(280.).least(inspector::LIST_MIN_HEIGHT),
    trail: Pane::new(280.).least(inspector::MIN_HEIGHT),
};

/// The status chips, failing first, and what each counts.
const TONES: [(Tone, &str, &str); 4] = [
    (Tone::Crit, "failing", "failing"),
    (Tone::Warn, "warning", "warning"),
    (Tone::Unknown, "waiting", "waiting or unknown"),
    (Tone::Good, "ok", "ok"),
];

/// A hop's state as a glyph's tone: waiting and unknown share the dashed
/// ring, and a hand approval is the blue dot.
pub(super) fn tone(state: HealthIndicator) -> Tone {
    match state {
        HealthIndicator::Healthy => Tone::Good,
        HealthIndicator::Info => Tone::Info,
        HealthIndicator::Pending | HealthIndicator::Unknown => Tone::Unknown,
        HealthIndicator::Warning => Tone::Warn,
        HealthIndicator::Error => Tone::Crit,
    }
}

/// The tone a row counts as: the blue dot is something to know, not a
/// fault, so it counts as ok in the chips, the filter and its group. A
/// Stage's group row takes the blue dot from its reach ([`stage_line`]),
/// where Claimed needs a look.
fn status(tone: Tone) -> Tone {
    match tone {
        Tone::Info => Tone::Good,
        tone => tone,
    }
}

/// How bad a tone is, for a group's worst row.
fn rank(tone: Tone) -> u8 {
    match tone {
        Tone::Crit | Tone::Died => 4,
        Tone::Warn => 3,
        Tone::Unknown => 2,
        _ => 1,
    }
}

/// A Stage's group row, after where it deploys and its words: how sure it
/// is that its pods run the change, when that is less than Confirmed, as
/// the Changes list's cell says, and a glyph the weaker of its rows' and
/// that. A Stage that runs other Freight keeps its rows' words and glyph.
pub fn stage_line(detail: &str, stage: &Stage, rows: Tone) -> (Vec<String>, Tone) {
    let mut words = vec![detail.to_owned(), stage.words.to_lowercase()];
    let on = stage
        .cluster
        .cluster()
        .map(|cluster| format!(" on {cluster}"))
        .unwrap_or_default();
    let reach = match &stage.reach {
        Reach::NotReached | Reach::Linked(Confidence::Confirmed) => return (words, rows),
        Reach::Linked(confidence) => {
            format!("pods{on} {}", link_word(*confidence).to_lowercase())
        }
        Reach::Refused(kinds) => format!("{kinds} refused"),
    };
    words.push(reach);
    let tone = reach_tone(&stage.reach);
    // The blue dot is weaker than Good, though both count as ok.
    let tone = match (rows, tone) {
        (Tone::Good, Tone::Info) => Tone::Info,
        _ if rank(tone) > rank(rows) => tone,
        _ => rows,
    };
    (words, tone)
}

/// How sure it is that a Stage's pods run the change, as a glyph's tone:
/// the Changes list's cell and a Stage's group row on the change page.
pub(super) fn reach_tone(reach: &Reach) -> Tone {
    match reach {
        Reach::NotReached => Tone::Outline,
        Reach::Linked(Confidence::Confirmed) => Tone::Good,
        Reach::Linked(Confidence::Claimed) => Tone::Info,
        Reach::Linked(Confidence::Unknown) | Reach::Refused(_) => Tone::Unknown,
    }
}

/// An instant as the page shows it: the local time of day.
pub(super) fn clock(at: DateTime<Utc>) -> String {
    at.with_timezone(&Local).format("%H:%M").to_string()
}

pub fn link_word(confidence: Confidence) -> &'static str {
    super::page::link_word(confidence)
}

/// The glyph a link shows, if any: Confirmed is the usual case and shows
/// none, so the other two stand out.
pub fn link_glyph(confidence: Confidence) -> Option<Tone> {
    match confidence {
        Confidence::Confirmed => None,
        Confidence::Claimed => Some(Tone::Info),
        Confidence::Unknown => Some(Tone::Unknown),
    }
}

/// One row's cells, derived with the change.
#[derive(Clone, Debug)]
pub struct HopRow {
    pub key: SharedString,
    pub group: usize,
    pub tone: Tone,
    pub name: SharedString,
    pub detail: SharedString,
    pub from: SharedString,
    pub time: SharedString,
    pub link: Option<Confidence>,
    /// Coroot's verdict on the revision, once the outcome was read. It is
    /// not the hop's state and not its link, and no chip counts it.
    pub coroot: Option<Mark>,
    /// The row's accessibility label.
    pub label: SharedString,
}

/// A group row's words, derived with the change.
#[derive(Clone, Debug)]
pub struct GroupLine {
    pub label: SharedString,
    pub detail: Vec<String>,
    pub tone: Tone,
    /// Only a Stage folds.
    pub folds: bool,
    /// Whether its rows were read: a Stage whose entry wasn't read is
    /// known only from Argo CD's report, so it never folds itself as fine,
    /// though its chevron still folds it.
    pub read_there: bool,
    pub rows: usize,
}

/// One line of the table: a group header, or a row by index.
#[derive(Clone, Copy)]
enum Entry {
    Group(usize),
    Row(usize),
}

pub struct ChangePage {
    /// What reads the change, and the read: its answer, its failure, and
    /// whether one is in flight.
    pub fetch: Fetch,
    /// The read, keyed by the change, and the work in flight. It is hidden
    /// while the page is, and its answer's age decides whether showing
    /// reads.
    pub loader: Loader<Change, String>,
    visible: bool,
    /// The window the page shows in, where an answer selects its Stage.
    window: AnyWindowHandle,
    /// The Stage to select once the first answer arrives.
    stage: Option<String>,
    /// A refresh that failed over an earlier answer, and its label for
    /// assistive technology, derived with it.
    stale: Option<(SharedString, SharedString)>,
    /// What the change may be missing, one sentence each that names the
    /// entry, and its words for assistive technology, derived with the
    /// change.
    missing: Option<(SharedString, SharedString)>,
    loading: kit::LoadingRows,
    loading_motion: Entity<kit::LoadingMotion>,
    /// The change shown: the last answer, or before one only its name.
    pub change: Change,
    /// The Freight's Kargo address, which the header opens and copies, or
    /// why there's none; derived with the change.
    page_link: Result<SharedString, SharedString>,
    /// Copy link has copied the address shown, so it says Copied until the
    /// address changes.
    copied: bool,
    connections: Connections,
    pub rows: Vec<HopRow>,
    pub groups: Vec<GroupLine>,
    /// Derived from the rows, the folds and the filter.
    lines: Vec<Entry>,
    /// How many rows each chip's tone has, folded or not.
    counts: [usize; TONES.len()],
    /// The Stage groups folded.
    folded: BTreeSet<usize>,
    filter: Option<Tone>,
    columns: Vec<table::Column>,
    table: TableState,
    focus: FocusHandle,
    split: InspectorSplit,
    /// The frame's scroll, used while the window is short.
    page_scroll: ScrollHandle,
    selected: Option<SharedString>,
    /// What the Inspector says of the selection.
    detail: Option<detail::Detail>,
    /// What Coroot found after each Deployment's deploy, read for the
    /// selected hop while the page shows.
    outcomes: outcome::Outcomes,
    /// What the breadcrumb goes back to, when it isn't the project's page.
    parent: Option<SharedString>,
}

impl EventEmitter<ChangeEvent> for ChangePage {}

impl ChangePage {
    /// The page on `stage`, reading the change `fetch` names at once
    /// when it is `visible`, else once it shows.
    pub(super) fn new(
        fetch: Fetch,
        stage: Option<&str>,
        connections: Connections,
        visible: bool,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let loading = kit::LoadingRows::new(PREFIX);
        let loading_motion = cx.new(|_| loading.motion(kit::Look::Pulse));
        let change = fetch.unread();
        let mut loader = Loader::default();
        loader.set_visible(visible, read::ON_SHOW, cx);
        let mut page = Self {
            page_link: page_link(&change),
            copied: false,
            change,
            fetch,
            loader,
            visible,
            window: window.window_handle(),
            stage: stage.map(str::to_owned),
            stale: None,
            missing: None,
            loading,
            loading_motion,
            connections,
            rows: Vec::new(),
            groups: Vec::new(),
            lines: Vec::new(),
            counts: [0; TONES.len()],
            folded: BTreeSet::new(),
            filter: None,
            columns: table::columns(),
            table: TableState::new(PREFIX),
            focus: cx.focus_handle(),
            split: InspectorSplit::new(PREFIX, cx).stacked(STACKED),
            page_scroll: ScrollHandle::new(),
            selected: None,
            detail: None,
            outcomes: outcome::Outcomes::default(),
            parent: None,
        };
        if visible {
            page.read(Some(window), cx);
        }
        page
    }

    /// Derives the rows, groups and chips from a change. The first answer
    /// folds the Stages that are fine; a later one keeps the folds, the
    /// filter and the selection where their rows still are.
    pub fn show(&mut self, change: Change) {
        let first = self.rows.is_empty();
        self.missing = (!change.missing.is_empty()).then(|| {
            let text = change
                .missing
                .iter()
                .map(|line| line.text.trim_end_matches('.'))
                .collect::<Vec<_>>()
                .join(". ");
            let label = format!("May be missing: {text}");
            (text.into(), label.into())
        });
        let rows: Vec<HopRow> = change
            .hops
            .iter()
            .map(|hop| {
                let time = hop.at.map(clock).unwrap_or_default();
                let link = hop
                    .link
                    .map(|link| format!(" · link {}", link_word(link)))
                    .unwrap_or_default();
                HopRow {
                    key: hop.key.clone().into(),
                    group: hop.group,
                    tone: tone(hop.state),
                    name: hop.name.clone().into(),
                    detail: hop.detail.clone().into(),
                    from: hop.from.clone().into(),
                    label: format!("{}: {}{link}", hop.name, hop.detail).into(),
                    time: time.into(),
                    link: hop.link,
                    coroot: None,
                }
            })
            .collect();
        let groups: Vec<GroupLine> = change
            .groups
            .iter()
            .enumerate()
            .map(|(ix, group)| {
                let of: Vec<&HopRow> = rows.iter().filter(|row| row.group == ix).collect();
                let tone = of
                    .iter()
                    .map(|row| status(row.tone))
                    .max_by_key(|tone| rank(*tone))
                    .unwrap_or(Tone::Good);
                let (label, detail, tone, folds, read_there) = match group.phase {
                    Phase::Build => (
                        "Build".to_owned(),
                        vec![group.detail.clone()],
                        tone,
                        false,
                        true,
                    ),
                    Phase::Freight => (
                        "Freight".to_owned(),
                        vec![group.detail.clone()],
                        tone,
                        false,
                        true,
                    ),
                    Phase::Stage(stage) => {
                        let stage = &change.stages[stage];
                        let (detail, tone) = stage_line(&group.detail, stage, tone);
                        (
                            format!("Stage {}", stage.name),
                            detail,
                            tone,
                            true,
                            !matches!(stage.cluster, Destination::Entry { .. }),
                        )
                    }
                };
                GroupLine {
                    label: label.into(),
                    detail,
                    tone,
                    folds,
                    read_there,
                    rows: of.len(),
                }
            })
            .collect();
        if first {
            // A Stage whose every row is fine folds while anything else on
            // the page needs a look; a waiting or failing one, or one whose
            // pods aren't confirmed to run it, never folds itself.
            let anything_wrong = groups.iter().any(|group| group.tone != Tone::Good);
            self.folded = groups
                .iter()
                .enumerate()
                .filter(|(_, group)| {
                    anything_wrong && group.folds && group.read_there && group.tone == Tone::Good
                })
                .map(|(ix, _)| ix)
                .collect();
        } else {
            self.folded
                .retain(|&ix| groups.get(ix).is_some_and(|g| g.folds));
        }
        let mut counts = [0; TONES.len()];
        for row in &rows {
            if let Some(ix) = TONES
                .iter()
                .position(|(tone, ..)| *tone == status(row.tone))
            {
                counts[ix] += 1;
            }
        }
        let link = page_link(&change);
        self.copied &= link == self.page_link;
        self.page_link = link;
        self.change = change;
        self.rows = rows;
        self.groups = groups;
        self.counts = counts;
        if self
            .selected
            .as_ref()
            .is_some_and(|key| !self.rows.iter().any(|row| &row.key == key))
        {
            self.selected = None;
        }
        self.prune_outcomes();
        self.sync_marks();
        self.detail = self.derive_detail();
        self.derive();
    }

    /// The lines, when the folds or the filter change; drawing only reads
    /// them. A filter shows its rows in every group, folded or not.
    fn derive(&mut self) {
        self.lines.clear();
        for group in 0..self.groups.len() {
            let rows: Vec<usize> = (0..self.rows.len())
                .filter(|&ix| self.rows[ix].group == group)
                .filter(|&ix| {
                    self.filter
                        .is_none_or(|tone| tone == status(self.rows[ix].tone))
                })
                .collect();
            if rows.is_empty() {
                continue;
            }
            self.lines.push(Entry::Group(group));
            if self.filter.is_none() && self.folded.contains(&group) {
                continue;
            }
            self.lines.extend(rows.into_iter().map(Entry::Row));
        }
    }

    /// The breadcrumb names `parent`, which it goes back to, in place of
    /// the project's page.
    pub(crate) fn set_parent(&mut self, parent: impl Into<SharedString>) {
        self.parent = Some(parent.into());
    }

    /// Puts the keyboard on the table, where its keys are bound.
    pub(crate) fn focus(&self, window: &mut Window, cx: &mut App) {
        window.focus(&self.focus, cx);
    }

    pub fn focus_handle(&self) -> FocusHandle {
        self.focus.clone()
    }

    /// Selects a hop, unfolding its group so its row shows.
    pub fn select(&mut self, key: &str, window: &Window, cx: &mut Context<Self>) {
        let Some(row) = self.rows.iter().find(|row| row.key.as_ref() == key) else {
            return;
        };
        let group = row.group;
        self.selected = Some(row.key.clone());
        self.detail = self.derive_detail();
        self.read_outcome(false, cx);
        if self.folded.remove(&group) {
            self.derive();
        }
        kit::reveal(self, ScrollStrategy::Nearest);
        // Stacked, the Inspector shrinks the table over a few frames: the
        // selection is revealed again until its height holds.
        if freshkube_ui::page::page_width(window) < inspector::SPLIT_WIDTH {
            kit::reveal_when_settled(
                cx.entity().downgrade(),
                |page| page.selected.is_some(),
                window,
            );
        }
        cx.notify();
    }

    /// Selects a Stage's first gate, which shows the Stage.
    pub(crate) fn select_stage(&mut self, stage: &str, window: &Window, cx: &mut Context<Self>) {
        let Some(ix) = self.change.stages.iter().position(|s| s.name == stage) else {
            return;
        };
        let key = self
            .change
            .hops
            .iter()
            .find(|hop| hop.shows == Shows::Stage(ix))
            .map(|hop| hop.key.clone());
        if let Some(key) = key {
            self.select(&key, window, cx);
        }
    }

    fn step(&mut self, delta: isize, window: &Window, cx: &mut Context<Self>) {
        if let Some(key) = kit::step(self, delta, cx) {
            self.select(&key, window, cx);
        }
    }

    fn toggle_fold(&mut self, group: usize, cx: &mut Context<Self>) {
        if !self.folded.remove(&group) {
            self.folded.insert(group);
        }
        self.derive();
        cx.notify();
    }

    fn unfold_all(&mut self, cx: &mut Context<Self>) {
        self.folded.clear();
        self.derive();
        cx.notify();
    }

    fn toggle_filter(&mut self, tone: Tone, cx: &mut Context<Self>) {
        self.filter = (self.filter != Some(tone)).then_some(tone);
        self.derive();
        cx.notify();
    }

    /// Escape: the selection first, then back.
    fn back(&mut self, cx: &mut Context<Self>) {
        if self.selected.take().is_some() {
            self.detail = None;
            self.hide_outcomes();
            cx.notify();
        } else {
            cx.emit(ChangeEvent::Back);
        }
    }

    /// Why a step pod's logs don't open: its cluster isn't the open one.
    fn logs_closed(&self, pod: &Object) -> Option<SharedString> {
        (!self
            .connections
            .opens(&SessionKey::new(pod.cluster.clone())))
        .then(|| {
            format!(
                "{} isn't the open cluster, so its logs don't open",
                self.cluster_name(&pod.cluster)
            )
            .into()
        })
    }

    /// Why a Deployment's revision doesn't open in Observability: Coroot is
    /// read for the open cluster only.
    fn revision_closed(&self, deployment: &Object) -> Option<SharedString> {
        (!self
            .connections
            .opens(&SessionKey::new(deployment.cluster.clone())))
        .then(|| {
            format!(
                "{} isn't the open cluster, so Observability doesn't read its revisions",
                self.cluster_name(&deployment.cluster)
            )
            .into()
        })
    }

    /// The link that opens an object in Resources, on its cluster's
    /// connection, and why it doesn't open when that cluster isn't open.
    fn object_link(&self, object: &Object) -> (ResourceLink, Option<SharedString>) {
        let session = SessionKey::new(object.cluster.clone());
        let link = ResourceLink::Object(
            ResourceKind::new(
                &object.group,
                &object.version,
                &object.kind,
                &object.plural,
                !object.namespace.is_empty(),
            ),
            ObjectRef {
                namespace: object.namespace.clone(),
                name: object.name.clone(),
                uid: String::new(),
                connection: Some(self.connections.of(&session)),
            },
            Tab::Overview,
        );
        let closed = (!self.connections.opens(&session)
            && self.connections.switch_to(&session).is_none())
        .then(|| {
            format!(
                "{} isn't the open cluster, so its objects don't open in Resources",
                self.cluster_name(&object.cluster)
            )
            .into()
        });
        (link, closed)
    }

    /// The workspace entry an object's cluster is, when it isn't the open
    /// one: its objects open there after a switch.
    pub(super) fn switch_of(&self, object: &Object) -> Option<SharedString> {
        let session = SessionKey::new(object.cluster.clone());
        self.connections
            .switch_to(&session)
            .map(|entry| entry.to_owned().into())
    }

    /// A cluster as the page names it: a live read's connection by its
    /// context, never by the key its objects carry.
    fn cluster_name<'a>(&'a self, key: &'a str) -> &'a str {
        match &self.fetch {
            Fetch::Live { place, .. } if place.cluster == key => &place.label,
            _ => key,
        }
    }

    /// What O opens: a gate's Stage, or the hop's first object that may
    /// be opened at all.
    fn selected_object(&self) -> Option<Object> {
        let hop = self.change.hop(self.selected.as_ref()?)?;
        match &hop.shows {
            Shows::Stage(stage) => Some(self.change.stages[*stage].object.clone()),
            Shows::Hop(detail) => detail
                .actions
                .iter()
                .find_map(|action| match &action.target {
                    Target::Resource { object, .. } if action.disabled.is_none() => {
                        Some(object.clone())
                    }
                    Target::OnEntry { entry, object, .. } => Some(on_entry(entry, object)),
                    _ => None,
                }),
        }
    }

    /// O: the selected hop's object in Resources, through the shell, which
    /// says so when its cluster isn't open.
    fn open_selected(&mut self, cx: &mut Context<Self>) {
        if let Some(object) = self.selected_object() {
            self.open(&object, cx);
        }
    }

    /// Opens an object in Resources, or, on another workspace entry's
    /// cluster, asks the shell to switch there first.
    pub(super) fn open(&mut self, object: &Object, cx: &mut Context<Self>) {
        let (link, _) = self.object_link(object);
        cx.emit(match self.switch_of(object) {
            Some(entry) => ChangeEvent::Switch(entry, Box::new(link)),
            None => ChangeEvent::Open(Box::new(link)),
        });
    }

    /// The Freight's page in Kargo, in the browser.
    fn open_page(&mut self, cx: &mut Context<Self>) {
        if let Ok(address) = &self.page_link {
            cx.open_url(address);
        }
    }

    /// Copies the Freight's Kargo address: user info, query and fragment
    /// were dropped when core read it.
    fn copy_link(&mut self, cx: &mut Context<Self>) {
        if let Ok(address) = &self.page_link {
            cx.write_to_clipboard(ClipboardItem::new_string(address.to_string()));
            self.copied = true;
            cx.notify();
        }
    }

    /// A step pod's logs in the dock, every step at once, as a pod's Logs
    /// opens them; `container` is the one a single-step pod falls back to.
    fn open_logs(&mut self, pod: &Object, container: Option<String>, cx: &mut Context<Self>) {
        let session = SessionKey::new(pod.cluster.clone());
        let kind = ResourceKind::new(&pod.group, &pod.version, &pod.kind, &pod.plural, true);
        let link = ResourceLink::Logs(LogsRequest {
            target: DetailTarget {
                identity: ResourceIdentity {
                    connection: self.connections.of(&session),
                    resource: kind.key(),
                    namespace: pod.namespace.clone(),
                    name: pod.name.clone(),
                    uid: String::new(),
                },
                kind,
            },
            at: container.map(|container| LogsAt {
                container,
                previous: false,
                all: true,
            }),
        });
        cx.emit(ChangeEvent::Open(Box::new(link)));
    }

    /// A Deployment's revision in Observability, through the shell, which
    /// finds it by its hash in Coroot's revisions of the Deployment.
    fn open_revision(&mut self, deployment: &Object, hash: &str, cx: &mut Context<Self>) {
        let session = SessionKey::new(deployment.cluster.clone());
        cx.emit(ChangeEvent::Revision(crate::observability::RevisionLink {
            access: self.connections.of(&session),
            cluster: self.cluster_name(&deployment.cluster).to_owned(),
            namespace: deployment.namespace.clone(),
            name: deployment.name.clone(),
            hash: hash.to_owned(),
        }));
    }

    /// How many rows the table shows, for the footer while Stages fold.
    fn shown_rows(&self) -> usize {
        self.lines
            .iter()
            .filter(|entry| matches!(entry, Entry::Row(_)))
            .count()
    }
}

#[cfg(any(test, feature = "testing"))]
impl ChangePage {
    /// Each line: a group's label, or a row's key.
    pub fn lines_text(&self) -> Vec<String> {
        self.lines
            .iter()
            .map(|entry| match *entry {
                Entry::Group(ix) => format!("# {}", self.groups[ix].label),
                Entry::Row(ix) => self.rows[ix].key.to_string(),
            })
            .collect()
    }

    pub fn selected(&self) -> Option<&SharedString> {
        self.selected.as_ref()
    }

    pub fn freight(&self) -> &str {
        &self.change.freight
    }
}

/// The change's Kargo address as the header shows it, or why there's none.
fn page_link(change: &Change) -> Result<SharedString, SharedString> {
    match &change.page {
        Ok(address) => Ok(address.to_string().into()),
        Err(why) => Err(why.clone().into()),
    }
}

/// An object Argo CD reports on another workspace entry, as one on that
/// entry's cluster: its key is the entry's id, which is a switch away.
pub(super) fn on_entry(entry: &str, object: &Object) -> Object {
    Object {
        cluster: entry.to_owned(),
        ..object.clone()
    }
}
