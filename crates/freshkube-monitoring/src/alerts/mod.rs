//! The Alerts, Rules and Silences pages (#884): what Alertmanager, vmalert
//! and PrometheusRule objects say, as table pages in the Monitoring area,
//! read-only. Core's `monitoring::alerts` finds the sources and reads them;
//! `display` derives what the pages show, `table` and `view` draw it.
//!
//! One entity, [`AlertsPage`], serves the three, told apart by [`Which`].
//! The Monitoring page owns them and tells each whether it shows. A page
//! reads only while shown: on showing once what it has is older than
//! [`FRESH_FOR`], on Refresh, and never while hidden, which drops the read
//! in flight. Another source or scope drops what was read, so one cluster's
//! alerts never show as another's.
//!
//! What was not read is never "no alerts": a source that wasn't found, was
//! refused, couldn't be read or lacks the central store's cluster label is
//! a state that says so, and a refresh that failed over an earlier read
//! keeps it, marked as the last read.
mod display;
mod table;
mod view;

#[cfg(test)]
mod tests;

use std::time::Duration;

use chrono::{DateTime, Utc};
use freshkube_core::cluster_source::{ClusterAccess, ClusterSource};
use freshkube_core::monitoring::alerts::{
    self as alerts, AlertsData, Remembered, RulesData, Scope, SilencesData, Unknown, example::Shape,
};
use freshkube_core::monitoring::{ErrorKind, QueryError, cluster_client};
use freshkube_reads::{Loader, OnShow};
use freshkube_ui::inspector::InspectorSplit;
use freshkube_ui::status::{Part, Segment};
use freshkube_ui::table::{self as kit, TableState};
use freshkube_ui::ui::Tone;
use gpui_kit::assets::IconName;
use gpui_kit::component::input::{InputEvent, InputState};
use gpui_kit::prelude::*;
use gpui_kit::*;
use tokio::runtime::Handle;

use display::{Body, Display, Filter};

/// A read this recent is shown again without reading.
const FRESH_FOR: Duration = Duration::from_secs(30);
const ON_SHOW: OnShow = OnShow::ReadIfOlder(FRESH_FOR);
/// How long one read may take in all.
const DEADLINE: Duration = Duration::from_secs(90);

/// Which of the three pages.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Which {
    Alerts,
    Rules,
    Silences,
}

impl Which {
    pub const ALL: [Self; 3] = [Self::Alerts, Self::Rules, Self::Silences];

    pub fn title(self) -> &'static str {
        match self {
            Self::Alerts => "Alerts",
            Self::Rules => "Rules",
            Self::Silences => "Silences",
        }
    }

    /// The page's id prefix: `alerts-title`, `alerts-row-<key>`, …
    pub fn slug(self) -> &'static str {
        match self {
            Self::Alerts => "alerts",
            Self::Rules => "rules",
            Self::Silences => "silences",
        }
    }

    pub fn icon(self) -> IconName {
        match self {
            Self::Alerts => IconName::BellRing,
            Self::Rules => IconName::ListChecks,
            Self::Silences => IconName::BellOff,
        }
    }

    pub fn tooltip(self) -> &'static str {
        match self {
            Self::Alerts => "Firing and pending alerts from Alertmanager and vmalert",
            Self::Rules => "Rule groups and how each rule evaluates",
            Self::Silences => "Active, pending and expired silences (read-only)",
        }
    }

    /// The page's key context, drawn in every state.
    pub fn context(self) -> &'static str {
        match self {
            Self::Alerts => "Alerts",
            Self::Rules => "Rules",
            Self::Silences => "Silences",
        }
    }
}

gpui_kit::actions!(
    alerts,
    [
        /// Selects the next row of an alerts, rules or silences page.
        NextAlert,
        /// Selects the previous row.
        PreviousAlert,
        /// Clears the selection, which closes the Inspector.
        ClearAlert
    ]
);

/// The keys of the three pages, each in its own context.
pub fn key_bindings() -> Vec<KeyBinding> {
    Which::ALL
        .into_iter()
        .flat_map(|which| {
            let context = Some(which.context());
            [
                KeyBinding::new("down", NextAlert, context),
                KeyBinding::new("up", PreviousAlert, context),
                KeyBinding::new("escape", ClearAlert, context),
            ]
        })
        .collect()
}

/// What the page asks of the shell: to open an object an alert's label
/// names.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AlertObject {
    /// The kubectl key of the kind: `pods`, `deployments.apps`, …
    pub kind: &'static str,
    /// Empty for a cluster-scoped kind.
    pub namespace: String,
    pub name: String,
}

