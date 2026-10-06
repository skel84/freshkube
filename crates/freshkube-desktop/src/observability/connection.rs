//! Connection lifetime and request guards owned by ObservabilityPage.
use super::*;
use crate::{
    backend::{self, OwnedJob},
    resources::KubeSource,
    state::Snapshot,
};
use freshkube_core::coroot as api;
use std::{future::Future, time::Duration};
use tokio::runtime::Handle;

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
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct ReadIdentity {
    source: api::Source,
    access: Option<String>,
    range: api::TimeRange,
    subject: Subject,
}
pub(super) struct ReadJob {
    _job: OwnedJob,
    _task: Task<()>,
}
pub(super) struct Live {
    pub runtime: Handle,
    pub visible: bool,
    pub access: Option<String>,
    pub provider: Option<api::Provider>,
    pub source: Option<api::Source>,
    pub projects: Vec<api::ProjectInfo>,
    pub project_label: String,
    pub project_labels: Vec<String>,
    pub generation: u64,
    pub jobs: Vec<ReadJob>,
    pub incident_job: Option<ReadJob>,
    pub traces_job: Option<ReadJob>,
    pub trace_job: Option<ReadJob>,
    pub profile_job: Option<ReadJob>,
    pub range: api::TimeRange,
    clock_origin: std::time::Instant,
    time_origin: chrono::DateTime<chrono::Utc>,
    pub range_label: String,
    pub connecting: bool,
    pub error: Option<String>,
    pub apps: Snapshot<Vec<api::Application>, ReadIdentity>,
    pub map: Snapshot<api::ServiceMap, ReadIdentity>,
    pub view: Snapshot<api::AppView, ReadIdentity>,
    /// The view's last read was refused, rather than failed.
    pub view_refused: bool,
    pub incidents: Snapshot<Vec<api::Incident>, ReadIdentity>,
    pub incident: Snapshot<api::IncidentView, ReadIdentity>,
    pub tracing: Snapshot<api::Tracing, ReadIdentity>,
    pub trace: Snapshot<api::Tracing, ReadIdentity>,
    pub profiling: Snapshot<api::Profiling, ReadIdentity>,
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
            access: None,
            provider: None,
            source: None,
            projects: vec![],
            project_label: "Choose a project".into(),
            project_labels: vec![],
            generation: 0,
            jobs: vec![],
            incident_job: None,
            traces_job: None,
            trace_job: None,
            profile_job: None,
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
            incidents: Snapshot::default(),
            incident: Snapshot::default(),
            tracing: Snapshot::default(),
            trace: Snapshot::default(),
            profiling: Snapshot::default(),
            capabilities: [api::Capability::Unchecked; 2],
        }
    }
    fn cancel(&mut self) {
        self.generation += 1;
        self.jobs.clear();
        self.incident_job = None;
        self.traces_job = None;
        self.trace_job = None;
        self.profile_job = None;
        self.connecting = false;
    }
    fn clear(&mut self) {
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
    pub(crate) fn link_is_current(
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

    pub(crate) fn set_source(&mut self, source: Option<KubeSource>, cx: &mut Context<Self>) {
        let access = source.map(|s| s.id);
        if self.live.access == access {
            return;
        }
        self.live.access = access;
        if let Some(source) = self.live.source.take() {
            self.live.source = Some(source.with_association(None));
        }
        self.clear_observations();
        self.refresh(cx);
    }
    pub(crate) fn set_visible(&mut self, visible: bool, cx: &mut Context<Self>) {
        if self.live.visible == visible {
            return;
        }
        self.live.visible = visible;
        if visible {
            self.refresh_current(cx);
            self.restore(cx);
        } else {
            self.live.cancel();
        }
    }
    pub(super) fn clear_observations(&mut self) {
        self.live.clear();
        self.incident_observations = self.incident_observations.cleared();
        self.live_traces = Default::default();
        self.live_profiles = Default::default();
        if !self.fixture {
            self.applications.clear();
            self.nodes = Default::default();
            self.connections = Default::default();
            self.map_display = Default::default();
            self.map_page = 0;
            self.matrix.clear();
            self.selected_app = None;
            self.selected_link = None;
            self.report_snapshot = None;
            self.categories = Default::default();
            self.namespaces = Default::default();
            self.cluster_ids.clear();
            self.counts = [0; 7];
            self.active_categories = Rc::new(["application".into()].into());
            self.all_categories = false;
            self.category_defaults_pending = true;
            self.shown_apps = 0;
            self.app_count = "0 apps".into();
            self.prepare_application_columns();
            self.namespace = None;
        }
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
        self.live_traces.reset();
        if !self.fixture {
            self.applications.clear();
            self.nodes = Default::default();
            self.connections = Default::default();
            self.map_display = Default::default();
            self.map_page = 0;
            self.project();
            self.prepare_application_columns();
            self.report_snapshot = None;
        }
        self.refresh(cx);
    }
    /// A user refresh/reopen starts a new absolute observation window. A retry
    /// and navigation among reports keep the captured window.
    pub(crate) fn refresh_current(&mut self, cx: &mut Context<Self>) {
        self.range_changed(cx);
    }
    pub(super) fn select_project(&mut self, project: &api::ProjectInfo, cx: &mut Context<Self>) {
        let Some(provider) = &self.live.provider else {
            return;
        };
        if self.live.source.as_ref().is_some_and(|source| {
            source.provider() == provider.id() && source.project() == project.id
        }) {
            return;
        }
        self.live.source = Some(provider.source(project));
        self.live.project_label = format!("{} · {}", project.name, project.id);
        self.remember_project(&project.id, cx);
        self.clear_observations();
        self.refresh_current(cx);
    }
    pub(super) fn associate(&mut self, cluster: Option<String>, cx: &mut Context<Self>) {
        let (Some(access), Some(source)) = (&self.live.access, &self.live.source) else {
            return;
        };
        self.live.source = Some(source.clone().with_association(
            cluster.map(|cluster| api::Association::new(access.clone(), cluster)),
        ));
        self.live.clear();
        self.incident_observations.clear_evidence();
        self.live_traces.reset();
        self.report_snapshot = None;
        self.refresh(cx);
    }
    pub(super) fn invalidate_connection(&mut self) {
        self.live.provider = None;
        self.live.source = None;
        self.live.projects.clear();
        self.live.project_labels.clear();
        self.live.project_label = "Choose a project".into();
        self.live.error = None;
        self.clear_observations();
    }
    pub(super) fn disconnect(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.invalidate_connection();
        self.forget_connection(cx);
        self.secret
            .update(cx, |input, cx| input.set_value("", window, cx));
        self.settings_open = true;
        cx.notify();
    }
    pub(super) fn connect(&mut self, cx: &mut Context<Self>) {
        if self.fixture || !self.live.visible {
            return;
        }
        // A pasted URL or key often brings a space or line break with it.
        let url = self.url.read(cx).value().trim().to_owned();
        let typed = self.secret.read(cx).value().trim().to_owned();
        let value = self.credential_value(&url, typed);
        let (auth, saved_url, saved_value) = (self.auth, url.clone(), value.clone());
        let credentials = match self.auth {
            0 => api::Credentials::ApiKey(value),
            1 => api::Credentials::Session(value),
            _ => api::Credentials::None,
        };
        // Replacing credentials invalidates everything before the new read starts,
        // even if construction/authentication later fails.
        self.invalidate_connection();
        self.live.connecting = true;
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
                        this.settings_open = false;
                        this.remember_connection(saved_url, auth, saved_value, cx);
                    }
                    Err(error) => this.live.error = Some(connect_error(error, auth)),
                }
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
        let generation = self.live.generation;
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
                if !this.live.visible || this.live.generation != generation {
                    return;
                }
                apply(this, result, cx);
            });
        });
        ReadJob {
            _job: job,
            _task: task,
        }
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
        if self.fixture {
            if self.destination == Destination::Application {
                self.prepare_report();
                self.read_embedded(cx);
            }
            self.answer_example_incidents();
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
                                this.cluster_ids = raw
                                    .nodes
                                    .iter()
                                    .filter(|n| !n.id.is_external())
                                    .map(|n| n.id.cluster_id().to_string())
                                    .filter(|id| !id.is_empty())
                                    .collect::<std::collections::BTreeSet<_>>()
                                    .into_iter()
                                    .collect();
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
            _ => {}
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
            let mut state = Snapshot::default();
            let old = state.begin(original.clone());
            let current = state.begin(changed);
            assert!(!state.apply(&old, Ok(99)));
            assert!(!state.apply(&old, Err("obsolete failure".into())));
            assert!(state.apply(&current, Ok(42)));
            assert_eq!(state.data(), Some(&42));
        }
    }
}
