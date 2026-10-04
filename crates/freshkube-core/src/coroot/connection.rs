use std::{
    future::Future,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    time::Duration,
};

use super::*;
use tokio::sync::Semaphore;

const DEADLINE: Duration = Duration::from_secs(20);

/// A process-local connection revision, replaced even for the same URL/key.
/// It contains no credential or credential fingerprint.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ProviderId(u64);

#[derive(Clone)]
pub struct Provider {
    id: ProviderId,
    pub(super) client: coroot_rs::Client,
    slots: Arc<Semaphore>,
}

impl Provider {
    /// Construct on the I/O worker. No default URL, profile or credentials are read.
    pub fn new(url: &str, credentials: Credentials) -> Result<Self, ReadError> {
        if matches!(&credentials, Credentials::ApiKey(value) | Credentials::Session(value) if value.trim().is_empty())
        {
            return Err(ReadError::Authentication);
        }
        let parsed = url::Url::parse(url).map_err(|_| ReadError::InvalidUrl)?;
        if !matches!(parsed.scheme(), "http" | "https")
            || parsed.host_str().is_none()
            || !parsed.username().is_empty()
            || parsed.password().is_some()
            || parsed.query().is_some()
            || parsed.fragment().is_some()
        {
            return Err(ReadError::InvalidUrl);
        }
        let http = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .connect_timeout(Duration::from_secs(5))
            .timeout(DEADLINE)
            .build()
            .map_err(|_| ReadError::Unreachable)?;
        let client = coroot_rs::Client::builder(url)
            .credentials(credentials)
            .max_response_bytes(8 * 1024 * 1024)
            .http_client(http)
            .build()
            .map_err(ReadError::from)?;
        static NEXT: AtomicU64 = AtomicU64::new(1);
        Ok(Self {
            id: ProviderId(NEXT.fetch_add(1, Ordering::Relaxed)),
            client,
            slots: Arc::new(Semaphore::new(2)),
        })
    }

    pub fn id(&self) -> ProviderId {
        self.id
    }
    pub fn url(&self) -> &str {
        self.client.base_url().as_str()
    }
    pub fn has_api_key(&self) -> bool {
        self.client.credentials().is_api_key()
    }

    pub(super) async fn read<T>(
        &self,
        work: impl Future<Output = coroot_rs::Result<T>>,
    ) -> Result<T, ReadError> {
        tokio::time::timeout(DEADLINE, async {
            let _slot = self.slots.acquire().await.map_err(|_| ReadError::Failed)?;
            work.await.map_err(ReadError::from)
        })
        .await
        .map_err(|_| ReadError::Timeout)?
    }

    pub async fn projects(&self) -> Result<Vec<ProjectInfo>, ReadError> {
        let projects = self.read(self.client.projects()).await?;
        limits::projects(&projects)?;
        Ok(projects)
    }

    /// The ID must come from explicit project selection, not a name lookup.
    pub fn source(&self, project: &ProjectInfo) -> Source {
        Source {
            provider: self.id,
            project: project.id.clone(),
            association: None,
        }
    }

    pub(super) fn project(
        &self,
        source: &Source,
        range: TimeRange,
    ) -> Result<coroot_rs::Project, ReadError> {
        if source.provider != self.id || source.project.is_empty() || source.project.len() > 256 {
            return Err(ReadError::InvalidSelection);
        }
        let (Some(from), Some(to)) = (range.from, range.to) else {
            return Err(ReadError::InvalidSelection);
        };
        if !(60..=7 * 86_400).contains(&(to - from).num_seconds()) {
            return Err(ReadError::InvalidSelection);
        }
        Ok(self.client.project(&source.project).with_range(range))
    }

    pub async fn applications(
        &self,
        source: &Source,
        range: TimeRange,
    ) -> Result<Vec<Application>, ReadError> {
        let project = self.project(source, range)?;
        let apps = self.read(project.applications()).await?;
        limits::applications(&apps)?;
        Ok(apps)
    }

    pub async fn service_map(
        &self,
        source: &Source,
        range: TimeRange,
    ) -> Result<ServiceMap, ReadError> {
        let project = self.project(source, range)?;
        let map = self.read(project.service_map()).await?;
        limits::map(&map)?;
        Ok(map)
    }

    pub async fn reports(
        &self,
        source: &Source,
        range: TimeRange,
        app: &AppId,
        extended: bool,
    ) -> Result<AppHealth, ReadError> {
        let project = self.project(source, range)?;
        if extended && !self.has_api_key() {
            return Err(ReadError::Unsupported);
        }
        let mut health = if extended {
            self.read(project.app_health(app)).await?
        } else {
            self.read(project.app_health_rest(app)).await?
        };
        if health.id != *app {
            return Err(ReadError::InvalidResponse);
        }
        limits::health(&health)?;
        plain_health(&mut health);
        Ok(health)
    }

    /// Latest project incidents, in server order, across all states. This bounded
    /// sample is not a history filtered by `range` or a complete incident count.
    pub async fn incidents(
        &self,
        source: &Source,
        range: TimeRange,
    ) -> Result<Vec<Incident>, ReadError> {
        let project = self.project(source, range)?;
        let query = IncidentQuery {
            app: None,
            state: StateFilter::Any,
            limit: 100,
        };
        let values = self.read(project.incidents(&query)).await?;
        limits::incidents(&values)?;
        Ok(values)
    }

    /// Detail uses Coroot's incident time context. The list's application identity
    /// is revalidated before this observation can be shown or linked.
    pub async fn incident(
        &self,
        source: &Source,
        range: TimeRange,
        key: &str,
        app: &AppId,
    ) -> Result<IncidentView, ReadError> {
        let project = self.project(source, range)?;
        if key.is_empty() || key.len() > 256 || app.as_str().is_empty() {
            return Err(ReadError::InvalidSelection);
        }
        let value = self.read(project.incident_view(key)).await?;
        if value.incident().key != key || value.incident().app != *app {
            return Err(ReadError::InvalidResponse);
        }
        limits::incident_view(&value)?;
        Ok(value)
    }
}

/// Explicit mapping of one Coroot cluster ID to the shell's canonical
/// `KubeSource.id` (the AccessIdentity key), never a context/display name.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Association {
    access: String,
    cluster: String,
}
impl Association {
    pub fn new(access: String, cluster: String) -> Self {
        Self { access, cluster }
    }
    pub fn access(&self) -> &str {
        &self.access
    }
    pub fn cluster(&self) -> &str {
        &self.cluster
    }
}

/// Provider, credential lifetime, selected project and optional cluster mapping.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Source {
    provider: ProviderId,
    project: String,
    association: Option<Association>,
}
impl Source {
    pub fn provider(&self) -> ProviderId {
        self.provider
    }
    pub fn project(&self) -> &str {
        &self.project
    }
    pub fn association(&self) -> Option<&Association> {
        self.association.as_ref()
    }
    pub fn with_association(mut self, association: Option<Association>) -> Self {
        self.association = association;
        self
    }
}

/// Coroot writes report titles and messages for its web page, with markup.
pub(super) fn plain_health(health: &mut AppHealth) {
    let plain = super::tracing::plain;
    for report in &mut health.reports {
        for issue in &mut report.issues {
            issue.title = plain(&issue.title);
            issue.message = plain(&issue.message);
        }
        for chart in &mut report.charts {
            chart.title = plain(&chart.title);
        }
    }
    for dependency in &mut health.dependencies {
        dependency.connectivity_message = plain(&dependency.connectivity_message);
    }
}
