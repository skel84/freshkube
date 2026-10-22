//! How the change page reaches Coroot: the shell hands it an
//! [`OutcomeReader`], which finds the one connection Observability holds
//! from the setup file when a read starts, so there is a single `Provider`
//! and a single connection identity for the whole app.
use super::*;
use freshkube_core::coroot as api;
use freshkube_core::job::{OwnedJob, spawn_job};
use std::time::Duration;
use tokio::runtime::Handle;
use tokio::sync::oneshot;

/// A little past core's own limit for a whole look, so core's answer
/// ("took too long") wins over the job's.
const DEADLINE: Duration = Duration::from_secs(55);

/// A little past core's own limit for reading databases' pages.
const DATABASES_DEADLINE: Duration = Duration::from_secs(50);

/// Reads the connection Observability holds, when a read starts.
#[derive(Clone)]
pub struct OutcomeReader {
    page: WeakEntity<ObservabilityPage>,
    #[cfg(any(test, feature = "testing"))]
    script: Option<Script>,
}

impl PartialEq for OutcomeReader {
    fn eq(&self, other: &Self) -> bool {
        self.page.entity_id() == other.page.entity_id() && {
            #[cfg(any(test, feature = "testing"))]
            {
                self.script == other.script
            }
            #[cfg(not(any(test, feature = "testing")))]
            {
                true
            }
        }
    }
}

impl std::fmt::Debug for OutcomeReader {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("OutcomeReader")
    }
}

/// The connection a read runs over.
#[derive(Clone)]
pub struct ReadConnection(Via);

#[derive(Clone)]
enum Via {
    Coroot {
        runtime: Handle,
        provider: api::Provider,
    },
    /// Looks the test answers itself, one at a time.
    #[cfg(any(test, feature = "testing"))]
    Scripted(Script),
}

impl OutcomeReader {
    pub fn new(page: &Entity<ObservabilityPage>) -> Self {
        Self {
            page: page.downgrade(),
            #[cfg(any(test, feature = "testing"))]
            script: None,
        }
    }

    /// The connection now, or `None` while Coroot isn't connected (or the
    /// source is example data).
    pub fn connection(&self, cx: &App) -> Option<ReadConnection> {
        #[cfg(any(test, feature = "testing"))]
        if let Some(script) = &self.script {
            return script
                .connected()
                .then(|| ReadConnection(Via::Scripted(script.clone())));
        }
        self.page
            .read_with(cx, |page, _| page.reader_connection())
            .ok()
            .flatten()
    }

    /// Says whether the change page `by` is shown and wants Coroot
    /// connected. It is that page's visibility, kept per page: while any
    /// wants it, a hidden Observability makes the provider the setup's
    /// connection names (and nothing else), when the person's choice is to
    /// reconnect. No longer wanting it drops no provider.
    pub fn want(&self, by: EntityId, wanted: bool, cx: &mut App) {
        #[cfg(any(test, feature = "testing"))]
        if let Some(script) = &self.script {
            let mut script = script.0.borrow_mut();
            if wanted {
                script.connects += 1;
                script.wanting.insert(by);
            } else {
                script.wanting.remove(&by);
            }
            return;
        }
        _ = self
            .page
            .update(cx, |page, cx| page.want_connection(by, wanted, cx));
    }

    /// Calls `then` when the connection starts, finishes, fails or drops.
    pub fn on_connection<T: 'static>(
        &self,
        cx: &mut Context<T>,
        then: impl Fn(&mut T, &mut Context<T>) + 'static,
    ) -> Option<Subscription> {
        let page = self.page.upgrade()?;
        Some(cx.subscribe(&page, move |this, _, event, cx| {
            if matches!(event, ObservabilityEvent::Connection) {
                then(this, cx);
            }
        }))
    }
}

impl ReadConnection {
    /// Which connection this is: an answer read over another is foreign.
    pub fn id(&self) -> api::ProviderId {
        match &self.0 {
            Via::Coroot { provider, .. } => provider.id(),
            #[cfg(any(test, feature = "testing"))]
            Via::Scripted(script) => script.id(),
        }
    }

    /// Looks for the revision `query` names, on Tokio. Dropping the job
    /// cancels the look.
    pub fn look(
        &self,
        query: api::Query,
        listed: Option<api::Listed>,
        window: api::RevisionWindow,
        now: chrono::DateTime<chrono::Utc>,
    ) -> (
        Option<OwnedJob>,
        oneshot::Receiver<Result<api::Answer, String>>,
    ) {
        match &self.0 {
            Via::Coroot { runtime, provider } => {
                let provider = provider.clone();
                let (job, receiver) = spawn_job(
                    runtime,
                    DEADLINE,
                    api::ReadError::Timeout.to_string(),
                    async move { Ok(provider.outcome(&query, listed.as_ref(), window, now).await) },
                );
                (Some(job), receiver)
            }
            #[cfg(any(test, feature = "testing"))]
            Via::Scripted(script) => (None, script.ask(query, listed, window)),
        }
    }
}

