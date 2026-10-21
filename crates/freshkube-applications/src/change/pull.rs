//! The pull request card's read, which is a placeholder row until the card
//! is designed: one line of text from what GitHub says of the change's pull
//! request. It is the page's own: it reads only while the page shows, a
//! hide drops the read and its delivery, and a late answer is checked
//! against its request and its key before it lands. The list never starts
//! it.
use std::time::Duration;

use freshkube_core::job as backend;
use freshkube_delivery::github::{
    ChecksState, DetailKey, PullDetail, ReviewState, example_detail, example_identity,
};
use freshkube_reads::{Loader, OnShow};

use super::*;

/// A card read this old is read again when the page shows.
const FRESH_FOR: Duration = Duration::from_secs(60);
const ON_SHOW: OnShow = OnShow::ReadIfOlder(FRESH_FOR);
/// Four calls in a row, each with its own deadline in the client.
const DEADLINE: Duration = Duration::from_secs(30);

/// The card's read, the key it was asked for, and the line it shows.
#[derive(Default)]
pub(super) struct Card {
    read: Loader<PullDetail, DetailKey>,
    key: Option<DetailKey>,
    /// The placeholder row's text and its label, derived when the read
    /// changes.
    line: Option<(SharedString, SharedString)>,
}

impl Card {
    pub(super) fn line(&self) -> Option<&(SharedString, SharedString)> {
        self.line.as_ref()
    }

    fn forget(&mut self) {
        self.read.reset();
        self.key = None;
        self.line = None;
    }
}

/// `2 approved, 1 asked for changes`, or `no reviews`.
fn reviews_words(detail: &PullDetail) -> String {
    let count = |wanted| {
        detail
            .reviews
            .iter()
            .filter(|review| review.state == wanted)
            .count()
    };
    let (approved, changes) = (
        count(ReviewState::Approved),
        count(ReviewState::ChangesRequested),
    );
    let mut parts = Vec::new();
    if approved > 0 {
        parts.push(format!("{approved} approved"));
    }
    if changes > 0 {
        parts.push(format!("{changes} asked for changes"));
    }
    let more = if detail.reviews_capped {
        " (the first 100)"
    } else {
        ""
    };
    if parts.is_empty() {
        format!("no reviews{more}")
    } else {
        format!("{}{more}", parts.join(", "))
    }
}

/// `checks passing (3)`, `100 of 112 checks pending`.
fn checks_words(detail: &PullDetail) -> String {
    let checks = &detail.checks;
    let state = match checks.state {
        ChecksState::None => return "no checks".into(),
        ChecksState::Pending => "pending",
        ChecksState::Passing => "passing",
        ChecksState::Failing => "failing",
    };
    match checks.capped {
        Some(capped) => format!("{} of {} checks {state}", capped.read, capped.total),
        None => format!(
            "checks {state} ({})",
            checks.runs.len() + checks.statuses.len()
        ),
    }
}

