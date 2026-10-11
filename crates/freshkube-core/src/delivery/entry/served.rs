//! The kubeconfigs Talos served for the entries opened through it, kept for
//! the session in memory alone: Talos signs a new admin certificate for
//! every one it serves, so each talosconfig and context asks it once.
//!
//! A key holds the talosconfig's content fingerprint too, so a changed file
//! asks again. Nothing here is logged, shown or written to disk.

use std::path::PathBuf;
use std::sync::{LazyLock, Mutex, PoisonError};
use std::time::Instant;

use kube::config::Kubeconfig;

use crate::client_cache::{ConfigIdentity, TtlCache, file_revision};

/// A talosconfig, what it held when read, and the context read from it.
#[derive(Clone, PartialEq, Eq, Hash)]
pub(super) struct Key {
    path: PathBuf,
    content: Option<ConfigIdentity>,
    context: String,
}

const MAX_ENTRIES: usize = 16;

static SERVED: LazyLock<Mutex<TtlCache<Key, Kubeconfig>>> =
    LazyLock::new(|| Mutex::new(TtlCache::new(None, MAX_ENTRIES)));

/// The key for `context` of the talosconfig at `path`, as it holds now.
/// Reads the file; call it off the async workers.
pub(super) fn key(path: PathBuf, context: String) -> Key {
    Key {
        content: file_revision(&path),
        path,
        context,
    }
}

pub(super) fn get(key: &Key) -> Option<Kubeconfig> {
    SERVED
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .get(key, Instant::now())
}

pub(super) fn store(key: Key, kubeconfig: Kubeconfig) {
    SERVED
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .insert(key, kubeconfig, Instant::now());
}
