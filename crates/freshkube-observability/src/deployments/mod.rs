//! Deployments: the chosen application's revisions as Coroot keeps them
//! and, for the selected one, Coroot's findings and the application's
//! charts around its start, split at it. Nothing is compared across the
//! split here: the findings are Coroot's own words, and the charts only say
//! which samples fall on each side.
use super::*;
use crate::application::{AppPage, Charts, example as app_example};
use crate::connection::{ReadJob, Subject};
use crate::tables::{ColumnKind, PageColumn, TableKey};
use freshkube_core::coroot as api;
mod detail;
mod table;
#[cfg(test)]
mod tests;
pub(super) use table::RevisionCells;

/// The windows around a start the inspector offers: each side's seconds,
/// and their name.
pub(super) const WINDOWS: [(u64, &str); 3] =
    [(1800, "±30 min"), (3600, "±1 h"), (3 * 3600, "±3 h")];

#[derive(Default)]
pub(super) struct Revisions {
    /// The application the rows are of.
    app: Option<api::AppId>,
    /// By revision id, never by position.
    selected: Option<TableKey>,
    rows: Vec<Row>,
    /// Why Coroot's revisions couldn't be read from the application's page.
    list_error: Option<String>,
    columns: Vec<PageColumn>,
    width: f32,
    /// What the status bar counts.
    count: String,
    /// Which of `WINDOWS` the inspector reads.
    window: usize,
    /// The report whose charts the inspector shows.
    report: String,
    /// Example data's answer for the window, as Coroot's would be.
    example: Option<Result<api::RevisionView, api::ReadError>>,
    detail: Option<detail::Detail>,
}

struct Row {
    key: TableKey,
    id: SharedString,
    revision: api::DeploymentRevision,
    status: Status,
    hash: SharedString,
    /// Coroot's label without its hash: the images it knows.
    image: SharedString,
    started: SharedString,
    /// Coroot's first finding, or its note.
    finding: SharedString,
    /// The whole row in words, for its tooltip and accessibility label.
    label: SharedString,
}

impl Revisions {
    /// Nothing from the last connection or application but the window and
    /// the report chosen.
    pub(super) fn cleared(&mut self) -> Self {
        Self {
            window: self.window,
            report: std::mem::take(&mut self.report),
            ..Default::default()
        }
    }

    /// Forgets the window's answer, which is read again.
    pub(super) fn clear_evidence(&mut self) {
        self.example = None;
        self.detail = None;
    }

    /// Derives the rows of `app`'s revisions. True when the selection
    /// changed, so the window is read again.
    fn prepare_list(
        &mut self,
        app: Option<&api::AppId>,
        revisions: Option<&Result<Vec<api::DeploymentRevision>, api::ReadError>>,
    ) -> bool {
        let previous = self.selected.clone();
        if self.app.as_ref() != app {
            *self = self.cleared();
            self.app = app.cloned();
        }
        let values = match revisions {
            Some(Ok(values)) => values.as_slice(),
            _ => &[],
        };
        self.list_error = match revisions {
            Some(Err(error)) => Some(error.to_string()),
            _ => None,
        };
        if !values
            .iter()
            .any(|r| self.selected.as_ref() == Some(&key(r)))
        {
            self.selected = values.first().map(key);
        }
        self.rows = values.iter().map(row).collect();
        self.count = match values.len() {
            1 => "1 deployment".into(),
            n => format!("{n} deployments"),
        };
        previous != self.selected
    }

    #[cfg(test)]
    pub(crate) fn list_error(&self) -> Option<&String> {
        self.list_error.as_ref()
    }

    fn selected(&self) -> Option<&Row> {
        self.rows
            .iter()
            .find(|r| Some(&r.key) == self.selected.as_ref())
    }

    fn window(&self) -> api::RevisionWindow {
        api::RevisionWindow::both(std::time::Duration::from_secs(WINDOWS[self.window].0))
    }
}

fn key(revision: &api::DeploymentRevision) -> TableKey {
    TableKey::Revision(revision.id.clone())
}

/// Coroot's label is the hash, then the images it knows.
fn image(revision: &api::DeploymentRevision) -> String {
    let rest = revision
        .version
        .strip_prefix(revision.hash.as_str())
        .map_or(revision.version.as_str(), |rest| {
            rest.trim_start_matches(':').trim()
        });
    if rest.is_empty() { "—" } else { rest }.to_owned()
}

fn row(revision: &api::DeploymentRevision) -> Row {
    let status = Status::from(revision.status);
    let image = image(revision);
    let started = format::local_time(revision.started_at);
    let finding = match revision.findings.as_slice() {
        [] => revision.note.clone().unwrap_or_else(|| "—".into()),
        [one] => one.message.clone(),
        [first, rest @ ..] => format!("{} (+{} more)", first.message, rest.len()),
    };
    Row {
        key: key(revision),
        id: format!("obs-revision-{}", revision.id.replace(':', "-")).into(),
        label: format!(
            "{} · {image} · started {started} · {} · {finding}",
            revision.hash,
            status.label()
        )
        .into(),
        revision: revision.clone(),
        status,
        hash: revision.hash.clone().into(),
        image: image.into(),
        started: started.into(),
        finding: finding.into(),
    }
}

