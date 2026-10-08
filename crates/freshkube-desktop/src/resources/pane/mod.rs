//! The detail pane beside the resource list: one object's overview, YAML,
//! events. Logs and shells open in the dock (`desktop/dock/`): the Logs
//! button and the Shell menu ask for them with `DetailEvent::Link`. It reads the object when
//! opened and again whenever the list shows a new version of it, at most once
//! a second, and watches the object's events while open on a visible page.
//! Secret values stay hidden until one is revealed. Nothing here changes the
//! cluster.

use freshkube_core::pluralize;
use std::cell::Cell;
use std::collections::HashMap;
use std::ops::Range;
use std::rc::Rc;
use std::time::Duration;

use chrono::{DateTime, Local, Utc};
use freshkube_core::resources::{
    EventScope, EventUpdate, Failure, FailureKind, ResourceKind, SecretValue, get_object,
    reveal_secret_value, runs_pods, watch_object_events,
};
use gpui_kit::component::input::{InputEvent, InputState};
use gpui_kit::prelude::*;
use gpui_kit::*;
use tokio::runtime::Handle;
use tokio::sync::mpsc;

use super::detail::{
    Detail, DetailTarget, DocumentRead, DocumentView, Follow, LineSelection, Reveal,
};
use super::model::ResourceIdentity;
use super::screen::KubeAccess;
use super::{LogsAt, LogsRequest, ResourceLink, ShellRequest, example, live};
use crate::backend::{self, OwnedJob};
use crate::monitoring::history::{HistorySource, HistoryView};
use freshkube_core::monitoring::history::Subject;

mod cross_links;
mod events;
mod overview;
mod ports;
#[cfg(test)]
mod tests;
mod view;
mod yaml;

use super::shell::Choice;
use events::EventLine;
use overview::Summary;
use ports::PortsView;

const CONTEXT: &str = "KubeDetail";
/// The key context of the tab strip, where the arrows move between tabs.
const TABS_CONTEXT: &str = "KubeDetailTabs";
/// How long one read of the object may take.
const READ_DEADLINE: Duration = Duration::from_secs(30);
/// Arrow keys open the row they land on once they pause this long, so
/// holding one down doesn't read every object it passes.
pub(crate) const KEYBOARD_PAUSE: Duration = Duration::from_millis(150);

actions!(
    kube_detail,
    [
        FindInYaml,
        SelectAllLines,
        CopyLines,
        Dismiss,
        NextTab,
        PreviousTab,
        FindNextMatch,
        FindPreviousMatch
    ]
);

/// What the pane asks of the page around it.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum DetailEvent {
    /// The user closed the pane.
    Closed,
    /// The user stepped back to the list; the pane stays open.
    Leave,
    /// The user chose to open this object: the one now at the address of
    /// the deleted object the pane shows.
    Open(ResourceIdentity),
    Link(super::ResourceLink),
}

/// Where the pane opens: a section of Details, YAML, or the dock's logs.
/// The strip has two tabs, Details and YAML; Overview, Ports and Events are
/// sections of Details, one scrolling page.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Tab {
    /// Details, from the top.
    Overview,
    Yaml,
    /// Details, scrolled to its events.
    Events,
    /// Not a tab: asking for it opens the object's logs in the dock, for
    /// pods and the workloads that run them.
    Logs,
    /// Details, scrolled to its ports: pods, Services and the workloads
    /// that run pods.
    Ports,
}

impl Tab {
    /// The strip's tabs, in order.
    const STRIP: [Tab; 2] = [Tab::Overview, Tab::Yaml];

    /// The strip's tab that shows this one.
    fn page(self) -> Tab {
        match self {
            Tab::Yaml => Tab::Yaml,
            _ => Tab::Overview,
        }
    }

    fn index(self) -> usize {
        Tab::STRIP
            .iter()
            .position(|tab| *tab == self.page())
            .unwrap_or(0)
    }
}

/// A part of Details, in the order the page shows them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Section {
    Overview,
    Ports,
    Events,
}

impl Section {
    /// A pod's, a Service's and a workload's: everything with ports.
    const POD: &[Section] = &[Section::Overview, Section::Ports, Section::Events];
    const OTHER: &[Section] = &[Section::Overview, Section::Events];

    /// The sections an object of `kind` has, in order.
    fn of(kind: &ResourceKind) -> &'static [Section] {
        if kind.is_pod() || ports::forwardable(kind) {
            Section::POD
        } else {
            Section::OTHER
        }
    }

    /// The section a request for `tab` scrolls to.
    fn of_tab(tab: Tab) -> Section {
        match tab {
            Tab::Events => Section::Events,
            Tab::Ports => Section::Ports,
            _ => Section::Overview,
        }
    }

    pub(crate) fn slug(self) -> &'static str {
        match self {
            Section::Overview => "overview",
            Section::Ports => "ports",
            Section::Events => "events",
        }
    }
}

