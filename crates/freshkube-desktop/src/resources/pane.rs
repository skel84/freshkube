//! The detail pane beside the resource list: one object's overview, YAML and
//! events. It reads the object when opened and again whenever the list shows
//! a new version of it, at most once a second, and watches the object's
//! events while open on a visible page. Read-only: Secret values stay hidden
//! until one is revealed, and nothing here changes the cluster.

use std::collections::HashMap;
use std::ops::Range;
use std::time::Duration;

use chrono::{DateTime, Local, Utc};
use freshkube_core::resources::{
    Condition, EventScope, EventUpdate, Failure, FailureKind, ObjectEvent, SecretValue, get_object,
    reveal_secret_value, watch_object_events,
};
use gpui_kit::assets::IconName;
use gpui_kit::component::{
    Icon, Sizable,
    button::{Button, ButtonVariants},
    h_flex,
    input::{Input, InputEvent, InputState},
    v_flex,
};
use gpui_kit::prelude::*;
use gpui_kit::*;
use tokio::runtime::Handle;
use tokio::sync::mpsc;

use super::detail::{
    Detail, DetailTarget, DocumentRead, DocumentView, EventsRead, Follow, LineSelection,
    MAX_EVENTS, MAX_MATCHES, Reveal, YamlLine,
};
use super::model::ResourceIdentity;
use super::screen::KubeAccess;
use super::{example, live};
use crate::backend::{self, OwnedJob};
use crate::palette::{Palette, palette};
use crate::screens::{field, panel};
use crate::ui::{self, MONO_FONT, Tone};

const CONTEXT: &str = "KubeDetail";
const LINE_HEIGHT: f32 = 20.;
/// How long one read of the object may take.
const READ_DEADLINE: Duration = Duration::from_secs(30);
/// Arrow keys open the row they land on once they pause this long, so
/// holding one down doesn't read every object it passes.
pub(crate) const KEYBOARD_PAUSE: Duration = Duration::from_millis(150);
const LABELS_SHOWN: usize = 12;
const ANNOTATIONS_SHOWN: usize = 6;
/// Even "Show all" draws at most this many labels or annotations; the YAML
/// has every one.
const MAX_SHOWN: usize = 200;
const ANNOTATION_CHARS: usize = 240;
/// Revealed values are cut to this many characters when drawn; Copy takes
/// the whole value.
const VALUE_CHARS: usize = 4_000;

actions!(
    kube_detail,
    [FindInYaml, SelectAllLines, CopyLines, Dismiss]
);

/// What the pane asks of the page around it.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum DetailEvent {
    /// The user closed the pane.
    Closed,
    /// The user chose to open this object: the one now at the address of
    /// the deleted object the pane shows.
    Open(ResourceIdentity),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Tab {
    Overview,
    Yaml,
    Events,
}

struct ConditionLine {
    kind: SharedString,
    status: SharedString,
    tone: Tone,
    detail: Option<SharedString>,
    changed: Option<SharedString>,
}

impl ConditionLine {
    fn new(condition: &Condition) -> Self {
        let detail = [condition.reason.as_str(), condition.message.as_str()]
            .into_iter()
            .filter(|text| !text.is_empty())
            .collect::<Vec<_>>()
            .join(" · ");
        Self {
            kind: condition.kind.clone().into(),
            status: condition.status.clone().into(),
            tone: condition_tone(&condition.kind, &condition.status),
            detail: (!detail.is_empty()).then(|| detail.into()),
            changed: condition
                .changed
                .map(|time| format!("since {}", local_time(time)).into()),
        }
    }
}

/// How a condition reads. Most are good when `True`; a few report a problem
/// when `True`. Types this doesn't know stay neutral rather than guess.
fn condition_tone(kind: &str, status: &str) -> Tone {
    const GOOD_WHEN_TRUE: [&str; 9] = [
        "Ready",
        "Available",
        "Initialized",
        "ContainersReady",
        "PodScheduled",
        "PodReadyToStartContainers",
        "Progressing",
        "Complete",
        "Established",
    ];
    const BAD_WHEN_TRUE: [&str; 6] = [
        "MemoryPressure",
        "DiskPressure",
        "PIDPressure",
        "NetworkUnavailable",
        "ReplicaFailure",
        "Failed",
    ];
    let good_when_true = if GOOD_WHEN_TRUE.contains(&kind) {
        true
    } else if BAD_WHEN_TRUE.contains(&kind) {
        false
    } else {
        return if status == "Unknown" {
            Tone::Unknown
        } else {
            Tone::Outline
        };
    };
    match status {
        "True" | "False" if (status == "True") == good_when_true => Tone::Good,
        "True" | "False" => Tone::Crit,
        "Unknown" => Tone::Unknown,
        _ => Tone::Outline,
    }
}

struct SecretLine {
    name: String,
    size: SharedString,
}

/// The overview's text, derived once per document read, never while drawing.
struct Summary {
    kind: SharedString,
    created: Option<SharedString>,
    deleting: Option<SharedString>,
    generation: Option<SharedString>,
    owners: Vec<SharedString>,
    conditions: Vec<ConditionLine>,
    labels: Vec<SharedString>,
    label_count: usize,
    annotations: Vec<(SharedString, SharedString)>,
    annotation_count: usize,
    finalizers: Vec<SharedString>,
    /// A Secret's type and keys with their sizes.
    secret: Option<(SharedString, Vec<SecretLine>)>,
}

impl Summary {
    fn new(view: &DocumentView) -> Self {
        let document = &view.document;
        let overview = &document.overview;
        Self {
            kind: format!("{} · {}", document.kind, document.api_version).into(),
            created: overview.created.map(|time| local_time(time).into()),
            deleting: overview.deleting.map(|time| local_time(time).into()),
            generation: overview.generation.map(|generation| {
                match overview.observed_generation {
                    Some(observed) if observed < generation => {
                        format!("{generation}; its controller has seen {observed}")
                    }
                    Some(_) => format!("{generation}, seen by its controller"),
                    None => generation.to_string(),
                }
                .into()
            }),
            owners: overview
                .owners
                .iter()
                .map(|owner| {
                    let controller = if owner.controller {
                        " (controller)"
                    } else {
                        ""
                    };
                    format!("{} {}{controller}", owner.kind, owner.name).into()
                })
                .collect(),
            conditions: overview.conditions.iter().map(ConditionLine::new).collect(),
            labels: overview
                .labels
                .iter()
                .take(MAX_SHOWN)
                .map(|(key, value)| format!("{key}={value}").into())
                .collect(),
            label_count: overview.labels.len(),
            annotations: overview
                .annotations
                .iter()
                .take(MAX_SHOWN)
                .map(|(key, value)| (key.clone().into(), shorten(value, ANNOTATION_CHARS).into()))
                .collect(),
            annotation_count: overview.annotations.len(),
            finalizers: overview
                .finalizers
                .iter()
                .map(|finalizer| finalizer.clone().into())
                .collect(),
            secret: overview.secret.as_ref().map(|secret| {
                (
                    secret.secret_type.clone().into(),
                    secret
                        .keys
                        .iter()
                        .map(|key| SecretLine {
                            name: key.name.clone(),
                            size: byte_size(key.bytes).into(),
                        })
                        .collect(),
                )
            }),
        }
    }
}

/// One event as listed, derived when the events change.
struct EventLine {
    warning: bool,
    /// What a screen reader says for the row.
    label: SharedString,
    reason: SharedString,
    message: SharedString,
    detail: SharedString,
}