/// The object a label names, when it names one: `pod`, `namespace`, `node`
/// and the workload kinds. The label for a namespaced kind needs the alert's
/// own `namespace`.
pub fn object_of(
    label: &str,
    value: &str,
    labels: &[(SharedString, SharedString)],
) -> Option<AlertObject> {
    let namespace = || {
        labels
            .iter()
            .find(|(name, _)| name == "namespace")
            .map(|(_, value)| value.to_string())
    };
    let (kind, scoped) = match label {
        "namespace" => ("namespaces", false),
        "node" => ("nodes", false),
        "pod" => ("pods", true),
        "deployment" => ("deployments.apps", true),
        "statefulset" => ("statefulsets.apps", true),
        "daemonset" => ("daemonsets.apps", true),
        "job_name" => ("jobs.batch", true),
        "service" => ("services", true),
        _ => return None,
    };
    if value.is_empty() {
        return None;
    }
    Some(AlertObject {
        kind,
        namespace: if scoped { namespace()? } else { String::new() },
        name: value.to_owned(),
    })
}

/// What a read gives, by page.
#[derive(Clone, Debug)]
enum Data {
    Alerts(AlertsData),
    Rules(RulesData),
    Silences(SilencesData),
}

/// An answer: the data, or that nothing is known and why. A read that
/// couldn't be read at all is an error, which keeps the answer before it.
#[derive(Clone, Debug)]
enum Answer {
    Read(Data),
    Unknown(Unknown),
}

/// Whose data this is: the source and the inputs' revision.
#[derive(Clone, PartialEq, Eq)]
struct ReadIdentity {
    source: Option<String>,
    revision: u64,
}

type Read = Loader<(), ReadIdentity>;
type Request = freshkube_core::snapshot::Request<ReadIdentity>;

/// What the page tells the shell.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AlertsEvent {
    /// A label's object, to open in Resources.
    Open(AlertObject),
}

pub struct AlertsPage {
    which: Which,
    runtime: Handle,
    source: Option<ClusterSource>,
    scope: Scope,
    /// The cluster that hosts a central store's Service, when it isn't the
    /// open one.
    host: Option<ClusterAccess>,
    /// The Services that last answered for this context, tried first.
    remembered: Remembered,
    /// Counts what changed the inputs, so an answer for old ones is dropped.
    revision: u64,
    visible: bool,
    /// What example data shows (`FRESHKUBE_ALERTS`).
    shape: Shape,
    loader: Read,
    /// The last good answer for this source, which a failed refresh keeps.
    answer: Option<Answer>,
    /// Why the last read failed, over `answer`, or alone.
    failure: Option<String>,
    display: Display,
    filter: Filter,
    input: Option<Entity<InputState>>,
    _input: Option<Subscription>,
    columns: Vec<table::Column>,
    width: f32,
    table: TableState,
    loading: kit::LoadingRows,
    loading_motion: Entity<kit::LoadingMotion>,
    focus: FocusHandle,
    split: InspectorSplit,
    page_scroll: ScrollHandle,
    selected: Option<SharedString>,
    inspect: bool,
    /// `3 active alerts · 1 muted`, in the status bar.
    pub(crate) status: Segment,
    /// The time ages count from. Tests fix it.
    now: fn() -> DateTime<Utc>,
}

impl EventEmitter<AlertsEvent> for AlertsPage {}

impl AlertsPage {
    pub fn new(which: Which, runtime: Handle, cx: &mut Context<Self>) -> Self {
        let prefix = which.slug();
        let loading = kit::LoadingRows::new(prefix);
        let loading_motion = cx.new(|_| loading.motion(kit::Look::Pulse));
        let (columns, width) = table::columns(which, false);
        // The page starts hidden, and so does its read.
        let mut loader = Loader::default();
        loader.set_visible(false, ON_SHOW, cx);
        Self {
            which,
            runtime,
            source: None,
            scope: Scope::Cluster,
            host: None,
            remembered: Remembered::default(),
            revision: 0,
            visible: false,
            shape: Shape::default(),
            loader,
            answer: None,
            failure: None,
            display: Display::loading(),
            filter: Filter::default(),
            input: None,
            _input: None,
            columns,
            width,
            table: TableState::new(prefix),
            loading,
            loading_motion,
            focus: cx.focus_handle(),
            split: InspectorSplit::new(prefix, cx),
            page_scroll: ScrollHandle::new(),
            selected: None,
            inspect: false,
            status: Segment::default(),
            now: Utc::now,
        }
    }

