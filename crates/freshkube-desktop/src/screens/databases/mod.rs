//! Read-only operator inventory. The page owns reads; core owns interpretation.
#[cfg(test)]
mod coroot_tests;
mod display;
mod table;
#[cfg(test)]
mod tests;
mod view;
#[cfg(test)]
mod wire_tests;
use super::{PageEventFn, PageSource, ScreenEvent, ScreenPanel, ScreenSource, TableLoading};
use crate::{
    observability::OutcomeReader,
    resources::{ClusterSource, ResourceLink},
    ui::{self, dp},
};
use chrono::Utc;
use display::{Display, Mode};
use freshkube_core::databases::{
    self, Inventory,
    coroot::{Absence, CorootState, Evidence, Wanted},
};
use freshkube_nav::ObservabilityLink;
use freshkube_reads::{Loader, OnShow};
use freshkube_ui::{
    inspector::InspectorSplit,
    status::{Part, Segment},
    table::TableState,
};
use gpui_kit::component::{
    Disableable, Sizable,
    input::{InputEvent, InputState},
};
use gpui_kit::prelude::*;
use gpui_kit::*;
use std::time::Duration;
use tokio::runtime::Handle;
const CONTEXT: &str = "Databases";
const NAMESPACE_PLACEHOLDER: &str = "Namespace · all";
const PREFIX: &str = "databases";
const ON_SHOW: OnShow = OnShow::ReadIfOlder(Duration::from_secs(30));
actions!(
    databases,
    [
        Next,
        Previous,
        First,
        Last,
        Close,
        OpenResource,
        OpenObservability,
        Find
    ]
);

/// What the shell hands the page of Coroot: Observability's connection, and
/// the open entry's link to a Coroot project and cluster, or why it has none.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct CorootInput {
    pub reader: Option<OutcomeReader>,
    /// The workspace entry the window is on, which links carry.
    pub entry: String,
    pub link: Result<freshkube_core::coroot::Link, Absence>,
}