fn local_time(time: DateTime<Utc>) -> String {
    time.with_timezone(&Local)
        .format("%Y-%m-%d %H:%M:%S")
        .to_string()
}

type Job = (OwnedJob, Task<()>);

pub(crate) struct DetailPane {
    embedded_node: bool,
    runtime: Handle,
    access: Option<KubeAccess>,
    /// Only an active pane, on a visible page, reads.
    active: bool,
    detail: Option<Detail>,
    title: SharedString,
    summary: Option<Summary>,
    node_rows: std::sync::Arc<Vec<crate::desktop::nodes::NodeRow>>,
    cross_links: cross_links::CrossLinks,
    pod_links: Option<freshkube_core::resources::PodLinks>,
    links_job: Option<Job>,
    links_seq: u64,
    event_lines: Vec<EventLine>,
    follow: Follow,
    /// Advance with each read and each events watch; results tagged with an
    /// older number are dropped.
    read_seq: u64,
    events_seq: u64,
    read_job: Option<Job>,
    events_job: Option<Job>,
    reveal_jobs: HashMap<String, Job>,
    /// A read waiting for arrow keys to pause, or for the follow interval.
    timer: Option<Task<()>>,
    /// `Overview` (Details) or `Yaml`; the node's inspector, which has no
    /// strip, also shows `Events` alone.
    tab: Tab,
    /// One per tab, in `Tab::STRIP`'s order, so the arrows can move between
    /// them.
    tab_focus: [FocusHandle; 2],
    /// Details' scroll, and the section at its top as last drawn.
    details_scroll: ScrollHandle,
    shown_section: Rc<Cell<Section>>,
    /// The section asked for, by its place in Details. Details keeps it
    /// at the top, or scrolled as far as it goes, and the index marks it
    /// until the user scrolls, while the document loads and the sections
    /// above it grow; it is resolved after layout (`watch_sections`).
    section_pinned: Rc<Cell<Option<usize>>>,
    /// How far the tabs scrolled when the pane is too narrow for them.
    tab_strip: freshkube_ui::inspector::TabStrip,
    find: Entity<InputState>,
    query: String,
    matches: Vec<(usize, Range<usize>)>,
    current: Option<usize>,
    selection: Option<LineSelection>,
    show_all_labels: bool,
    /// Details lists the newest `EVENTS_SHOWN` events until asked for all.
    show_all_events: bool,
    show_all_annotations: bool,
    feedback: Option<SharedString>,
    focus: FocusHandle,
    yaml_scroll: UniformListScrollHandle,
    /// The Shell menu's containers, derived when the document changes.
    shell_choices: Rc<Vec<Choice>>,
    /// The object's ports, and the forwards running from them.
    ports: Entity<PortsView>,
    /// A pod's CPU and memory over the last hour, on its Overview.
    history: Entity<HistoryView>,
    _subscriptions: Vec<Subscription>,
}

impl EventEmitter<DetailEvent> for DetailPane {}

/// Runs `then` on the pane after `delay`, unless the task is dropped first.
fn after(
    delay: Duration,
    cx: &mut Context<DetailPane>,
    then: fn(&mut DetailPane, &mut Context<DetailPane>),
) -> Task<()> {
    cx.spawn(async move |this, cx| {
        cx.background_executor().timer(delay).await;
        _ = this.update(cx, |pane, cx| {
            pane.timer = None;
            then(pane, cx);
        });
    })
}