impl EventLine {
    fn new(event: &ObjectEvent) -> Self {
        let when = match (event.count, event.last_seen.or(event.first_seen)) {
            (count, Some(last)) if count > 1 => {
                format!("{count} times, last at {}", local_time(last))
            }
            (_, Some(last)) => local_time(last),
            (count, None) if count > 1 => format!("{count} times"),
            _ => String::new(),
        };
        let detail = [when.as_str(), &event.source, &event.field_path]
            .into_iter()
            .filter(|text| !text.is_empty())
            .collect::<Vec<_>>()
            .join(" · ");
        Self {
            warning: event.is_warning(),
            label: format!("{}: {}", event.reason, event.message).into(),
            reason: event.reason.clone().into(),
            message: event.message.clone().into(),
            detail: detail.into(),
        }
    }
}

fn local_time(time: DateTime<Utc>) -> String {
    time.with_timezone(&Local)
        .format("%Y-%m-%d %H:%M:%S")
        .to_string()
}

fn byte_size(bytes: usize) -> String {
    match bytes {
        1 => "1 byte".into(),
        0..1_024 => format!("{bytes} bytes"),
        _ => format!("{:.1} KiB", bytes as f64 / 1_024.),
    }
}

/// `text` on one line, cut to `max` characters.
fn shorten(text: &str, max: usize) -> String {
    let line = text.trim_end().replace('\n', " ↵ ");
    match line.char_indices().nth(max) {
        Some((cut, _)) => format!("{}…", &line[..cut]),
        None => line,
    }
}

