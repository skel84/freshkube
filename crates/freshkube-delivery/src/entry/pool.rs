//! One read across the workspace's entries: where each part is read, the
//! deadlines each entry has, and the words for an entry that wasn't read.
//! Applications' read ([`applications::across`](crate::applications::across))
//! and the change page's and the Changes list's reads share them.
//!
//! - A part is read where [`Sources`] says: the open connection when it is
//!   the open entry or the workspace of one, else a client made for this
//!   read alone ([`Place`]).
//! - Kargo's, the builds' and Argo CD's entries are the first round, with
//!   the rest of the read's budget; a destination's entry has
//!   [`ENTRY_DEADLINE`] from when it is asked for, its wait for a turn to
//!   connect and connecting included, and never past the read's end
//!   ([`Until`]). The workspace of one has neither: the read's own deadline
//!   bounds it.
//! - An entry that didn't connect by its deadline is unreachable in its own
//!   name; one that connected but didn't finish says so; a part no entry
//!   holds is missing ([`Unread`]). None of them reads as none found.
//!
//! [`Pool`] keeps the clients of one read that follows many changes: each
//! entry is connected to once, at most [`MAX_AT_ONCE`] connecting at a
//! time (the limit covers connecting only; entries already connected read
//! at once), and each entry's questions are asked once through its own
//! [`SharedReader`]. Every client is dropped with the pool, when the read
//! ends.

use std::collections::HashMap;
use std::future::Future;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use serde_json::Value;
use tokio::sync::{OnceCell, Semaphore};
use tokio::time::Instant;

use super::{MAX_AT_ONCE, OPEN};
use crate::read::{ListRequest, Listing, Reader, Resource, ServedGroup};
use crate::shared::SharedReader;
use crate::words;
use freshkube_core::resources::{Failure, FailureKind};
use freshkube_core::workspace::{Chosen, Entry, Part, Sources};

/// How long a destination's entry may take, connecting and reading: room
/// for one listing at its longest. Kargo's, the builds' and Argo CD's
/// entries have the rest of the read's budget.
pub(crate) const ENTRY_DEADLINE: Duration = Duration::from_secs(30);

/// Where one part or entry is read.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Place {
    /// The open connection: the open entry, or the workspace of one.
    Open,
    /// Another entry, through a client made for this read alone.
    Entry(Entry),
    /// No entry holds it, or the one chosen is gone: the words say which.
    Missing(String),
}

impl Place {
    /// Where `chosen` is read, with `open` the workspace entry the open
    /// connection is, if it is one.
    pub fn of(chosen: &Chosen, open: Option<&str>) -> Self {
        match chosen {
            Chosen::Open => Place::Open,
            Chosen::Entry { entry, .. } if open == Some(entry.id.as_str()) => Place::Open,
            Chosen::Entry { entry, .. } => Place::Entry(entry.clone()),
            Chosen::NoEntry(_) | Chosen::Gone(_) => {
                Place::Missing(chosen.missing().unwrap_or_default())
            }
        }
    }
}

/// Why a part wasn't read, in the words Applications uses.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Unread {
    /// No entry holds it, or the one chosen is gone: the words say which. A
    /// [`FailureKind::Config`].
    Missing(Failure),
    /// The entry couldn't be reached by its deadline: a connect deadline,
    /// DNS, TLS, or its exec plugin failed or waits for a sign-in.
    Unreachable { entry: String, why: Failure },
    /// The entry answered, but not everything asked of it did in time. A
    /// [`FailureKind::Timeout`].
    Unfinished { entry: String, why: Failure },
}

impl Unread {
    /// No entry holds it.
    pub fn missing(why: impl Into<String>) -> Self {
        Unread::Missing(Failure::new(FailureKind::Config, why))
    }

    /// `entry` wasn't reached, for a reason with no kind of its own.
    pub fn unreachable(entry: impl Into<String>, why: impl Into<String>) -> Self {
        Unread::Unreachable {
            entry: entry.into(),
            why: Failure::new(FailureKind::Unreachable, why),
        }
    }

    /// `entry` answered but didn't finish by `until`.
    pub fn unfinished(entry: impl Into<String>, until: &Until) -> Self {
        Unread::Unfinished {
            entry: entry.into(),
            why: Failure::new(FailureKind::Timeout, until.unfinished()),
        }
    }

    /// The state in a sentence that names the entry.
    pub fn words(&self) -> String {
        match self {
            Unread::Missing(why) => words::failed(why.kind, "", "", &why.message),
            Unread::Unreachable { entry, why } => words::unreachable(entry, &why.message),
            Unread::Unfinished { entry, why } => words::unfinished(entry, &why.message),
        }
    }