impl DetailPane {
    pub(crate) fn new(runtime: Handle, window: &mut Window, cx: &mut Context<Self>) -> Self {
        cx.bind_keys([
            KeyBinding::new("secondary-f", FindInYaml, Some(CONTEXT)),
            KeyBinding::new("secondary-a", SelectAllLines, Some(CONTEXT)),
            KeyBinding::new("secondary-c", CopyLines, Some(CONTEXT)),
            KeyBinding::new("escape", Dismiss, Some(CONTEXT)),
            // Command-Shift-] and [, as macOS reports them.
            KeyBinding::new("secondary-}", NextTab, Some(CONTEXT)),
            KeyBinding::new("secondary-{", PreviousTab, Some(CONTEXT)),
            KeyBinding::new("secondary-g", FindNextMatch, Some(CONTEXT)),
            KeyBinding::new("secondary-shift-g", FindPreviousMatch, Some(CONTEXT)),
            KeyBinding::new("f3", FindNextMatch, Some(CONTEXT)),
            KeyBinding::new("shift-f3", FindPreviousMatch, Some(CONTEXT)),
            KeyBinding::new("right", NextTab, Some(TABS_CONTEXT)),
            KeyBinding::new("left", PreviousTab, Some(TABS_CONTEXT)),
        ]);
        cx.bind_keys([
            KeyBinding::new("secondary-f", FindInYaml, Some("NodeDocument")),
            KeyBinding::new("secondary-a", SelectAllLines, Some("NodeDocument")),
            KeyBinding::new("secondary-c", CopyLines, Some("NodeDocument")),
            KeyBinding::new("secondary-g", FindNextMatch, Some("NodeDocument")),
            KeyBinding::new("secondary-shift-g", FindPreviousMatch, Some("NodeDocument")),
            // As in the drawer: clear find, leave it, clear a selection,
            // then hand the keyboard back to the node table (`Leave`).
            KeyBinding::new("escape", Dismiss, Some("NodeDocument")),
        ]);
        let find = cx.new(|cx| InputState::new(window, cx).placeholder("Find in YAML"));
        let ports = cx.new(|cx| PortsView::new(runtime.clone(), window, cx));
        let history = cx.new(|_| HistoryView::new(runtime.clone(), "pod"));
        let subscriptions = vec![
            // Whether the pod's history shows, which this cached view draws.
            cx.observe(&history, |_, _, cx| cx.notify()),
            cx.subscribe_in(&find, window, |this, input, event, _, cx| match event {
                InputEvent::Change => {
                    let query = input.read(cx).value().to_string();
                    this.set_query(query, cx);
                }
                InputEvent::PressEnter { shift, .. } => {
                    this.step_match(if *shift { -1 } else { 1 }, cx)
                }
                _ => {}
            }),
            // The input's caret and selection redraw it, and this view is
            // cached, so it has to hear about them.
            cx.observe(&find, |_, _, cx| cx.notify()),
        ];
        Self {
            embedded_node: false,
            runtime,
            access: None,
            active: false,
            detail: None,
            title: SharedString::default(),
            summary: None,
            node_rows: Default::default(),
            cross_links: Default::default(),
            pod_links: None,
            links_job: None,
            links_seq: 0,
            event_lines: Vec::new(),
            follow: Follow::default(),
            read_seq: 0,
            events_seq: 0,
            read_job: None,
            events_job: None,
            reveal_jobs: HashMap::new(),
            timer: None,
            tab: Tab::Overview,
            tab_focus: std::array::from_fn(|_| cx.focus_handle().tab_stop(true)),
            details_scroll: ScrollHandle::new(),
            shown_section: Rc::new(Cell::new(Section::Overview)),
            section_pinned: Rc::default(),
            tab_strip: Default::default(),
            find,
            query: String::new(),
            matches: Vec::new(),
            current: None,
            selection: None,
            show_all_labels: false,
            show_all_events: false,
            show_all_annotations: false,
            feedback: None,
            focus: cx.focus_handle(),
            yaml_scroll: UniformListScrollHandle::new(),
            shell_choices: Rc::default(),
            ports,
            history,
            _subscriptions: subscriptions,
        }
    }

    pub(crate) fn embed_node(&mut self, tab: Tab, cx: &mut Context<Self>) {
        self.embedded_node = true;
        self.set_tab(tab, cx);
    }

    pub(crate) fn target_identity(&self) -> Option<&ResourceIdentity> {
        self.detail.as_ref().map(|detail| &detail.target.identity)
    }

    pub(crate) fn target(&self) -> Option<&DetailTarget> {
        self.detail.as_ref().map(|detail| &detail.target)
    }

    /// Fresh handles for the same connection.
    pub(crate) fn set_access(&mut self, access: KubeAccess, cx: &mut Context<Self>) {
        self.ports
            .update(cx, |ports, _| ports.set_access(access.clone()));
        self.access = Some(access);
    }

    /// The context objects open in, which a forward started here names.
    pub(crate) fn set_context(&mut self, context: String, cx: &mut Context<Self>) {
        self.ports.update(cx, |ports, _| ports.set_context(context));
    }