/// Styles for one drawn line: the key tinted, and search matches marked,
/// the current one more strongly. `StyledText` needs ranges in order and
/// apart, so the line is cut at every boundary first.
fn line_highlights(
    key: Option<Range<usize>>,
    matches: &[(Range<usize>, bool)],
    key_color: Hsla,
    mark: Hsla,
) -> Vec<(Range<usize>, HighlightStyle)> {
    let mut cuts: Vec<usize> = key
        .iter()
        .chain(matches.iter().map(|(range, _)| range))
        .flat_map(|range| [range.start, range.end])
        .collect();
    cuts.sort_unstable();
    cuts.dedup();
    let mut highlights = Vec::new();
    for pair in cuts.windows(2) {
        let range = pair[0]..pair[1];
        let within = |outer: &Range<usize>| outer.start <= range.start && range.end <= outer.end;
        let keyed = key.as_ref().is_some_and(within);
        let found = matches.iter().find(|(outer, _)| within(outer));
        if !keyed && found.is_none() {
            continue;
        }
        highlights.push((
            range,
            HighlightStyle {
                color: keyed.then_some(key_color),
                background_color: found.map(
                    |(_, current)| {
                        if *current { mark } else { mark.opacity(0.4) }
                    },
                ),
                ..HighlightStyle::default()
            },
        ));
    }
    highlights
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
        ]);
        let find = cx.new(|cx| InputState::new(window, cx).placeholder("Find in YAML"));
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
            _subscriptions: subscriptions,
        }
    }

    pub(crate) fn target_identity(&self) -> Option<&ResourceIdentity> {
        self.detail.as_ref().map(|detail| &detail.target.identity)
    }

    /// Fresh handles for the same connection.
    pub(crate) fn set_access(&mut self, access: KubeAccess) {
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
        self.access = Some(access);
        if self.target_identity() == Some(&target.identity) {
            return;
        }
        self.stop_reads();
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
            self.document_changed();
        }
        cx.notify();
        self.schedule_follow(cx);
    }

    /// Derives what the new document shows: its overview, and the search
    /// and selection over its lines.
    fn document_changed(&mut self) {
        let Some(view) = self.detail.as_ref().and_then(|detail| detail.view.clone()) else {
            return;
        };
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
        cx.notify();
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

    /// Escape: leaves the search, then drops a line selection, then closes
    /// the pane.
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
        } else if self.selection.take().is_some() {
            cx.notify();
        } else {
            cx.emit(DetailEvent::Closed);
        }
    }

    fn header(&self, detail: &Detail, cx: &mut Context<Self>) -> Div {
        let state = match &detail.read {
            DocumentRead::Loading => Some((Tone::Unknown, "Reading")),
            DocumentRead::Loaded => None,
            DocumentRead::Refused(_) => Some((Tone::Crit, "Not permitted")),
            DocumentRead::Failed(_) => Some((Tone::Crit, "Failed")),
            DocumentRead::Stale(_) => Some((Tone::Warn, "Stale")),
            DocumentRead::Deleted => Some((Tone::Crit, "Deleted")),
        };
        h_flex()
            .items_start()
            .gap_2()
            .px_4()
            .pt_3()
            .pb_2()
            .child(
                v_flex()
                    .flex_1()
                    .min_w_0()
                    .gap_0p5()
                    .child(ui::caption(&detail.target.kind.kind, cx))
                    .child(
                        div()
                            .id("detail-title")
                            .test_support()
                            .aria_label(self.title.clone())
                            .font_family(MONO_FONT)
                            .text_size(px(13.5))
                            .truncate()
                            .child(self.title.clone()),
                    ),
            )
            .children(state.map(|(tone, text)| {
                div()
                    .id("detail-state")
                    .test_support()
                    .role(Role::Status)
                    .aria_label(text)
                    .mt(px(14.))
                    .child(ui::tag(tone, None, text, cx))
            }))
            .when(detail.view.is_some(), |this| {
                this.child(
                    div().flex_none().mt(px(10.)).child(
                        Button::new("detail-copy-yaml")
                            .outline()
                            .xsmall()
                            .icon(IconName::Copy)
                            .label("Copy YAML")
                            .tooltip("Copies the whole document as read; Secret values stay hidden")
                            .on_click(cx.listener(|pane, _, _, cx| pane.copy_document(cx))),
                    ),
                )
            })
            .child(
                div().flex_none().mt(px(10.)).child(
                    Button::new("detail-close")
                        .ghost()
                        .xsmall()
                        .icon(IconName::X)
                        .tooltip("Close")
                        .accessibility_label("Close details")
                        .on_click(cx.listener(|_, _, _, cx| cx.emit(DetailEvent::Closed))),
                ),
            )
    }

    /// Deleted and stale documents say so above the tabs.
    fn notice(&self, detail: &Detail, cx: &mut Context<Self>) -> Option<AnyElement> {
        let (id, banner) = match (&detail.read, &detail.recreated) {
            (DocumentRead::Deleted, Some(successor)) => {
                let successor = successor.clone();
                (
                    "detail-deleted",
                    ui::warning_banner(
                        Some("Deleted and created again.".into()),
                        "An object with this name exists again, with a new UID. This pane still shows the one that was deleted.",
                        Some(
                            Button::new("detail-open-recreated")
                                .outline()
                                .small()
                                .label("Open the new one")
                                .on_click(cx.listener(move |_, _, _, cx| {
                                    cx.emit(DetailEvent::Open(successor.clone()))
                                }))
                                .into_any_element(),
                        ),
                        cx,
                    ),
                )
            }
            (DocumentRead::Deleted, None) if detail.view.is_some() => (
                "detail-deleted",
                ui::warning_banner(
                    Some("This object was deleted.".into()),
                    "Showing it as last read.",
                    None,
                    cx,
                ),
            ),
            (DocumentRead::Stale(reason), _) => (
                "detail-stale",
                ui::warning_banner(
                    Some("Couldn't read it again.".into()),
                    format!("Showing it as last read. {reason}"),
                    Some(
                        Button::new("detail-stale-retry")
                            .outline()
                            .small()
                            .icon(IconName::RefreshCw)
                            .label("Retry")
                            .on_click(cx.listener(|pane, _, _, cx| pane.refresh(cx)))
                            .into_any_element(),
                    ),
                    cx,
                ),
            ),
            _ => return None,
        };
        Some(
            div()
                .id(id)
                .test_support()
                .role(Role::Status)
                .px_4()
                .pb_2()
                .child(banner)
                .into_any_element(),
        )
    }

    fn tabs(&self, detail: &Detail, cx: &mut Context<Self>) -> Div {
        let p = palette(cx);
        let events = &detail.events;
        let tab = |id: &'static str, tab: Tab, label: SharedString, extra: Option<Div>| {
            let active = self.tab == tab;
            h_flex()
                .id(id)
                .test_support()
                .role(Role::Tab)
                .aria_selected(active)
                .aria_label(label.clone())
                .h(px(32.))
                .px_2p5()
                .gap_1p5()
                .cursor_pointer()
                .text_size(px(12.5))
                .border_b_2()
                .map(|this| {
                    if active {
                        this.border_color(p.accent)
                            .text_color(p.ink)
                            .font_weight(FontWeight::SEMIBOLD)
                    } else {
                        this.border_color(ui::transparent())
                            .text_color(p.muted)
                            .hover(|style| style.text_color(p.ink))
                    }
                })
                .child(label)
                .children(extra)
                .on_click(cx.listener(move |pane, _, _, cx| pane.set_tab(tab, cx)))
                .into_any_element()
        };
        let count = match events.read() {
            EventsRead::Loaded | EventsRead::Stale(_) => Some(events.len()),
            _ => None,
        };
        let warnings = events.warnings();
        h_flex()
            .px_3()
            .gap_1()
            .border_b_1()
            .border_color(p.line)
            .child(tab(
                "detail-tab-overview",
                Tab::Overview,
                "Overview".into(),
                None,
            ))
            .child(tab("detail-tab-yaml", Tab::Yaml, "YAML".into(), None))
            .child(tab(
                "detail-tab-events",
                Tab::Events,
                match count {
                    Some(count) => format!("Events {count}").into(),
                    None => "Events".into(),
                },
                (warnings > 0).then(|| {
                    ui::tag(
                        Tone::Warn,
                        None,
                        match warnings {
                            1 => "1 warning".to_owned(),
                            count => format!("{count} warnings"),
                        },
                        cx,
                    )
                }),
            ))
    }

    /// What replaces the overview and YAML while there is no document.
    fn document_state(&self, detail: &Detail, cx: &mut Context<Self>) -> AnyElement {
        let kind = detail.target.kind.kind.to_lowercase();
        let state = |id: &'static str, element: Div| {
            element
                .id(id)
                .test_support()
                .role(Role::Status)
                .into_any_element()
        };
        match &detail.read {
            DocumentRead::Refused(reason) => state(
                "detail-refused",
                ui::empty_state(
                    IconName::ShieldX,
                    format!("Not permitted to read this {kind}"),
                    "The identity may list these objects but not read this one in full. Its events may still be readable.",
                    Some(reason.clone()),
                    Vec::new(),
                    cx,
                ),
            ),
            DocumentRead::Failed(reason) => state(
                "detail-failed",
                ui::empty_state(
                    IconName::CircleDashed,
                    format!("Couldn't read this {kind}"),
                    "Nothing was read, so nothing is shown.",
                    Some(reason.clone()),
                    vec![
                        Button::new("detail-retry")
                            .primary()
                            .icon(IconName::RefreshCw)
                            .label("Retry")
                            .on_click(cx.listener(|pane, _, _, cx| pane.refresh(cx)))
                            .into_any_element(),
                    ],
                    cx,
                ),
            ),
            DocumentRead::Deleted => state(
                "detail-gone",
                ui::empty_state(
                    IconName::Trash,
                    format!("This {kind} was deleted"),
                    "It was deleted before it could be read.",
                    None,
                    Vec::new(),
                    cx,
                ),
            ),
            DocumentRead::Loading | DocumentRead::Loaded | DocumentRead::Stale(_) => state(
                "detail-loading",
                v_flex()
                    .p_4()
                    .gap_3()
                    .children((0..8).map(|_| ui::skeleton(relative(0.7), px(12.)))),
            ),
        }
    }

    fn overview(&self, detail: &Detail, summary: &Summary, cx: &mut Context<Self>) -> AnyElement {
        let p = palette(cx);
        let identity = &detail.target.identity;
        let section = |title: &str, cx: &App| v_flex().gap_2().child(ui::caption(title, cx));
        let mut body = v_flex().gap(px(18.)).child(
            v_flex()
                .gap_2()
                .child(field("Kind", summary.kind.clone(), cx))
                .when(!identity.namespace.is_empty(), |this| {
                    this.child(field("Namespace", identity.namespace.clone(), cx))
                })
                .child(field(
                    "UID",
                    div()
                        .font_family(MONO_FONT)
                        .text_size(px(12.))
                        .child(identity.uid.clone()),
                    cx,
                ))
                .children(
                    summary
                        .created
                        .clone()
                        .map(|created| field("Created", created, cx)),
                )
                .children(summary.deleting.clone().map(|deleting| {
                    field(
                        "Deleting since",
                        div().text_color(p.crit_ink).child(deleting),
                        cx,
                    )
                }))
                .children(
                    summary
                        .generation
                        .clone()
                        .map(|generation| field("Generation", generation, cx)),
                ),
        );
        if let Some((secret_type, keys)) = &summary.secret {
            body = body.child(self.secret(detail, secret_type, keys, cx));
        }
        if !summary.conditions.is_empty() {
            body =
                body.child(
                    section("Conditions", cx).children(summary.conditions.iter().enumerate().map(
                        |(ix, condition)| {
                            h_flex()
                                .id(("detail-condition", ix))
                                .test_support()
                                .items_start()
                                .gap_2()
                                .child(
                                    div()
                                        .w(px(132.))
                                        .flex_none()
                                        .text_size(px(12.5))
                                        .child(condition.kind.clone()),
                                )
                                .child(ui::tag(condition.tone, None, condition.status.clone(), cx))
                                .child(
                                    v_flex()
                                        .flex_1()
                                        .min_w_0()
                                        .text_size(px(12.))
                                        .children(condition.detail.clone())
                                        .children(condition.changed.clone().map(|changed| {
                                            div().text_color(p.muted).child(changed)
                                        })),
                                )
                                .into_any_element()
                        },
                    )),
                );
        }
        if !summary.owners.is_empty() {
            body = body.child(
                section("Owned by", cx).children(
                    summary
                        .owners
                        .iter()
                        .map(|owner| div().text_size(px(12.5)).child(owner.clone())),
                ),
            );
        }
        if summary.label_count > 0 {
            let shown = if self.show_all_labels {
                summary.labels.len()
            } else {
                LABELS_SHOWN.min(summary.labels.len())
            };
            body = body.child(
                section("Labels", cx)
                    .child(
                        h_flex()
                            .id("detail-labels")
                            .test_support()
                            .flex_wrap()
                            .gap_1p5()
                            .children(summary.labels[..shown].iter().map(|label| {
                                div()
                                    .max_w_full()
                                    .px_1p5()
                                    .py_0p5()
                                    .rounded(px(5.))
                                    .bg(p.surface_2)
                                    .border_1()
                                    .border_color(p.line)
                                    .font_family(MONO_FONT)
                                    .text_size(px(11.5))
                                    .truncate()
                                    .child(label.clone())
                            })),
                    )
                    .children(self.show_more(
                        "detail-labels-all",
                        shown,
                        summary.label_count,
                        "labels",
                        |pane| &mut pane.show_all_labels,
                        cx,
                    )),
            );
        }
        if summary.annotation_count > 0 {
            let shown = if self.show_all_annotations {
                summary.annotations.len()
            } else {
                ANNOTATIONS_SHOWN.min(summary.annotations.len())
            };
            body = body.child(
                section("Annotations", cx)
                    .child(
                        v_flex()
                            .id("detail-annotations")
                            .test_support()
                            .gap_1p5()
                            .children(summary.annotations[..shown].iter().map(|(key, value)| {
                                v_flex()
                                    .min_w_0()
                                    .child(
                                        div()
                                            .font_family(MONO_FONT)
                                            .text_size(px(11.5))
                                            .text_color(p.ink_2)
                                            .truncate()
                                            .child(key.clone()),
                                    )
                                    .child(
                                        div()
                                            .font_family(MONO_FONT)
                                            .text_size(px(11.5))
                                            .child(value.clone()),
                                    )
                            })),
                    )
                    .children(self.show_more(
                        "detail-annotations-all",
                        shown,
                        summary.annotation_count,
                        "annotations",
                        |pane| &mut pane.show_all_annotations,
                        cx,
                    )),
            );
        }
        if !summary.finalizers.is_empty() {
            body = body.child(
                section("Finalizers", cx).children(summary.finalizers.iter().map(|finalizer| {
                    div()
                        .font_family(MONO_FONT)
                        .text_size(px(12.))
                        .child(finalizer.clone())
                })),
            );
        }
        div()
            .id("detail-overview")
            .test_support()
            .size_full()
            .overflow_y_scroll()
            .px_4()
            .py_3()
            .child(body)
            .into_any_element()
    }

    /// "Show all" under a capped list, or how many more the YAML has.
    fn show_more(
        &self,
        id: &'static str,
        shown: usize,
        total: usize,
        what: &str,
        flag: fn(&mut Self) -> &mut bool,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        if shown == total {
            return None;
        }
        let p = palette(cx);
        if shown == MAX_SHOWN {
            return Some(
                div()
                    .text_size(px(12.))
                    .text_color(p.muted)
                    .child(format!("{} more in the YAML", total - shown))
                    .into_any_element(),
            );
        }
        Some(
            div()
                .child(
                    Button::new(id)
                        .ghost()
                        .xsmall()
                        .label(format!("Show all {total} {what}"))
                        .on_click(cx.listener(move |pane, _, _, cx| {
                            *flag(pane) = true;
                            cx.notify();
                        })),
                )
                .into_any_element(),
        )
    }

    fn secret(
        &self,
        detail: &Detail,
        secret_type: &SharedString,
        keys: &[SecretLine],
        cx: &mut Context<Self>,
    ) -> Div {
        let p = palette(cx);
        v_flex()
            .gap_2()
            .child(ui::caption("Secret data", cx))
            .child(field("Type", secret_type.clone(), cx))
            .child(div().text_size(px(12.)).text_color(p.muted).child(
                "Values are hidden. Reveal reads one value afresh; Copy YAML never includes them.",
            ))
            .children(keys.iter().enumerate().map(|(ix, key)| {
                let reveal = detail.reveals.get(&key.name);
                let name = key.name.clone();
                let actions = match reveal {
                    None | Some(Reveal::Failed(_)) => vec![
                        Button::new(("detail-secret-reveal", ix))
                            .outline()
                            .xsmall()
                            .icon(IconName::Eye)
                            .label("Reveal")
                            .on_click(cx.listener({
                                let name = name.clone();
                                move |pane, _, _, cx| pane.reveal(name.clone(), cx)
                            }))
                            .into_any_element(),
                    ],
                    Some(Reveal::Reading) => vec![
                        Button::new(("detail-secret-reveal", ix))
                            .outline()
                            .xsmall()
                            .loading(true)
                            .label("Reading")
                            .into_any_element(),
                    ],
                    Some(Reveal::Shown(value)) => {
                        let mut actions = vec![
                            Button::new(("detail-secret-hide", ix))
                                .outline()
                                .xsmall()
                                .icon(IconName::EyeOff)
                                .label("Hide")
                                .on_click(cx.listener({
                                    let name = name.clone();
                                    move |pane, _, _, cx| pane.hide(&name, cx)
                                }))
                                .into_any_element(),
                        ];
                        if let SecretValue::Text(_) = value {
                            actions.push(
                                Button::new(("detail-secret-copy", ix))
                                    .ghost()
                                    .xsmall()
                                    .icon(IconName::Copy)
                                    .label("Copy")
                                    .on_click(cx.listener({
                                        let name = name.clone();
                                        move |pane, _, _, cx| pane.copy_value(&name, cx)
                                    }))
                                    .into_any_element(),
                            );
                        }
                        actions
                    }
                };
                let value = match reveal {
                    Some(Reveal::Shown(SecretValue::Text(text))) => Some(
                        div()
                            .id(("detail-secret-value", ix))
                            .test_support()
                            .max_h(px(160.))
                            .overflow_y_scroll()
                            .px_2()
                            .py_1p5()
                            .rounded(px(6.))
                            .bg(p.surface_2)
                            .font_family(MONO_FONT)
                            .text_size(px(12.))
                            .child(match text.char_indices().nth(VALUE_CHARS) {
                                Some((cut, _)) => {
                                    format!("{}… (Copy takes the whole value)", &text[..cut])
                                }
                                None => text.clone(),
                            })
                            .into_any_element(),
                    ),
                    Some(Reveal::Shown(SecretValue::Binary(bytes))) => Some(
                        div()
                            .id(("detail-secret-value", ix))
                            .test_support()
                            .text_size(px(12.))
                            .text_color(p.muted)
                            .child(format!("Binary data, {}; not shown.", byte_size(*bytes)))
                            .into_any_element(),
                    ),
                    Some(Reveal::Failed(reason)) => Some(
                        div()
                            .id(("detail-secret-error", ix))
                            .test_support()
                            .text_size(px(12.))
                            .text_color(p.crit_ink)
                            .child(reason.clone())
                            .into_any_element(),
                    ),
                    _ => None,
                };
                v_flex()
                    .gap_1()
                    .child(
                        h_flex()
                            .id(("detail-secret-key", ix))
                            .test_support()
                            .gap_2()
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .font_family(MONO_FONT)
                                    .text_size(px(12.5))
                                    .truncate()
                                    .child(key.name.clone()),
                            )
                            .child(
                                div()
                                    .flex_none()
                                    .text_size(px(12.))
                                    .text_color(p.muted)
                                    .child(key.size.clone()),
                            )
                            .children(actions),
                    )
                    .children(value)
            }))
    }

    fn yaml(&self, view: &DocumentView, cx: &mut Context<Self>) -> AnyElement {
        let p = palette(cx);
        let count = self.matches.len();
        let counter = (!self.query.is_empty()).then(|| match (count, self.current) {
            (0, _) => "No matches".to_owned(),
            (count, Some(ix)) if count == MAX_MATCHES => format!("{} of {count}+", ix + 1),
            (count, Some(ix)) => format!("{} of {count}", ix + 1),
            (count, None) => format!("{count} matches"),
        });
        let selected = self.selection.map(|selection| selection.range().len());
        let toolbar = h_flex()
            .gap_1()
            .px_3()
            .py_2()
            .child(
                div().flex_1().min_w_0().child(
                    Input::new(&self.find)
                        .id("detail-find")
                        .aria_label("Find in the YAML; Enter for the next match, Shift-Enter for the previous")
                        .small()
                        .cleanable(true)
                        .prefix(Icon::new(IconName::Search).with_size(px(14.))),
                ),
            )
            .children(counter.map(|counter| {
                div()
                    .id("detail-find-count")
                    .test_support()
                    .aria_label(counter.clone())
                    .flex_none()
                    .text_size(px(11.5))
                    .text_color(p.muted)
                    .child(counter)
            }))
            .child(
                Button::new("detail-find-previous")
                    .ghost()
                    .xsmall()
                    .icon(IconName::ChevronUp)
                    .tooltip("Previous match")
                    .on_click(cx.listener(|pane, _, _, cx| pane.step_match(-1, cx))),
            )
            .child(
                Button::new("detail-find-next")
                    .ghost()
                    .xsmall()
                    .icon(IconName::ChevronDown)
                    .tooltip("Next match")
                    .on_click(cx.listener(|pane, _, _, cx| pane.step_match(1, cx))),
            )
            .children(selected.map(|lines| {
                Button::new("detail-copy-lines")
                    .outline()
                    .xsmall()
                    .icon(IconName::Copy)
                    .label(match lines {
                        1 => "Copy 1 line".to_owned(),
                        lines => format!("Copy {lines} lines"),
                    })
                    .on_click(cx.listener(|pane, _, _, cx| pane.copy_lines(cx)))
            }));
        let lines = view.lines.len();
        // Room for the widest line number.
        let gutter = px(lines.to_string().len() as f32 * 7.2 + 20.);
        v_flex()
            .size_full()
            .child(toolbar)
            .child(
                div()
                    .id("detail-yaml")
                    .test_support()
                    .role(Role::ListBox)
                    .aria_label("YAML lines; click selects, Shift-click extends, Command-C copies")
                    .flex_1()
                    .min_h_0()
                    .border_t_1()
                    .border_color(p.line)
                    .child(
                        uniform_list(
                            "detail-yaml-lines",
                            lines,
                            cx.processor(move |pane, range: Range<usize>, _, cx| {
                                range
                                    .filter_map(|ix| pane.render_line(ix, gutter, cx))
                                    .collect::<Vec<_>>()
                            }),
                        )
                        .with_horizontal_sizing_behavior(
                            ListHorizontalSizingBehavior::Unconstrained,
                        )
                        .with_width_from_item(Some(view.longest))
                        .track_scroll(&self.yaml_scroll)
                        .size_full(),
                    ),
            )
            .into_any_element()
    }

    fn render_line(&self, ix: usize, gutter: Pixels, cx: &mut Context<Self>) -> Option<AnyElement> {
        let view = self.view()?;
        let line: &YamlLine = view.lines.get(ix)?;
        let p = palette(cx);
        let selected = self
            .selection
            .is_some_and(|selection| selection.contains(ix));
        let start = self.matches.partition_point(|(at, _)| *at < ix);
        let matches: Vec<(Range<usize>, bool)> = self.matches[start..]
            .iter()
            .enumerate()
            .take_while(|(_, (at, _))| *at == ix)
            .filter_map(|(offset, (_, range))| {
                let range = range.start.min(line.drawn)..range.end.min(line.drawn);
                (!range.is_empty()).then(|| (range, self.current == Some(start + offset)))
            })
            .collect();
        let highlights = line_highlights(line.key.clone(), &matches, key_color(&p), p.mark);
        Some(
            h_flex()
                .id(("detail-yaml-line", ix))
                .test_support()
                .role(Role::ListBoxOption)
                .aria_selected(selected)
                .aria_label(view.line(ix).to_owned())
                .w_full()
                .h(px(LINE_HEIGHT))
                .font_family(MONO_FONT)
                .text_size(px(12.))
                .whitespace_nowrap()
                .cursor_text()
                .when(selected, |this| this.bg(p.accent_soft))
                .when(!selected, |this| this.hover(|style| style.bg(p.hover)))
                .child(
                    div()
                        .flex_none()
                        .w(gutter)
                        .pr_3()
                        .text_right()
                        .text_color(p.faint)
                        .child((ix + 1).to_string()),
                )
                .child(
                    div()
                        .flex_none()
                        .pr_4()
                        .child(StyledText::new(line.text.clone()).with_highlights(highlights)),
                )
                .on_click(cx.listener(move |pane, event: &ClickEvent, window, cx| {
                    pane.select_line(ix, event.modifiers().shift, cx);
                    window.focus(&pane.focus, cx);
                }))
                .into_any_element(),
        )
    }

    fn events(&self, detail: &Detail, cx: &mut Context<Self>) -> AnyElement {
        let p = palette(cx);
        let events = &detail.events;
        let state = |id: &'static str, element: Div| {
            element
                .id(id)
                .test_support()
                .role(Role::Status)
                .into_any_element()
        };
        match events.read() {
            EventsRead::Loading => {
                return state(
                    "detail-events-loading",
                    v_flex()
                        .p_4()
                        .gap_3()
                        .children((0..4).map(|_| ui::skeleton(relative(0.8), px(12.)))),
                );
            }
            EventsRead::Refused(reason) => {
                return state(
                    "detail-events-refused",
                    ui::empty_state(
                        IconName::ShieldX,
                        "Not permitted to list events",
                        "The identity may not list events here. That says nothing about whether any exist.",
                        Some(reason.clone()),
                        Vec::new(),
                        cx,
                    ),
                );
            }
            EventsRead::Failed(reason) => {
                return state(
                    "detail-events-failed",
                    ui::empty_state(
                        IconName::CircleDashed,
                        "Couldn't list events",
                        "Nothing is known yet, so nothing is shown as missing. Listing retries by itself.",
                        Some(reason.clone()),
                        Vec::new(),
                        cx,
                    ),
                );
            }
            EventsRead::Loaded | EventsRead::Stale(_) => {}
        }
        let stale = match events.read() {
            EventsRead::Stale(reason) => Some(
                div()
                    .id("detail-events-stale")
                    .test_support()
                    .role(Role::Status)
                    .child(ui::warning_banner(
                        Some("The events watch was interrupted; reconnecting.".into()),
                        format!("Showing events as last seen. {reason}"),
                        None,
                        cx,
                    )),
            ),
            _ => None,
        };
        let body = if self.event_lines.is_empty() {
            div()
                .id("detail-events-empty")
                .test_support()
                .text_size(px(12.5))
                .text_color(p.muted)
                .child("No events recorded. Kubernetes keeps events for about an hour, so a quiet object has none.")
                .into_any_element()
        } else {
            v_flex()
                .gap_3()
                .children(self.event_lines.iter().enumerate().map(|(ix, line)| {
                    v_flex()
                        .id(("detail-event", ix))
                        .test_support()
                        .aria_label(line.label.clone())
                        .gap_0p5()
                        .child(
                            h_flex()
                                .gap_2()
                                .child(if line.warning {
                                    ui::tag(
                                        Tone::Warn,
                                        Some(IconName::TriangleAlert),
                                        "Warning",
                                        cx,
                                    )
                                } else {
                                    ui::tag(Tone::Outline, None, "Normal", cx)
                                })
                                .child(
                                    div()
                                        .text_size(px(12.5))
                                        .font_weight(FontWeight::SEMIBOLD)
                                        .child(line.reason.clone()),
                                ),
                        )
                        .child(div().text_size(px(12.5)).child(line.message.clone()))
                        .child(
                            div()
                                .text_size(px(11.5))
                                .text_color(p.muted)
                                .child(line.detail.clone()),
                        )
                }))
                .when(events.len() > MAX_EVENTS, |this| {
                    this.child(
                        div()
                            .id("detail-events-capped")
                            .test_support()
                            .text_size(px(12.))
                            .text_color(p.muted)
                            .child(format!(
                                "Showing the newest {MAX_EVENTS} of {} events.",
                                events.len()
                            )),
                    )
                })
                .into_any_element()
        };
        div()
            .id("detail-events")
            .test_support()
            .size_full()
            .overflow_y_scroll()
            .px_4()
            .py_3()
            .child(v_flex().gap_3().children(stale).child(body))
            .into_any_element()
    }
}

