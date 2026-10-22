//! What Coroot found after a deploy: the outcome of looking, from a
//! Deployment hop's workspace entry, namespace, name and pod-template hash,
//! for Coroot's revision and the window around its start
//! (`docs/COROOT.md#what-its-metrics-did-after-it`).
//!
//! Every way a look can end is its own [`Outcome`], with its why, and none
//! reads as fine. Coroot's analysis stays Coroot's: the status and findings
//! are its own words on the [`DeploymentRevision`], and nothing is computed
//! across the split at the start. Coroot's verdict is not an input to a hop's
//! status chips.
//!
//! **Another entry's application never answers for this one.** An
//! [`Answer`] carries the [`Query`] it was read for, which names the entry
//! and the link; a caller publishes it only when [`Answer::is_for`] holds for
//! the query it now shows, besides its request generation. The application
//! page's own id is checked against the one asked for inside
//! [`Provider::app_view`](Provider), and the revision comes from that page's
//! own revisions.
use super::*;
use crate::workspace::Entry;
use chrono::{DateTime, Utc};
use std::time::Duration;

/// The longest a whole look may take: the applications list, the page and
/// the window, each with the provider's own deadline besides.
const LOOK: Duration = Duration::from_secs(50);

/// How far back the applications list and the application's page are read
/// to find the application and its revisions.
const SURVEY: Duration = Duration::from_secs(3600);

/// Which workspace entry a look is for, and the Coroot project and cluster
/// that entry is.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Link {
    entry: String,
    project: String,
    cluster: String,
}

impl Link {
    /// An entry's link: its `coroot` key, with the project its own or else
    /// the connection's. Matching names link nothing; an entry without the
    /// key is not linked, even when the workspace has one entry.
    pub fn of(entry: &Entry, connection_project: Option<&str>) -> Result<Self, Outcome> {
        let Some(link) = &entry.coroot else {
            return Err(Outcome::NotLinked);
        };
        let cluster = link.cluster.trim();
        if cluster.is_empty() {
            return Err(Outcome::NotLinked);
        }
        // An explicit blank project is no project, as none at all is: the
        // cluster is named, so the entry is linked and lacks its project.
        let project = link
            .project
            .as_deref()
            .or(connection_project)
            .map(str::trim)
            .filter(|project| !project.is_empty());
        match project {
            Some(project) => Ok(Self {
                entry: entry.id.clone(),
                project: project.to_owned(),
                cluster: cluster.to_owned(),
            }),
            None => Err(Outcome::NoProjectForEntry),
        }
    }

    pub fn entry(&self) -> &str {
        &self.entry
    }
    pub fn project(&self) -> &str {
        &self.project
    }
    pub fn cluster(&self) -> &str {
        &self.cluster
    }
}

impl Provider {
    /// The source of the project a workspace entry's link names, which is
    /// not one a person selected from [`projects`](Self::projects).
    pub fn link_source(&self, link: &Link) -> Source {
        self.linked_source(&link.project)
    }
}

/// What kind of workload the hop is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Workload {
    Deployment,
    /// An Argo Rollout: Coroot keeps revisions of Deployments only.
    Rollout,
}

/// A look at one hop.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Query {
    pub link: Link,
    pub workload: Workload,
    pub namespace: String,
    pub name: String,
    /// The current ReplicaSet's `pod-template-hash`; none when the
    /// ReplicaSet isn't known or its pods aren't confirmed to run the
    /// Freight, so nothing is read.
    pub hash: Option<String>,
}

impl Query {
    /// Coroot's application for a Deployment of this link's cluster.
    fn app(&self) -> AppId {
        AppId::new(format!(
            "{}:{}:Deployment:{}",
            self.link.cluster, self.namespace, self.name
        ))
    }
}

/// Which read a refusal was for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Refused {
    /// The applications list or the application's page.
    Application,
    /// The window around the revision.
    Window,
}

/// How a look ended.
#[derive(Clone, Debug)]
pub enum Outcome {
    Found(Box<RevisionView>),
    /// Found, and the window's charts stop before its end, at the time given.
    EndsEarly(Box<RevisionView>, DateTime<Utc>),
    /// The application's revisions don't hold the hash.
    NoRevision,
    /// Coroot lists no such application in the project.
    NoApplication,
    /// Coroot lists no such project.
    NoProject,
    /// Coroot isn't connected. Made by the caller: there is no provider.
    NoConnection,
    /// The entry names no Coroot cluster.
    NotLinked,
    /// The entry names a Coroot cluster, and neither it nor the connection
    /// names a project.
    NoProjectForEntry,
    Refused(Refused),
    /// Coroot has no data for the window.
    NoData,
    Failed(ReadError),
    /// An Argo Rollout: Coroot keeps revisions of Deployments only.
    NotDeployment,
    /// The hop's ReplicaSet isn't known or its pods aren't confirmed to run
    /// the Freight, so its revision may be another change's.
    NotLooked,
}

/// The applications of a link's project, with the link they were read for.
/// A list from another link, or another connection, is never trusted.
#[derive(Clone, Debug)]
pub struct Listed {
    provider: ProviderId,
    link: Link,
    apps: Vec<Application>,
}

impl Listed {
    pub fn new(provider: ProviderId, link: Link, apps: Vec<Application>) -> Self {
        Self {
            provider,
            link,
            apps,
        }
    }

    fn holds_for(&self, provider: ProviderId, link: &Link) -> bool {
        self.provider == provider && self.link == *link
    }
}