    /// Shows `target`, reading it after `delay`. `version` is the one the
    /// list shows. The tab and the search carry over from the last object.
    pub(crate) fn open(
        &mut self,
        target: DetailTarget,
        access: KubeAccess,
        version: &str,
        delay: Duration,
        cx: &mut Context<Self>,
    ) {
        self.access = Some(access.clone());
        if self.target_identity() == Some(&target.identity) {
            // Asked for at once while still waiting for the keys to pause.
            let waiting = self
                .detail
                .as_ref()
                .is_some_and(|detail| detail.read == DocumentRead::Loading);
            if delay.is_zero() && waiting && self.timer.is_some() {
                self.start(cx);
            }
            return;
        }
        self.stop_reads();
        // Another object starts at the top of its details.
        self.details_scroll = ScrollHandle::new();
        self.shown_section.set(Section::Overview);
        self.section_pinned.set(None);
        self.ports.update(cx, |ports, cx| {
            ports.show(
                Some((target.identity.clone(), target.kind.clone())),
                Some(access.clone()),
                cx,
            )
        });
        self.shell_choices = Rc::default();
        self.title = target.identity.address().into();
        self.detail = Some(Detail::new(target));
        // The strip starts unscrolled for each object.
        self.tab_strip = Default::default();
        self.follow = Follow::new(version);
        self.summary = None;
        self.cross_links = Default::default();
        self.pod_links = None;
        self.event_lines.clear();
        self.matches.clear();
        self.current = None;
        self.selection = None;
        self.show_all_labels = false;
        self.show_all_events = false;
        self.show_all_annotations = false;
        self.feedback = None;
        self.yaml_scroll.scroll_to_item(0, ScrollStrategy::Top);
        self.show_tab(cx);
        cx.notify();
        if !self.active {
            return;
        }
        if delay.is_zero() {
            self.start(cx);
        } else {
            self.timer = Some(after(delay, cx, Self::start));
        }
    }

    pub(crate) fn close(&mut self, cx: &mut Context<Self>) {
        self.stop_reads();
        self.shell_choices = Rc::default();
        self.ports
            .update(cx, |ports, cx| ports.show(None, None, cx));
        self.detail = None;
        self.summary = None;
        self.cross_links = Default::default();
        self.pod_links = None;
        self.event_lines.clear();
        self.matches.clear();
        self.current = None;
        self.selection = None;
        self.feedback = None;
        self.show_history(cx);
        cx.notify();
    }

    /// Hiding the page stops every read and hides revealed values; showing
    /// it reads the open object again.
    pub(crate) fn set_active(&mut self, active: bool, cx: &mut Context<Self>) {
        if self.active == active {
            return;
        }
        self.active = active;
        self.show_tab(cx);
        if active {
            self.start(cx);
        } else {
            self.stop_reads();
            cx.notify();
        }
    }

    /// Reads the object again and lists its events again.
    pub(crate) fn refresh(&mut self, cx: &mut Context<Self>) {
        self.read_job = None;
        self.history.update(cx, |history, cx| history.refresh(cx));
        self.start(cx);
    }

    /// The list shows `version` of the open object.
    pub(crate) fn observed(&mut self, version: &str, cx: &mut Context<Self>) {
        self.follow.observe(version);
        self.schedule_follow(cx);
    }

    /// The list no longer has the open object; `successor` is one it has at
    /// the same address.
    pub(crate) fn gone(&mut self, successor: Option<ResourceIdentity>, cx: &mut Context<Self>) {
        let Some(detail) = self.detail.as_mut() else {
            return;
        };
        if detail.gone(successor) {
            // Nothing read now could bring it back.
            self.read_job = None;
            self.timer = None;
            self.reveal_jobs.clear();
            detail.clear_reveals();
            cx.notify();
        }
    }

    fn stop_reads(&mut self) {
        self.read_job = None;
        self.events_job = None;
        self.timer = None;
        self.reveal_jobs.clear();
        self.read_seq += 1;
        self.events_seq += 1;
        self.links_job = None;
        self.links_seq = self.links_seq.wrapping_add(1);
        if let Some(detail) = self.detail.as_mut() {
            detail.clear_reveals();
        }
    }

    /// Reads the open object and watches its events, now.
    fn start(&mut self, cx: &mut Context<Self>) {
        self.timer = None;
        if !self.active || self.detail.is_none() {
            return;
        }
        self.read_document(cx);
        self.watch_events(cx);
    }

    fn read_document(&mut self, cx: &mut Context<Self>) {
        self.timer = None;
        let (Some(detail), Some(access)) = (self.detail.as_ref(), self.access.clone()) else {
            return;
        };
        if detail.read == DocumentRead::Deleted {
            return;
        }
        let target = detail.target.clone();
        self.read_seq += 1;
        let seq = self.read_seq;
        self.follow.begin(cx.background_executor().now());
        if let KubeAccess::Example = access {
            let result = example::document(&target.identity, live::now())
                .map(DocumentView::new)
                .ok_or_else(|| {
                    Failure::new(FailureKind::NotFound, "Example data has no such object")
                });
            self.finish_read(&target.identity, seq, result, cx);
            return;
        }
        let identity = target.identity.clone();
        let (job, receiver) = backend::spawn_job(
            &self.runtime,
            READ_DEADLINE,
            format!("No answer within {} seconds", READ_DEADLINE.as_secs()),
            async move {
                let client = match access.client().await {
                    Ok(client) => client,
                    Err(error) => {
                        access.forget();
                        return Ok(Err(Failure::new(FailureKind::Other, error)));
                    }
                };
                let identity = &target.identity;
                let namespace = Some(identity.namespace.as_str()).filter(|ns| !ns.is_empty());
                let result = get_object(&client, &target.kind, namespace, &identity.name).await;
                if result
                    .as_ref()
                    .is_err_and(|failure| !failure.kind.is_permanent())
                {
                    access.forget();
                }
                // Split into lines on Tokio, off the UI thread.
                Ok(result.map(DocumentView::new))
            },
        );
        let task = cx.spawn(async move |this, cx| {
            let result = match receiver.await {
                Ok(Ok(result)) => result,
                Ok(Err(message)) => Err(Failure::new(FailureKind::Timeout, message)),
                Err(_) => return,
            };
            _ = this.update(cx, |pane, cx| pane.finish_read(&identity, seq, result, cx));
        });
        self.read_job = Some((job, task));
    }