fn key_color(p: &Palette) -> Hsla {
    p.accent
}

impl Render for DetailPane {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        crate::desktop::probe::hit("resource-detail");
        let Some(detail) = self.detail.as_ref() else {
            return div().into_any_element();
        };
        let p = palette(cx);
        let body = match (self.tab, &detail.view, &self.summary) {
            (Tab::Overview, Some(_), Some(summary)) => self.overview(detail, summary, cx),
            (Tab::Yaml, Some(view), _) => self.yaml(view, cx),
            (Tab::Events, ..) => self.events(detail, cx),
            _ => self.document_state(detail, cx),
        };
        panel(cx)
            .id("resource-detail")
            .test_support()
            .key_context(CONTEXT)
            .track_focus(&self.focus)
            .on_action(cx.listener(|pane, _: &FindInYaml, window, cx| pane.focus_find(window, cx)))
            .on_action(cx.listener(|pane, _: &SelectAllLines, _, cx| pane.select_all(cx)))
            .on_action(cx.listener(|pane, _: &CopyLines, _, cx| pane.copy_lines(cx)))
            .on_action(cx.listener(|pane, _: &Dismiss, window, cx| pane.dismiss(window, cx)))
            .size_full()
            .overflow_hidden()
            .child(self.header(detail, cx))
            .children(self.notice(detail, cx))
            .child(self.tabs(detail, cx))
            .child(div().flex_1().min_h_0().child(body))
            .children(self.feedback.clone().map(|feedback| {
                div()
                    .id("detail-feedback")
                    .test_support()
                    .role(Role::Status)
                    .aria_label(feedback.clone())
                    .px_4()
                    .py_1p5()
                    .border_t_1()
                    .border_color(p.line)
                    .text_size(px(12.))
                    .text_color(p.muted)
                    .child(feedback)
            }))
            .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::{Tone, condition_tone, line_highlights, shorten};
    use gpui_kit::{Hsla, black, white};

