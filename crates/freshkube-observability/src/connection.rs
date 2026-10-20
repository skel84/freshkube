//! Connection lifetime and request guards owned by ObservabilityPage.
use super::*;
use crate::{backend, state::Snapshot};
use freshkube_core::cluster_source::{ClusterAccess, ClusterSource};
use freshkube_core::coroot as api;
use freshkube_core::job::OwnedJob;
use freshkube_reads::Loader;
use std::{cell::RefCell, future::Future, time::Duration};
use tokio::runtime::Handle;

/// Only an explicit Connect in Settings, or a change page reading Coroot,
/// may connect a hidden page.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum ConnectOrigin {
    Page,
    Settings,
    /// A change page reads Coroot while this page is hidden.
    Change,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum Subject {
    Applications,
    Map,
    /// Coroot's own view of an application.
    View(api::AppId),
    Incidents,
    Incident(String, api::AppId),
    /// An application, its trace source and the spans listed.
    Tracing(api::AppId, String, api::TraceSelection),
    /// One trace of an application, by id.
    Trace(api::AppId, String, String),
    Profiling(api::AppId, api::ProfileQuery),
    /// An application's logs, by the query without its `since`.
    Logs(api::AppId, api::LogQuery),
    /// The window around one of an application's revisions: its id and
    /// each side's seconds.
    Revision(api::AppId, String, u64),
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct ReadIdentity {
    source: api::Source,
    access: Option<String>,
    range: api::TimeRange,
    subject: Subject,
}
// Each answer owns its completion loader. The delivery holds only a weak
// reference, so replacing a report's slot drops both work and delivery.
#[derive(Clone, PartialEq, Eq)]
pub(super) struct ReadScope {
    source: Option<api::Source>,
    access: Option<String>,
    range: api::TimeRange,
}
pub(super) type ReadJob = Rc<RefCell<Loader<(), Option<ReadScope>>>>;
pub(super) struct Live {
    pub runtime: Handle,
    pub visible: bool,
    /// The provider was made for a change page while this page was hidden:
    /// it is nothing but the provider, and showing the page lists the
    /// projects and chooses the remembered one.
    pub provider_only: bool,
    /// Building the provider for a change page, on the I/O worker.
    pub provider_job: Option<(OwnedJob, Task<()>)>,
    pub access: Option<String>,
    pub provider: Option<api::Provider>,
    pub source: Option<api::Source>,
    pub projects: Vec<api::ProjectInfo>,
    pub project_label: String,
    pub project_labels: Vec<String>,
    pub jobs: Vec<ReadJob>,
    pub incident_job: Option<ReadJob>,
    pub traces_job: Option<ReadJob>,
    pub trace_job: Option<ReadJob>,
    pub profile_job: Option<ReadJob>,
    pub logs_job: Option<ReadJob>,
    pub revision_job: Option<ReadJob>,
    pub range: api::TimeRange,
    clock_origin: std::time::Instant,
    time_origin: chrono::DateTime<chrono::Utc>,
    pub range_label: String,
    pub connecting: bool,
    pub error: Option<String>,
    pub apps: Snapshot<Vec<api::Application>, ReadIdentity, String>,
    pub map: Snapshot<api::ServiceMap, ReadIdentity, String>,
    pub view: Snapshot<api::AppView, ReadIdentity, String>,
    /// The view's last read was refused, rather than failed.
    pub view_refused: bool,
    /// Coroot refused the last window read around a revision.
    pub revision_refused: bool,
    pub incidents: Snapshot<Vec<api::Incident>, ReadIdentity, String>,
    pub incident: Snapshot<api::IncidentView, ReadIdentity, String>,
    pub tracing: Snapshot<api::Tracing, ReadIdentity, String>,
    pub trace: Snapshot<api::Tracing, ReadIdentity, String>,
    pub profiling: Snapshot<api::Profiling, ReadIdentity, String>,
    pub logs: Snapshot<api::LogsView, ReadIdentity, String>,
    pub revision: Snapshot<api::RevisionView, ReadIdentity, String>,
    /// What reading the applications and the service map showed of access.
    pub capabilities: [api::Capability; 2],
}
impl Live {
    pub(super) fn identity(&self, subject: Subject) -> Option<ReadIdentity> {
        Some(ReadIdentity {
            source: self.source.clone()?,
            access: self.access.clone(),
            range: self.range,
            subject,
        })
    }
    pub fn new(runtime: Handle, now: std::time::Instant) -> Self {
        let time_origin = chrono::Utc::now();
        let range = range(3, time_origin);
        Self {
            runtime,
            visible: false,
            provider_only: false,
            provider_job: None,
            access: None,
            provider: None,
            source: None,
            projects: vec![],
            project_label: "Choose a project".into(),
            project_labels: vec![],
            jobs: vec![],
            incident_job: None,
            traces_job: None,
            trace_job: None,
            profile_job: None,
            logs_job: None,
            revision_job: None,
            range,
            clock_origin: now,
            time_origin,
            range_label: range_label(range),
            connecting: false,
            error: None,
            apps: Snapshot::default(),
            map: Snapshot::default(),
            view: Snapshot::default(),
            view_refused: false,
            revision_refused: false,
            incidents: Snapshot::default(),
            incident: Snapshot::default(),
            tracing: Snapshot::default(),
            trace: Snapshot::default(),
            profiling: Snapshot::default(),
            logs: Snapshot::default(),
            revision: Snapshot::default(),
            capabilities: [api::Capability::Unchecked; 2],
        }
    }
    fn cancel(&mut self) {
        for read in self.jobs.iter().chain(
            [
                &self.incident_job,
                &self.traces_job,
                &self.trace_job,
                &self.profile_job,
                &self.logs_job,
                &self.revision_job,
            ]
            .into_iter()
            .flatten(),
        ) {
            // Keep the request sequence even if a completion still holds its
            // loader while cancelling the page's other reads.
            read.borrow_mut().resolve(None, Err(String::new()));
        }
        self.jobs.clear();
        self.incident_job = None;
        self.traces_job = None;
        self.trace_job = None;
        self.profile_job = None;
        self.logs_job = None;
        self.revision_job = None;
        self.connecting = false;
    }
    pub(super) fn clear(&mut self) {
        self.cancel();
        self.apps = Snapshot::default();
        self.map = Snapshot::default();
        self.view = Snapshot::default();
        self.view_refused = false;
        self.incidents = Snapshot::default();
        self.incident = Snapshot::default();
        self.tracing = Snapshot::default();
        self.trace = Snapshot::default();
        self.profiling = Snapshot::default();
        self.logs = Snapshot::default();
        self.revision = Snapshot::default();
        self.revision_refused = false;
        self.capabilities = [api::Capability::Unchecked; 2];
    }
}
fn range(hours: u32, to: chrono::DateTime<chrono::Utc>) -> api::TimeRange {
    api::TimeRange::between(to - chrono::Duration::hours(i64::from(hours)), to)
}
fn range_label(range: api::TimeRange) -> String {
    format!(
        "{} — {}",
        range
            .from
            .unwrap()
            .with_timezone(&chrono::Local)
            .format("%Y-%m-%d %H:%M:%S"),
        range
            .to
            .unwrap()
            .with_timezone(&chrono::Local)
            .format("%Y-%m-%d %H:%M:%S")
    )
}

impl ObservabilityPage {
    pub fn link_is_current(
        &self,
        source: &api::Source,
        app: &api::AppId,
        subject: &api::ObjectSubject,
    ) -> bool {
        self.live.visible
            && self.live.access.as_deref() == Some(subject.access())
            && self.live.source.as_ref() == Some(source)
            && self.selected_app.as_ref() == Some(app)
            && api::ObjectSubject::for_app(source, app).as_ref() == Some(subject)
    }

    /// The shell's connection. Its id keys Coroot's access, and answers for
    /// another one are dropped. Example access answers from the examples,
    /// unless its hold keeps back the applications, incidents and traces
    /// (`FRESHKUBE_FIXTURE_HOLD=coroot`), so their tables stay on their
    /// loading rows.
    /// The workspace entry the connection is rides with it: its `coroot`
    /// key says which Coroot cluster the Kubernetes links read.
    pub fn set_source(
        &mut self,
        source: Option<&ClusterSource>,
        entry: Option<&str>,
        cx: &mut Context<Self>,
    ) {
        let held = match source.map(|source| &source.access) {
            Some(ClusterAccess::Example(hold)) => Some(hold.coroot),
            _ => None,
        };
        self.set_access_and_entry(source.map(|source| source.id.clone()), held, entry, cx);
    }
    /// The connection's id and the entry it is: the link follows both, and
    /// the page reads again once, whichever of them changed.
    pub(super) fn set_access_and_entry(
        &mut self,
        access: Option<String>,
        held: Option<bool>,
        entry: Option<&str>,
        cx: &mut Context<Self>,
    ) {
        let relinked = self.set_open_entry(entry, held.is_some());
        if !self.set_access(access, held, cx) && relinked {
            self.relink(cx);
        }
    }
    /// The connection's id and, for example data, whether its examples are
    /// held.
    /// Whether it changed anything, and so read again.
    pub(super) fn set_access(
        &mut self,
        access: Option<String>,
        held: Option<bool>,
        cx: &mut Context<Self>,
    ) -> bool {
        let examples = (held.is_some(), held.unwrap_or_default());
        let shown = (self.fixture, self.hold);
        if self.live.access == access && shown == examples {
            return false;
        }
        self.live.access = access;
        // The entry's linked project is the one to read, before any read.
        if let Some(project) = self.linked_choice() {
            self.install_project(&project, false, cx);
        } else if let Some(source) = self.live.source.take() {
            let association = self.association_for(source.project());
            self.live.source = Some(source.with_association(association));
            self.sync_note();
        }
        if shown != examples && self.fixture {
            self.forget_examples();
        }
        (self.fixture, self.hold) = examples;
        self.clear_observations();
        if shown != examples && self.fixture {
            self.show_examples();
        }
        self.refresh(cx);
        true
    }
    /// What example data shows before its first read: the map and the
    /// flame graph, and the applications with the worker chosen unless they
    /// are held. The incidents are answered by [`Self::refresh`].
    fn show_examples(&mut self) {
        (self.nodes, self.connections) = projection::map(&example::map());
        self.frames = example::flame();
        self.visible_frames = (0..self.frames.len()).collect();
        self.flame_matches = vec![true; self.frames.len()];
        if !self.hold {
            self.selected_app = Some(example::id(example::WORKER));
            self.apply_applications(&example::applications());
        }
        self.project();
        // The headers stay known, so a held table shows them over its
        // loading rows, as on a first read.
        self.prepare_incident_columns();
        self.prepare_revision_columns();
        self.prepare_map();
        self.prepare_report();
    }
    /// Back to the page's first state from example data.
    fn forget_examples(&mut self) {
        self.nodes = Default::default();
        self.connections = Default::default();
        self.map_display = super::map::display();
        self.forget_applications();
        self.app_choices = Rc::new([]);
        self.frames.clear();
        self.visible_frames.clear();
        self.flame_matches.clear();
    }
    pub fn set_visible(&mut self, visible: bool, cx: &mut Context<Self>) {
        if self.live.visible == visible {
            return;
        }
        self.live.visible = visible;
        if visible {
            self.refresh_current(cx);
            self.restore(ConnectOrigin::Page, cx);
            self.finish_provider_only(cx);
        } else {
            self.live.cancel();
        }
    }
    pub(super) fn clear_observations(&mut self) {
        self.focused_revision = None;
        self.live.clear();
        self.incident_observations = self.incident_observations.cleared();
        self.revision_observations = self.revision_observations.cleared();
        self.live_traces = self.live_traces.cleared();
        // The headers stay known, so a first read shows them over its loading rows.
        self.prepare_trace_columns();
        self.live_profiles = self.live_profiles.cleared();
        self.live_logs.forget();
        if !self.fixture {
            self.nodes = Default::default();
            self.connections = Default::default();
            self.map_display = super::map::display();
            self.forget_applications();
        }
    }
    /// Forgets the applications answered and what was derived from them,
    /// back to the page's first state; the map stays.
    fn forget_applications(&mut self) {
        self.applications.clear();
        self.matrix.clear();
        self.selected_app = None;
        self.report_snapshot = None;
        self.categories = Default::default();
        self.namespaces = Default::default();
        self.counts = [0; 7];
        self.active_categories = Rc::new(["application".into()].into());
        self.all_categories = false;
        self.category_defaults_pending = true;
        self.shown_apps = 0;
        self.app_count = "0 apps".into();
        self.prepare_application_columns();
        self.namespace = None;
    }
    pub(super) fn range_changed(&mut self, cx: &mut Context<Self>) {
        let elapsed = cx
            .background_executor()
            .now()
            .duration_since(self.live.clock_origin);
        let to = self.live.time_origin + chrono::Duration::from_std(elapsed).unwrap_or_default();
        self.live.range = range(self.hours, to);
        self.live.range_label = range_label(self.live.range);
        self.live.clear();
        self.incident_observations.clear_evidence();
        self.revision_observations.clear_evidence();
        self.revision_observations.seek_again();
        self.live_traces.reset();
        if !self.fixture {
            self.applications.clear();
            self.nodes = Default::default();
            self.connections = Default::default();
            self.map_display.reload();
            self.project();
            self.prepare_application_columns();
            self.report_snapshot = None;
        }
        self.refresh(cx);
    }
    /// A user refresh/reopen starts a new absolute observation window. A retry
    /// and navigation among reports keep the captured window.
    pub fn refresh_current(&mut self, cx: &mut Context<Self>) {
        self.range_changed(cx);
    }
    pub(super) fn select_project(&mut self, project: &api::ProjectInfo, cx: &mut Context<Self>) {
        self.select_project_as(project, true, cx);
    }
    /// Reads `project`; `remember` keeps it for the next launch, which a
    /// project an entry's link chose does not.
    pub(super) fn select_project_as(
        &mut self,
        project: &api::ProjectInfo,
        remember: bool,
        cx: &mut Context<Self>,
    ) {
        let Some(provider) = &self.live.provider else {
            return;
        };
        if self.live.source.as_ref().is_some_and(|source| {
            source.provider() == provider.id() && source.project() == project.id
        }) {
            return;
        }
        self.install_project(project, remember, cx);
        self.clear_observations();
        self.refresh_current(cx);
    }
    /// Makes `project` the shown one, with the open entry's link for it, and
    /// reads nothing.
    pub(super) fn install_project(
        &mut self,
        project: &api::ProjectInfo,
        remember: bool,
        cx: &mut Context<Self>,
    ) {
        let Some(provider) = &self.live.provider else {
            return;
        };
        let association = self.association_for(&project.id);
        self.live.source = Some(provider.source(project).with_association(association));
        self.live.project_label = format!("{} · {}", project.name, project.id);
        if remember {
            self.remember_project(&project.id, cx);
        }
        self.sync_note();
    }
    pub(super) fn invalidate_connection(&mut self) {
        self.live.provider = None;
        self.live.provider_only = false;
        self.live.provider_job = None;
        self.live.source = None;
        self.live.projects.clear();
        self.live.project_labels.clear();
        self.live.project_label = "Choose a project".into();
        self.live.error = None;
        self.sync_note();
        self.clear_observations();
    }
    pub(super) fn disconnect(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.invalidate_connection();
        self.forget_connection(cx);
        self.secret
            .update(cx, |input, cx| input.set_value("", window, cx));
        cx.emit(ObservabilityEvent::Connection);
        cx.notify();
    }
    /// Settings' Disconnect: drops the connection and the session's
    /// reconnect choice, and keeps the configured block.
    pub fn disconnect_from_settings(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.fixture {
            return;
        }
        self.disconnect(window, cx);
    }
    /// Where the connection stands.
    pub fn connection_state(&self) -> ConnectionState {
        if self.live.connecting {
            ConnectionState::Connecting
        } else if self.live.provider.is_some() && self.live.provider_only {
            // Made for a change page and never asked anything: a wrong key
            // would read as connected, so it says nothing of the sort.
            ConnectionState::Disconnected
        } else if self.live.provider.is_some() {
            ConnectionState::Connected
        } else if let Some(error) = &self.live.error {
            ConnectionState::Failed(error.clone())
        } else {
            ConnectionState::Disconnected
        }
    }
    /// Settings' Connect, after it saved the block: connects to `server` as
    /// `auth` (0 API key, 1 session, 2 anonymous) with the key just typed,
    /// or else the one the store keeps. A connect runs while the page is
    /// hidden, so Settings can say how it went.
    pub fn connect_from_settings(
        &mut self,
        server: &str,
        auth: usize,
        typed: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.fixture {
            return;
        }
        self.invalidate_connection();
        self.auth = auth.min(2);
        self.url.update(cx, |input, cx| {
            input.set_value(server.to_owned(), window, cx)
        });
        let typed = typed.trim().to_owned();
        self.secret
            .update(cx, |input, cx| input.set_value(typed.clone(), window, cx));
        // The block names `keys.json`, so the key is kept now, whether or not
        // this connect works.
        if auth != 2 && !typed.is_empty() {
            self.keep_key(server, &typed, cx);
        }
        if typed.is_empty() && auth != 2 {
            // The key is in the store: read it, then connect.
            if let Some(memory) = &mut self.memory {
                memory.restore = true;
            }
            self.restore(ConnectOrigin::Settings, cx);
        } else {
            self.connect(ConnectOrigin::Settings, cx);
        }
        cx.emit(ObservabilityEvent::Connection);
        cx.notify();
    }
    /// Settings' Forget: the store no longer keeps the key for `server`.
    pub fn forget_key_from_settings(&mut self, server: &str, cx: &mut Context<Self>) {
        self.forget_stored_key(server, cx);
    }
    /// What the form says to connect with: the address, the credentials,
    /// the sign-in and the value kept for it. The only place the key is
    /// read from the form, and it never reaches a log, an error or `Debug`.
    fn credentials(&self, cx: &App) -> (String, api::Credentials, usize, String) {
        // A pasted URL or key often brings a space or line break with it.
        let url = self.url.read(cx).value().trim().to_owned();
        let typed = self.secret.read(cx).value().trim().to_owned();
        let value = self.credential_value(&url, typed);
        let credentials = match self.auth {
            0 => api::Credentials::ApiKey(value.clone()),
            1 => api::Credentials::Session(value.clone()),
            _ => api::Credentials::None,
        };
        (url, credentials, self.auth, value)
    }
    pub(super) fn connect(&mut self, origin: ConnectOrigin, cx: &mut Context<Self>) {
        if self.fixture || (!self.live.visible && origin == ConnectOrigin::Page) {
            return;
        }
        if origin == ConnectOrigin::Change && !self.live.visible {
            self.connect_provider_only(cx);
            return;
        }
        let (url, credentials, auth, value) = self.credentials(cx);
        let (saved_url, saved_value) = (url.clone(), value);
        // Replacing credentials invalidates everything before the new read starts,
        // even if construction/authentication later fails.
        self.invalidate_connection();
        self.live.connecting = true;
        cx.emit(ObservabilityEvent::Connection);
        self.spawn_read(
            async move {
                let provider = api::Provider::new(&url, credentials)?;
                let projects = provider.projects().await?;
                Ok((provider, projects))
            },
            move |this, result, cx| {
                this.live.connecting = false;
                match result {
                    Ok((provider, projects)) => {
                        this.live.provider = Some(provider);
                        this.live.project_labels = projects
                            .iter()
                            .map(|p| format!("{} · {}", p.name, p.id))
                            .collect();
                        this.live.projects = projects;
                        this.sync_note();
                        this.settings_open = false;
                        this.remember_connection(saved_url, auth, saved_value, cx);
                        this.select_linked_project(cx);
                    }
                    Err(error) => {
                        this.live.provider = None;
                        this.live.error = Some(connect_error(error, auth));
                    }
                }
                cx.emit(ObservabilityEvent::Connection);
                cx.notify();
            },
            cx,
        );
    }
    /// The provider for a change page while this page is hidden, and
    /// nothing else: no request is made, no project listed, no read
    /// started. It is built on the I/O worker, as `connect` builds it.
    /// Showing the page finishes the connection
    /// ([`Self::finish_provider_only`]).
    fn connect_provider_only(&mut self, cx: &mut Context<Self>) {
        let (url, credentials, auth, _) = self.credentials(cx);
        self.invalidate_connection();
        let (job, receiver) = backend::spawn_job(
            &self.live.runtime,
            Duration::from_secs(5),
            api::ReadError::Timeout,
            async move { api::Provider::new(&url, credentials) },
        );
        let task = cx.spawn(async move |this, cx| {
            let result = receiver.await.unwrap_or(Err(api::ReadError::Failed));
            _ = this.update(cx, |this, cx| {
                // Replaced, shown, or connected another way meanwhile: this
                // provider is no longer the one asked for.
                if this.live.provider_job.take().is_none() || this.live.provider.is_some() {
                    return;
                }
                match result {
                    Ok(provider) => {
                        this.live.provider = Some(provider);
                        this.live.provider_only = true;
                    }
                    Err(error) => this.live.error = Some(connect_error(error, auth)),
                }
                cx.emit(ObservabilityEvent::Connection);
                cx.notify();
            });
        });
        self.live.provider_job = Some((job, task));
    }
    /// Shown, a provider made for a change page lists the projects as
    /// the first connection does, over the same provider. The page counts
    /// as connected only when the list lands: hiding it mid-listing leaves
    /// the provider as it was, to list again on the next show.
    fn finish_provider_only(&mut self, cx: &mut Context<Self>) {
        if !self.live.provider_only || self.live.connecting {
            return;
        }
        let Some(provider) = self.live.provider.clone() else {
            return;
        };
        let (url, _, auth, value) = self.credentials(cx);
        self.live.connecting = true;
        cx.emit(ObservabilityEvent::Connection);
        self.spawn_read(
            async move {
                provider
                    .projects()
                    .await
                    .map(|projects| (provider, projects))
            },
            move |this, result, cx| {
                this.live.connecting = false;
                this.live.provider_only = false;
                match result {
                    Ok((provider, projects)) => {
                        this.live.provider = Some(provider);
                        this.live.project_labels = projects
                            .iter()
                            .map(|p| format!("{} · {}", p.name, p.id))
                            .collect();
                        this.live.projects = projects;
                        this.sync_note();
                        this.settings_open = false;
                        this.remember_connection(url, auth, value, cx);
                        this.select_linked_project(cx);
                    }
                    Err(error) => {
                        this.live.provider = None;
                        this.live.error = Some(connect_error(error, auth));
                    }
                }
                cx.emit(ObservabilityEvent::Connection);
                cx.notify();
            },
            cx,
        );
    }
    pub(super) fn spawn_read<T: Send + 'static>(
        &mut self,
        work: impl Future<Output = Result<T, api::ReadError>> + Send + 'static,
        apply: impl FnOnce(&mut Self, Result<T, api::ReadError>, &mut Context<Self>) + 'static,
        cx: &mut Context<Self>,
    ) {
        let job = self.spawn_owned_read(work, apply, cx);
        self.live.jobs.push(job);
    }
    pub(super) fn spawn_owned_read<T: Send + 'static>(
        &mut self,
        work: impl Future<Output = Result<T, api::ReadError>> + Send + 'static,
        apply: impl FnOnce(&mut Self, Result<T, api::ReadError>, &mut Context<Self>) + 'static,
        cx: &mut Context<Self>,
    ) -> ReadJob {
        // The snapshots retain each report's subject and stale evidence. This
        // completion owner keys the same connection, access and window; the
        // request token replaces the page-wide cancellation generation.
        let read = Rc::new(RefCell::new(Loader::default()));
        let request = read.borrow_mut().start(Some(ReadScope {
            source: self.live.source.clone(),
            access: self.live.access.clone(),
            range: self.live.range,
        }));
        let weak = Rc::downgrade(&read);
        let (job, receiver) = backend::spawn_job(
            &self.live.runtime,
            Duration::from_secs(25),
            api::ReadError::Timeout.to_string(),
            async move { Ok(work.await) },
        );
        let task = cx.spawn(async move |this, cx| {
            let result = match receiver.await {
                Ok(Ok(result)) => result,
                Ok(Err(_)) => Err(api::ReadError::Timeout),
                Err(_) => Err(api::ReadError::Failed),
            };
            _ = this.update(cx, |this, cx| {
                let Some(read) = weak.upgrade() else {
                    return;
                };
                if !(this.live.visible || this.live.connecting)
                    || !read.borrow_mut().land(&request, Ok(()), cx)
                {
                    return;
                }
                apply(this, result, cx);
            });
        });
        read.borrow_mut().run(Some(job), task);
        read
    }
    fn read_applications(
        &mut self,
        provider: api::Provider,
        source: api::Source,
        identity: ReadIdentity,
        cx: &mut Context<Self>,
    ) {
        let range = self.live.range;
        let request = self.live.apps.begin(identity);
        self.spawn_read(
            async move { provider.applications(&source, range).await },
            move |this, result, cx| {
                this.live.capabilities[0] = api::Capability::from_result(&result);
                if this
                    .live
                    .apps
                    .apply(&request, result.map_err(|e| e.to_string()))
                {
                    if let Some(raw) = this.live.apps.data().cloned() {
                        this.apply_applications(&raw);
                    }
                    cx.notify();
                }
            },
            cx,
        );
    }
    pub(crate) fn refresh(&mut self, cx: &mut Context<Self>) {
        self.live.cancel();
        self.seek_revision();
        if self.fixture {
            if self.destination == Destination::Application {
                self.prepare_report();
                self.read_embedded(cx);
            }
            self.answer_example_incidents();
            if self.destination == Destination::Deployments && !self.prepare_revisions(cx) {
                self.read_revision(cx);
            }
            if self.destination == Destination::Traces {
                self.read_traces(cx);
            }
            cx.notify();
            return;
        }
        if !self.live.visible {
            return;
        }
        let (Some(provider), Some(source)) = (self.live.provider.clone(), self.live.source.clone())
        else {
            cx.notify();
            return;
        };
        let identity = self
            .live
            .identity(Subject::Applications)
            .expect("selected source");
        let range = self.live.range;
        match self.destination {
            Destination::Incidents => self.read_incidents(provider, source, range, cx),
            Destination::Applications => self.read_applications(provider, source, identity, cx),
            Destination::Traces | Destination::Profiling => {
                if self.applications.is_empty() {
                    self.read_applications(provider, source, identity, cx);
                }
                if self.destination == Destination::Traces {
                    self.read_traces(cx);
                } else {
                    self.read_profiling(cx);
                }
            }
            Destination::ServiceMap => {
                let request = self.live.map.begin(ReadIdentity {
                    subject: Subject::Map,
                    ..identity
                });
                self.spawn_read(
                    async move { provider.service_map(&source, range).await },
                    move |this, result, cx| {
                        this.live.capabilities[1] = api::Capability::from_result(&result);
                        if this
                            .live
                            .map
                            .apply(&request, result.map_err(|e| e.to_string()))
                        {
                            if let Some(raw) = this.live.map.data() {
                                (this.nodes, this.connections) = projection::map(raw);
                                this.prepare_map();
                            }
                            cx.notify();
                        }
                    },
                    cx,
                );
            }
            Destination::Application => {
                if self.selected_app.is_none() {
                    cx.notify();
                    return;
                }
                self.read_view(provider.clone(), source.clone(), cx);
                self.read_embedded(cx);
            }
            Destination::Deployments => {
                if self.applications.is_empty() {
                    self.read_applications(provider.clone(), source.clone(), identity, cx);
                }
                if self.selected_app.is_some() {
                    // What the page already read shows while it's read
                    // again: never an empty list it hasn't answered.
                    let reading = self.prepare_revisions(cx);
                    self.read_view(provider, source, cx);
                    if !reading {
                        self.read_revision(cx);
                    }
                }
            }
        }
        cx.notify();
    }
}

