//! The page's read: example data's change, or the live one core's
//! `delivery::change::live` reads across the workspace, each part from its
//! entry, through a pool of clients made for the read alone. It reads while
//! the page shows: on opening, on showing again once what it has is older
//! than [`FRESH_FOR`], and on Refresh. Hiding drops the read in flight; an
//! answer for another change, or a superseded one, never lands. A failed
//! refresh keeps the last answer, marked stale.
use std::time::Duration;

use chrono::{DateTime, Utc};
use freshkube_core::snapshot::Request;
use freshkube_delivery::change::Change;
use freshkube_delivery::change::example::Shape;
use freshkube_delivery::change::live::{self, Place};
use freshkube_delivery::changes::example as changes_example;
use freshkube_delivery::entry::{self, pool};
use freshkube_reads::OnShow;

use super::*;
use crate::across::{Across, Alone};
use crate::backend;
use crate::resources::ClusterAccess;

/// A change read this recent is shown again without reading.
const FRESH_FOR: Duration = Duration::from_secs(30);
/// Showing the page reads again once what it has is [`FRESH_FOR`] old.
pub(super) const ON_SHOW: OnShow = OnShow::ReadIfOlder(FRESH_FOR);
/// How long one change's read may take before it fails whole: past
/// core's budget, at which it answers with what was read in time.
const DEADLINE: Duration = Duration::from_secs(live::BUDGET.as_secs() + 15);
/// The workspace of one reads the open connection as before there was a
/// workspace: each request under the read's deadline alone.
pub(super) const ALONE: Alone = Alone::Within(DEADLINE);

/// Where the page's change comes from.
#[derive(Clone)]
pub enum Fetch {
    /// Example data's, its times counted from `now`; it answers after
    /// `delay`, which only tests set, so a read stays in flight.
    Example {
        project: String,
        freight: String,
        now: DateTime<Utc>,
        /// What the example couldn't read.
        shape: Shape,
        delay: Duration,
    },
    /// A live read's, across the workspace from the open connection.
    Live {
        runtime: tokio::runtime::Handle,
        access: ClusterAccess,
        secrets: Option<freshkube_core::secrets::Secrets>,
        /// Distinguishes reloads even when the key path is unchanged.
        setup_revision: u64,
        place: Box<Place>,
        /// The workspace's kubeconfig, `None` for the automatic one.
        kubeconfig: Option<std::path::PathBuf>,
    },
}

impl Fetch {
    /// Which change this is: an answer for another is never shown as its.
    pub(crate) fn identity(&self) -> String {
        match self {
            Self::Example {
                project, freight, ..
            } => format!("example/{project}/{freight}"),
            Self::Live {
                place,
                setup_revision,
                ..
            } => {
                format!(
                    "{}/{}/{}/setup/{setup_revision}",
                    place.cluster, place.project, place.freight
                )
            }
        }
    }

    /// The change before anything was read: its project and Freight alone.
    pub(super) fn unread(&self) -> Change {
        let (project, freight, cluster) = match self {
            Self::Example {
                project, freight, ..
            } => (project.clone(), freight.clone(), String::new()),
            Self::Live { place, .. } => (
                place.project.clone(),
                place.freight.clone(),
                place.label.clone(),
            ),
        };
        Change {
            project,
            freight,
            kargo_cluster: cluster,
            observed_at: DateTime::<Utc>::MIN_UTC,
            page: Err("Nothing is read yet".into()),
            origin: Default::default(),
            missing: Vec::new(),
            groups: Vec::new(),
            stages: Vec::new(),
            hops: Vec::new(),
        }
    }
}