    #[test]
    fn conditions_read_good_or_bad_only_when_their_meaning_is_known() {
        assert_eq!(condition_tone("Ready", "True"), Tone::Good);
        assert_eq!(condition_tone("Ready", "False"), Tone::Crit);
        assert_eq!(condition_tone("MemoryPressure", "False"), Tone::Good);
        assert_eq!(condition_tone("MemoryPressure", "True"), Tone::Crit);
        assert_eq!(condition_tone("Ready", "Unknown"), Tone::Unknown);
        assert_eq!(condition_tone("SomethingCustom", "True"), Tone::Outline);
        assert_eq!(condition_tone("SomethingCustom", "Unknown"), Tone::Unknown);
    }

    #[test]
    fn highlights_split_where_a_key_and_matches_overlap() {
        let (key, mark): (Hsla, Hsla) = (black(), white());
        // "  name: web-name": the key is 2..6; matches "name" at 2..6 (current)
        // and 12..16, and "me: w" at 4..9 straddling the key's end.
        let highlights = line_highlights(Some(2..6), &[(2..6, true), (12..16, false)], key, mark);
        let ranges: Vec<_> = highlights.iter().map(|(range, _)| range.clone()).collect();
        assert_eq!(ranges, [2..6, 12..16]);
        assert_eq!(highlights[0].1.color, Some(key));
        assert_eq!(highlights[0].1.background_color, Some(mark));
        assert_eq!(highlights[1].1.color, None);
        assert_eq!(highlights[1].1.background_color, Some(mark.opacity(0.4)));

        let highlights = line_highlights(Some(2..6), &[(4..9, false)], key, mark);
        let ranges: Vec<_> = highlights.iter().map(|(range, _)| range.clone()).collect();
        assert_eq!(ranges, [2..4, 4..6, 6..9]);
        assert!(
            highlights
                .windows(2)
                .all(|pair| pair[0].0.end <= pair[1].0.start)
        );
        assert_eq!(highlights[1].1.color, Some(key));
        assert!(highlights[1].1.background_color.is_some());
        assert_eq!(highlights[2].1.color, None);
        assert!(line_highlights(None, &[], key, mark).is_empty());
    }

