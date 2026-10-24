//! The Routes list (#949, N2a): every HTTPRoute with a verdict, grouped not
//! serving first, each row reading where its trace stops and why. Read-only.
//! The page owns its read; core's `resources::traffic::routes` owns the
//! interpretation, and the Inspector draws the same trace the route's pane
//! does. Nothing here applies a fix: a proposed ReferenceGrant is text to copy.
mod display;
mod table;
#[cfg(test)]
mod tests;
mod view;

use super::{PageEventFn, PageSource, ScreenEvent, ScreenPanel, ScreenSource, TableLoading};
use crate::{
    resources::{ClusterSource, ResourceLink},
    ui::{self, dp},
};
use display::Display;
use freshkube_core::cluster_source::ClusterAccess;
use freshkube_core::resources::traffic::routes::{self, GATEWAY_GROUP, RoutesRead};
use freshkube_reads::{Loader, OnShow};
use freshkube_ui::{
    inspector::InspectorSplit,
    status::{Part, Segment},
    table::TableState,
};
use gpui_kit::component::input::{InputEvent, InputState};
use gpui_kit::prelude::*;
use gpui_kit::*;
use std::time::Duration;
use tokio::runtime::Handle;

const CONTEXT: &str = "Routes";
/// The page width, in dp, under which the toolbar's buttons are icons.
const NARROW_TOOLBAR: f32 = 720.;
const PREFIX: &str = "routes";
/// A read this old is read again when the page shows.
const ON_SHOW: OnShow = OnShow::ReadIfOlder(Duration::from_secs(30));
/// The whole read: discovery, lists and up to 100 backends.
const READ_DEADLINE: Duration = Duration::from_secs(45);

actions!(
    routes,
    [
        Next, Previous, First, Last, Close, OpenTrace, OpenRoute, Find
    ]
);

pub(crate) struct RoutesScreen {
    runtime: Handle,
    source: Option<ClusterSource>,
    visible: bool,
    loader: Loader<RoutesRead, String>,
    selected: Option<SharedString>,
    focus: FocusHandle,
    filter_input: Entity<InputState>,
    filter_text: String,
    _filter: Subscription,
    display: Display,
    /// The revision of the read the display was prepared from.
    prepared: u64,
    table: TableState,
    split: InspectorSplit,
    notes_scroll: ScrollHandle,
    loading: TableLoading,
    /// `FRESHKUBE_ROUTES`: the example's shape.
    shape: routes::example::Shape,
    /// When the example data was read, which a loader does not keep.
    example_read: Option<std::time::SystemTime>,
}

impl EventEmitter<ScreenEvent> for RoutesScreen {}
impl EventEmitter<ResourceLink> for RoutesScreen {}

