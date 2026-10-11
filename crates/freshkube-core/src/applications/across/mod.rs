//! Reading the applications across a workspace: each part from the entry
//! that holds it ([`Sources`]), not everything from the open connection.
//!
//! - Kargo from [`Part::Kargo`]'s entry;
//! - Argo CD from [`Part::ArgoCd`]'s, in the namespaces
//!   [`argo_namespaces`](super::read::argo_namespaces) finds there;
//! - the `part-of` labelled workloads from each entry an Argo CD
//!   destination maps to ([`Sources::workloads`]).
//!
//! An entry is read once for everything asked of it. Kargo's and Argo CD's
//! entries are read at once, then the destinations' entries, at most
//! [`MAX_AT_ONCE`] at a time. The open entry is read through the open
//! connection; every other entry through a client made for this read alone,
//! dropped when its read ends. The workspace of one reads the open
//! connection for everything, as [`read_session`](super::read::read_session)
//! does.
//!
//! A part whose entry couldn't be reached, or that no entry holds, is a
//! session [`Absent`] for it: its coverage says so in the entry's name, and
//! nothing reads it as none found.

use std::future::Future;

use futures::StreamExt;

use super::read::read_asked;
use super::{Absent, Asked, SessionInputs, SessionKey};
use crate::delivery::entry::MAX_AT_ONCE;
use crate::delivery::read::Reader;
use crate::workspace::{Chosen, Entry, Part, Sources};

/// How a part no workspace entry holds is keyed: the workspace, which is
/// where it is missing.
pub const WORKSPACE: &str = "the workspace";

/// The open connection, which the open entry and the workspace of one are
/// read through.
pub struct Open<'a, R> {
    /// The workspace entry it is, if it is one.
    pub entry: Option<&'a str>,
    /// How the workspace of one keys it ([`Chosen::Open`]).
    pub key: SessionKey,
    /// Its reader, or why there is none.
    pub reader: Result<&'a R, String>,
}

/// Where one session is read.
enum Place {
    Open,
    Entry(Entry),
    Missing(String),
}

struct Target {
    key: SessionKey,
    place: Place,
    asked: Asked,
}

/// What every part's entry holds, read across the workspace, in this
/// order: Kargo's entry, Argo CD's, then each destination's. `connect`
/// makes a client for an entry, or says why it couldn't.
pub async fn read_workspace<R, C, F, Fut>(
    sources: &Sources,
    open: Option<Open<'_, R>>,
    connect: F,
) -> Vec<SessionInputs>
where
    R: Reader,
    C: Reader,
    F: Fn(Entry) -> Fut,
    Fut: Future<Output = Result<C, String>>,
{
    let targets = targets(sources, open.as_ref());
    let read = |target: Target| {
        let (open, connect) = (&open, &connect);
        async move {
            let Target { key, place, asked } = target;
            let unreachable =
                |why: String| SessionInputs::absent(key.clone(), asked, Absent::Unreachable(why));
            match place {
                Place::Missing(why) => {
                    SessionInputs::absent(key, asked, Absent::NotInWorkspace(why))
                }
                Place::Open => match open.as_ref().map(|open| &open.reader) {
                    Some(Ok(reader)) => read_asked(*reader, key, asked).await,
                    Some(Err(why)) => unreachable(why.clone()),
                    None => unreachable("no cluster is open".into()),
                },
                // The client lives for this read alone.
                Place::Entry(entry) => match connect(entry).await {
                    Ok(reader) => read_asked(&reader, key, asked).await,
                    Err(why) => unreachable(why),
                },
            }
        }
    };
    let (first, rest): (Vec<Target>, Vec<Target>) = targets
        .into_iter()
        .partition(|target| target.asked.kargo || target.asked.argo_cd);
    let mut sessions = futures::future::join_all(first.into_iter().map(read)).await;
    sessions.extend(
        futures::stream::iter(rest)
            .map(read)
            .buffered(MAX_AT_ONCE)
            .collect::<Vec<_>>()
            .await,
    );
    sessions
}

/// Each place to read and what to ask it, one per entry, in the parts'
/// order. Parts no entry holds aren't merged: each says its own why.
fn targets<R>(sources: &Sources, open: Option<&Open<'_, R>>) -> Vec<Target> {
    let kargo = Asked {
        kargo: true,
        ..Asked::NONE
    };
    let argo_cd = Asked {
        argo_cd: true,
        ..Asked::NONE
    };
    let workloads = Asked {
        workloads: true,
        ..Asked::NONE
    };
    let asks = [
        (sources.get(Part::Kargo), kargo),
        (sources.get(Part::ArgoCd), argo_cd),
    ]
    .into_iter()
    .chain(sources.workloads().iter().map(|chosen| (chosen, workloads)));
    let mut targets: Vec<Target> = Vec::new();
    for (chosen, asked) in asks {
        let (key, place) = match chosen {
            Chosen::Open => (
                open.map_or_else(|| SessionKey::new(WORKSPACE), |open| open.key.clone()),
                Place::Open,
            ),
            Chosen::Entry { entry, .. } => (
                SessionKey::new(&entry.id),
                match open.is_some_and(|open| open.entry == Some(entry.id.as_str())) {
                    true => Place::Open,
                    false => Place::Entry(entry.clone()),
                },
            ),
            Chosen::NoEntry(_) => (
                SessionKey::new(WORKSPACE),
                Place::Missing(chosen.missing().unwrap_or_default()),
            ),
            Chosen::Gone(id) => (
                SessionKey::new(id),
                Place::Missing(chosen.missing().unwrap_or_default()),
            ),
        };
        let merges = !matches!(place, Place::Missing(_));
        match targets.iter_mut().find(|target| {
            merges && target.key == key && !matches!(target.place, Place::Missing(_))
        }) {
            Some(target) => target.asked = target.asked.and(asked),
            None => targets.push(Target { key, place, asked }),
        }
    }
    targets
}

#[cfg(test)]
mod tests;
