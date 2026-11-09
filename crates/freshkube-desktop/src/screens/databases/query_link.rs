//! What the explorer hands the dock to open a Query tab (#948 E2).
//!
//! The Query tab uses the session of the explorer tab it was opened from: no
//! second forward and no second Secret read. It asks for it each time it
//! runs, through [`QueryRequest::live`], so an explorer that connected again
//! is followed and one that closed is "Connection closed". The request holds
//! no credential.
use freshkube_db::{Session, wire::Encryption};
use gpui_kit::*;
use std::{rc::Rc, sync::Arc};

/// The session every read of an explorer tab, and its Query tab, goes through,
/// one statement at a time.
pub(crate) type Shared = Arc<tokio::sync::Mutex<Box<dyn Session>>>;

/// A connected session, as the explorer holds it right now.
#[derive(Clone)]
pub(crate) struct Live {
    pub(crate) session: Shared,
    /// Who the session is: user and Secret kind, never a password.
    pub(crate) who: SharedString,
    /// The pod and local port.
    pub(crate) route: Option<SharedString>,
    pub(crate) encryption: Encryption,
}

/// Finds the explorer's session now.
pub(crate) type LiveFn = Rc<dyn Fn(&App) -> Option<Live>>;

/// Open the Query tab of one database.
#[derive(Clone)]
pub(crate) struct QueryRequest {
    /// The database's UID: one Query tab per database.
    pub(crate) key: String,
    /// The database's name.
    pub(crate) title: String,
    /// The explorer's session now, `None` while it isn't connected or once
    /// its tab is closed.
    pub(crate) live: LiveFn,
    /// Example data only (`FRESHKUBE_DB_EXPLORE=query`): statements the tab
    /// opens with, selected and run.
    pub(crate) start: Option<String>,
}

// By hand: the closure is not printable, and nothing here is a secret.
impl std::fmt::Debug for QueryRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("QueryRequest")
            .field("key", &self.key)
            .field("title", &self.title)
            .finish_non_exhaustive()
    }
}