/// Coroot has two kinds of API key, and only a user's reads its API.
fn connect_error(error: api::ReadError, auth: usize) -> String {
    if error == api::ReadError::Authentication && auth == 0 {
        format!(
            "{error} Coroot reads need a user API key (crt_…), made under the user menu → API keys; a project's API keys only send data."
        )
    } else {
        error.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::{ReadIdentity, Subject};
    use crate::state::Snapshot;
    use freshkube_core::coroot::{
        AppId, Association, Credentials, ProjectInfo, Provider, TimeRange,
    };
    #[test]
    fn day_and_week_tooltips_include_both_local_dates() {
        let end = chrono::DateTime::from_timestamp(1_790_000_000, 0).unwrap();
        for hours in [24, 168] {
            let range = super::range(hours, end);
            let label = super::range_label(range);
            let (from, to) = label.split_once(" — ").unwrap();
            assert_eq!(
                from,
                range
                    .from
                    .unwrap()
                    .with_timezone(&chrono::Local)
                    .format("%Y-%m-%d %H:%M:%S")
                    .to_string()
            );
            assert_eq!(
                to,
                end.with_timezone(&chrono::Local)
                    .format("%Y-%m-%d %H:%M:%S")
                    .to_string()
            );
            assert_ne!(&from[..10], &to[..10]);
        }
    }

    #[test]
    fn every_request_coordinate_rejects_delayed_success_and_failure() {
        let provider = Provider::new("https://coroot.example.com", Credentials::None).unwrap();
        let project = ProjectInfo {
            id: "p1".into(),
            name: "Production".into(),
        };
        let end = chrono::DateTime::from_timestamp(1_790_000_000, 0).unwrap();
        let original = ReadIdentity {
            source: provider.source(&project),
            access: Some("access:a".into()),
            range: TimeRange::between(end - chrono::Duration::hours(1), end),
            subject: Subject::View(AppId::new("c:ns:Deployment:a")),
        };
        let credential = Provider::new(
            "https://coroot.example.com",
            Credentials::ApiKey("changed".into()),
        )
        .unwrap();
        let server = Provider::new("https://other.example.com", Credentials::None).unwrap();
        let changes = [
            ReadIdentity {
                source: credential.source(&project),
                ..original.clone()
            },
            ReadIdentity {
                source: server.source(&project),
                ..original.clone()
            },
            ReadIdentity {
                source: provider.source(&ProjectInfo {
                    id: "p2".into(),
                    name: "Production".into(),
                }),
                ..original.clone()
            },
            ReadIdentity {
                access: Some("access:b".into()),
                ..original.clone()
            },
            ReadIdentity {
                source: original
                    .source
                    .clone()
                    .with_association(Some(Association::new(
                        "access:a".into(),
                        "another-coroot-cluster".into(),
                    ))),
                ..original.clone()
            },
            ReadIdentity {
                range: TimeRange::between(end - chrono::Duration::hours(24), end),
                ..original.clone()
            },
            ReadIdentity {
                subject: Subject::View(AppId::new("c:ns:Deployment:b")),
                ..original.clone()
            },
            ReadIdentity {
                subject: Subject::Map,
                ..original.clone()
            },
            original.clone(), // a new generation with the same identity
        ];
        for changed in changes {
            let mut state = Snapshot::<i32, ReadIdentity, String>::default();
            let old = state.begin(original.clone());
            let current = state.begin(changed);
            assert!(!state.apply(&old, Ok(99)));
            assert!(!state.apply(&old, Err("obsolete failure".into())));
            assert!(state.apply(&current, Ok(42)));
            assert_eq!(state.data(), Some(&42));
        }
    }
}
