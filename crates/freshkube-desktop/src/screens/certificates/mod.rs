//! The Certificates page (#949, N4): every TLS Secret a Gateway listener or
//! an Ingress serves, plus the cert-manager Certificates nothing serves yet,
//! with its names, its expiry, any host served under it that it doesn't name,
//! and, for a cert-manager Certificate, the issuance chain and where it
//! stopped. Read-only. Where cert-manager keeps the Secret its Certificate's
//! status answers and no Secret is read; any other Secret is read only by the
//! Read Secret control, once, and only the parsed certificate's facts are
//! kept (safety guarantee 23).
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
use freshkube_core::job::OwnedJob;
use freshkube_core::resources::traffic::certs::{
    self, CertsRead, CertsView, SecretOutcome, read_certificates, read_tls_secret,
};
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

/// The page width, in dp, under which the toolbar's buttons are icons.
const NARROW_TOOLBAR: f32 = 720.;
/// The logical cell that is the state in words.
const STATUS: usize = 1;
/// A read this old is read again when the page shows.
const ON_SHOW: OnShow = OnShow::ReadIfOlder(Duration::from_secs(30));
/// The whole read: discovery, lists and the Gateway API's backends.
const READ_DEADLINE: Duration = Duration::from_secs(45);
/// Reading one Secret.
const SECRET_DEADLINE: Duration = Duration::from_secs(15);

actions!(
    certificates,
    [
        Next, Previous, First, Last, Close, OpenObject, ReadSecret, Find
    ]
);

/// A Secret read in flight, dropped (and so cancelled) with the page's
/// source, its visibility or the read it was made for.
struct SecretJob {
    _key: String,
    _job: OwnedJob,
    _task: Task<()>,
}

pub(crate) struct CertificatesScreen {
    runtime: Handle,
    source: Option<ClusterSource>,
    visible: bool,
    loader: Loader<CertsRead, String>,
    /// What the page draws, from the last read, with any Secret the person
    /// has read applied. Held in memory only: dropped with the read, the
    /// source and the page's visibility.
    view: Option<CertsView>,
    /// Unix seconds when the read was made, which expiry is counted from.
    read_at: i64,
    selected: Option<SharedString>,
    /// A row a link asked for before the read had landed.
    wanted: Option<String>,
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
    /// `FRESHKUBE_CERTS`: the example's shape.
    shape: certs::example::Shape,
    /// When the example data was read, which a loader does not keep.
    example_read: Option<std::time::SystemTime>,
    secret_job: Option<SecretJob>,
    /// What the person read of each Secret (`namespace/name`), kept across
    /// the shell's automatic refreshes of the same source and applied to each
    /// new read; dropped when the page hides or the source changes.
    secrets: std::collections::BTreeMap<String, SecretOutcome>,
}

impl EventEmitter<ScreenEvent> for CertificatesScreen {}
impl EventEmitter<ResourceLink> for CertificatesScreen {}

fn unix_now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |since| {
            i64::try_from(since.as_secs()).unwrap_or(i64::MAX)
        })
}

