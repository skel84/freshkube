//! The pull request card's read, which is a placeholder row until the card
//! is designed: one line of text from what GitHub says of the change's pull
//! request. It is the page's own: it reads only while the page shows, a
//! hide drops the read and its delivery, and a late answer is checked
//! against its request and its key before it lands. The list never starts
//! it.
use std::time::{Duration, Instant};

use freshkube_core::job as backend;
use freshkube_delivery::github::{
    ChecksState, DetailKey, PullDetail, ReviewState, example_detail, example_identity,
};
use freshkube_reads::{Loader, OnShow};

use super::*;

/// A card read this old is read again: when the page shows, and when the
/// change is read again.
const FRESH_FOR: Duration = Duration::from_secs(60);
const ON_SHOW: OnShow = OnShow::ReadIfOlder(FRESH_FOR);
/// Four calls in a row, each with its own deadline in the client.
const DEADLINE: Duration = Duration::from_secs(30);
/// How long a failed read waits before the next, doubling with each failure
/// in a row up to [`LONGEST_BACKOFF`], so a page that is read again every
/// few seconds doesn't spend an anonymous budget on a read that fails.
const BACKOFF: Duration = Duration::from_secs(60);
const LONGEST_BACKOFF: Duration = Duration::from_secs(10 * 60);

/// The card's read, the key it was asked for, and the line it shows.
#[derive(Default)]
pub(super) struct Card {
    read: Loader<PullDetail, DetailKey>,
    key: Option<DetailKey>,
    /// When the last read of `key` answered, by the executor's clock.
    answered: Option<Instant>,
    /// How many reads of `key` have failed in a row, and when the next may
    /// start.
    failures: u32,
    retry_after: Option<Instant>,
    /// The placeholder row: its text, its words for assistive technology
    /// and its tone, derived when the read changes.
    line: Option<Line>,
}

#[derive(Clone)]
pub(super) struct Line {
    pub text: SharedString,
    pub tone: Tone,
}

impl Card {
    pub(super) fn line(&self) -> Option<&Line> {
        self.line.as_ref()
    }

    fn forget(&mut self) {
        *self = Self::default();
    }

    /// Whether what is held for `key` is fresh enough not to read again.
    fn is_fresh(&self, key: &DetailKey, now: Instant) -> bool {
        self.key.as_ref() == Some(key)
            && self.read.error().is_none()
            && self.read.data().is_some()
            && self
                .answered
                .is_some_and(|at| now.saturating_duration_since(at) < FRESH_FOR)
    }

    /// Whether a failed read of `key` is still being waited out.
    fn is_backing_off(&self, key: &DetailKey, now: Instant) -> bool {
        self.key.as_ref() == Some(key) && self.retry_after.is_some_and(|until| now < until)
    }

    fn failed(&mut self, now: Instant) {
        self.failures = self.failures.saturating_add(1);
        let wait = BACKOFF
            .saturating_mul(1u32 << (self.failures - 1).min(10))
            .min(LONGEST_BACKOFF);
        self.retry_after = Some(now + wait);
    }

    fn answered(&mut self, now: Instant) {
        self.answered = Some(now);
        self.failures = 0;
        self.retry_after = None;
    }
}