    fn finish_read(
        &mut self,
        identity: &ResourceIdentity,
        seq: u64,
        result: Result<DocumentView, Failure>,
        cx: &mut Context<Self>,
    ) {
        if seq != self.read_seq {
            return;
        }
        let Some(detail) = self
            .detail
            .as_mut()
            .filter(|detail| &detail.target.identity == identity)
        else {
            return;
        };
        self.read_job = None;
        let (before, epoch) = (detail.view.clone(), detail.reveal_epoch);
        detail.finish_read(result);
        if detail.reveal_epoch != epoch {
            self.reveal_jobs.clear();
        }
        let changed = match (&before, &detail.view) {
            (Some(before), Some(after)) => !std::sync::Arc::ptr_eq(before, after),
            (None, after) => after.is_some(),
            _ => false,
        };
        if changed {
            self.document_changed(cx);
        }
        cx.notify();
        self.schedule_follow(cx);
    }

    /// Derives what the new document shows: its overview, and the search
    /// and selection over its lines.
    fn document_changed(&mut self, cx: &mut Context<Self>) {
        let Some(view) = self.detail.as_ref().and_then(|detail| detail.view.clone()) else {
            return;
        };
        if let Some(containers) = &view.document.overview.pod {
            self.shell_choices = Rc::new(super::shell::choices(containers));
        }
        if let Some(declared) = view.document.overview.ports.clone() {
            self.ports
                .update(cx, |ports, cx| ports.set_ports(declared, cx));
        }
        let connection = self
            .detail
            .as_ref()
            .map(|detail| detail.target.identity.connection.clone());
        self.summary = Some(Summary::new(&view, connection.as_deref()));
        self.rebuild_links();
        self.start_links(cx);
        self.matches = view.find(&self.query);
        self.current = match self.current {
            Some(current) if current < self.matches.len() => Some(current),
            _ => (!self.matches.is_empty()).then_some(0),
        };
        self.selection = None;
    }

    /// Reads the object again if the list has shown a version the pane
    /// hasn't read, once the follow interval allows.
    fn schedule_follow(&mut self, cx: &mut Context<Self>) {
        let Some(detail) = self.detail.as_ref() else {
            return;
        };
        if !self.active
            || self.read_job.is_some()
            || self.timer.is_some()
            || detail.read == DocumentRead::Deleted
            || !self.follow.wanted(detail.version())
        {
            return;
        }
        let delay = self.follow.delay(cx.background_executor().now());
        if delay.is_zero() {
            self.read_document(cx);
        } else {
            self.timer = Some(after(delay, cx, Self::read_document));
        }
    }

    fn watch_events(&mut self, cx: &mut Context<Self>) {
        self.events_job = None;
        self.events_seq += 1;
        let seq = self.events_seq;
        let (Some(detail), Some(access)) = (self.detail.as_ref(), self.access.clone()) else {
            return;
        };
        let target = detail.target.clone();
        let identity = target.identity.clone();
        if let KubeAccess::Example = access {
            let events = example::events(&identity, live::now());
            self.apply_events(&identity, seq, EventUpdate::Reset(events), cx);
            return;
        }
        let scope = EventScope::new(
            &target.kind,
            Some(identity.namespace.as_str()).filter(|ns| !ns.is_empty()),
            &identity.name,
            &identity.uid,
        );
        let (sender, mut receiver) = mpsc::channel(16);
        let job = OwnedJob::new(self.runtime.spawn(async move {
            match access.client().await {
                Ok(client) => watch_object_events(client, scope, sender).await,
                Err(error) => {
                    access.forget();
                    let _ = sender
                        .send(EventUpdate::Failed {
                            failure: Failure::new(FailureKind::Other, error),
                            retrying: false,
                        })
                        .await;
                }
            }
        }));
        let task = cx.spawn(async move |this, cx| {
            while let Some(update) = receiver.recv().await {
                let applied =
                    this.update(cx, |pane, cx| pane.apply_events(&identity, seq, update, cx));
                if applied.is_err() {
                    break;
                }
            }
        });
        self.events_job = Some((job, task));
    }