impl ChangePage {
    /// Reads the change again, whatever is in flight. With a window at
    /// hand, example data answers at once.
    pub(super) fn read(&mut self, window: Option<&Window>, cx: &mut Context<Self>) {
        let request = self.loader.start(self.fetch.identity());
        match self.fetch.clone() {
            Fetch::Example {
                project,
                freight,
                now,
                shape,
                delay,
            } => {
                let result = changes_example::change_of(&project, &freight, now)
                    .ok_or_else(|| format!("Example data has no change for Freight {freight}"))
                    .and_then(|change| shape.apply(change).map_err(|failure| failure.message));
                match window {
                    Some(window) if delay.is_zero() => self.answer(&request, result, window, cx),
                    _ => {
                        let handle = self.window;
                        let task = cx.spawn(async move |this, cx| {
                            cx.background_executor().timer(delay).await;
                            deliver(this, handle, request, result, cx);
                        });
                        self.loader.run(None, task);
                    }
                }
            }
            Fetch::Live {
                runtime,
                access,
                secrets,
                place,
                kubeconfig,
                ..
            } => {
                let across = Across {
                    mapping: place.mapping.clone(),
                    sources: place.sources.clone(),
                    kubeconfig,
                    public_key_file: place.public_key_file.clone(),
                    transparency_log: place.transparency_log,
                    transparency_url: place.transparency_url.clone(),
                    evidence_result: place.evidence_result.clone(),
                    build_namespace: place.build_namespace.clone(),
                    stage_naming: place.stage_naming.clone(),
                    github: place.github.clone(),
                };
                let (job, receiver) = backend::spawn_job(
                    &runtime,
                    DEADLINE,
                    "Reading the change timed out".into(),
                    async move {
                        let open = across.open(&access, ALONE).await?;
                        let kubeconfigs = across.kubeconfigs();
                        let pool = pool::Pool::new(
                            &place.sources,
                            Some(pool::Open {
                                entry: place.mapping.open.as_deref(),
                                reader: open.as_ref().map_err(Clone::clone),
                            }),
                            |chosen| entry::connect_entry(kubeconfigs.clone(), chosen),
                            live::BUDGET,
                        )
                        .with_secrets(secrets);
                        live::read_in(&pool, &place, Utc::now())
                            .await
                            .map_err(|failure| failure.message)
                    },
                );
                let handle = self.window;
                let task = cx.spawn(async move |this, cx| {
                    let result = receiver
                        .await
                        .unwrap_or_else(|_| Err("Reading the change stopped".into()));
                    deliver(this, handle, request, result, cx);
                });
                self.loader.run(Some(job), task);
            }
        }
        cx.notify();
    }

    /// An answer, published only while it is the one in flight. The first
    /// selects the Stage the page was opened on.
    fn answer(
        &mut self,
        request: &Request<String>,
        result: Result<Change, String>,
        window: &Window,
        cx: &mut Context<Self>,
    ) {
        let answered = result.is_ok();
        if !self.loader.land(request, result, cx) {
            return;
        }
        self.stale = self.loader.data().and(self.loader.error()).map(|why| {
            let text = format!("Showing the last read. {why}");
            let label = format!("Couldn't read again: {text}");
            (text.into(), label.into())
        });
        if let Some(change) = self.loader.data().filter(|_| answered).cloned() {
            let graph_focused = self
                .graph
                .focus_handle()
                .is_some_and(|focus| focus.is_focused(window));
            let focus_chain = self.rows.is_empty()
                && self.view == ChangeView::Chain
                && self.focus.is_focused(window);
            self.show(change);
            self.read_pull(false, cx);
            if focus_chain && !self.rows.is_empty() {
                let this = cx.entity().downgrade();
                window.on_next_frame(move |window, cx| {
                    _ = this.update(cx, |page, cx| {
                        if page.visible
                            && page.view == ChangeView::Chain
                            && !page.rows.is_empty()
                            && page.focus.is_focused(window)
                        {
                            page.focus(window, cx);
                            cx.notify();
                        }
                    });
                });
            }
            if graph_focused && self.rows.is_empty() {
                let this = cx.entity().downgrade();
                window.on_next_frame(move |window, cx| {
                    _ = this.update(cx, |page, cx| {
                        if page.visible && page.rows.is_empty() {
                            page.focus(window, cx);
                        }
                    });
                });
            }
            if let Some(stage) = self.stage.take() {
                self.select_stage(&stage, window, cx);
            }
            // The refreshed change may name another revision: the selected
            // hop's outcome is asked again if what is kept isn't fresh.
            self.read_outcome(false, cx);
        }
        cx.notify();
    }

    /// Shown, the page reads unless what it has is fresh; hidden, it drops
    /// the read in flight.
    pub(crate) fn set_visible(&mut self, visible: bool, cx: &mut Context<Self>) {
        if visible == self.visible {
            return;
        }
        self.visible = visible;
        let reads_again = self.loader.set_visible(visible, ON_SHOW, cx);
        if !visible {
            // Hidden, nothing is read: the outcome's read is dropped with
            // the change's, and a late answer never lands.
            self.hide_outcomes();
            self.want_coroot(false, cx);
            self.pull_visible(false, cx);
            cx.notify();
            return;
        }
        if reads_again {
            self.read(None, cx);
        }
        self.pull_visible(true, cx);
        self.want_coroot(true, cx);
        self.read_outcome(false, cx);
    }

    /// The workspace's entries or destination mappings changed, as a save
    /// in Settings › Workspace or a switch does: the links follow at once,
    /// and a live change resolves its Stages again by the new mapping, now
    /// while shown, else when it next shows.
    pub(crate) fn set_clusters(
        &mut self,
        across: Across,
        connections: Connections,
        cx: &mut Context<Self>,
    ) {
        self.connections = connections;
        self.detail = self.derive_detail();
        if let Fetch::Live {
            place, kubeconfig, ..
        } = &mut self.fetch
            && (&place.mapping, &place.sources, &*kubeconfig, &place.github)
                != (
                    &across.mapping,
                    &across.sources,
                    &across.kubeconfig,
                    &across.github,
                )
        {
            place.mapping = across.mapping;
            place.sources = across.sources;
            *kubeconfig = across.kubeconfig;
            place.github = across.github;
            self.pull_reset();
            self.loader.expire();
            self.refresh(cx);
        }
        cx.notify();
    }