/// What the reviews say, all of them: `1 approved, 1 asked for changes, 2
/// commented, 1 dismissed`, or `no reviews`; an approval of an older
/// commit than the head says so.
fn reviews_words(detail: &PullDetail) -> String {
    let head = detail.summary.head.as_deref();
    let of = |wanted| detail.reviews.iter().filter(move |r| r.state == wanted);
    let approvals = of(ReviewState::Approved).count();
    let older = of(ReviewState::Approved)
        .filter(|review| {
            matches!((review.commit.as_deref(), head), (Some(given), Some(head)) if given != head)
        })
        .count();
    let mut parts = Vec::new();
    if approvals > 0 {
        let note = match older {
            0 => String::new(),
            n if n == approvals => " (of an older commit)".to_owned(),
            n => format!(" ({n} of an older commit)"),
        };
        parts.push(format!("{approvals} approved{note}"));
    }
    for (state, words) in [
        (ReviewState::ChangesRequested, "asked for changes"),
        (ReviewState::Commented, "commented"),
        (ReviewState::Dismissed, "dismissed"),
    ] {
        let count = of(state).count();
        if count > 0 {
            parts.push(format!("{count} {words}"));
        }
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

/// `checks passing (3)`, `100 of 112 checks read, none failing so far`.
fn checks_words(detail: &PullDetail) -> String {
    let checks = &detail.checks;
    let count = checks.runs.len() + checks.statuses.len();
    let cut = match (checks.capped, checks.statuses_capped) {
        (Some(cut), _) => Some(format!("{} of {} checks read", cut.read, cut.total)),
        (None, Some(cut)) => Some(format!("{} of {} statuses read", cut.read, cut.total)),
        (None, None) => None,
    };
    match (checks.state, cut) {
        (ChecksState::None, _) => "no checks".into(),
        (ChecksState::Pending, None) => format!("checks pending ({count})"),
        (ChecksState::Pending, Some(cut)) => format!("{cut}, some still running"),
        (ChecksState::Failing, None) => format!("checks failing ({count})"),
        (ChecksState::Failing, Some(cut)) => format!("{cut}, some failing"),
        (ChecksState::Passing, _) => format!("checks passing ({count})"),
        (ChecksState::Ended, _) => format!("checks ended without passing ({count})"),
        (ChecksState::Partial, cut) => format!(
            "{}, none failing so far",
            cut.unwrap_or_else(|| format!("{count} checks read"))
        ),
    }
}

/// Whether two commits, as written in full or in part, are one.
fn same_commit(a: &str, b: &str) -> bool {
    let (a, b) = (a.to_ascii_lowercase(), b.to_ascii_lowercase());
    a.starts_with(&b) || b.starts_with(&a)
}

impl ChangePage {
    /// Reads the pull request's detail for the change shown, while the page
    /// shows. What is held is read again when it is over a minute old, with
    /// the old line standing meanwhile; `force` reads whatever is held, and
    /// a failed read waits out its backoff either way.
    pub(super) fn read_pull(&mut self, force: bool, cx: &mut Context<Self>) {
        if !self.visible {
            return;
        }
        let Some(summary) = self.change.origin.pull.clone() else {
            self.pull.forget();
            return;
        };
        let now = cx.background_executor().now();
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
                if held && (self.pull.read.is_loading() || self.pull.is_backing_off(&key, now)) {
                    return;
                }
                if !force && self.pull.is_fresh(&key, now) {
                    return;
                }
                if !held {
                    // Another client, pull request or head: what was read
                    // for the last is never shown as this one's.
                    self.pull.forget();
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
        let ok = result.is_ok();
        if self.pull.read.land(request, result, cx) {
            let now = cx.background_executor().now();
            if ok {
                self.pull.answered(now);
            } else {
                self.pull.failed(now);
            }
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
        let (rest, tone) = match (self.pull.read.data(), self.pull.read.error()) {
            (Some(detail), error) => {
                let mut parts = vec![
                    detail.summary.state.label().to_owned(),
                    reviews_words(detail),
                    checks_words(detail),
                ];
                // The checks are the head commit's; say so when that is not
                // the commit the change deploys.
                if let (Some(head), Some(deployed)) = (
                    detail.summary.head.as_deref(),
                    self.change.origin.commit.as_deref(),
                ) && !same_commit(head, deployed)
                {
                    parts.push("checks are of the head commit, not the deployed one".into());
                }
                let tone = match (&detail.stale, error) {
                    (Some(stale), _) => {
                        // Why is the banner's to say; the line says only that it is old.
                        parts.push(stale.read_words());
                        Tone::Warn
                    }
                    (None, Some(error)) => {
                        parts.push(format!("Couldn't read again: {error}"));
                        Tone::Warn
                    }
                    (None, None) => Tone::Info,
                };
                (parts.join(" · "), tone)
            }
            (None, Some(error)) => (error.to_owned(), Tone::Warn),
            (None, None) if self.pull.read.is_loading() => {
                ("reading GitHub…".to_owned(), Tone::Info)
            }
            (None, None) => (summary.state.label().to_owned(), Tone::Info),
        };
        self.pull.line = Some(Line {
            text: format!("{name} · {rest}").into(),
            tone,
        });
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
        self.pull.line().map(|line| line.text.as_ref())
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

#[cfg(test)]
mod tests {
    // Named, not globbed: the page's glob brings in `gpui_kit`'s own `test`.
    use freshkube_delivery::github::{ChecksState, PullDetail, ReviewState, example_detail};

    use super::{checks_words, reviews_words, same_commit};

    fn example() -> PullDetail {
        // The example's pull request 418, as GitHub reads it.
        let change = freshkube_delivery::testing::change(chrono::Utc::now());
        example_detail(
            &change.origin.pull.expect("the example's pull request"),
            None,
        )
        .unwrap()
    }

    #[test]
    fn every_kind_of_review_is_counted_and_an_old_approval_says_so() {
        let mut detail = example();
        assert_eq!(reviews_words(&detail), "1 approved, 1 commented");
        let head = detail.summary.head.clone();
        let review = detail.reviews[0].clone();
        let with = |state: ReviewState, commit: Option<String>| {
            let mut detail = detail.clone();
            detail.reviews = vec![freshkube_delivery::github::Review {
                state,
                commit,
                ..review.clone()
            }];
            detail
        };
        assert_eq!(
            reviews_words(&with(ReviewState::Commented, head.clone())),
            "1 commented",
            "a comment alone is a review, not none"
        );
        assert_eq!(
            reviews_words(&with(ReviewState::Dismissed, head.clone())),
            "1 dismissed"
        );
        assert_eq!(
            reviews_words(&with(ReviewState::ChangesRequested, head.clone())),
            "1 asked for changes"
        );
        assert_eq!(
            reviews_words(&with(ReviewState::Approved, Some("f".repeat(40)))),
            "1 approved (of an older commit)"
        );
        detail.reviews.clear();
        assert_eq!(reviews_words(&detail), "no reviews");
        detail.reviews_capped = true;
        assert_eq!(reviews_words(&detail), "no reviews (the first 100)");
    }

    #[test]
    fn checks_that_were_not_all_read_never_say_passing() {
        use freshkube_delivery::github::Capped;
        let mut detail = example();
        assert_eq!(checks_words(&detail), "checks passing (3)");
        for (state, words) in [
            (ChecksState::None, "no checks"),
            (ChecksState::Pending, "checks pending (3)"),
            (ChecksState::Failing, "checks failing (3)"),
            (ChecksState::Ended, "checks ended without passing (3)"),
            (ChecksState::Partial, "3 checks read, none failing so far"),
        ] {
            detail.checks.state = state;
            assert_eq!(checks_words(&detail), words);
        }
        detail.checks.capped = Some(Capped {
            read: 100,
            total: 112,
        });
        for (state, words) in [
            (
                ChecksState::Partial,
                "100 of 112 checks read, none failing so far",
            ),
            (
                ChecksState::Pending,
                "100 of 112 checks read, some still running",
            ),
            (ChecksState::Failing, "100 of 112 checks read, some failing"),
        ] {
            detail.checks.state = state;
            assert_eq!(checks_words(&detail), words);
        }
        detail.checks.capped = None;
        detail.checks.statuses_capped = Some(Capped {
            read: 30,
            total: 41,
        });
        detail.checks.state = ChecksState::Partial;
        assert_eq!(
            checks_words(&detail),
            "30 of 41 statuses read, none failing so far"
        );
    }

    #[test]
    fn a_commit_written_in_part_is_the_commit_it_starts() {
        assert!(same_commit(
            "a1f3c9e",
            &format!("a1f3c9e{}", "0".repeat(33))
        ));
        assert!(same_commit("A1F3C9E0", "a1f3c9e"));
        assert!(!same_commit("a1f3c9e", "b1f3c9e"));
    }
}
