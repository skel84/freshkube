//! An application's page around one deployment revision's start: Coroot's
//! own view over the window coroot-rs bounds, so its charts draw as the
//! application page's do, and which of their samples fall before and after
//! the start. Nothing is computed across the split; Coroot's findings on
//! the revision are its own ([`DeploymentRevision::findings`]).
use super::app_view::{AppView, decode_all};
use super::*;
use chrono::{DateTime, Utc};
pub use coroot_rs::{ChartSplit, RevisionWindow, SideCoverage};

/// What Coroot answered for the window around a revision.
#[derive(Clone, Debug)]
pub struct RevisionView {
    /// The revision the window is around.
    pub revision: DeploymentRevision,
    /// The window asked for, in whole seconds.
    pub from: DateTime<Utc>,
    pub to: DateTime<Utc>,
    pub answer: Around,
}

#[derive(Clone, Debug)]
pub enum Around {
    /// The application's page over the window.
    View(Box<AppView>),
    /// Coroot's 404 "Application not found" for the window: the application
    /// has no metrics in it. The window starts after Coroot's newest data,
    /// or the application sent none in it (it wasn't running, or the window
    /// is older than Coroot's cache keeps). Never "no such application":
    /// the revision was read from the application's own page.
    NoData,
}

impl RevisionView {
    /// Which of the chart's sample places fall before the revision started,
    /// and which at or after it.
    pub fn split(&self, history: &ChartHistory) -> ChartSplit {
        history.split_at(self.revision.started_at)
    }

    /// Whether Coroot cut the chart before the window's end, at its newest
    /// data: the revision started less than the window's after side ago, or
    /// Coroot's cache is behind.
    pub fn ends_early(&self, history: &ChartHistory) -> bool {
        history.to < self.to
    }
}

impl Provider {
    /// The application's page over `window` around `revision`'s start,
    /// whatever the page's own range is. `revision` must come from `app`'s
    /// own page ([`AppView::revisions`]): Coroot answers a mistyped
    /// application as it answers a window without data.
    pub async fn revision_view(
        &self,
        source: &Source,
        app: &AppId,
        revision: &DeploymentRevision,
        window: RevisionWindow,
    ) -> Result<RevisionView, ReadError> {
        if app.as_str().is_empty() {
            return Err(ReadError::InvalidSelection);
        }
        let range = window
            .range(revision.started_at)
            .map_err(|_| ReadError::InvalidSelection)?;
        let (Some(from), Some(to)) = (range.from, range.to) else {
            return Err(ReadError::InvalidSelection);
        };
        let project = self.project(source, range)?;
        let path = format!("app/{}", coroot_rs::util::encode_segment(app.as_str()));
        // Only Coroot's own 404 for the window is no data; any other stays
        // an error, as `Missing` or whatever it is.
        let envelope = self
            .read(async {
                match project.get(&path, &[]).await {
                    Ok(envelope) => Ok(Some(envelope)),
                    Err(error) if coroot_rs::AroundRevision::is_no_data(&error) => Ok(None),
                    Err(error) => Err(error),
                }
            })
            .await?;
        let answer = match envelope {
            Some(envelope) => Around::View(Box::new(decode_all(envelope, app)?)),
            None => Around::NoData,
        };
        Ok(RevisionView {
            revision: revision.clone(),
            from,
            to,
            answer,
        })
    }
}