    /// The same, its entry named `entry`: as a page names the open
    /// cluster, by its context.
    pub fn named(self, entry: String) -> Unread {
        match self {
            Unread::Missing(why) => Unread::Missing(why),
            Unread::Unreachable { why, .. } => Unread::Unreachable { entry, why },
            Unread::Unfinished { why, .. } => Unread::Unfinished { entry, why },
        }
    }

    /// The short reason alone, without the entry's name, as Applications
    /// keeps it beside the entry.
    pub fn why(&self) -> &str {
        match self {
            Unread::Missing(why)
            | Unread::Unreachable { why, .. }
            | Unread::Unfinished { why, .. } => &why.message,
        }
    }
}

/// When one entry's read must end, and how to say it: the first round's at
/// the read's end, a destination's [`ENTRY_DEADLINE`] after it starts. The
/// workspace of one has none.
#[derive(Clone, Copy, Debug)]
pub struct Until {
    at: Option<Instant>,
    started: Instant,
    /// The whole read's.
    budget: Duration,
    /// Whether the read's end, not an entry's own deadline, bounds it: the
    /// first round's, and a destination started too late for its own.
    by_budget: bool,
}

impl Until {
    /// No deadline: the read's own bounds it.
    pub fn none() -> Self {
        Self {
            at: None,
            started: Instant::now(),
            budget: Duration::ZERO,
            by_budget: true,
        }
    }

    /// The first round's: the read's end, `ends`, of a read of `budget`.
    pub fn first(ends: Instant, budget: Duration) -> Self {
        Self {
            at: Some(ends),
            started: Instant::now(),
            budget,
            by_budget: true,
        }
    }

    /// A destination's: [`ENTRY_DEADLINE`] from now, within the read's end,
    /// `ends`, of a read of `budget`. One started late has only what the
    /// budget leaves: the read's end, not its own deadline, cuts it short.
    pub fn destination(ends: Instant, budget: Duration) -> Self {
        let started = Instant::now();
        let (at, by_budget) = match started + ENTRY_DEADLINE {
            own if own <= ends => (own, false),
            _ => (ends, true),
        };
        Self {
            at: Some(at),
            started,
            budget,
            by_budget,
        }
    }

    pub fn at(&self) -> Option<Instant> {
        self.at
    }

    /// Whether the read had already ended when this one started.
    pub fn passed(&self) -> bool {
        self.at.is_some_and(|at| self.started >= at)
    }

    /// `the read's 45 s`, `30 s`.
    pub fn within(&self) -> String {
        match self.by_budget {
            true => format!("the read's {} s", self.budget.as_secs()),
            false => format!("{} s", ENTRY_DEADLINE.as_secs()),
        }
    }

    /// `future`'s answer, or none when it hasn't one by the deadline; so a
    /// step that runs out says which step it was.
    pub async fn bound<T>(&self, future: impl Future<Output = T>) -> Option<T> {
        match self.at {
            Some(at) => tokio::time::timeout_at(at, future).await.ok(),
            None => Some(future.await),
        }
    }

    /// Why an entry that answered didn't finish: `didn't finish reading
    /// within 30 s`.
    pub fn unfinished(&self) -> String {
        format!("didn't finish reading within {}", self.within())
    }
}

/// The reader one place is read through: the open connection's, or a
/// client made for the read.
#[derive(Clone)]
pub enum Via<O, E> {
    Open(O),
    Entry(E),
}

impl<O: Reader, E: Reader> Reader for Via<O, E> {
    async fn get(
        &self,
        resource: &Resource,
        namespace: Option<&str>,
        name: &str,
    ) -> Result<Value, Failure> {
        match self {
            Via::Open(reader) => reader.get(resource, namespace, name).await,
            Via::Entry(reader) => reader.get(resource, namespace, name).await,
        }
    }
    async fn list(&self, request: &ListRequest) -> Result<Listing, Failure> {
        match self {
            Via::Open(reader) => reader.list(request).await,
            Via::Entry(reader) => reader.list(request).await,
        }
    }
    async fn probe(&self, request: &ListRequest) -> Result<(), Failure> {
        match self {
            Via::Open(reader) => reader.probe(request).await,
            Via::Entry(reader) => reader.probe(request).await,
        }
    }
    async fn groups(&self) -> Result<Vec<ServedGroup>, Failure> {
        match self {
            Via::Open(reader) => reader.groups().await,
            Via::Entry(reader) => reader.groups().await,
        }
    }
    async fn plurals(&self, group: &str, version: &str) -> Result<Vec<String>, Failure> {
        match self {
            Via::Open(reader) => reader.plurals(group, version).await,
            Via::Entry(reader) => reader.plurals(group, version).await,
        }
    }
}