impl RoutesScreen {
    pub(crate) fn new(runtime: Handle, window: &mut Window, cx: &mut Context<Self>) -> Self {
        cx.bind_keys([
            KeyBinding::new("down", Next, Some(CONTEXT)),
            KeyBinding::new("up", Previous, Some(CONTEXT)),
            KeyBinding::new("home", First, Some(CONTEXT)),
            KeyBinding::new("end", Last, Some(CONTEXT)),
            KeyBinding::new("escape", Close, Some(CONTEXT)),
            KeyBinding::new("enter", OpenTrace, Some(CONTEXT)),
            KeyBinding::new("t", OpenTrace, Some(CONTEXT)),
            KeyBinding::new("o", OpenRoute, Some(CONTEXT)),
            KeyBinding::new("/", Find, Some(CONTEXT)),
        ]);
        let filter_input = cx.new(|cx| InputState::new(window, cx).placeholder("Filter  /"));
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
            selected: None,
            focus: cx.focus_handle(),
            filter_input,
            filter_text: String::new(),
            _filter: filter,
            display: Display::default(),
            prepared: u64::MAX,
            table: TableState::new(PREFIX),
            split: InspectorSplit::new(PREFIX, cx),
            notes_scroll: ScrollHandle::new(),
            loading: TableLoading::new(PREFIX, cx),
            shape: routes::example::Shape::default(),
            example_read: None,
        }
    }

    pub(crate) fn with_example(mut self, shape: routes::example::Shape) -> Self {
        self.shape = shape;
        self
    }

    fn identity(&self) -> Option<String> {
        Some(self.source.as_ref()?.id.clone())
    }

    fn read(&mut self, cx: &mut Context<Self>) {
        if !self.visible || self.loader.is_loading() {
            return;
        }
        let Some(source) = self.source.clone() else {
            return;
        };
        let id = self.identity().unwrap();
        if let ClusterAccess::Example(hold) = &source.access {
            // Held, the read stays in flight, so the page keeps its loading
            // rows.
            if hold.routes {
                self.loader.start(id);
                cx.notify();
                return;
            }
            let read = routes::example::read(self.shape)
                .map_err(|failure| format!("{}: {}", failure.kind.label(), failure.message));
            self.example_read = Some(std::time::SystemTime::now());
            self.loader.resolve(id, read);
            cx.notify();
            return;
        }
        let request = self.loader.start(id);
        let (job, receiver) = freshkube_core::job::spawn_job(
            &self.runtime,
            READ_DEADLINE,
            "Reading routes timed out".to_owned(),
            async move {
                let client = tokio::time::timeout(Duration::from_secs(5), source.access.client())
                    .await
                    .map_err(|_| "Connecting timed out".to_owned())?
                    .map_err(|failure| format!("Connecting: {}", failure.kind.label()))?;
                routes::read_routes(&client, None)
                    .await
                    .map_err(|failure| format!("{}: {}", failure.kind.label(), failure.message))
            },
        );
        let task = cx.spawn(async move |this, cx| {
            let result = receiver
                .await
                .unwrap_or_else(|_| Err("Reading routes stopped unexpectedly".into()));
            _ = this.update(cx, |view, cx| {
                view.loader.land(&request, result, cx);
                cx.notify();
            });
        });
        self.loader.run(Some(job), task);
        cx.notify();
    }

    /// Prepares rows when the read changed; not while drawing.
    fn sync(&mut self) {
        let now = self.loader.revision();
        if self.prepared == now {
            return;
        }
        self.display = Display::new(self.loader.data());
        self.prepared = now;
        let filter = self.filter_text.clone();
        self.display.set_filter(&filter);
        // A selection the new read no longer lists is dropped.
        if self
            .selected
            .as_ref()
            .is_some_and(|key| !self.display.rows.iter().any(|row| &row.key == key))
        {
            self.selected = None;
        }
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

    /// Opens the selected route in Resources, on its Trace tab or Details.
    fn open(&mut self, tab: crate::resources::Tab, cx: &mut Context<Self>) {
        let Some(row) = self.selected_row() else {
            return;
        };
        let kind = self
            .loader
            .data()
            .and_then(|read| read.route_kind.clone())
            .unwrap_or_else(|| {
                freshkube_core::resources::ResourceKind::new(
                    GATEWAY_GROUP,
                    "v1",
                    "HTTPRoute",
                    "httproutes",
                    true,
                )
            });
        // Example routes are invented here and listed again by Resources'
        // example cluster under identities of its own, so a link to one names
        // it and lets Resources find the object; a cluster's carries its UID,
        // which keeps the link from opening a route that replaced this one.
        let example = self
            .source
            .as_ref()
            .is_some_and(|source| source.access.is_example());
        let object = crate::resources::model::ObjectRef {
            namespace: row.namespace.clone(),
            name: row.name.clone(),
            uid: if example {
                String::new()
            } else {
                row.uid.clone()
            },
            connection: self.source.as_ref().map(|source| source.id.clone()),
        };
        cx.emit(ResourceLink::Object(kind, object, tab));
    }

    pub(super) fn retry(&self, cx: &mut Context<Self>) -> gpui_kit::component::button::Button {
        use gpui_kit::component::button::Button;
        Button::new("routes-retry")
            .icon(gpui_kit::assets::IconName::RefreshCw)
            .label("Retry")
            .on_click(cx.listener(|this, _, _, cx| this.read(cx)))
    }
}

impl ScreenPanel for RoutesScreen {
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
        // Hiding drops a read in flight; showing reads again when it is old.
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
        let mut parts = vec![Part::new("All namespaces")];
        if !self.display.summary.is_empty() {
            parts.push(Part::new(self.display.summary.clone()));
        }
        for (count, words, tone) in &self.display.attention {
            parts.push(Part::new(format!("{count} {words}")).tone(*tone));
        }
        if self.loader.is_loading() {
            parts.push(Part::new("Reading…"))
        }
        if self.loader.error().is_some() {
            let words = if self.loader.data().is_some() {
                "Couldn't refresh · last read retained"
            } else {
                "Couldn't read routes"
            };
            parts.push(Part::new(words).tone(ui::Tone::Warn))
        }
        if let Some(at) = self.loader.last_successful().or(self.example_read) {
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