pub(crate) struct DatabasesScreen {
    runtime: Handle,
    source: Option<ClusterSource>,
    visible: bool,
    loader: Loader<Inventory, String>,
    /// What Coroot says of the inventory's databases; read after it.
    evidence: Loader<Evidence, String>,
    coroot: Option<CorootInput>,
    _connection: Option<Subscription>,
    _release: Option<Subscription>,
    selected: Option<SharedString>,
    focus: FocusHandle,
    namespace: Option<String>,
    namespace_input: Entity<InputState>,
    filter_input: Entity<InputState>,
    filter_text: String,
    _input: Subscription,
    _filter: Subscription,
    mode: Mode,
    display: Display,
    /// The data revisions and view the display was prepared from.
    prepared: (u64, u64, Option<Mode>),
    table: TableState,
    split: InspectorSplit,
    loading: TableLoading,
    shape: String,
}
impl EventEmitter<ScreenEvent> for DatabasesScreen {}
impl EventEmitter<ResourceLink> for DatabasesScreen {}
impl EventEmitter<ObservabilityLink> for DatabasesScreen {}
impl DatabasesScreen {
    pub(crate) fn new(runtime: Handle, window: &mut Window, cx: &mut Context<Self>) -> Self {
        cx.bind_keys([
            KeyBinding::new("down", Next, Some(CONTEXT)),
            KeyBinding::new("up", Previous, Some(CONTEXT)),
            KeyBinding::new("home", First, Some(CONTEXT)),
            KeyBinding::new("end", Last, Some(CONTEXT)),
            KeyBinding::new("escape", Close, Some(CONTEXT)),
            KeyBinding::new("o", OpenResource, Some(CONTEXT)),
            KeyBinding::new("shift-o", OpenObservability, Some(CONTEXT)),
            KeyBinding::new("/", Find, Some(CONTEXT)),
        ]);
        let namespace_input =
            cx.new(|cx| InputState::new(window, cx).placeholder(NAMESPACE_PLACEHOLDER));
        let input = cx.subscribe_in(&namespace_input, window, |this, _, event, window, cx| {
            if matches!(event, InputEvent::PressEnter { .. }) {
                let value = this.namespace_input.read(cx).value().trim().to_owned();
                let namespace = (!value.is_empty()).then_some(value);
                if namespace != this.namespace {
                    this.namespace = namespace;
                    this.loader.reset();
                    this.evidence.reset();
                    this.selected = None;
                    this.read(cx);
                }
                window.focus(&this.focus, cx);
            }
        });
        let filter_input = cx.new(|cx| InputState::new(window, cx).placeholder("Filter"));
        let filter = cx.subscribe_in(&filter_input, window, |this, _, event, _, cx| {
            if matches!(event, InputEvent::Change) {
                let text = this.filter_input.read(cx).value().to_string();
                this.display.set_filter(&text);
                this.filter_text = text;
                cx.notify();
            }
        });
        Self {
            runtime,
            source: None,
            visible: false,
            loader: Loader::default(),
            evidence: Loader::default(),
            coroot: None,
            _connection: None,
            _release: None,
            selected: None,
            focus: cx.focus_handle(),
            namespace: None,
            namespace_input,
            filter_input,
            filter_text: String::new(),
            _input: input,
            _filter: filter,
            mode: Mode::Clusters,
            display: Display::default(),
            prepared: (u64::MAX, u64::MAX, None),
            table: TableState::new(PREFIX),
            split: InspectorSplit::new(PREFIX, cx),
            loading: TableLoading::new(PREFIX, cx),
            shape: String::new(),
        }
    }
    pub(crate) fn with_example(mut self, shape: String) -> Self {
        self.shape = shape;
        self
    }
    fn identity(&self) -> Option<String> {
        Some(format!("{}/{:?}", self.source.as_ref()?.id, self.namespace))
    }
    fn read(&mut self, cx: &mut Context<Self>) {
        if !self.visible || self.loader.is_loading() {
            return;
        }
        let Some(source) = self.source.clone() else {
            return;
        };
        let id = self.identity().unwrap();
        if self
            .namespace
            .as_deref()
            .is_some_and(|namespace| !databases::valid_namespace(namespace))
        {
            self.loader.resolve(
                id,
                Err("Namespace must be a Kubernetes DNS label, or blank for all namespaces".into()),
            );
            cx.notify();
            return;
        }
        if source.access.is_example() {
            let mut inventory = databases::example::shaped(Utc::now(), &self.shape);
            if let Some(namespace) = &self.namespace {
                inventory
                    .databases
                    .items
                    .retain(|db| db.identity.namespace == *namespace);
                inventory
                    .backups
                    .items
                    .retain(|backup| backup.identity.namespace == *namespace);
                inventory
                    .schedules
                    .items
                    .retain(|schedule| schedule.identity.namespace == *namespace);
            }
            self.loader.resolve(id, Ok(inventory));
            self.inventory_changed(cx);
            cx.notify();
            return;
        }
        let namespace = self.namespace.clone();
        let previous = self.loader.data().cloned();
        let request = self.loader.start(id);
        // Connection plus discovery/per-kind budgets leave a margin before
        // the loader deadline, preserving successful kinds.
        let (job, receiver) = freshkube_core::job::spawn_job(
            &self.runtime,
            databases::READ_BUDGET + Duration::from_secs(10),
            "Loading databases timed out".to_owned(),
            async move {
                let client = tokio::time::timeout(Duration::from_secs(5), source.access.client())
                    .await
                    .map_err(|_| "Database connection timed out".to_owned())?
                    .map_err(|failure| format!("Database connection: {}", failure.kind.label()))?;
                let mut inventory =
                    databases::read(&client, namespace.as_deref(), Utc::now()).await;
                if let Some(previous) = previous {
                    inventory.retain(&previous)
                }
                Ok(inventory)
            },
        );
        let task = cx.spawn(async move |this, cx| {
            let result = receiver
                .await
                .unwrap_or_else(|_| Err("Loading databases stopped unexpectedly".into()));
            _ = this.update(cx, |view, cx| {
                if view.loader.land(&request, result, cx) {
                    view.inventory_changed(cx);
                }
                cx.notify();
            });
        });
        self.loader.run(Some(job), task);
        cx.notify();
    }

    /// The shell's Coroot: the connection Observability holds and this
    /// entry's link. A different one makes what was read foreign.
    pub(crate) fn set_coroot(&mut self, input: Option<CorootInput>, cx: &mut Context<Self>) {
        if self.coroot == input {
            return;
        }
        let reader_of = |input: &Option<CorootInput>| input.as_ref().and_then(|i| i.reader.clone());
        let (old, new) = (reader_of(&self.coroot), reader_of(&input));
        self.coroot = input;
        self.evidence.reset();
        if old != new {
            if let Some(old) = old {
                old.want(cx.entity_id(), false, cx);
            }
            self._connection = new
                .as_ref()
                .and_then(|reader| reader.on_connection(cx, |page, cx| page.read_coroot(cx)));
            let id = cx.entity_id();
            self._release = new
                .clone()
                .map(|reader| cx.on_release(move |_, cx| reader.want(id, false, cx)));
        }
        self.want_coroot(cx);
        self.read_coroot(cx);
        cx.notify();
    }

    /// Tells Observability whether this page wants Coroot connected: only
    /// while shown, and only when the open entry has a Coroot link.
    fn want_coroot(&self, cx: &mut Context<Self>) {
        let Some(input) = &self.coroot else {
            return;
        };
        if let Some(reader) = &input.reader {
            reader.want(cx.entity_id(), self.visible && input.link.is_ok(), cx);
        }
    }

