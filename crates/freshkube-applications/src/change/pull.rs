//! The pull request card's read, which is a placeholder row until the card
//! is designed: one line of text from what GitHub says of the change's pull
//! request. It is the page's own: it reads only while the page shows, a
//! hide drops the read and its delivery, and a late answer is checked
//! against its request and its key before it lands. The list never starts
//! it.
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use freshkube_core::job as backend;
use freshkube_delivery::github::avatars::Avatars;
use freshkube_delivery::github::{
    AvatarUrl, DetailKey, PullDetail, example_detail, example_identity,
};
use freshkube_reads::{Loader, OnShow};

use super::pull_card::{self, Known, PullCard, Reading};
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
    /// Whether the app has a client for GitHub at all, as of the last read.
    no_client: bool,
    /// The card's words, derived when the read or the change changes.
    card: Option<PullCard>,
    /// Avatars on disk for the people on the card, and the one job that
    /// fetches what is missing; dropped with the card and on hide.
    avatars: HashMap<AvatarUrl, PathBuf>,
    /// The avatars asked for for this card, so none is asked for twice.
    avatars_tried: std::collections::HashSet<AvatarUrl>,
    avatar_job: Option<(backend::OwnedJob, Task<()>)>,
}

/// The app's folder of avatars, installed once by the shell for a live
/// session. Example data installs none, so a fixture never reaches GitHub's
/// avatar host and draws initials.
#[derive(Clone)]
struct AvatarCache(Arc<Avatars>);

impl gpui_kit::Global for AvatarCache {}

/// Keeps avatars in `dir` from now on. A folder that can't be had leaves the
/// card with initials.
pub fn install_avatar_folder(dir: PathBuf, cx: &mut App) {
    if let Ok(avatars) = Avatars::new(dir) {
        cx.set_global(AvatarCache(Arc::new(avatars)));
    }
}

impl Card {
    pub(super) fn card(&self) -> Option<&PullCard> {
        self.card.as_ref()
    }

    /// The avatar's file, if it is on disk.
    pub(super) fn avatar_path(&self, avatar: &AvatarUrl) -> Option<PathBuf> {
        self.avatars.get(avatar).cloned()
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

impl ChangePage {
    /// Whether the selection is the pull request's hop, where the full card
    /// shows.
    pub(super) fn pull_selected(&self) -> bool {
        let pr = self.strip.cards.iter().find(|card| card.id == "pr");
        matches!((&self.selected, pr), (Some(key), Some(card)) if card.hops.contains(key))
    }

    /// Reads the pull request's detail for the change shown, while the page
    /// shows. What is held is read again when it is over a minute old, with
    /// the old line standing meanwhile; `force` reads whatever is held, and
    /// a failed read waits out its backoff either way.
    pub(super) fn read_pull(&mut self, force: bool, cx: &mut Context<Self>) {
        if !self.visible {
            return;
        }
        let Some(summary) = self.change.origin.pull.clone() else {
            // No pull request was read: the card says what the cluster knows
            // and, for a live read with no client, that GitHub isn't set up.
            self.pull.forget();
            self.pull.no_client =
                matches!(&self.fetch, Fetch::Live { place, .. } if place.github.is_none());
            self.derive_pull(cx);
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
                    self.pull.no_client = true;
                    self.derive_pull(cx);
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

    /// The card's words, from what is held.
    fn derive_pull(&mut self, cx: &mut Context<Self>) {
        let known = Known {
            number: self.change.origin.pull_request,
            commit: self.change.origin.commit.clone(),
        };
        let reading = if self.pull.no_client {
            Reading::NoClient
        } else if self.pull.read.data().is_none() && self.pull.read.error().is_some() {
            Reading::Failed
        } else if self.pull.read.is_loading() && self.pull.read.data().is_none() {
            Reading::Loading
        } else if self.pull.read.data().is_some() {
            Reading::Held
        } else {
            Reading::Nothing
        };
        let card = pull_card::derive(
            &known,
            self.change.origin.pull.as_ref(),
            self.pull.read.data(),
            self.pull.read.error(),
            reading,
        );
        // A change with no pull request and nothing known of one has no card.
        let nothing =
            known.number.is_none() && self.change.origin.pull.is_none() && !self.pull.no_client;
        self.pull.card = (!nothing).then_some(card);
        self.load_avatars(cx);
        cx.notify();
    }

    /// Fetches the avatars of the card's people that aren't held yet, in one
    /// job on the page's own lifetime. The disk is asked inside the job,
    /// never on the UI thread, and each avatar is tried once per card: a
    /// failure isn't asked for again until the card is forgotten (the client
    /// backs it off for five minutes besides). When a job lands, people
    /// who were added meanwhile get theirs.
    fn load_avatars(&mut self, cx: &mut Context<Self>) {
        let (Some(AvatarCache(avatars)), Fetch::Live { runtime, .. }) =
            (cx.try_global::<AvatarCache>().cloned(), &self.fetch)
        else {
            return;
        };
        let Some(card) = self.pull.card.as_ref() else {
            return;
        };
        if !self.visible || self.pull.avatar_job.is_some() {
            return;
        }
        let mut wanted: Vec<AvatarUrl> = Vec::new();
        for url in card.people.iter().filter_map(|person| person.avatar) {
            if !wanted.contains(&url) && !self.pull.avatars_tried.contains(&url) {
                wanted.push(url);
            }
        }
        if wanted.is_empty() {
            return;
        }
        self.pull.avatars_tried.extend(wanted.iter().copied());
        let size = super::pull_view::AVATAR_PX;
        let (job, receiver) = backend::spawn_job(runtime, DEADLINE, (), async move {
            let mut got = Vec::new();
            for url in wanted {
                // A fresh file is the answer; an old one is replaced by one
                // fetch, and stands if that fails.
                if let Ok(avatar) = avatars.ensure(&url, size).await {
                    got.push((url, avatar.path));
                }
            }
            Ok::<_, ()>(got)
        });
        let task = cx.spawn(async move |this, cx| {
            let got = receiver.await.ok().and_then(Result::ok).unwrap_or_default();
            _ = this.update(cx, |page, cx| {
                page.pull.avatar_job = None;
                page.pull.avatars.extend(got);
                page.load_avatars(cx);
                cx.notify();
            });
        });
        self.pull.avatar_job = Some((job, task));
    }

    /// The page shows or hides: hidden, the read and its delivery are
    /// dropped and a late answer never lands; shown, it reads again when
    /// what it holds is old.
    pub(super) fn pull_visible(&mut self, visible: bool, cx: &mut Context<Self>) {
        let reads_again = self.pull.read.set_visible(visible, ON_SHOW, cx);
        if visible {
            self.read_pull(reads_again, cx);
        } else {
            self.pull.avatar_job = None;
            self.derive_pull(cx);
        }
    }

    /// The client, the setup or what is mapped changed: nothing held was
    /// read for it.
    pub(super) fn pull_reset(&mut self) {
        self.pull.forget();
    }

    /// The card's second line, for tests.
    #[cfg(any(test, feature = "testing"))]
    pub fn pull_line(&self) -> Option<&str> {
        self.pull.card().map(|card| card.compact.as_ref())
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