/// The reader for `place` by `until`: the open one, `open`, or a client
/// `connect` makes, for this read alone. An entry not connected by then is
/// unreachable in its own name.
pub async fn reach<'r, R, C, F, Fut>(
    place: Place,
    open: Option<Result<&'r R, &Failure>>,
    connect: F,
    until: &Until,
) -> Result<Via<&'r R, C>, Unread>
where
    F: FnOnce(Entry) -> Fut,
    Fut: Future<Output = Result<C, Failure>>,
{
    let unreachable = |entry: &str, why: Failure| Unread::Unreachable {
        entry: entry.to_owned(),
        why,
    };
    let entry = match &place {
        Place::Missing(why) => return Err(Unread::missing(why.clone())),
        Place::Open => OPEN.to_owned(),
        Place::Entry(entry) => entry.id.clone(),
    };
    if until.passed() {
        return Err(unreachable(
            &entry,
            Failure::new(
                FailureKind::Timeout,
                format!("not read within the read's {} s", until.budget.as_secs()),
            ),
        ));
    }
    match place {
        Place::Missing(_) => unreachable!("answered above"),
        Place::Open => match open {
            Some(Ok(reader)) => Ok(Via::Open(reader)),
            Some(Err(why)) => Err(unreachable(&entry, why.clone())),
            None => Err(unreachable(
                &entry,
                Failure::new(FailureKind::Unreachable, "no cluster is open"),
            )),
        },
        Place::Entry(chosen) => match until.bound(connect(chosen)).await {
            Some(Ok(reader)) => Ok(Via::Entry(reader)),
            Some(Err(why)) => Err(unreachable(&entry, why)),
            None => Err(unreachable(
                &entry,
                Failure::new(
                    FailureKind::Timeout,
                    format!("no answer within {}", until.within()),
                ),
            )),
        },
    }
}

/// The open connection, which the open entry and the workspace of one are
/// read through.
pub struct Open<'a, R> {
    /// The workspace entry it is, if it is one.
    pub entry: Option<&'a str>,
    /// Its reader, or why there is none.
    pub reader: Result<&'a R, Failure>,
}

/// Where a part was read, so what it shows opens there.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Read {
    /// The open connection.
    Open,
    /// Another workspace entry, by id.
    Entry(String),
}

/// One entry's client for the read, its questions shared.
type Client<C> = Arc<SharedReader<C>>;
type Slot<C> = Arc<OnceCell<Result<Client<C>, Unread>>>;

/// A reader of the pool: the open connection's, or an entry's.
pub(crate) type PoolReader<'p, R, C> = Via<&'p SharedReader<&'p R>, Client<C>>;

/// The clients of one read across the workspace. See the module's notes.
pub struct Pool<'a, R, C, F> {
    sources: &'a Sources,
    open_entry: Option<&'a str>,
    open: Option<Result<SharedReader<&'a R>, Failure>>,
    connect: F,
    slots: Mutex<HashMap<String, Slot<C>>>,
    connecting: Semaphore,
    /// When the read ends, unless it is the workspace of one.
    ends: Option<Instant>,
    budget: Duration,
}