    /// GitHub's client a live read asks, for tests.
    #[cfg(test)]
    pub(crate) fn github(&self) -> Option<&freshkube_delivery::github::Handle> {
        match &self.fetch {
            Fetch::Live { place, .. } => place.github.as_ref(),
            Fetch::Example { .. } => None,
        }
    }

    /// The build namespace a live read searches, for tests.
    #[cfg(test)]
    pub(crate) fn build_namespace(&self) -> Option<&str> {
        match &self.fetch {
            Fetch::Live { place, .. } => place.build_namespace.as_deref(),
            Fetch::Example { .. } => None,
        }
    }

    /// Trust belongs to this setup, including when the file at its path
    /// changed. Reset cancels both tasks and rejects any old request ticket.
    pub(crate) fn reload_setup(&mut self, across: &Across, cx: &mut Context<Self>) {
        if let Fetch::Live {
            place,
            setup_revision,
            ..
        } = &mut self.fetch
        {
            *setup_revision += 1;
            place.public_key_file = across.public_key_file.clone();
            place.transparency_log = across.transparency_log;
            place.transparency_url = across.transparency_url.clone();
            place.evidence_result = across.evidence_result.clone();
            place.build_namespace = across.build_namespace.clone();
            place.stage_naming = across.stage_naming.clone();
            place.github = across.github.clone();
            self.pull_reset();
            self.loader.reset();
            self.stale = None;
            self.hide_outcomes();
            self.show(self.fetch.unread());
            self.refresh(cx);
            cx.notify();
        }
    }

    /// Refresh: reads again while shown.
    pub(crate) fn refresh(&mut self, cx: &mut Context<Self>) {
        if self.visible {
            self.read(None, cx);
        }
    }

    /// Why the change couldn't be read, when nothing was.
    pub(super) fn failure(&self) -> Option<&str> {
        self.loader
            .data()
            .is_none()
            .then(|| self.loader.error())
            .flatten()
    }

    /// The loading rows, until the first answer.
    pub(super) fn loading_rows(&self) -> Option<&kit::LoadingRows> {
        (self.loader.is_loading() && self.loader.data().is_none() && self.loader.error().is_none())
            .then_some(&self.loading)
    }

    /// The motion over the loading rows, which the shell mounts beside the
    /// cached page.
    pub(crate) fn loading_motion(&self) -> Option<Entity<kit::LoadingMotion>> {
        self.loading_rows()
            .is_some()
            .then(|| self.loading_motion.clone())
    }
}

/// Hands an answer to the page in its window, if both are still there.
fn deliver(
    this: WeakEntity<ChangePage>,
    handle: AnyWindowHandle,
    request: Request<String>,
    result: Result<Change, String>,
    cx: &mut AsyncApp,
) {
    _ = cx.update_window(handle, |_, window, cx| {
        _ = this.update(cx, |page, cx| page.answer(&request, result, window, cx));
    });
}

#[cfg(any(test, feature = "testing"))]
impl ChangePage {
    pub fn is_reading(&self) -> bool {
        self.loader.is_loading()
    }

    /// Shows or hides the page, as the shell does.
    pub fn set_shown(&mut self, visible: bool, cx: &mut Context<Self>) {
        self.set_visible(visible, cx);
    }

    /// Whether an answer was shown.
    pub fn answered(&self) -> bool {
        self.loader.data().is_some()
    }

    /// The selection's actions, as the Inspector's footer labels them.
    pub fn action_labels(&self) -> Vec<String> {
        self.detail
            .as_ref()
            .map(|detail| detail.action_labels())
            .unwrap_or_default()
    }

    /// Why the selection's action labelled `label` is greyed out, if it is.
    pub fn action_why(&self, label: &str) -> Option<String> {
        self.detail.as_ref()?.action_why(label)
    }

    /// The change shown.
    pub fn shown(&self) -> &Change {
        &self.change
    }

    /// An answer, as a read brings it.
    pub fn read_answer(&mut self, change: Change, window: &Window, cx: &mut Context<Self>) {
        let request = self.loader.start(self.fetch.identity());
        self.answer(&request, Ok(change), window, cx);
    }

    /// A read that failed, as a refused or unreachable cluster answers.
    pub fn fail_read(&mut self, why: &str, window: &Window, cx: &mut Context<Self>) {
        let request = self.loader.start(self.fetch.identity());
        self.answer(&request, Err(why.into()), window, cx);
    }
}

#[cfg(test)]
mod tests {
    use super::{ALONE, Alone, DEADLINE, entry};

    #[test]
    fn the_workspace_of_one_reads_under_the_pages_deadline() {
        assert_eq!(ALONE, Alone::Within(DEADLINE));
        assert!(DEADLINE > entry::REQUEST_DEADLINE);
    }
}