impl ObservabilityPage {
    /// Derives the rows from the chosen application's page, Coroot's or
    /// the example's, and reads the selected revision's window again when
    /// the selection changed.
    pub(super) fn prepare_revisions(&mut self, cx: &mut Context<Self>) {
        if self.fixture && self.hold {
            return;
        }
        let app = self.selected_app.clone();
        let example;
        let view = match &app {
            None => None,
            Some(app) if self.fixture => {
                example = app_example::app_view(app);
                Some(&example)
            }
            Some(app) => self.live.view.data().filter(|view| view.map.app.id == *app),
        };
        let revisions = view.and_then(|view| view.revisions.as_ref());
        let changed = self
            .revision_observations
            .prepare_list(app.as_ref(), revisions);
        self.prepare_revision_columns();
        if changed {
            self.revision_observations.clear_evidence();
            self.read_revision(cx);
        }
    }

    /// Reads the window around the selected revision: Coroot's page of the
    /// application over it, or the example's. Read again for the same
    /// revision and window, the last answer stays until the next arrives.
    pub(super) fn read_revision(&mut self, cx: &mut Context<Self>) {
        self.live.revision_job = None;
        let state = &self.revision_observations;
        let selected = state
            .selected()
            .filter(|_| state.app.is_some() && state.app == self.selected_app);
        let (Some(app), Some(row)) = (state.app.clone(), selected) else {
            self.live.revision = Default::default();
            self.revision_observations.clear_evidence();
            return;
        };
        let revision = row.revision.clone();
        let window = state.window();
        if self.fixture {
            if !self.hold {
                self.revision_observations.example =
                    Some(app_example::around(&app, &revision, window));
                self.prepare_revision_detail();
            }
            return;
        }
        let (Some(provider), Some(source)) = (self.live.provider.clone(), self.live.source.clone())
        else {
            return;
        };
        let subject = Subject::Revision(app.clone(), revision.id.clone(), WINDOWS[state.window].0);
        let Some(identity) = self.live.identity(subject) else {
            return;
        };
        let request = self.live.revision.begin(identity);
        let job: ReadJob = self.spawn_owned_read(
            async move {
                provider
                    .revision_view(&source, &app, &revision, window)
                    .await
            },
            move |this, result, cx| {
                if this
                    .live
                    .revision
                    .apply(&request, result.map_err(|e| e.to_string()))
                {
                    this.prepare_revision_detail();
                    cx.notify();
                }
            },
            cx,
        );
        self.live.revision_job = Some(job);
    }

    /// The window's answer: the example's, or Coroot's last.
    fn revision_answer(&self) -> Option<&api::RevisionView> {
        if self.fixture {
            self.revision_observations.example.as_ref()?.as_ref().ok()
        } else {
            self.live.revision.data()
        }
    }

    /// Why the window's read failed, if it did.
    fn revision_error(&self) -> Option<String> {
        if self.fixture {
            match &self.revision_observations.example {
                Some(Err(error)) => Some(error.to_string()),
                _ => None,
            }
        } else {
            self.live.revision.error().map(str::to_owned)
        }
    }

    /// Derives the inspector from the window's answer and the report
    /// chosen.
    pub(super) fn prepare_revision_detail(&mut self) {
        let state = &self.revision_observations;
        let detail = match (self.revision_answer(), state.selected()) {
            (Some(answer), Some(row)) if answer.revision.id == row.revision.id => {
                Some(detail::prepare(answer, &state.report))
            }
            _ => None,
        };
        let state = &mut self.revision_observations;
        if let Some(detail) = &detail {
            state.report = detail.report.clone();
        }
        state.detail = detail;
    }

    /// The charts the inspector shows, for the panels kept beside them.
    pub(super) fn revision_charts(&self) -> &[Rc<Charts>] {
        self.revision_observations
            .detail
            .as_ref()
            .map_or(&[], |detail| detail.charts())
    }

    pub(super) fn select_revision(&mut self, id: String, cx: &mut Context<Self>) {
        let state = &mut self.revision_observations;
        let key = TableKey::Revision(id);
        if state.selected.as_ref() == Some(&key) || !state.rows.iter().any(|r| r.key == key) {
            return;
        }
        state.selected = Some(key);
        state.clear_evidence();
        self.read_revision(cx);
        cx.notify();
    }

    pub(super) fn choose_revision_window(&mut self, window: usize, cx: &mut Context<Self>) {
        if self.revision_observations.window == window || window >= WINDOWS.len() {
            return;
        }
        self.revision_observations.window = window;
        self.revision_observations.clear_evidence();
        self.read_revision(cx);
        cx.notify();
    }

    pub(super) fn choose_revision_report(&mut self, report: String, cx: &mut Context<Self>) {
        if self.revision_observations.report == report {
            return;
        }
        self.revision_observations.report = report;
        self.prepare_revision_detail();
        cx.notify();
    }

    /// The table, with the selected revision's inspector beside it on a
    /// wide page and below it on a narrow one.
    pub(super) fn render_deployments(&self, window: &Window, cx: &mut Context<Self>) -> AnyElement {
        let table = self.render_revision_table(window, cx);
        let detail = self.render_revision_detail(cx);
        let open = detail.is_some();
        let split = freshkube_ui::inspector::split(
            "obs-deployments-split",
            &self.revision_split,
            inspector_beside(window),
            table,
            detail,
            window,
        );
        self.sized_split(split, open, window)
    }
}