    pub fn which(&self) -> Which {
        self.which
    }

    /// What example data shows: one of `FRESHKUBE_ALERTS`'s shapes.
    pub fn set_shape(&mut self, shape: Shape, cx: &mut Context<Self>) {
        if self.shape == shape {
            return;
        }
        self.shape = shape;
        if self.source.as_ref().is_some_and(|s| s.access.is_example()) {
            self.changed_inputs(cx);
        }
    }

    /// The cluster the page reads. Another connection drops what was read,
    /// what is in flight and the selection; the page reads again if shown.
    pub fn set_source(&mut self, source: Option<ClusterSource>, cx: &mut Context<Self>) {
        let same = self.source.as_ref().map(|s| &s.id) == source.as_ref().map(|s| &s.id);
        self.source = source;
        if !same {
            self.changed_inputs(cx);
        }
    }

    /// How the setup file says the open entry's metrics are found, which is
    /// where alerts are looked for, and the cluster that hosts a central
    /// store's Service.
    pub fn set_scope(&mut self, scope: Scope, host: Option<ClusterAccess>, cx: &mut Context<Self>) {
        let same = self.scope == scope;
        self.scope = scope;
        self.host = host;
        if !same {
            self.changed_inputs(cx);
        }
    }

    /// The Services that last answered for the open context. They only say
    /// where to look first, so what was read stays.
    pub fn set_remembered(&mut self, remembered: Remembered, _: &mut Context<Self>) {
        self.remembered = remembered;
    }

    /// Something the answer depends on changed: what was read is another
    /// source's, and what is in flight is dropped. Reads again if shown.
    fn changed_inputs(&mut self, cx: &mut Context<Self>) {
        self.revision += 1;
        self.loader.reset();
        self.answer = None;
        self.failure = None;
        self.selected = None;
        self.inspect = false;
        self.show_read(cx);
        if self.visible {
            self.read(cx);
        }
    }

    /// Shown, the page reads unless what it has is fresh; hidden, it drops
    /// the read in flight.
    pub fn set_visible(&mut self, visible: bool, cx: &mut Context<Self>) {
        if visible == self.visible {
            return;
        }
        self.visible = visible;
        let reads_again = self.loader.set_visible(visible, ON_SHOW, cx);
        if !visible {
            cx.notify();
            return;
        }
        if reads_again || self.answer.is_none() {
            self.read(cx);
        }
    }

    pub fn is_visible(&self) -> bool {
        self.visible
    }

    /// Refresh: reads again while shown.
    pub fn refresh(&mut self, cx: &mut Context<Self>) {
        if self.visible {
            self.read(cx);
        }
    }

    pub fn focus_handle(&self) -> &FocusHandle {
        &self.focus
    }

    /// Puts the keyboard on the page.
    pub fn focus(&self, window: &mut Window, cx: &mut App) {
        window.focus(&self.focus, cx);
    }

    fn identity(&self) -> ReadIdentity {
        ReadIdentity {
            source: self.source.as_ref().map(|source| source.id.clone()),
            revision: self.revision,
        }
    }

    fn read(&mut self, cx: &mut Context<Self>) {
        let Some(source) = self.source.clone() else {
            return;
        };
        let request = self.loader.start(self.identity());
        let which = self.which;
        let example = source.access.is_example();
        if let ClusterAccess::Example(hold) = &source.access
            && hold.monitoring
        {
            // Held, it stays in flight, so the page keeps its loading rows.
            cx.notify();
            return;
        }
        let (shape, now) = (self.shape, (self.now)());
        let (scope, host, access) = (self.scope.clone(), self.host.clone(), source.access.clone());
        let remembered = self.remembered.clone();
        let work = async move {
            if example {
                return Ok(example_answer(which, shape, now));
            }
            live(which, &access, host.as_ref(), &scope, &remembered).await
        };
        let captured = request.clone();
        let (job, task) = crate::dispatch(
            example,
            &self.runtime,
            DEADLINE,
            work,
            cx,
            move |this, result, cx| this.land(&captured, result, cx),
        );
        self.loader.run(job, task);
        cx.notify();
    }