impl ChangePage {
    /// Reads the pull request's detail for the change shown, while the page
    /// shows; `force` reads again whatever is held.
    pub(super) fn read_pull(&mut self, force: bool, cx: &mut Context<Self>) {
        if !self.visible {
            return;
        }
        let Some(summary) = self.change.origin.pull.clone() else {
            self.pull.forget();
            return;
        };
        let (key, work) = match &self.fetch {
            Fetch::Example { shape, .. } => {
                let Some(key) = DetailKey::of(&example_identity(), &summary) else {
                    return;
                };
                if !force && self.pull.key.as_ref() == Some(&key) && self.pull.read.data().is_some()
                {
                    return;
                }
                let result = example_detail(&summary, shape.github());
                self.pull.key = Some(key.clone());
                self.pull.read.resolve(key, result);
                self.derive_pull(cx);
                return;
            }
            Fetch::Live { runtime, place, .. } => {
                let Some(github) = place.github.clone() else {
                    self.pull.forget();
                    return;
                };
                let Some(key) = DetailKey::of(github.identity(), &summary) else {
                    self.pull.forget();
                    return;
                };
                let held = self.pull.key.as_ref() == Some(&key);
                if !force
                    && held
                    && (self.pull.read.is_loading() || self.pull.read.data().is_some())
                {
                    return;
                }
                if !held {
                    // Another client, pull request or head: what was read
                    // for the last is never shown as this one's.
                    self.pull.read.reset();
                }
                let (repo, number) = (summary.repo.clone(), summary.number);
                let runtime = runtime.clone();
                let work = backend::spawn_job(
                    &runtime,
                    DEADLINE,
                    "Reading the pull request timed out".into(),
                    async move {
                        github
                            .detail(&repo, number)
                            .await
                            .map_err(|error| error.words(&repo))
                    },
                );
                (key, work)
            }
        };
        let (job, receiver) = work;
        let request = self.pull.read.start(key.clone());
        self.pull.key = Some(key);
        let task = cx.spawn(async move |this, cx| {
            let result = receiver
                .await
                .unwrap_or_else(|_| Err("Reading the pull request stopped".into()));
            _ = this.update(cx, |page, cx| page.pull_landed(&request, result, cx));
        });
        self.pull.read.run(Some(job), task);
        self.derive_pull(cx);
    }

    fn pull_landed(
        &mut self,
        request: &freshkube_core::snapshot::Request<DetailKey>,
        result: Result<PullDetail, String>,
        cx: &mut Context<Self>,
    ) {
        if self.pull.read.land(request, result, cx) {
            self.derive_pull(cx);
        }
    }

    /// The row's words, from what is held.
    fn derive_pull(&mut self, cx: &mut Context<Self>) {
        let Some(summary) = self.change.origin.pull.as_ref() else {
            self.pull.line = None;
            cx.notify();
            return;
        };
        let name = format!("Pull request #{}", summary.number);
        let text = match (self.pull.read.data(), self.pull.read.error()) {
            (Some(detail), error) => {
                let mut parts = vec![
                    detail.summary.state.label().to_owned(),
                    reviews_words(detail),
                    checks_words(detail),
                ];
                match (&detail.stale, error) {
                    (Some(stale), _) => parts.push(format!(
                        "{}; showing what was read at {}",
                        stale.why,
                        stale.read_at.format("%H:%M UTC")
                    )),
                    (None, Some(error)) => parts.push(format!("Couldn't read again: {error}")),
                    (None, None) => {}
                }
                format!("{name} · {}", parts.join(" · "))
            }
            (None, Some(error)) => format!("{name} · {error}"),
            (None, None) if self.pull.read.is_loading() => format!("{name} · reading GitHub…"),
            (None, None) => format!("{name} · {}", summary.state.label()),
        };
        self.pull.line = Some((text.clone().into(), format!("Pull request: {text}").into()));
        cx.notify();
    }

    /// The page shows or hides: hidden, the read and its delivery are
    /// dropped and a late answer never lands; shown, it reads again when
    /// what it holds is old.
    pub(super) fn pull_visible(&mut self, visible: bool, cx: &mut Context<Self>) {
        let reads_again = self.pull.read.set_visible(visible, ON_SHOW, cx);
        if visible {
            self.read_pull(reads_again, cx);
        } else {
            self.derive_pull(cx);
        }
    }

    /// The client, the setup or what is mapped changed: nothing held was
    /// read for it.
    pub(super) fn pull_reset(&mut self) {
        self.pull.forget();
    }

    /// The placeholder row's text, for tests.
    #[cfg(any(test, feature = "testing"))]
    pub fn pull_line(&self) -> Option<&str> {
        self.pull.line().map(|(text, _)| text.as_ref())
    }

    /// Whether the card's read is in flight, for tests.
    #[cfg(any(test, feature = "testing"))]
    pub fn pull_reading(&self) -> bool {
        self.pull.read.is_loading()
    }

    /// The key of the card's read, for tests.
    #[cfg(any(test, feature = "testing"))]
    pub fn pull_key(&self) -> Option<&DetailKey> {
        self.pull.key.as_ref()
    }
}