    fn apply_events(
        &mut self,
        identity: &ResourceIdentity,
        seq: u64,
        update: EventUpdate,
        cx: &mut Context<Self>,
    ) {
        if seq != self.events_seq {
            return;
        }
        let Some(detail) = self
            .detail
            .as_mut()
            .filter(|detail| &detail.target.identity == identity)
        else {
            return;
        };
        detail.events.apply(update);
        self.event_lines = detail.events.shown().iter().map(EventLine::new).collect();
        self.rebuild_links();
        cx.notify();
    }

    /// Reads one Secret value afresh and shows it.
    fn reveal(&mut self, key: String, cx: &mut Context<Self>) {
        let (Some(detail), Some(access)) = (self.detail.as_mut(), self.access.clone()) else {
            return;
        };
        let identity = detail.target.identity.clone();
        let epoch = detail.reveal_epoch;
        detail.reveals.insert(key.clone(), Reveal::Reading);
        self.feedback = None;
        cx.notify();
        if let KubeAccess::Example = access {
            let result = example::secret_value(&identity, &key, live::now()).ok_or_else(|| {
                Failure::new(FailureKind::NotFound, "Example data has no such value")
            });
            self.finish_reveal(&identity, epoch, key, result, cx);
            return;
        }
        let (job, receiver) = backend::spawn_job(
            &self.runtime,
            READ_DEADLINE,
            format!("No answer within {} seconds", READ_DEADLINE.as_secs()),
            {
                let (identity, key) = (identity.clone(), key.clone());
                async move {
                    let client = match access.client().await {
                        Ok(client) => client,
                        Err(error) => return Ok(Err(Failure::new(FailureKind::Other, error))),
                    };
                    Ok(reveal_secret_value(
                        &client,
                        &identity.namespace,
                        &identity.name,
                        &identity.uid,
                        &key,
                    )
                    .await)
                }
            },
        );
        let task = cx.spawn({
            let key = key.clone();
            async move |this, cx| {
                let result = match receiver.await {
                    Ok(Ok(result)) => result,
                    Ok(Err(message)) => Err(Failure::new(FailureKind::Timeout, message)),
                    Err(_) => return,
                };
                _ = this.update(cx, |pane, cx| {
                    pane.finish_reveal(&identity, epoch, key, result, cx)
                });
            }
        });
        self.reveal_jobs.insert(key, (job, task));
    }

    fn finish_reveal(
        &mut self,
        identity: &ResourceIdentity,
        epoch: u64,
        key: String,
        result: Result<SecretValue, Failure>,
        cx: &mut Context<Self>,
    ) {
        let Some(detail) = self
            .detail
            .as_mut()
            .filter(|detail| &detail.target.identity == identity && detail.reveal_epoch == epoch)
        else {
            return;
        };
        if detail.reveals.get(&key) != Some(&Reveal::Reading) {
            return;
        }
        self.reveal_jobs.remove(&key);
        let reveal = match result {
            Ok(value) => Reveal::Shown(value),
            Err(failure) => Reveal::Failed(failure.to_string()),
        };
        detail.reveals.insert(key, reveal);
        cx.notify();
    }

    fn hide(&mut self, key: &str, cx: &mut Context<Self>) {
        if let Some(detail) = self.detail.as_mut() {
            detail.reveals.remove(key);
        }
        self.reveal_jobs.remove(key);
        cx.notify();
    }

    fn copy_value(&mut self, key: &str, cx: &mut Context<Self>) {
        let Some(Reveal::Shown(SecretValue::Text(text))) = self
            .detail
            .as_ref()
            .and_then(|detail| detail.reveals.get(key))
        else {
            return;
        };
        cx.write_to_clipboard(ClipboardItem::new_string(text.clone()));
        self.feedback = Some(format!("Copied the value of {key}").into());
        cx.notify();
    }

    /// The object's name alone, without its namespace, as kubectl takes it.
    fn copy_name(&mut self, cx: &mut Context<Self>) {
        let Some(name) = self
            .detail
            .as_ref()
            .map(|detail| detail.target.identity.name.clone())
        else {
            return;
        };
        cx.write_to_clipboard(ClipboardItem::new_string(name));
        self.feedback = Some("Copied the name".into());
        cx.notify();
    }

    fn view(&self) -> Option<&DocumentView> {
        self.detail.as_ref()?.view.as_deref()
    }

    /// The whole document, as read: a Secret's values stay hidden.
    fn copy_document(&mut self, cx: &mut Context<Self>) {
        let Some(yaml) = self.view().map(|view| view.document.yaml.clone()) else {
            return;
        };
        cx.write_to_clipboard(ClipboardItem::new_string(yaml));
        self.feedback = Some("Copied the YAML".into());
        cx.notify();
    }