    /// The inventory changed: what Coroot says of it is asked again.
    fn inventory_changed(&mut self, cx: &mut Context<Self>) {
        self.read_coroot(cx);
    }

    /// Asks Coroot about the inventory's databases, only while shown, over
    /// the open entry's link and never another. With no link, no connection
    /// or example data nothing is requested.
    fn read_coroot(&mut self, cx: &mut Context<Self>) {
        if !self.visible {
            return;
        }
        let Some(inventory) = self.loader.data() else {
            return;
        };
        let uids: Vec<String> = inventory
            .databases
            .items
            .iter()
            .map(|db| db.identity.uid.clone())
            .collect();
        let revision = self.loader.revision();
        let example = self
            .source
            .as_ref()
            .is_some_and(|source| source.access.is_example());
        if example {
            let evidence = databases::example::coroot(Utc::now(), &self.shape, inventory);
            self.evidence
                .resolve(format!("{revision}/example"), Ok(evidence));
            cx.notify();
            return;
        }
        let absent = |why| Evidence::absent(uids.clone(), why);
        let Some(input) = self.coroot.clone() else {
            let evidence = absent(Absence::NotConnected);
            self.evidence
                .resolve(format!("{revision}/none"), Ok(evidence));
            cx.notify();
            return;
        };
        let link = match input.link {
            Ok(link) => link,
            Err(why) => {
                self.evidence
                    .resolve(format!("{revision}/{why:?}"), Ok(absent(why)));
                cx.notify();
                return;
            }
        };
        let connection = input
            .reader
            .as_ref()
            .and_then(|reader| reader.connection(cx));
        let Some(connection) = connection else {
            let evidence = absent(Absence::NotConnected);
            self.evidence
                .resolve(format!("{revision}/disconnected"), Ok(evidence));
            cx.notify();
            return;
        };
        let wanted: Vec<Wanted> = inventory
            .databases
            .items
            .iter()
            .map(|db| Wanted {
                uid: db.identity.uid.clone(),
                engine: db.engine,
                namespace: db.identity.namespace.clone(),
                name: db.identity.name.clone(),
            })
            .collect();
        let previous = self.evidence.data().cloned();
        let identity = format!("{revision}/{:?}/{link:?}", connection.id());
        let request = self.evidence.start(identity);
        let (job, receiver) = connection.databases(link, wanted, Utc::now());
        let task = cx.spawn(async move |this, cx| {
            let result = receiver.await.unwrap_or_else(|_| Err("stopped".into()));
            _ = this.update(cx, |view, cx| {
                let result = match result {
                    Ok(evidence) => evidence,
                    // A timeout or a lost job is Coroot not answering.
                    Err(_) => Evidence {
                        states: uids
                            .iter()
                            .map(|uid| {
                                (
                                    uid.clone(),
                                    CorootState::Unreachable {
                                        why: freshkube_core::coroot::ReadError::Timeout,
                                        last: None,
                                    },
                                )
                            })
                            .collect(),
                    },
                };
                let mut result = result;
                if let Some(previous) = &previous {
                    result.retain(previous);
                }
                view.evidence.land(&request, Ok(result), cx);
                cx.notify();
            });
        });
        self.evidence.run(job, task);
        cx.notify();
    }