impl CertificatesScreen {
    pub(crate) fn new(runtime: Handle, window: &mut Window, cx: &mut Context<Self>) -> Self {
        cx.bind_keys([
            KeyBinding::new("down", Next, Some("Certificates")),
            KeyBinding::new("up", Previous, Some("Certificates")),
            KeyBinding::new("home", First, Some("Certificates")),
            KeyBinding::new("end", Last, Some("Certificates")),
            KeyBinding::new("escape", Close, Some("Certificates")),
            KeyBinding::new("enter", OpenObject, Some("Certificates")),
            KeyBinding::new("o", OpenObject, Some("Certificates")),
            KeyBinding::new("r", ReadSecret, Some("Certificates")),
            KeyBinding::new("/", Find, Some("Certificates")),
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
            view: None,
            read_at: 0,
            selected: None,
            wanted: None,
            focus: cx.focus_handle(),
            filter_input,
            filter_text: String::new(),
            _filter: filter,
            display: Display::empty(),
            prepared: u64::MAX,
            table: TableState::new("certificates"),
            split: InspectorSplit::new("certificates", cx),
            notes_scroll: ScrollHandle::new(),
            loading: TableLoading::new("certificates", cx),
            shape: certs::example::Shape::default(),
            example_read: None,
            secret_job: None,
            secrets: std::collections::BTreeMap::new(),
        }
    }

    pub(crate) fn with_example(mut self, shape: certs::example::Shape) -> Self {
        self.shape = shape;
        self
    }

    fn identity(&self) -> Option<String> {
        Some(self.source.as_ref()?.id.clone())
    }

    /// Forgets every Secret's facts and any read in flight.
    fn forget_secrets(&mut self) {
        self.secret_job = None;
        self.secrets.clear();
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
            self.read_at = unix_now();
            let read = certs::example::read(self.shape, self.read_at);
            self.example_read = Some(std::time::SystemTime::now());
            self.loader.resolve(id, Ok(read));
            cx.notify();
            return;
        }
        let request = self.loader.start(id);
        let (job, receiver) = freshkube_core::job::spawn_job(
            &self.runtime,
            READ_DEADLINE,
            "Reading certificates timed out".to_owned(),
            async move {
                let client = tokio::time::timeout(Duration::from_secs(5), source.access.client())
                    .await
                    .map_err(|_| "Connecting timed out".to_owned())?
                    .map_err(|failure| format!("Connecting: {}", failure.kind.label()))?;
                Ok(read_certificates(&client).await)
            },
        );
        let task = cx.spawn(async move |this, cx| {
            let result = receiver
                .await
                .unwrap_or_else(|_| Err("Reading stopped unexpectedly".into()));
            _ = this.update(cx, |view, cx| {
                view.read_at = unix_now();
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
        self.prepared = now;
        // A new read of the same source (the shell refreshes on its own) keeps
        // what the person read of each Secret: it is applied to the new rows.
        self.view = self
            .loader
            .data()
            .map(|read| CertsView::new(read, self.read_at));
        if let Some(view) = &mut self.view {
            // Only to a row that still takes it: one a Certificate now keeps,
            // or whose reading is withheld, loses what was kept for it.
            self.secrets.retain(|key, _| view.takes_secret(key));
            for (key, outcome) in &self.secrets {
                view.apply_secret(key, outcome.clone(), self.read_at);
            }
        }
        self.rebuild();
        if self.view.is_some()
            && let Some(key) = self.wanted.take()
            && self.display.rows.iter().any(|row| row.key == key.as_str())
        {
            self.selected = Some(key.into());
        }
        // A selection the new read no longer lists is dropped.
        if self
            .selected
            .as_ref()
            .is_some_and(|key| !self.display.rows.iter().any(|row| &row.key == key))
        {
            self.selected = None;
        }
    }

    fn rebuild(&mut self) {
        self.display = Display::new(self.view.as_ref());
        let filter = self.filter_text.clone();
        self.display.set_filter(&filter);
    }

    /// Selects a Secret's row, as a link from a route's trace asks. A row
    /// not listed yet is selected when the read lands. Reads nothing.
    pub(crate) fn select(&mut self, namespace: &str, name: &str, cx: &mut Context<Self>) {
        let key = format!("{namespace}/{name}");
        if self.display.rows.iter().any(|row| row.key == key.as_str()) {
            self.selected = Some(key.into());
        } else {
            self.wanted = Some(key);
        }
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

    /// Opens the cert-manager Certificate that keeps the selected Secret.
    /// A Secret has nothing to open: it is read, not browsed.
    fn open(&mut self, cx: &mut Context<Self>) {
        let Some(certificate) = self.selected_row().and_then(|row| row.certificate.clone()) else {
            return;
        };
        let kind = self
            .loader
            .data()
            .and_then(|read| read.certificate_kind.clone())
            .unwrap_or_else(|| {
                freshkube_core::resources::ResourceKind::new(
                    certs::CERT_MANAGER_GROUP,
                    "v1",
                    "Certificate",
                    "certificates",
                    true,
                )
            });
        // A cluster's link carries the object's UID, which keeps it from
        // opening one that replaced this; the example cluster lists its
        // objects under identities of its own, so its links name the object.
        let example = self
            .source
            .as_ref()
            .is_some_and(|source| source.access.is_example());
        let object = crate::resources::model::ObjectRef {
            namespace: certificate.namespace,
            name: certificate.name,
            uid: if example {
                String::new()
            } else {
                certificate.uid
            },
            connection: self.source.as_ref().map(|source| source.id.clone()),
        };
        cx.emit(ResourceLink::Object(
            kind,
            object,
            crate::resources::Tab::Overview,
        ));
    }

    /// Reads the selected Secret, once, because the person asked. Nothing
    /// else on this page reads a Secret, and core offers no request for a
    /// Secret a Certificate answers for.
    fn read_secret(&mut self, cx: &mut Context<Self>) {
        if self.secret_job.is_some() {
            return;
        }
        let Some(key) = self
            .selected_row()
            .filter(|row| row.can_read)
            .map(|row| row.key.to_string())
        else {
            return;
        };
        let Some(request) = self
            .view
            .as_ref()
            .and_then(|view| view.entry(&key))
            .and_then(|entry| entry.secret_request())
        else {
            return;
        };
        let Some(source) = self.source.clone() else {
            return;
        };
        if source.access.is_example() {
            let outcome = certs::example::read_secret(self.shape, &request, self.read_at);
            self.land_secret(&key, outcome, cx);
            return;
        }
        let (job, receiver) = freshkube_core::job::spawn_job(
            &self.runtime,
            SECRET_DEADLINE,
            "Reading the Secret timed out".to_owned(),
            async move {
                let client = tokio::time::timeout(Duration::from_secs(5), source.access.client())
                    .await
                    .map_err(|_| "Connecting timed out".to_owned())?
                    .map_err(|failure| format!("Connecting: {}", failure.kind.label()))?;
                Ok(read_tls_secret(&client, &request).await)
            },
        );
        let waiting = key.clone();
        let task = cx.spawn(async move |this, cx| {
            let outcome = match receiver.await {
                Ok(Ok(outcome)) => outcome,
                Ok(Err(words)) => SecretOutcome::Refused(freshkube_core::failure::Failure::new(
                    freshkube_core::failure::FailureKind::Other,
                    words,
                )),
                Err(_) => return,
            };
            _ = this.update(cx, |view, cx| view.land_secret(&waiting, outcome, cx));
        });
        self.secret_job = Some(SecretJob {
            _key: key,
            _job: job,
            _task: task,
        });
        cx.notify();
    }

    fn land_secret(&mut self, key: &str, outcome: SecretOutcome, cx: &mut Context<Self>) {
        self.secret_job = None;
        self.secrets.insert(key.to_owned(), outcome.clone());
        if let Some(view) = &mut self.view {
            view.apply_secret(key, outcome, self.read_at);
        }
        self.rebuild();
        cx.notify();
    }

    /// The selected row's key, for tests of a link into the page.
    #[cfg(test)]
    pub(crate) fn selected_key(&self) -> Option<String> {
        self.selected.as_ref().map(ToString::to_string)
    }

    pub(super) fn retry(&self, cx: &mut Context<Self>) -> gpui_kit::component::button::Button {
        use gpui_kit::component::button::Button;
        Button::new("certificates-retry")
            .icon(gpui_kit::assets::IconName::RefreshCw)
            .label("Retry")
            .on_click(cx.listener(|this, _, _, cx| this.read(cx)))
    }
}

impl ScreenPanel for CertificatesScreen {
    fn set_source(&mut self, _: Option<ScreenSource>, _: &mut Window, _: &mut Context<Self>) {}

    fn set_page_source(&mut self, source: PageSource, _: &mut Window, cx: &mut Context<Self>) {
        let changed = self.source.as_ref().map(|source| &source.id)
            != source.kube.as_ref().map(|source| &source.id);
        self.source = source.kube;
        if changed {
            // A different context or person: nothing read before is theirs.
            self.loader.reset();
            self.selected = None;
            self.view = None;
            self.forget_secrets();
            self.prepared = u64::MAX;
            self.display = Display::empty();
        }
        cx.notify();
    }

    fn set_visible(&mut self, visible: bool, _: &mut Window, cx: &mut Context<Self>) {
        self.visible = visible;
        if !visible {
            // Hidden: what was read of any Secret goes, so the next show
            // starts every Secret unread.
            self.forget_secrets();
            if let Some(view) = &mut self.view {
                view.forget_secrets(self.read_at);
            }
            self.rebuild();
        }
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
        let example = self
            .source
            .as_ref()
            .is_some_and(|source| source.access.is_example());
        let scope = "All namespaces";
        let mut parts = Vec::new();
        if example {
            parts.push(Part::new("Example data").keep());
        }
        let time = self.loader.last_successful().or(self.example_read);
        match (example, time) {
            (true, Some(at)) => {
                parts.push(Part::new(ui::clock(at)).keep());
                parts.push(Part::new(scope));
            }
            (false, Some(at)) => {
                parts.push(Part::new(scope));
                parts.push(Part::new(format!("updated {}", ui::clock(at))).keep());
            }
            (_, None) => parts.push(Part::new(scope)),
        }
        if !self.display.summary.is_empty() {
            parts.push(Part::new(self.display.summary.clone()));
        }
        for (count, words, tone) in &self.display.attention {
            parts.push(Part::new(format!("{count} {words}")).tone(*tone));
        }
        if self.loader.is_loading() {
            parts.push(Part::new("Reading…"))
        }
        if self.loader.error().is_some() || self.display.error.is_some() {
            let words = if self.loader.data().is_some() && self.display.error.is_none() {
                "Couldn't refresh · last read retained"
            } else {
                "Couldn't read"
            };
            parts.push(Part::new(words).tone(ui::Tone::Warn))
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