impl ReadConnection {
    /// What Coroot says of `wanted` databases over `link`'s project and
    /// cluster, on Tokio. Dropping the job cancels the read.
    pub fn databases(
        &self,
        link: api::Link,
        wanted: Vec<freshkube_core::databases::coroot::Wanted>,
        now: chrono::DateTime<chrono::Utc>,
    ) -> (
        Option<OwnedJob>,
        oneshot::Receiver<Result<freshkube_core::databases::coroot::Evidence, String>>,
    ) {
        match &self.0 {
            Via::Coroot { runtime, provider } => {
                let provider = provider.clone();
                let (job, receiver) = spawn_job(
                    runtime,
                    DATABASES_DEADLINE,
                    api::ReadError::Timeout.to_string(),
                    async move {
                        Ok(
                            freshkube_core::databases::coroot::read(&provider, &link, wanted, now)
                                .await,
                        )
                    },
                );
                (Some(job), receiver)
            }
            #[cfg(any(test, feature = "testing"))]
            Via::Scripted(script) => (None, script.ask_databases(link, wanted)),
        }
    }
}

impl ObservabilityPage {
    /// The connection a change page reads over, when one is open.
    pub(crate) fn reader_connection(&self) -> Option<ReadConnection> {
        if self.fixture {
            return None;
        }
        Some(ReadConnection(Via::Coroot {
            runtime: self.live.runtime.clone(),
            provider: self.live.provider.clone()?,
        }))
    }

    /// A change page is shown, or hidden, by its entity: wanted by any, and
    /// when this page is hidden and not connected, it connects as the first
    /// show would. Idempotent: it does nothing while connected, connecting
    /// or shown, or when the reconnect choice is off.
    pub(crate) fn want_connection(&mut self, by: EntityId, wanted: bool, cx: &mut Context<Self>) {
        if wanted {
            self.wanting.insert(by);
        } else {
            self.wanting.remove(&by);
        }
        if !wanted
            || self.fixture
            || self.live.visible
            || self.live.provider.is_some()
            || self.live.connecting
        {
            return;
        }
        self.restore(crate::connection::ConnectOrigin::Change, cx);
    }
}

/// A reader a test drives: it answers each look when the test says.
#[cfg(any(test, feature = "testing"))]
#[derive(Clone)]
pub struct Script(std::rc::Rc<std::cell::RefCell<Scripted>>);

#[cfg(any(test, feature = "testing"))]
struct Scripted {
    id: api::ProviderId,
    connected: bool,
    asked: Vec<Asked>,
    databases: Vec<AskedDatabases>,
    connects: usize,
    /// The change pages that want the connection.
    wanting: std::collections::BTreeSet<EntityId>,
}

/// A read of databases' pages asked of the script.
#[cfg(any(test, feature = "testing"))]
pub struct AskedDatabases {
    pub id: api::ProviderId,
    pub link: api::Link,
    pub wanted: Vec<freshkube_core::databases::coroot::Wanted>,
    answer: Option<oneshot::Sender<Result<freshkube_core::databases::coroot::Evidence, String>>>,
}

/// A look asked of the script.
#[cfg(any(test, feature = "testing"))]
pub struct Asked {
    /// The connection the look was asked over.
    pub id: api::ProviderId,
    pub query: api::Query,
    pub listed: Option<api::Listed>,
    pub window: api::RevisionWindow,
    answer: Option<oneshot::Sender<Result<api::Answer, String>>>,
}

#[cfg(any(test, feature = "testing"))]
impl PartialEq for Script {
    fn eq(&self, other: &Self) -> bool {
        std::rc::Rc::ptr_eq(&self.0, &other.0)
    }
}

#[cfg(any(test, feature = "testing"))]
impl Default for Script {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(any(test, feature = "testing"))]
impl Script {
    /// A connected script with a connection of its own, that of a server
    /// at an address nobody serves.
    pub fn new() -> Self {
        let provider = api::Provider::new("http://127.0.0.1:1", api::Credentials::None)
            .expect("a local address");
        Self(std::rc::Rc::new(std::cell::RefCell::new(Scripted {
            id: provider.id(),
            connected: true,
            asked: Vec::new(),
            databases: Vec::new(),
            connects: 0,
            wanting: Default::default(),
        })))
    }

    pub fn id(&self) -> api::ProviderId {
        self.0.borrow().id
    }

    pub fn connected(&self) -> bool {
        self.0.borrow().connected
    }

    /// Connects or drops the connection, as the person's Connect and
    /// Disconnect do.
    pub fn set_connected(&self, connected: bool) {
        self.0.borrow_mut().connected = connected;
    }

