//! Read-only operator inventory. The page owns reads; core owns interpretation.
mod display;
mod table;
#[cfg(test)]
mod tests;
mod view;
#[cfg(test)]
mod wire_tests;
use super::{PageEventFn, PageSource, ScreenEvent, ScreenPanel, ScreenSource, TableLoading};
use crate::{
    resources::{ClusterSource, ResourceLink},
    ui::{self, dp},
};
use chrono::Utc;
use display::{Display, Mode};
use freshkube_core::databases::{self, Inventory};
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
const PREFIX: &str = "databases";
const ON_SHOW: OnShow = OnShow::ReadIfOlder(Duration::from_secs(30));
actions!(
    databases,
    [Next, Previous, First, Last, Close, OpenResource, Find]
);
pub(crate) struct DatabasesScreen {
    runtime: Handle,
    source: Option<ClusterSource>,
    visible: bool,
    loader: Loader<Inventory, String>,
    selected: Option<SharedString>,
    focus: FocusHandle,
    namespace: Option<String>,
    namespace_input: Entity<InputState>,
    _input: Subscription,
    mode: Mode,
    display: Display,
    revision: u64,
    table: TableState,
    split: InspectorSplit,
    loading: TableLoading,
    shape: String,
}
impl EventEmitter<ScreenEvent> for DatabasesScreen {}
impl EventEmitter<ResourceLink> for DatabasesScreen {}
impl DatabasesScreen {
    pub(crate) fn new(runtime: Handle, window: &mut Window, cx: &mut Context<Self>) -> Self {
        cx.bind_keys([
            KeyBinding::new("down", Next, Some(CONTEXT)),
            KeyBinding::new("up", Previous, Some(CONTEXT)),
            KeyBinding::new("home", First, Some(CONTEXT)),
            KeyBinding::new("end", Last, Some(CONTEXT)),
            KeyBinding::new("escape", Close, Some(CONTEXT)),
            KeyBinding::new("o", OpenResource, Some(CONTEXT)),
            KeyBinding::new("/", Find, Some(CONTEXT)),
        ]);
        let namespace_input =
            cx.new(|cx| InputState::new(window, cx).placeholder("Namespace · all /"));
        let input = cx.subscribe_in(&namespace_input, window, |this, _, event, window, cx| {
            if matches!(event, InputEvent::PressEnter { .. }) {
                let value = this.namespace_input.read(cx).value().trim().to_owned();
                let namespace = (!value.is_empty()).then_some(value);
                if namespace != this.namespace {
                    this.namespace = namespace;
                    this.loader.reset();
                    this.selected = None;
                    this.read(cx);
                }
                window.focus(&this.focus, cx);
            }
        });
        Self {
            runtime,
            source: None,
            visible: false,
            loader: Loader::default(),
            selected: None,
            focus: cx.focus_handle(),
            namespace: None,
            namespace_input,
            _input: input,
            mode: Mode::Clusters,
            display: Display::default(),
            revision: u64::MAX,
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
            cx.notify();
            return;
        }
        let namespace = self.namespace.clone();
        let previous = self.loader.data().cloned();
        self.loader.load(
            id,
            &self.runtime,
            Duration::from_secs(30),
            "databases",
            async move {
                let client =
                    source.access.client().await.map_err(|failure| {
                        format!("Database connection: {}", failure.kind.label())
                    })?;
                let mut inventory =
                    databases::read(&client, namespace.as_deref(), Utc::now()).await;
                if let Some(previous) = previous {
                    inventory.retain(&previous)
                }
                Ok(inventory)
            },
            |this: &mut Self| &mut this.loader,
            cx,
        );
        cx.notify();
    }
    fn sync(&mut self) {
        if self.revision == self.loader.revision() {
            return;
        }
        self.display = Display::new(self.loader.data(), self.mode);
        self.revision = self.loader.revision();
    }
    fn mode(&mut self, mode: Mode, cx: &mut Context<Self>) {
        self.mode = mode;
        self.revision = u64::MAX;
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
    fn open(&mut self, cx: &mut Context<Self>) {
        let Some(row) = self
            .display
            .rows
            .iter()
            .find(|row| Some(&row.key) == self.selected.as_ref())
        else {
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
}
impl ScreenPanel for DatabasesScreen {
    fn set_source(&mut self, _: Option<ScreenSource>, _: &mut Window, _: &mut Context<Self>) {}
    fn set_page_source(&mut self, source: PageSource, _: &mut Window, cx: &mut Context<Self>) {
        let changed = self.source.as_ref().map(|source| &source.id)
            != source.kube.as_ref().map(|source| &source.id);
        self.source = source.kube;
        if changed {
            self.loader.reset();
            self.selected = None;
        }
        cx.notify();
    }
    fn set_visible(&mut self, visible: bool, _: &mut Window, cx: &mut Context<Self>) {
        self.visible = visible;
        if self.loader.set_visible(visible, ON_SHOW, cx) {
            self.read(cx)
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
                    .unwrap_or_else(|| "All namespaces".into()),
            ),
            Part::new(self.display.summary.clone()),
        ];
        if self.loader.is_loading() {
            parts.push(Part::new("Reading…"))
        }
        if self.loader.error().is_some() {
            parts.push(Part::new("Couldn't refresh · last read retained").tone(ui::Tone::Warn))
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
        vec![
            window.subscribe(this, cx, move |_, link: &ResourceLink, window, cx| {
                on_event(&link.clone().into(), window, cx)
            }),
        ]
    }
}