    #[test]
    fn long_values_are_shortened_onto_one_line() {
        assert_eq!(shorten("a\nb\n", 10), "a ↵ b");
        assert_eq!(shorten("ééééé", 3), "ééé…");
        assert_eq!(shorten("abc", 3), "abc");
    }
}

#[cfg(test)]
mod ui_tests {
    use std::cell::RefCell;
    use std::rc::Rc;
    use std::time::Duration;

    use freshkube_core::resources::{Failure, FailureKind, SecretValue, builtin};
    use gpui_kit::component::Root;
    use gpui_kit::test::TestWindowExt;
    use gpui_kit::{AnyWindowHandle, App, AppContext, Entity, Focusable, TestAppContext, px, size};
    use tokio::runtime::Runtime;

    // Not `super::*`: gpui_kit's glob would shadow the built-in `#[test]`.
    use super::{DetailEvent, DetailPane};
    use crate::resources::detail::{
        DetailTarget, DocumentRead, DocumentView, FOLLOW_INTERVAL, Reveal,
    };
    use crate::resources::model::ResourceIdentity;
    use crate::resources::screen::KubeAccess;
    use crate::resources::{example, live};

    const CONTEXT: &str = "homelab";

    type Emitted = Rc<RefCell<Vec<DetailEvent>>>;

    /// The pane on its own, active, with what it emits collected.
    fn mount(cx: &mut TestAppContext) -> (Runtime, Entity<DetailPane>, AnyWindowHandle, Emitted) {
        cx.update(|cx| {
            gpui_kit::init(cx);
            crate::theme::install(cx);
            cx.set_reduce_motion(true);
        });
        let runtime = Runtime::new().unwrap();
        let mut pane = None;
        let handle = cx.open_window(size(px(560.), px(820.)), |window, cx| {
            let view = cx.new(|cx| {
                let mut pane = DetailPane::new(runtime.handle().clone(), window, cx);
                pane.set_active(true, cx);
                pane
            });
            pane = Some(view.clone());
            Root::new(view, window, cx)
        });
        let pane = pane.unwrap();
        let emitted = Emitted::default();
        let sink = emitted.clone();
        cx.update(|cx| {
            cx.subscribe(&pane, move |_, event: &DetailEvent, _| {
                sink.borrow_mut().push(event.clone())
            })
            .detach()
        });
        cx.run_until_parked();
        (runtime, pane, handle.into(), emitted)
    }

    /// The first example object of `key` that `pick` accepts, with the
    /// version the list shows.
    fn target(key: &str, pick: impl Fn(&[String], &str) -> bool) -> (DetailTarget, String) {
        let (_, rows) = example::read(CONTEXT, key, None, live::now()).unwrap();
        let row = rows
            .iter()
            .find(|row| pick(&row.cells, &row.identity.name))
            .unwrap();
        let target = DetailTarget {
            identity: row.identity.clone(),
            kind: builtin(key).unwrap(),
        };
        (target, row.resource_version.clone())
    }

    fn crashing_pod() -> (DetailTarget, String) {
        target("pods", |cells, _| cells[2] == "CrashLoopBackOff")
    }

    fn open(pane: &Entity<DetailPane>, target: &DetailTarget, delay: Duration, cx: &mut App) {
        let target = target.clone();
        pane.update(cx, |pane, cx| {
            pane.open(target, KubeAccess::Example, "1", delay, cx)
        });
    }

    fn read(pane: &Entity<DetailPane>, cx: &App) -> DocumentRead {
        pane.read(cx).detail.as_ref().unwrap().read.clone()
    }

    fn example_view(target: &DetailTarget) -> DocumentView {
        DocumentView::new(example::document(&target.identity, live::now()).unwrap())
    }