    /// Prepares rows when the inventory, Coroot's answers or the view
    /// changed; not while drawing.
    fn sync(&mut self) {
        let now = (
            self.loader.revision(),
            self.evidence.revision(),
            Some(self.mode),
        );
        if self.prepared == (now.0, now.1, now.2) {
            return;
        }
        let empty = Evidence::default();
        let evidence = self.evidence.data().unwrap_or(&empty);
        let shown = self.display.engine_shown();
        self.display = Display::new(self.loader.data(), evidence, self.mode, Utc::now());
        self.display.set_engine_shown(shown);
        self.prepared = now;
        let filter = self.filter_text.clone();
        self.display.set_filter(&filter);
    }
    fn mode(&mut self, mode: Mode, cx: &mut Context<Self>) {
        self.mode = mode;
        self.prepared = (u64::MAX, u64::MAX, None);
        self.selected = None;
        self.sync();
        cx.notify();
    }
    fn choose(&mut self, key: SharedString, cx: &mut Context<Self>) {
        self.selected = Some(key);
        freshkube_ui::table::reveal(self, ScrollStrategy::Nearest);
        cx.notify();
    }
    fn step(&mut self, delta: isize, cx: &mut Context<Self>) {
        self.sync();
        if let Some(key) = freshkube_ui::table::step(self, delta, cx) {
            self.choose(key, cx)
        }
    }
    fn selected_row(&self) -> Option<&display::Row> {
        let key = self.selected.as_ref()?;
        self.display.rows.iter().find(|row| &row.key == key)
    }
    fn selected_observe(&self) -> Option<&display::Observe> {
        self.selected_row()?.observe.as_ref()
    }
    fn open(&mut self, cx: &mut Context<Self>) {
        let Some(row) = self.selected_row() else {
            return;
        };
        let id = &row.identity;
        cx.emit(ResourceLink::Object(
            freshkube_core::resources::ResourceKind::new(
                &id.group,
                &id.version,
                &id.kind,
                &id.plural,
                true,
            ),
            crate::resources::model::ObjectRef {
                namespace: id.namespace.clone(),
                name: id.name.clone(),
                uid: id.uid.clone(),
                connection: self.source.as_ref().map(|source| source.id.clone()),
            },
            crate::resources::Tab::Overview,
        ));
    }
    /// Opens Coroot's application page for the selected database, on the
    /// report of its engine.
    fn open_observability(&mut self, cx: &mut Context<Self>) {
        let Some(observe) = self.selected_observe().cloned() else {
            return;
        };
        let entry = self
            .coroot
            .as_ref()
            .map(|input| input.entry.clone())
            .unwrap_or_default();
        cx.emit(ObservabilityLink {
            entry,
            app: observe.app,
            tab: observe.tab,
        });
    }
}
impl ScreenPanel for DatabasesScreen {
    fn set_source(&mut self, _: Option<ScreenSource>, _: &mut Window, _: &mut Context<Self>) {}
    fn set_page_source(&mut self, source: PageSource, _: &mut Window, cx: &mut Context<Self>) {
        let changed = self.source.as_ref().map(|source| &source.id)
            != source.kube.as_ref().map(|source| &source.id);
        self.source = source.kube;
        if changed {
            self.loader.reset();
            self.evidence.reset();
            self.selected = None;
        }
        cx.notify();
    }
    fn set_visible(&mut self, visible: bool, _: &mut Window, cx: &mut Context<Self>) {
        self.visible = visible;
        // Hiding drops the Coroot read with the inventory's.
        let again = self.loader.set_visible(visible, ON_SHOW, cx);
        let coroot_again = self.evidence.set_visible(visible, ON_SHOW, cx);
        self.want_coroot(cx);
        if again {
            self.read(cx)
        } else if coroot_again {
            self.read_coroot(cx)
        }
        cx.notify();
    }
    fn activate(&mut self, _: &mut Window, cx: &mut Context<Self>) {
        if self.loader.data().is_none() {
            self.read(cx)
        }
    }
    fn refresh(&mut self, _: &mut Window, cx: &mut Context<Self>) {
        self.read(cx)
    }
    fn focus(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        window.focus(&self.focus, cx)
    }
    fn loading_motion(&self, _: &App) -> Option<Entity<freshkube_ui::table::LoadingMotion>> {
        self.loading
            .motion(self.loader.is_loading() && self.loader.data().is_none())
    }
    fn status(&mut self, _: &App) -> Option<Segment> {
        self.sync();
        let mut parts = vec![
            Part::new(
                self.namespace
                    .clone()
                    .map(|namespace| format!("Namespace · {namespace}"))
                    .unwrap_or_else(|| "All namespaces".into()),
            ),
            Part::new(self.display.summary.clone()),
        ];
        parts.extend(self.display.attention.clone().map(Part::new));
        if self.display.notice.is_some() {
            parts.push(Part::new("coverage incomplete").tone(ui::Tone::Warn));
        }
        if self.loader.is_loading() {
            parts.push(Part::new("Reading…"))
        }
        if self.loader.error().is_some() {
            let words = if self.loader.data().is_some() {
                "Couldn't refresh · last read retained"
            } else {
                "Couldn't read databases"
            };
            parts.push(Part::new(words).tone(ui::Tone::Warn))
        }
        if let Some(at) = self.loader.last_successful() {
            parts.push(Part::new(format!("updated {}", ui::clock(at))).minor())
        }
        if self
            .source
            .as_ref()
            .is_some_and(|source| source.access.is_example())
        {
            parts.push(Part::new("example data").minor())
        }
        Some(Segment::new(
            self.source.as_ref().map(|source| source.context.clone()),
            parts,
        ))
    }
    fn subscribe_page(
        this: &Entity<Self>,
        on_event: PageEventFn,
        window: &mut Window,
        cx: &mut App,
    ) -> Vec<Subscription> {
        let on_link = on_event.clone();
        vec![
            window.subscribe(this, cx, move |_, link: &ResourceLink, window, cx| {
                on_link(&link.clone().into(), window, cx)
            }),
            window.subscribe(this, cx, move |_, link: &ObservabilityLink, window, cx| {
                on_event(&link.clone().into(), window, cx)
            }),
        ]
    }
}