    fn copy_lines(&mut self, cx: &mut Context<Self>) {
        let (Some(view), Some(selection)) = (self.view(), self.selection) else {
            return;
        };
        let range = selection.range();
        let count = range.len();
        let text = view.text(range).to_owned();
        cx.write_to_clipboard(ClipboardItem::new_string(text));
        self.feedback = Some(format!("Copied {}", pluralize(count, "line", "lines")).into());
        cx.notify();
    }

    /// What the pane shows: YAML, or the section at the top of Details.
    #[cfg(test)]
    pub(crate) fn tab(&self) -> Tab {
        match (self.tab, self.shown_section.get()) {
            (Tab::Overview, Section::Ports) => Tab::Ports,
            (Tab::Overview, Section::Events) => Tab::Events,
            (tab, _) => tab,
        }
    }

    pub(crate) fn set_tab(&mut self, tab: Tab, cx: &mut Context<Self>) {
        if tab == Tab::Logs {
            self.request_logs(None, cx);
            return;
        }
        if self.embedded_node {
            self.tab = tab;
        } else {
            self.tab = tab.page();
            if tab != Tab::Yaml {
                self.show_section(Section::of_tab(tab));
            }
        }
        self.feedback = None;
        self.show_tab(cx);
        cx.notify();
    }

    /// Shows the strip's `tab`, where Details was last scrolled to.
    fn show_page(&mut self, tab: Tab, cx: &mut Context<Self>) {
        if tab == self.tab {
            return;
        }
        self.tab = tab;
        self.feedback = None;
        self.show_tab(cx);
        cx.notify();
    }

    /// Scrolls Details to `section`, or to its top when this object has
    /// no such section. The scroll happens once Details has laid out, since
    /// a newly opened object's handle knows no bounds yet.
    fn show_section(&mut self, section: Section) {
        let sections = self
            .detail
            .as_ref()
            .map_or(Section::OTHER, |detail| Section::of(&detail.target.kind));
        let ix = sections.iter().position(|s| *s == section).unwrap_or(0);
        self.shown_section.set(sections[ix]);
        self.section_pinned.set(Some(ix));
    }

    /// Moves `delta` tabs along, wrapping, from outside the pane.
    pub(crate) fn turn_tab(&mut self, delta: isize, cx: &mut Context<Self>) {
        self.step_tab(delta, cx);
    }

    /// Moves `delta` tabs along, wrapping, and returns the tab now shown.
    fn step_tab(&mut self, delta: isize, cx: &mut Context<Self>) -> Option<Tab> {
        self.detail.as_ref()?;
        let tabs = Tab::STRIP;
        let next = (self.tab.index() as isize + delta).rem_euclid(tabs.len() as isize) as usize;
        let tab = tabs[next];
        self.show_page(tab, cx);
        Some(tab)
    }

    /// Puts the keyboard on the pane.
    pub(crate) fn focus(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        window.focus(&self.focus, cx);
    }

    /// Command-Shift-] and [ in the pane: the next tab, with the keyboard
    /// on what it shows.
    fn switch_tab(&mut self, delta: isize, window: &mut Window, cx: &mut Context<Self>) {
        if self.step_tab(delta, cx).is_some() {
            self.focus(window, cx);
        }
    }

    /// The arrows on a focused tab: the next tab, keeping the keyboard on
    /// the tabs.
    fn move_tab(&mut self, delta: isize, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(tab) = self.step_tab(delta, cx) {
            window.focus(&self.tab_focus[tab.index()], cx);
        }
    }

    /// Whether the open object has logs: a pod, or a workload that runs
    /// pods.
    pub(crate) fn has_logs(&self) -> bool {
        self.detail
            .as_ref()
            .is_some_and(|detail| runs_pods(&detail.target.kind) || detail.target.kind.is_pod())
    }

    /// Asks for the open object's logs in the dock, on `at`'s container
    /// and instance when given.
    pub(crate) fn request_logs(&mut self, at: Option<LogsAt>, cx: &mut Context<Self>) {
        if !self.has_logs() {
            return;
        }
        let Some(detail) = self.detail.as_ref() else {
            return;
        };
        cx.emit(DetailEvent::Link(ResourceLink::Logs(LogsRequest {
            target: detail.target.clone(),
            at,
        })));
    }