    #[gpui_kit::test]
    fn an_object_reads_at_once_with_its_overview_yaml_and_events(cx: &mut TestAppContext) {
        let (_runtime, pane, handle, _) = mount(cx);
        let (pod, _) = crashing_pod();
        cx.update_window(handle, |_, window, cx| {
            open(&pane, &pod, Duration::ZERO, cx);
            window.render_frame(cx);
            assert_eq!(
                window.find("detail-title").label(),
                Some(pod.identity.address().as_str())
            );
            assert!(window.try_find("detail-state").is_none());
            assert_eq!(window.find("detail-tab-overview").selected(), Some(true));
            assert!(window.find("detail-overview").visible());
            assert!(window.find(("detail-condition", 0usize)).visible());

            window.click("detail-tab-yaml", cx);
            window.render_frame(cx);
            assert_eq!(
                window.find(("detail-yaml-line", 0usize)).label(),
                Some("apiVersion: v1")
            );

            // Scheduled, Pulled, Created and Started, then the newest: a
            // warning that it keeps crashing.
            window.click("detail-tab-events", cx);
            window.render_frame(cx);
            assert_eq!(window.find("detail-tab-events").label(), Some("Events 5"));
            let newest = window.find(("detail-event", 0usize));
            assert!(
                newest.label().unwrap().starts_with("BackOff: "),
                "{newest:?}"
            );
            assert!(window.try_find(("detail-event", 5usize)).is_none());
        })
        .unwrap();
    }

    #[gpui_kit::test]
    fn reads_for_another_object_or_an_older_request_are_dropped(cx: &mut TestAppContext) {
        let (_runtime, pane, handle, _) = mount(cx);
        let (pod, _) = crashing_pod();
        let (other, _) = target("pods", |cells, _| cells[2] == "Running");
        cx.update_window(handle, |_, window, cx| {
            // A keyboard pause holds the read back, so nothing has landed.
            open(&pane, &pod, Duration::from_secs(1), cx);
            window.render_frame(cx);
            assert_eq!(read(&pane, cx), DocumentRead::Loading);
            assert!(window.find("detail-loading").visible());
            assert_eq!(window.find("detail-state").label(), Some("Reading"));

            let seq = pane.read(cx).read_seq;
            pane.update(cx, |pane, cx| {
                pane.finish_read(&other.identity, seq, Ok(example_view(&other)), cx);
                pane.finish_read(&pod.identity, seq - 1, Ok(example_view(&pod)), cx);
            });
            assert_eq!(read(&pane, cx), DocumentRead::Loading);
            pane.update(cx, |pane, cx| {
                pane.finish_read(&pod.identity, seq, Ok(example_view(&pod)), cx)
            });
            assert_eq!(read(&pane, cx), DocumentRead::Loaded);
        })
        .unwrap();
    }

    #[gpui_kit::test]
    fn refusals_failures_and_stale_reads_each_say_so(cx: &mut TestAppContext) {
        let (_runtime, pane, handle, _) = mount(cx);
        let (pod, _) = crashing_pod();
        let finish = |result, cx: &mut App| {
            pane.update(cx, |pane, cx| {
                let seq = pane.read_seq;
                pane.finish_read(&pod.identity, seq, result, cx)
            })
        };
        cx.update_window(handle, |_, window, cx| {
            open(&pane, &pod, Duration::from_secs(1), cx);
            finish(
                Err(Failure::new(FailureKind::Forbidden, "pods is forbidden")),
                cx,
            );
            window.render_frame(cx);
            assert!(window.find("detail-refused").visible());
            assert_eq!(window.find("detail-state").label(), Some("Not permitted"));

            finish(Err(Failure::new(FailureKind::Unreachable, "down")), cx);
            window.render_frame(cx);
            assert!(window.find("detail-failed").visible());
            window.click("detail-retry", cx);
            window.render_frame(cx);
            assert_eq!(read(&pane, cx), DocumentRead::Loaded);
            assert!(window.find("detail-overview").visible());

            // With a document shown, a failed read keeps it and says so.
            finish(Err(Failure::new(FailureKind::Timeout, "slow")), cx);
            window.render_frame(cx);
            assert!(window.find("detail-stale").visible());
            assert!(window.find("detail-overview").visible());
            assert_eq!(window.find("detail-state").label(), Some("Stale"));
            window.click("detail-stale-retry", cx);
            window.render_frame(cx);
            assert!(window.try_find("detail-stale").is_none());
            assert!(window.try_find("detail-state").is_none());
        })
        .unwrap();
    }