    /// An answer, kept only while its request is the one in flight.
    fn land(
        &mut self,
        request: &Request,
        result: Result<Answer, QueryError>,
        cx: &mut Context<Self>,
    ) {
        let completion = result
            .as_ref()
            .map(|_| ())
            .map_err(|error| error.message.clone());
        if !self.loader.land(request, completion, cx) {
            return;
        }
        match result {
            Ok(answer) => {
                self.answer = Some(answer);
                // The example that shows a stale read: its refresh failed.
                self.failure = (self.shape == Shape::Stale
                    && self.source.as_ref().is_some_and(|s| s.access.is_example()))
                .then(|| "Alertmanager didn't answer within 20 s".to_owned());
            }
            Err(error) => self.failure = Some(error.message),
        }
        self.show_read(cx);
    }

    /// Derives the display from what was read, when the answer, the filter
    /// or the toggle change. A selection that went is no longer selected.
    fn show_read(&mut self, cx: &mut Context<Self>) {
        let unread;
        let answer = match (&self.answer, &self.failure) {
            (Some(answer), _) => Some(answer),
            (None, Some(why)) => {
                unread = Answer::Unknown(Unknown::Unread(why.clone()));
                Some(&unread)
            }
            (None, None) => None,
        };
        let stale = self.answer.as_ref().and(self.failure.as_deref());
        let central = match &self.scope {
            Scope::Central { matcher, .. } => Some(matcher.value().to_owned()),
            _ => None,
        };
        self.display = display::derive(
            self.which,
            answer,
            stale,
            &self.filter,
            central.as_deref(),
            (self.now)(),
        );
        let (columns, width) = table::columns(self.which, self.display.central);
        self.columns = columns;
        self.width = width;
        self.status = self.segment();
        if self
            .selected
            .as_ref()
            .is_some_and(|key| !self.display.rows.iter().any(|row| &row.key == key))
        {
            self.selected = None;
            self.inspect = false;
        }
        cx.notify();
    }

    fn segment(&self) -> Segment {
        let context = self.source.as_ref().map(|source| source.context.clone());
        if let Body::State(state) = &self.display.body {
            let part = Part::new(state.title.to_string()).tone(match state.icon {
                display::StateIcon::Quiet | display::StateIcon::Filtered => Tone::Good,
                display::StateIcon::Refused | display::StateIcon::Missing => Tone::Unknown,
                display::StateIcon::Broken => Tone::Warn,
            });
            return match state.icon {
                display::StateIcon::Quiet | display::StateIcon::Filtered => {
                    self.parts_segment(context)
                }
                _ => Segment::new(context, vec![part]),
            };
        }
        self.parts_segment(context)
    }

    fn parts_segment(&self, context: Option<String>) -> Segment {
        if self.display.status.is_empty() {
            return Segment::default();
        }
        let parts: Vec<Part> = self
            .display
            .status
            .iter()
            .map(|(text, tone)| {
                let part = Part::new(text.clone());
                match tone {
                    Some(tone) => part.tone(*tone),
                    None => part,
                }
            })
            .collect();
        Segment::new(context, parts)
    }

    /// The motion over the loading rows, which the shell mounts beside the
    /// page.
    pub fn loading_motion(&self) -> Option<Entity<kit::LoadingMotion>> {
        matches!(self.display.body, Body::Loading)
            .then(|| self.loading_motion.clone())
            .filter(|_| self.loader.is_loading() || self.source.is_some())
    }

    /// Selects a row, showing it in the Inspector.
    pub fn select(&mut self, key: &str, window: &Window, cx: &mut Context<Self>) {
        let Some(row) = self.display.rows.iter().find(|row| row.key == key) else {
            return;
        };
        self.selected = Some(row.key.clone());
        self.inspect = true;
        kit::reveal(self, ScrollStrategy::Nearest);
        if freshkube_ui::page::page_width(window) < freshkube_ui::inspector::SPLIT_WIDTH {
            kit::reveal_when_settled(
                cx.entity().downgrade(),
                |page| page.selected.is_some(),
                window,
            );
        }
        cx.notify();
    }