    /// The Shell menu's pick: a shell in `container`, in a dock tab. The
    /// pick is the explicit Start.
    pub(crate) fn request_shell(&mut self, container: String, cx: &mut Context<Self>) {
        let Some(detail) = self.detail.as_ref() else {
            return;
        };
        if !detail.target.kind.is_pod() {
            return;
        }
        let containers = detail
            .view
            .as_ref()
            .and_then(|view| view.document.overview.pod.clone());
        cx.emit(DetailEvent::Link(ResourceLink::Shell(ShellRequest {
            target: detail.target.clone(),
            container,
            containers,
        })));
    }

    /// Tells the history whether the Overview shows.
    fn show_tab(&mut self, cx: &mut Context<Self>) {
        self.show_history(cx);
    }

    /// Where a pod's CPU and memory history reads, from the Monitoring page.
    pub(crate) fn set_history(&mut self, history: Option<HistorySource>, cx: &mut Context<Self>) {
        self.history
            .update(cx, |view, cx| view.set_source(history, cx));
    }

    /// Debug fixture checks: answers the pod's history now.
    #[cfg(any(debug_assertions, feature = "stress"))]
    pub(crate) fn answer_history_now(&mut self, cx: &mut Context<Self>) {
        self.history
            .update(cx, |history, cx| history.answer_example_now(cx));
    }

    /// Tells the history which pod the Overview shows, and whether it does.
    fn show_history(&mut self, cx: &mut Context<Self>) {
        let subject = self
            .detail
            .as_ref()
            .filter(|detail| detail.target.kind.is_pod() && !self.embedded_node)
            .map(|detail| Subject::Pod {
                namespace: detail.target.identity.namespace.clone(),
                name: detail.target.identity.name.clone(),
            });
        let shown = self.active && self.tab == Tab::Overview && subject.is_some();
        self.history.update(cx, |history, cx| {
            history.set_subject(subject, cx);
            history.set_visible(shown, cx);
        });
    }

    fn set_query(&mut self, query: String, cx: &mut Context<Self>) {
        self.query = query;
        self.matches = self
            .view()
            .map(|view| view.find(&self.query))
            .unwrap_or_default();
        self.current = (!self.matches.is_empty()).then_some(0);
        self.reveal_current();
        cx.notify();
    }

    fn step_match(&mut self, delta: isize, cx: &mut Context<Self>) {
        let count = self.matches.len();
        if count == 0 {
            return;
        }
        self.current = Some(match self.current {
            Some(current) => (current as isize + delta).rem_euclid(count as isize) as usize,
            None if delta < 0 => count - 1,
            None => 0,
        });
        self.reveal_current();
        cx.notify();
    }

    fn reveal_current(&mut self) {
        if let Some((line, _)) = self.current.and_then(|ix| self.matches.get(ix)) {
            self.yaml_scroll
                .scroll_to_item(*line, ScrollStrategy::Center);
        }
    }

    fn select_line(&mut self, ix: usize, extend: bool, cx: &mut Context<Self>) {
        self.selection = Some(match self.selection {
            Some(selection) if extend => LineSelection {
                anchor: selection.anchor,
                cursor: ix,
            },
            _ => LineSelection {
                anchor: ix,
                cursor: ix,
            },
        });
        self.feedback = None;
        cx.notify();
    }

    /// Command-G and F3: the next match in the tab's search.
    fn find_match(&mut self, forward: bool, cx: &mut Context<Self>) {
        match self.tab {
            Tab::Yaml => self.step_match(if forward { 1 } else { -1 }, cx),
            Tab::Overview | Tab::Events | Tab::Logs | Tab::Ports => {}
        }
    }

    fn select_all(&mut self, cx: &mut Context<Self>) {
        match self.tab {
            Tab::Logs | Tab::Ports => return,
            _ => {}
        }
        let Some(count) = self.view().map(|view| view.lines.len()) else {
            return;
        };
        if self.tab != Tab::Yaml || count == 0 {
            return;
        }
        self.selection = Some(LineSelection {
            anchor: 0,
            cursor: count - 1,
        });
        cx.notify();
    }

    /// Command-F: the YAML search. The dock's logs have their own.
    fn focus_find(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        match self.tab {
            Tab::Logs | Tab::Ports => return,
            _ => {}
        }
        self.set_tab(Tab::Yaml, cx);
        let focus = self.find.read(cx).focus_handle(cx);
        window.focus(&focus, cx);
    }

    /// Escape: leaves the search, then drops a line selection, then steps
    /// back to the list.
    fn dismiss(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.find.read(cx).focus_handle(cx).is_focused(window) {
            if self.query.is_empty() {
                window.focus(&self.focus, cx);
            } else {
                // Setting the value from code emits no change event.
                self.find
                    .update(cx, |input, cx| input.set_value("", window, cx));
                self.set_query(String::new(), cx);
            }
        } else if self.tab == Tab::Yaml && self.selection.take().is_some() {
            cx.notify();
        } else {
            cx.emit(DetailEvent::Leave);
        }
    }
}