    /// Another connection: a reconnected server has another id.
    pub fn reconnect(&self) {
        let provider = api::Provider::new("http://127.0.0.1:1", api::Credentials::None)
            .expect("a local address");
        self.0.borrow_mut().id = provider.id();
    }

    /// How many times a page said it wants Coroot connected.
    pub fn connects(&self) -> usize {
        self.0.borrow().connects
    }

    /// How many change pages want Coroot connected now.
    pub fn wanting(&self) -> usize {
        self.0.borrow().wanting.len()
    }

    /// The connection the look at `index` was asked over.
    pub fn id_of(&self, index: usize) -> api::ProviderId {
        self.0.borrow().asked[index].id
    }

    /// The looks asked and not yet answered, oldest first.
    pub fn pending(&self) -> Vec<(api::Query, Option<api::Listed>)> {
        self.0
            .borrow()
            .asked
            .iter()
            .filter(|asked| asked.answer.as_ref().is_some_and(|a| !a.is_closed()))
            .map(|asked| (asked.query.clone(), asked.listed.clone()))
            .collect()
    }

    /// Looks asked, answered or not, since the script began.
    pub fn asked(&self) -> usize {
        self.0.borrow().asked.len()
    }

    /// Whether the look at `index` was dropped, as hiding or replacing a
    /// read drops it.
    pub fn dropped(&self, index: usize) -> bool {
        self.0.borrow().asked[index]
            .answer
            .as_ref()
            .is_none_or(|a| a.is_closed())
    }

    /// Answers the look at `index` with `outcome`, as for the query it was
    /// asked, in the window it was asked, over the connection it was asked
    /// over. Returns whether anyone still waited for it.
    pub fn answer(&self, index: usize, outcome: api::Outcome) -> bool {
        let (id, query, window) = {
            let script = self.0.borrow();
            let asked = &script.asked[index];
            (asked.id, asked.query.clone(), asked.window)
        };
        self.answer_as(
            index,
            api::Answer {
                query,
                window,
                provider: id,
                outcome,
                listed: None,
            },
        )
    }

    /// Answers the look at `index` with `answer`, whoever it is for.
    pub fn answer_as(&self, index: usize, answer: api::Answer) -> bool {
        let sender = self.0.borrow_mut().asked[index].answer.take();
        sender.is_some_and(|sender| sender.send(Ok(answer)).is_ok())
    }

    /// Reads of databases asked, answered or not.
    pub fn databases_asked(&self) -> usize {
        self.0.borrow().databases.len()
    }

    /// What the read at `index` was asked: its link and databases.
    pub fn databases_at(
        &self,
        index: usize,
    ) -> (api::Link, Vec<freshkube_core::databases::coroot::Wanted>) {
        let script = self.0.borrow();
        let asked = &script.databases[index];
        (asked.link.clone(), asked.wanted.clone())
    }

    /// The connection the read at `index` was asked over.
    pub fn databases_id_of(&self, index: usize) -> api::ProviderId {
        self.0.borrow().databases[index].id
    }

    /// Whether the read at `index` was dropped, as hiding drops it.
    pub fn databases_dropped(&self, index: usize) -> bool {
        self.0.borrow().databases[index]
            .answer
            .as_ref()
            .is_none_or(|a| a.is_closed())
    }

    /// Answers the read at `index`; whether anyone still waited for it.
    pub fn answer_databases(
        &self,
        index: usize,
        evidence: freshkube_core::databases::coroot::Evidence,
    ) -> bool {
        let sender = self.0.borrow_mut().databases[index].answer.take();
        sender.is_some_and(|sender| sender.send(Ok(evidence)).is_ok())
    }

    fn ask_databases(
        &self,
        link: api::Link,
        wanted: Vec<freshkube_core::databases::coroot::Wanted>,
    ) -> oneshot::Receiver<Result<freshkube_core::databases::coroot::Evidence, String>> {
        let (sender, receiver) = oneshot::channel();
        let id = self.id();
        self.0.borrow_mut().databases.push(AskedDatabases {
            id,
            link,
            wanted,
            answer: Some(sender),
        });
        receiver
    }

    fn ask(
        &self,
        query: api::Query,
        listed: Option<api::Listed>,
        window: api::RevisionWindow,
    ) -> oneshot::Receiver<Result<api::Answer, String>> {
        let (sender, receiver) = oneshot::channel();
        let id = self.id();
        self.0.borrow_mut().asked.push(Asked {
            id,
            query,
            listed,
            window,
            answer: Some(sender),
        });
        receiver
    }
}

#[cfg(any(test, feature = "testing"))]
impl OutcomeReader {
    /// A reader that looks as `script` says, in place of Observability's
    /// connection.
    pub fn scripted(page: &Entity<ObservabilityPage>, script: &Script) -> Self {
        Self {
            page: page.downgrade(),
            script: Some(script.clone()),
        }
    }
}