    fn step(&mut self, delta: isize, window: &Window, cx: &mut Context<Self>) {
        if let Some(key) = kit::step(self, delta, cx) {
            self.select(&key, window, cx);
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

    /// The filter's text changed.
    fn refilter(&mut self, cx: &mut Context<Self>) {
        let query = self
            .input
            .as_ref()
            .map(|input| input.read(cx).value().to_lowercase())
            .unwrap_or_default();
        if query == self.filter.query {
            return;
        }
        self.filter.query = query;
        self.show_read(cx);
    }

    /// Whether silenced and inhibited alerts show.
    fn toggle_muted(&mut self, cx: &mut Context<Self>) {
        self.filter.muted = !self.filter.muted;
        self.show_read(cx);
    }

    /// The filter's input, made when the page first draws it: it needs a
    /// window.
    fn input(&mut self, window: &mut Window, cx: &mut Context<Self>) -> Entity<InputState> {
        if let Some(input) = &self.input {
            return input.clone();
        }
        let placeholder = match self.which {
            Which::Alerts => "Filter alerts…",
            Which::Rules => "Filter rules…",
            Which::Silences => "Filter silences…",
        };
        let input = cx.new(|cx| InputState::new(window, cx).placeholder(placeholder));
        self._input = Some(cx.subscribe(&input, |this, _, event, cx| {
            if matches!(event, InputEvent::Change) {
                this.refilter(cx);
            }
        }));
        self.input = Some(input.clone());
        input
    }

    /// Asks the shell to open the object a label names.
    fn open_label(&mut self, label: &str, value: &str, cx: &mut Context<Self>) {
        let Some(row) = self
            .selected
            .as_ref()
            .and_then(|key| self.display.rows.iter().find(|row| &row.key == key))
        else {
            return;
        };
        if let Some(object) = object_of(label, value, &row.detail.labels) {
            cx.emit(AlertsEvent::Open(object));
        }
    }
}

/// The example answer for a page under a shape.
fn example_answer(which: Which, shape: Shape, now: DateTime<Utc>) -> Answer {
    use alerts::example;
    let answer = match which {
        Which::Alerts => example::alerts_page(shape, now).map(Data::Alerts),
        Which::Rules => example::rules_page(shape, now).map(Data::Rules),
        Which::Silences => example::silences_page(shape, now).map(Data::Silences),
    };
    match answer {
        Ok(data) => Answer::Read(data),
        Err(unknown) => Answer::Unknown(unknown),
    }
}

/// A live read of one page. A source that couldn't be read at all is an
/// error, which keeps the last answer as stale; one that answered that
/// nothing is known is an answer.
async fn live(
    which: Which,
    access: &ClusterAccess,
    host: Option<&ClusterAccess>,
    scope: &Scope,
    remembered: &Remembered,
) -> Result<Answer, QueryError> {
    let unread = |why: String| QueryError::new(ErrorKind::Unavailable, why);
    let connected = alerts::connect(access, host, scope, remembered).await;
    let found = match (&connected, which) {
        (Ok(found), _) => found.clone(),
        (Err(Unknown::Unread(why)), _) => return Err(unread(why.clone())),
        // PrometheusRule objects are listed whatever was found.
        (Err(Unknown::NotFound(_) | Unknown::RulesOnly), Which::Rules) => alerts::Found {
            sources: alerts::Sources::default(),
            others: Vec::new(),
            tried: Vec::new(),
        },
        (Err(Unknown::RulesOnly), Which::Silences) => {
            return Ok(Answer::Unknown(Unknown::NotFound(String::new())));
        }
        (Err(unknown), _) => return Ok(Answer::Unknown(unknown.clone())),
    };
    let data = match which {
        Which::Alerts => alerts::alerts_page(&found.sources, &found.others, scope)
            .await
            .map(Data::Alerts),
        Which::Silences => alerts::silences_page(&found.sources, scope)
            .await
            .map(Data::Silences),
        Which::Rules => {
            let own = cluster_client(access).await.ok();
            alerts::rules_page(&found.sources, scope, own.as_ref())
                .await
                .map(Data::Rules)
        }
    };
    match data {
        Ok(data) => Ok(Answer::Read(data)),
        Err(Unknown::Unread(why)) => Err(unread(why)),
        Err(unknown) => Ok(Answer::Unknown(unknown)),
    }
}

#[cfg(any(test, feature = "testing"))]
impl AlertsPage {
    /// The lines in the order the table shows them: `# group`, then rows.
    pub fn lines_text(&self) -> Vec<String> {
        self.display
            .lines
            .iter()
            .map(|entry| match entry {
                display::Entry::Group(group) => format!("# {}", self.display.groups[*group].label),
                display::Entry::Row(row) => self.display.rows[*row].cells[0].text.to_string(),
            })
            .collect()
    }

    pub fn selected(&self) -> Option<&SharedString> {
        self.selected.as_ref()
    }

    pub fn is_reading(&self) -> bool {
        self.loader.is_loading()
    }

    pub fn display(&self) -> &Display {
        &self.display
    }

    pub fn set_now(&mut self, now: fn() -> DateTime<Utc>) {
        self.now = now;
    }
}