/// An outcome with the question it answers.
#[derive(Clone, Debug)]
pub struct Answer {
    pub query: Query,
    pub window: RevisionWindow,
    /// The connection it was read over; a reconnected server has another.
    pub provider: ProviderId,
    pub outcome: Outcome,
    /// The project's applications, when this look read them itself, so a
    /// caller can hand them to the next look of the same link.
    pub listed: Option<Listed>,
}

impl Answer {
    /// Whether this answers `query` over `window` on `provider`'s connection:
    /// the same entry, link, workload, hash, window and connection. Editing
    /// an entry's link, asking another window or reconnecting makes every
    /// earlier answer foreign.
    pub fn is_for(&self, query: &Query, window: RevisionWindow, provider: ProviderId) -> bool {
        self.query == *query && self.window == window && self.provider == provider
    }
}

impl Provider {
    /// Looks for the revision `query` names and what Coroot saw around its
    /// start. `listed` is the project's applications when the caller holds
    /// them already, so a second hop of the same link costs two requests
    /// instead of three. Dropping the future cancels the look; it ends in
    /// [`LOOK`] at the longest.
    pub async fn outcome(
        &self,
        query: &Query,
        listed: Option<&Listed>,
        window: RevisionWindow,
        now: DateTime<Utc>,
    ) -> Answer {
        self.outcome_within(LOOK, query, listed, window, now).await
    }

    /// [`outcome`](Self::outcome) with the whole look's limit given, so a
    /// test can end one on the paused clock.
    pub(super) async fn outcome_within(
        &self,
        limit: Duration,
        query: &Query,
        listed: Option<&Listed>,
        window: RevisionWindow,
        now: DateTime<Utc>,
    ) -> Answer {
        let (outcome, read) =
            match tokio::time::timeout(limit, self.look(query, listed, window, now)).await {
                Ok(looked) => looked,
                Err(_) => (Outcome::Failed(ReadError::Timeout), None),
            };
        Answer {
            query: query.clone(),
            window,
            provider: self.id(),
            outcome,
            listed: read,
        }
    }

    async fn look(
        &self,
        query: &Query,
        listed: Option<&Listed>,
        window: RevisionWindow,
        now: DateTime<Utc>,
    ) -> (Outcome, Option<Listed>) {
        if query.workload != Workload::Deployment {
            return (Outcome::NotDeployment, None);
        }
        let Some(hash) = query.hash.as_deref().filter(|hash| !hash.is_empty()) else {
            return (Outcome::NotLooked, None);
        };
        let source = self.linked_source(&query.link.project);
        let survey = TimeRange::between(
            now - chrono::Duration::from_std(SURVEY).unwrap_or_default(),
            now,
        );
        let app = query.app();
        let mut read = None;
        let listed = match listed.filter(|l| l.holds_for(self.id(), &query.link)) {
            Some(listed) => listed,
            None => {
                let applications = match self.applications(&source, survey).await {
                    Ok(applications) => applications,
                    // The list belongs to the project: its 404 is no project.
                    Err(ReadError::Missing) => return (Outcome::NoProject, None),
                    Err(error) => return (failed(error, Refused::Application), None),
                };
                &*read.insert(Listed::new(self.id(), query.link.clone(), applications))
            }
        };
        let rest = async {
            if !listed.apps.iter().any(|a| a.id == app) {
                return Outcome::NoApplication;
            }
            let page = match self.app_view(&source, survey, &app).await {
                Ok(page) => page,
                Err(ReadError::Missing) => return Outcome::NoApplication,
                Err(error) => return failed(error, Refused::Application),
            };
            let revisions = match page.revisions {
                Some(Ok(revisions)) => revisions,
                Some(Err(error)) => return failed(error, Refused::Application),
                None => return Outcome::Failed(ReadError::InvalidResponse),
            };
            let Some(revision) = revisions.iter().find(|revision| revision.hash == hash) else {
                return Outcome::NoRevision;
            };
            match self.revision_view(&source, &app, revision, window).await {
                Ok(view) => Outcome::of_window(view),
                Err(error) => failed(error, Refused::Window),
            }
        };
        let outcome = rest.await;
        (outcome, read)
    }
}

fn failed(error: ReadError, read: Refused) -> Outcome {
    match error {
        ReadError::Refused => Outcome::Refused(read),
        error => Outcome::Failed(error),
    }
}

impl Outcome {
    /// What the window around a revision holds: Coroot's page with
    /// histories, none at all, or histories that stop before the window's
    /// end.
    pub fn of_window(view: RevisionView) -> Self {
        found(view)
    }
}

fn found(view: RevisionView) -> Outcome {
    let Around::View(page) = &view.answer else {
        return Outcome::NoData;
    };
    if let Some(error) = page.history_error {
        return Outcome::Failed(error);
    }
    let mut last: Option<DateTime<Utc>> = None;
    for report in &page.reports {
        for widget in &report.widgets {
            let charts: &[AppChart] = match &widget.kind {
                WidgetKind::Chart(chart) => std::slice::from_ref(chart),
                WidgetKind::ChartGroup { charts, .. } => charts,
                _ => continue,
            };
            for history in charts.iter().filter_map(|chart| chart.history.as_deref()) {
                last = last.max(Some(history.to));
            }
        }
    }
    match last {
        None => Outcome::NoData,
        Some(at) if at < view.to => Outcome::EndsEarly(Box::new(view), at),
        Some(_) => Outcome::Found(Box::new(view)),
    }
}
