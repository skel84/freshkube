//! The detail pane beside the resource list: one object's overview, YAML,
//! events and, for a pod, its logs. It reads the object when opened and again whenever the list shows
//! a new version of it, at most once a second, and watches the object's
//! events while open on a visible page. Read-only: Secret values stay hidden
//! until one is revealed, and nothing here changes the cluster.

use std::collections::HashMap;
use std::ops::Range;
use std::time::Duration;

use chrono::{DateTime, Local, Utc};
use freshkube_core::resources::{
    EventScope, EventUpdate, Failure, FailureKind, SecretValue, get_object, reveal_secret_value,
    watch_object_events,
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
use super::{example, live};
use crate::backend::{self, OwnedJob};
use crate::logs::PodLogView;

mod events;
mod overview;
#[cfg(test)]
mod tests;
mod view;
mod yaml;

use events::EventLine;
use overview::Summary;

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
        PreviousTab
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
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Tab {
    Overview,
    Yaml,
    Events,
    /// Pods only.
    Logs,
}

impl Tab {
    const ALL: [Tab; 4] = [Tab::Overview, Tab::Yaml, Tab::Events, Tab::Logs];

    fn index(self) -> usize {
        self as usize
    }
}

fn local_time(time: DateTime<Utc>) -> String {
    time.with_timezone(&Local)
        .format("%Y-%m-%d %H:%M:%S")
        .to_string()
}

type Job = (OwnedJob, Task<()>);

pub(crate) struct DetailPane {
    runtime: Handle,
    access: Option<KubeAccess>,
    /// Only an active pane, on a visible page, reads.
    active: bool,
    detail: Option<Detail>,
    title: SharedString,
    summary: Option<Summary>,
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
    tab: Tab,
    /// One per tab, in `Tab::ALL` order, so the arrows can move between them.
    tab_focus: [FocusHandle; 4],
    find: Entity<InputState>,
    query: String,
    matches: Vec<(usize, Range<usize>)>,
    current: Option<usize>,
    selection: Option<LineSelection>,
    show_all_labels: bool,
    show_all_annotations: bool,
    feedback: Option<SharedString>,
    focus: FocusHandle,
    yaml_scroll: UniformListScrollHandle,
    /// The open pod's logs. Its stream lives while the pod stays open,
    /// whichever tab shows.
    logs: Entity<PodLogView>,
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
            KeyBinding::new("right", NextTab, Some(TABS_CONTEXT)),
            KeyBinding::new("left", PreviousTab, Some(TABS_CONTEXT)),
        ]);
        let find = cx.new(|cx| InputState::new(window, cx).placeholder("Find in YAML"));
        let logs = cx.new(|cx| PodLogView::for_pods(runtime.clone(), window, cx));
        let subscriptions = vec![
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
            runtime,
            access: None,
            active: false,
            detail: None,
            title: SharedString::default(),
            summary: None,
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
            find,
            query: String::new(),
            matches: Vec::new(),
            current: None,
            selection: None,
            show_all_labels: false,
            show_all_annotations: false,
            feedback: None,
            focus: cx.focus_handle(),
            yaml_scroll: UniformListScrollHandle::new(),
            logs,
            _subscriptions: subscriptions,
        }
    }

    pub(crate) fn target_identity(&self) -> Option<&ResourceIdentity> {
        self.detail.as_ref().map(|detail| &detail.target.identity)
    }

    /// Fresh handles for the same connection.
    pub(crate) fn set_access(&mut self, access: KubeAccess, cx: &mut Context<Self>) {
        self.logs
            .update(cx, |logs, _| logs.set_access(access.clone()));
        self.access = Some(access);
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
        let pod = target.kind.is_pod().then(|| target.identity.clone());
        if pod.is_none() && self.tab == Tab::Logs {
            self.tab = Tab::Overview;
        }
        self.logs
            .update(cx, |logs, cx| logs.show_pod(pod, Some(access), cx));
        self.title = target.identity.address().into();
        self.detail = Some(Detail::new(target));
        self.follow = Follow::new(version);
        self.summary = None;
        self.event_lines.clear();
        self.matches.clear();
        self.current = None;
        self.selection = None;
        self.show_all_labels = false;
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
        self.logs
            .update(cx, |logs, cx| logs.show_pod(None, None, cx));
        self.detail = None;
        self.summary = None;
        self.event_lines.clear();
        self.matches.clear();
        self.current = None;
        self.selection = None;
        self.feedback = None;
        cx.notify();
    }

    /// Hiding the page stops every read and hides revealed values; showing
    /// it reads the open object again.
    pub(crate) fn set_active(&mut self, active: bool, cx: &mut Context<Self>) {
        if self.active == active {
            return;
        }
        self.active = active;
        self.logs.update(cx, |logs, cx| logs.set_active(active, cx));
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
        if let Some(containers) = view.document.overview.pod.clone() {
            self.logs
                .update(cx, |logs, cx| logs.set_containers(containers, cx));
        }
        self.summary = Some(Summary::new(&view));
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
        self.feedback = Some(
            match count {
                1 => "Copied 1 line".to_owned(),
                count => format!("Copied {count} lines"),
            }
            .into(),
        );
        cx.notify();
    }

    fn set_tab(&mut self, tab: Tab, cx: &mut Context<Self>) {
        self.tab = tab;
        self.feedback = None;
        self.show_tab(cx);
        cx.notify();
    }

    /// Moves `delta` tabs along, wrapping, from outside the pane.
    pub(crate) fn turn_tab(&mut self, delta: isize, cx: &mut Context<Self>) {
        self.step_tab(delta, cx);
    }

    /// Moves `delta` tabs along, wrapping, and returns the tab now shown.
    fn step_tab(&mut self, delta: isize, cx: &mut Context<Self>) -> Option<Tab> {
        let detail = self.detail.as_ref()?;
        let count = if detail.target.kind.is_pod() { 4 } else { 3 };
        let next = (self.tab.index() as isize + delta).rem_euclid(count) as usize;
        let tab = Tab::ALL[next];
        self.set_tab(tab, cx);
        Some(tab)
    }

    /// Puts the keyboard on what the current tab shows: the lines on the
    /// Logs tab, the pane on the others.
    pub(crate) fn focus(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.tab == Tab::Logs {
            self.logs
                .update(cx, |logs, cx| logs.focus_lines(window, cx));
        } else {
            window.focus(&self.focus, cx);
        }
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

    /// Tells the logs whether they show, and asks for them the first time
    /// they do for this pod.
    fn show_tab(&mut self, cx: &mut Context<Self>) {
        let shown = self.tab == Tab::Logs && self.detail.is_some();
        let active = self.active;
        self.logs.update(cx, |logs, cx| {
            if shown {
                logs.want(cx);
            }
            logs.set_visible(shown && active, cx);
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

    fn select_all(&mut self, cx: &mut Context<Self>) {
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

    fn focus_find(&mut self, window: &mut Window, cx: &mut Context<Self>) {
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