impl<'a, R, C, F, Fut> Pool<'a, R, C, F>
where
    R: Reader,
    C: Reader,
    F: Fn(Entry) -> Fut,
    Fut: Future<Output = Result<C, Failure>>,
{
    /// A pool for `sources`, with `open` the open connection, `connect`
    /// making a client for any other entry, and `budget` the whole read's,
    /// from now. The workspace of one, every part on the open connection,
    /// isn't bounded by it.
    pub fn new(
        sources: &'a Sources,
        open: Option<Open<'a, R>>,
        connect: F,
        budget: Duration,
    ) -> Self {
        let alone = sources
            .iter()
            .all(|(_, chosen)| matches!(chosen, Chosen::Open));
        let (open_entry, open) = match open {
            Some(open) => (open.entry, Some(open.reader.map(SharedReader::new))),
            None => (None, None),
        };
        Self {
            sources,
            open_entry,
            open,
            connect,
            slots: Mutex::default(),
            connecting: Semaphore::new(MAX_AT_ONCE),
            ends: (!alone).then(|| Instant::now() + budget),
            budget,
        }
    }

    pub fn sources(&self) -> &Sources {
        self.sources
    }

    /// The workspace entry the open connection is, if it is one.
    pub fn open_entry(&self) -> Option<&str> {
        self.open_entry
    }

    /// The first round's deadline: the read's end.
    pub fn first(&self) -> Until {
        match self.ends {
            Some(ends) => Until::first(ends, self.budget),
            None => Until::none(),
        }
    }

    /// A destination's deadline, from now.
    pub fn destination(&self) -> Until {
        match self.ends {
            Some(ends) => Until::destination(ends, self.budget),
            None => Until::none(),
        }
    }

    /// The reader of `part`, and where it was read, by the first round's
    /// deadline.
    pub async fn part(&self, part: Part) -> Result<(PoolReader<'_, R, C>, Read), Unread> {
        let place = Place::of(self.sources.get(part), self.open_entry);
        self.place(place, &self.first()).await
    }

    /// The reader of the entry `id`, by `until`. The open entry is read
    /// through the open connection; an entry `sources` doesn't hold was
    /// never connected to.
    pub async fn entry(
        &self,
        id: &str,
        until: &Until,
    ) -> Result<(PoolReader<'_, R, C>, Read), Unread> {
        if self.open_entry == Some(id) {
            return self.place(Place::Open, until).await;
        }
        match self.sources.entry(id) {
            Some(entry) => self.place(Place::Entry(entry.clone()), until).await,
            None => Err(Unread::unreachable(id, "it wasn't connected to")),
        }
    }

    /// The open connection's reader, for the workspace of one's own
    /// cluster.
    pub async fn open(&self, until: &Until) -> Result<(PoolReader<'_, R, C>, Read), Unread> {
        self.place(Place::Open, until).await
    }

    async fn place(
        &self,
        place: Place,
        until: &Until,
    ) -> Result<(PoolReader<'_, R, C>, Read), Unread> {
        let Place::Entry(entry) = place else {
            let open = self
                .open
                .as_ref()
                .map(|open| open.as_ref().map_err(|why| why));
            let no_entry = |_| async { Err::<C, _>(Failure::new(FailureKind::Config, "")) };
            // The open cluster is named by its entry when it is one.
            let named = |unread| match (unread, self.open_entry) {
                (Unread::Unreachable { entry, why }, Some(open)) if entry == OPEN => {
                    Unread::Unreachable {
                        entry: open.to_owned(),
                        why,
                    }
                }
                (unread, _) => unread,
            };
            return match reach(place, open, no_entry, until).await.map_err(named)? {
                Via::Open(reader) => Ok((Via::Open(reader), Read::Open)),
                Via::Entry(_) => unreachable!("the open connection is never connected to"),
            };
        };
        let id = entry.id.clone();
        let slot = self
            .slots
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .entry(id.clone())
            .or_default()
            .clone();
        // Connected once, whoever asks first; an entry that couldn't be
        // reached is said so to every later asker.
        let client = slot
            .get_or_init(|| async {
                let _turn = self.connecting.acquire().await;
                let place = Place::Entry(entry);
                match reach::<R, C, _, _>(place, None, &self.connect, until).await? {
                    Via::Entry(client) => Ok(Arc::new(SharedReader::new(client))),
                    Via::Open(_) => unreachable!("an entry is connected to"),
                }
            })
            .await
            .clone()?;
        Ok((Via::Entry(client), Read::Entry(id)))
    }
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;
    use std::rc::Rc;

    use super::*;
    use crate::fixtures::FixtureReader;
    use freshkube_core::workspace::{Destination, Key, Role, Workspace};

    /// Argo CD on `core`, Kargo and the builds on `cicd`, and the
    /// destinations `environments`.
    fn sources(environments: &[String]) -> Sources {
        let mut clusters = vec![
            Entry::new("core", Role::Core, "core"),
            Entry::new("cicd", Role::Cicd, "cicd"),
        ];
        clusters.extend(
            environments
                .iter()
                .map(|id| Entry::new(id.as_str(), Role::Environment, id.as_str())),
        );
        let mut workspace = Workspace::new(clusters);
        workspace.destinations = environments
            .iter()
            .map(|id| Destination::new(Key::Name(id.clone()), id.as_str()))
            .collect();
        Sources::of(&workspace)
    }

    const BUDGET: Duration = Duration::from_secs(45);

    /// Kargo and the builds share one client, and the entry asked by its id
    /// is that client too.
    #[tokio::test]
    async fn an_entry_is_connected_once() {
        let sources = sources(&[]);
        let connects = Cell::new(0);
        let pool = Pool::new(
            &sources,
            None::<Open<'_, FixtureReader>>,
            |_: Entry| {
                connects.set(connects.get() + 1);
                async { Ok::<_, Failure>(FixtureReader::default()) }
            },
            BUDGET,
        );
        let (_, kargo) = pool.part(Part::Kargo).await.expect("kargo");
        let (_, builds) = pool.part(Part::Builds).await.expect("builds");
        let (_, asked) = pool.entry("cicd", &pool.destination()).await.expect("cicd");
        assert_eq!(
            [kargo, builds, asked],
            [(); 3].map(|_| Read::Entry("cicd".into()))
        );
        assert_eq!(connects.get(), 1);
    }

    /// A part on the open entry whose client couldn't be made says so in
    /// the entry's name, not as "the open cluster".
    #[tokio::test]
    async fn the_open_entry_unreached_is_named_by_its_entry() {
        let sources = sources(&[]);
        let pool = Pool::new(
            &sources,
            Some(Open::<'_, FixtureReader> {
                entry: Some("core"),
                reader: Err(Failure::new(
                    FailureKind::Unreachable,
                    "no answer within 5 s",
                )),
            }),
            |_: Entry| async { Ok::<_, Failure>(FixtureReader::default()) },
            BUDGET,
        );
        let Err(unread) = pool.part(Part::ArgoCd).await else {
            panic!("Argo CD is read on the open entry, which has no client");
        };
        assert_eq!(unread.words(), "Couldn't reach core: no answer within 5 s");
    }

    /// Clients are made at most [`MAX_AT_ONCE`] at a time.
    #[tokio::test(start_paused = true)]
    async fn at_most_four_entries_connect_at_once() {
        let environments: Vec<String> = (0..7).map(|n| format!("env-{n}")).collect();
        let sources = sources(&environments);
        let (now, most) = (Rc::new(Cell::new(0)), Rc::new(Cell::new(0)));
        let pool = Pool::new(
            &sources,
            None::<Open<'_, FixtureReader>>,
            |_: Entry| {
                let (now, most) = (now.clone(), most.clone());
                async move {
                    now.set(now.get() + 1);
                    most.set(most.get().max(now.get()));
                    tokio::time::sleep(Duration::from_millis(10)).await;
                    now.set(now.get() - 1);
                    Ok::<_, Failure>(FixtureReader::default())
                }
            },
            BUDGET,
        );
        let until = pool.destination();
        let asked =
            futures::future::join_all(environments.iter().map(|id| pool.entry(id, &until))).await;
        assert!(asked.iter().all(Result::is_ok));
        assert_eq!(most.get(), MAX_AT_ONCE);
    }

    /// An entry that never answers is unreachable once the read's budget
    /// ends, in its own name; one asked after is never connected to.
    #[tokio::test(start_paused = true)]
    async fn an_entry_that_never_answers_ends_with_the_budget() {
        let sources = sources(&[]);
        let pool = Pool::new(
            &sources,
            None::<Open<'_, FixtureReader>>,
            |_: Entry| std::future::pending::<Result<FixtureReader, Failure>>(),
            BUDGET,
        );
        let Err(unread) = pool.part(Part::Kargo).await else {
            panic!("connected");
        };
        assert_eq!(
            unread.words(),
            "Couldn't reach cicd: no answer within the read's 45 s"
        );
        let Err(unread) = pool.part(Part::ArgoCd).await else {
            panic!("connected");
        };
        assert_eq!(
            unread.words(),
            "Couldn't reach core: not read within the read's 45 s"
        );
    }

    /// The workspace of one reads the open connection alone, whatever its
    /// budget, and fails as it does.
    #[tokio::test]
    async fn the_workspace_of_one_reads_the_open_connection() {
        let sources = Sources::default();
        let reader = FixtureReader::default();
        let pool = Pool::new(
            &sources,
            Some(Open {
                entry: None,
                reader: Ok(&reader),
            }),
            |_: Entry| async {
                Err::<FixtureReader, _>(Failure::new(FailureKind::Unreachable, "never"))
            },
            Duration::ZERO,
        );
        for part in Part::ALL {
            let (_, read) = pool.part(part).await.expect("open");
            assert_eq!(read, Read::Open);
        }
        let pool = Pool::new(
            &sources,
            Some(Open::<FixtureReader> {
                entry: None,
                reader: Err(Failure::new(FailureKind::Unreachable, "Unauthorized")),
            }),
            |_: Entry| async {
                Err::<FixtureReader, _>(Failure::new(FailureKind::Unreachable, "never"))
            },
            BUDGET,
        );
        let Err(unread) = pool.part(Part::Kargo).await else {
            panic!("read");
        };
        assert_eq!(
            unread.words(),
            "Couldn't reach the open cluster: Unauthorized"
        );
    }
}