    #[gpui_kit::test]
    fn new_versions_are_read_once_each_and_at_most_once_a_second(cx: &mut TestAppContext) {
        let (_runtime, pane, handle, _) = mount(cx);
        let (pod, _) = crashing_pod();
        let reads = |cx: &mut TestAppContext| pane.read_with(cx, |pane, _| pane.read_seq);
        let observe = |version: &'static str, cx: &mut TestAppContext| {
            pane.update(cx, |pane, cx| pane.observed(version, cx))
        };
        cx.update_window(handle, |_, _, cx| open(&pane, &pod, Duration::ZERO, cx))
            .unwrap();
        let first = reads(cx);
        // The example always reads as version 1; the list has moved on.
        observe("2", cx);
        assert_eq!(reads(cx), first, "read again within a second");
        cx.executor().advance_clock(FOLLOW_INTERVAL);
        cx.run_until_parked();
        assert_eq!(reads(cx), first + 1);
        // Version 2 was asked for; the older answer doesn't ask again.
        observe("2", cx);
        cx.executor().advance_clock(FOLLOW_INTERVAL * 3);
        cx.run_until_parked();
        assert_eq!(reads(cx), first + 1);
        observe("3", cx);
        cx.run_until_parked();
        assert_eq!(reads(cx), first + 2, "a second has passed, so at once");
    }

    #[gpui_kit::test]
    fn a_hidden_pane_reads_nothing_and_hides_revealed_values(cx: &mut TestAppContext) {
        let (_runtime, pane, handle, _) = mount(cx);
        let (secret, _) = target("secrets", |_, name| name == "ledger-database");
        cx.update_window(handle, |_, window, cx| {
            open(&pane, &secret, Duration::ZERO, cx);
            pane.update(cx, |pane, cx| pane.reveal("username".into(), cx));
            assert_eq!(pane.read(cx).detail.as_ref().unwrap().reveals.len(), 1);
            pane.update(cx, |pane, cx| pane.set_active(false, cx));
            assert!(pane.read(cx).detail.as_ref().unwrap().reveals.is_empty());

            let (pod, _) = crashing_pod();
            open(&pane, &pod, Duration::ZERO, cx);
            window.render_frame(cx);
            assert_eq!(read(&pane, cx), DocumentRead::Loading);
            pane.update(cx, |pane, cx| pane.observed("9", cx));
            assert_eq!(read(&pane, cx), DocumentRead::Loading);
            pane.update(cx, |pane, cx| pane.set_active(true, cx));
            assert_eq!(read(&pane, cx), DocumentRead::Loaded);
        })
        .unwrap();
    }

    #[gpui_kit::test]
    fn a_deleted_object_stays_deleted_and_offers_its_successor(cx: &mut TestAppContext) {
        let (_runtime, pane, handle, emitted) = mount(cx);
        let (pod, _) = crashing_pod();
        let successor = ResourceIdentity {
            uid: "another-incarnation".into(),
            ..pod.identity.clone()
        };
        cx.update_window(handle, |_, window, cx| {
            open(&pane, &pod, Duration::ZERO, cx);
            pane.update(cx, |pane, cx| pane.gone(None, cx));
            window.render_frame(cx);
            assert!(window.find("detail-deleted").visible());
            assert_eq!(window.find("detail-state").label(), Some("Deleted"));
            // The last read still shows, and nothing read later undoes it.
            assert!(window.find("detail-overview").visible());
            pane.update(cx, |pane, cx| pane.refresh(cx));
            assert_eq!(read(&pane, cx), DocumentRead::Deleted);

            pane.update(cx, |pane, cx| pane.gone(Some(successor.clone()), cx));
            window.render_frame(cx);
            window.click("detail-open-recreated", cx);
        })
        .unwrap();
        assert_eq!(*emitted.borrow(), [DetailEvent::Open(successor)]);
    }

    #[gpui_kit::test]
    fn search_marks_matches_steps_through_them_and_escape_backs_out(cx: &mut TestAppContext) {
        let (_runtime, pane, handle, emitted) = mount(cx);
        let (pod, _) = crashing_pod();
        let step = |cx: &mut TestAppContext, act: &dyn Fn(&mut gpui_kit::Window, &mut App)| {
            cx.update_window(handle, |_, window, cx| {
                window.render_frame(cx);
                act(window, cx);
            })
            .unwrap();
            cx.run_until_parked();
        };
        let count = |cx: &mut TestAppContext| {
            cx.update_window(handle, |_, window, cx| {
                window.render_frame(cx);
                window
                    .try_find("detail-find-count")
                    .and_then(|count| count.label().map(str::to_owned))
            })
            .unwrap()
        };
        step(cx, &|window, cx| {
            open(&pane, &pod, Duration::ZERO, cx);
            window.render_frame(cx);
            window.click("detail-overview", cx);
        });
        step(cx, &|window, cx| window.press("cmd-f", cx));
        step(cx, &|window, cx| {
            assert_eq!(window.find("detail-tab-yaml").selected(), Some(true));
            window.input("CONTAINER", cx);
        });
        let found = pane.read_with(cx, |pane, _| pane.matches.len());
        assert!(found > 2, "{found}");
        assert_eq!(count(cx), Some(format!("1 of {found}")));
        step(cx, &|window, cx| window.press("enter", cx));
        assert_eq!(count(cx), Some(format!("2 of {found}")));
        step(cx, &|window, cx| window.press("shift-enter", cx));
        step(cx, &|window, cx| window.press("shift-enter", cx));
        assert_eq!(count(cx), Some(format!("{found} of {found}")));
        step(cx, &|window, cx| window.click("detail-find-next", cx));
        assert_eq!(count(cx), Some(format!("1 of {found}")));

        // Escape clears the search, then leaves it, then closes the pane.
        step(cx, &|window, cx| {
            let focus = pane.read(cx).find.read(cx).focus_handle(cx);
            window.focus(&focus, cx);
        });
        step(cx, &|window, cx| window.press("escape", cx));
        assert_eq!(count(cx), None);
        assert!(pane.read_with(cx, |pane, _| pane.matches.is_empty()));
        step(cx, &|window, cx| window.press("escape", cx));
        assert!(emitted.borrow().is_empty());
        step(cx, &|window, cx| window.press("escape", cx));
        assert_eq!(*emitted.borrow(), [DetailEvent::Closed]);
    }

    #[gpui_kit::test]
    fn lines_select_and_copy_and_copy_yaml_takes_the_whole_document(cx: &mut TestAppContext) {
        let (_runtime, pane, handle, _) = mount(cx);
        let (deployment, _) = target("deployments.apps", |_, _| true);
        let clipboard = |cx: &mut TestAppContext| {
            cx.read_from_clipboard()
                .and_then(|item| item.text())
                .unwrap_or_default()
        };
        cx.update_window(handle, |_, window, cx| {
            open(&pane, &deployment, Duration::ZERO, cx);
            window.render_frame(cx);
            window.click("detail-tab-yaml", cx);
            window.render_frame(cx);
            window.click(("detail-yaml-line", 1usize), cx);
            pane.update(cx, |pane, cx| pane.select_line(3, true, cx));
            window.render_frame(cx);
            assert_eq!(
                window.find(("detail-yaml-line", 2usize)).selected(),
                Some(true)
            );
            assert_eq!(
                window.find(("detail-yaml-line", 0usize)).selected(),
                Some(false)
            );
            assert_eq!(
                window.find("detail-copy-lines").label(),
                Some("Copy 3 lines")
            );
            window.press("cmd-c", cx);
        })
        .unwrap();
        let yaml = pane.read_with(cx, |pane, _| pane.view().unwrap().document.yaml.clone());
        let lines: Vec<&str> = yaml.lines().collect();
        assert_eq!(clipboard(cx), lines[1..4].join("\n"));

        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            assert_eq!(
                window.find("detail-feedback").label(),
                Some("Copied 3 lines")
            );
            window.press("cmd-a", cx);
            window.render_frame(cx);
            assert_eq!(
                window.find("detail-copy-lines").label(),
                Some(format!("Copy {} lines", lines.len()).as_str())
            );
            window.click("detail-copy-yaml", cx);
        })
        .unwrap();
        assert_eq!(clipboard(cx), yaml);
    }

    #[gpui_kit::test]
    fn secret_values_stay_hidden_until_one_is_revealed(cx: &mut TestAppContext) {
        let (_runtime, pane, handle, _) = mount(cx);
        let (secret, _) = target("secrets", |_, name| name == "ledger-database");
        let reveal = |key: &str, cx: &App| {
            pane.read(cx)
                .detail
                .as_ref()
                .unwrap()
                .reveals
                .get(key)
                .cloned()
        };
        cx.update_window(handle, |_, window, cx| {
            open(&pane, &secret, Duration::ZERO, cx);
            window.render_frame(cx);
            // Keys and sizes show; no value does, in the overview or YAML.
            for ix in 0..3usize {
                assert!(window.find(("detail-secret-key", ix)).visible());
                assert!(window.try_find(("detail-secret-value", ix)).is_none());
            }
            let yaml = pane.read(cx).view().unwrap().document.yaml.clone();
            assert!(!yaml.contains("example-only-password"));
            assert!(!yaml.contains("ZXhhbXBsZS1vbmx5LXBhc3N3b3Jk"));

            window.click(("detail-secret-reveal", 1usize), cx);
            window.render_frame(cx);
            assert_eq!(
                reveal("password", cx),
                Some(Reveal::Shown(SecretValue::Text(
                    "example-only-password".into()
                )))
            );
            assert!(window.find(("detail-secret-value", 1usize)).visible());
            assert!(window.try_find(("detail-secret-value", 0usize)).is_none());
            window.click(("detail-secret-copy", 1usize), cx);
        })
        .unwrap();
        assert_eq!(
            cx.read_from_clipboard().and_then(|item| item.text()),
            Some("example-only-password".into())
        );

        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            // Copy YAML still copies the hidden form.
            window.click("detail-copy-yaml", cx);
            window.click(("detail-secret-reveal", 2usize), cx);
            window.render_frame(cx);
            assert_eq!(
                reveal("keystore.p12", cx),
                Some(Reveal::Shown(SecretValue::Binary(6)))
            );
            assert!(window.try_find(("detail-secret-copy", 2usize)).is_none());

            window.click(("detail-secret-hide", 1usize), cx);
            window.render_frame(cx);
            assert_eq!(reveal("password", cx), None);
            assert!(window.try_find(("detail-secret-value", 1usize)).is_none());

            // A new version of the Secret hides every value again.
            let mut changed = example::document(&secret.identity, live::now()).unwrap();
            changed.resource_version = "2".into();
            pane.update(cx, |pane, cx| {
                let seq = pane.read_seq;
                pane.finish_read(&secret.identity, seq, Ok(DocumentView::new(changed)), cx)
            });
            assert_eq!(reveal("keystore.p12", cx), None);
        })
        .unwrap();
        let copied = cx
            .read_from_clipboard()
            .and_then(|item| item.text())
            .unwrap_or_default();
        assert!(copied.contains("ledger-database"));
        assert!(!copied.contains("example-only-password"));
    }
}
