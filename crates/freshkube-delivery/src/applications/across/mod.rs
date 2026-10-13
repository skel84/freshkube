//! Reading the applications across a workspace: each part from the entry
//! that holds it ([`Sources`]), not everything from the open connection.
//!
//! - Kargo from [`Part::Kargo`]'s entry;
//! - Argo CD from [`Part::ArgoCd`]'s, in the namespaces
//!   [`argo_namespaces`](super::read::argo_namespaces) finds there;
//! - the `part-of` labelled workloads from Argo CD's entry, its own
//!   cluster, and from each entry an Argo CD destination maps to
//!   ([`Sources::workloads`]).
//!
//! An entry is read once for everything asked of it, at most [`MAX_AT_ONCE`]
//! at a time, Kargo's and Argo CD's entries first, with the rest of the
//! whole read's [`BUDGET`]; a destination's has [`ENTRY_DEADLINE`],
//! connecting included. An entry that didn't connect by then is unreachable
//! in its own name; one that connected but didn't finish says so, and
//! neither keeps another from starting or from being shown. The open entry is
//! read through the open connection; every other entry through a client
//! made for this read alone, dropped when its read ends. The workspace of
//! one reads the open connection for everything, as
//! [`read_session`](super::read::read_session) does, without those
//! deadlines.
//!
//! A part whose entry couldn't be reached, or that no entry holds, is a
//! session [`Absent`] for it: its coverage says so in the entry's name, and
//! nothing reads it as none found.

use std::future::Future;
use std::time::Duration;

use futures::StreamExt;

use super::read::read_asked;
use super::{Absent, Asked, SessionInputs, SessionKey};
use crate::entry::MAX_AT_ONCE;
#[cfg(test)]
use crate::entry::pool::ENTRY_DEADLINE;
use crate::entry::pool::{Place, Unread, Until, Via, reach};
use crate::read::Reader;
use freshkube_core::resources::Failure;
use freshkube_core::workspace::{Chosen, Entry, Part, Sources};
/// How long the whole read may take: two rounds of entries, well inside the
/// page's own deadline. What answered by then is kept.
pub const BUDGET: Duration = Duration::from_secs(45);

/// How a part no workspace entry holds is keyed: the workspace, which is
/// where it is missing, and the part, so two missing parts stay two.
pub const WORKSPACE: &str = "the workspace";

/// The session of `part` when no workspace entry holds it.
pub(crate) fn missing_key(part: Part) -> SessionKey {
    SessionKey::new(format!("{WORKSPACE} ({})", part.label()))
}

/// The open connection, which the open entry and the workspace of one are
/// read through.
pub struct Open<'a, R> {
    /// The workspace entry it is, if it is one.
    pub entry: Option<&'a str>,
    /// How the workspace of one keys it ([`Chosen::Open`]).
    pub key: SessionKey,
    /// Its reader, or why there is none.
    pub reader: Result<&'a R, Failure>,
}

struct Target {
    key: SessionKey,
    place: Place,
    asked: Asked,
    /// Under [`BUDGET`]: everything but the workspace of one.
    bounded: bool,
    /// Kargo's or Argo CD's: read first, with the rest of the budget.
    first: bool,
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
    Fut: Future<Output = Result<C, Failure>>,
{
    let targets = targets(sources, open.as_ref());
    let shapes: Vec<(SessionKey, Asked)> = targets
        .iter()
        .map(|target| (target.key.clone(), target.asked))
        .collect();
    let ends = tokio::time::Instant::now() + BUDGET;
    let read = |(number, target): (usize, Target)| {
        let (open, connect) = (&open, &connect);
        async move {
            let Target {
                key,
                place,
                asked,
                bounded,
                first,
            } = target;
            let until = match (bounded, first) {
                (false, _) => Until::none(),
                (true, true) => Until::first(ends, BUDGET),
                (true, false) => Until::destination(ends, BUDGET),
            };
            let open = open
                .as_ref()
                .map(|open| open.reader.as_ref().map_err(|why| why).copied());
            let session = match reach(place, open, connect, &until).await {
                Err(Unread::Missing(why)) => {
                    SessionInputs::absent(key, asked, Absent::NotInWorkspace(why.message))
                }
                Err(unread) => {
                    SessionInputs::absent(key, asked, Absent::Unreachable(unread.why().to_owned()))
                }
                // The cluster answered, but not everything asked of it did.
                Ok(reader) => {
                    let read = match &reader {
                        Via::Open(reader) => {
                            until.bound(read_asked(*reader, key.clone(), asked)).await
                        }
                        // The client lives for this read alone.
                        Via::Entry(reader) => {
                            until.bound(read_asked(reader, key.clone(), asked)).await
                        }
                    };
                    read.unwrap_or_else(|| {
                        SessionInputs::unfinished(key, asked, &until.unfinished())
                    })
                }
            };
            (number, session)
        }
    };
    // One queue, in the parts' order, so Kargo's and Argo CD's entries
    // start first; each answer is kept as it comes, whatever its place.
    // Every bounded read ends by itself by the budget's end, saying why.
    let mut sessions: Vec<Option<SessionInputs>> = vec![None; shapes.len()];
    let mut answers = futures::stream::iter(targets.into_iter().enumerate())
        .map(read)
        .buffer_unordered(MAX_AT_ONCE);
    while let Some((number, session)) = answers.next().await {
        sessions[number] = Some(session);
    }
    sessions
        .into_iter()
        .zip(shapes)
        .map(|(session, (key, asked))| answered(session, key, asked))
        .collect()
}

/// A session's answer, or, for one that never answered, unreachable: never
/// [`SessionInputs::asking`], which reads as read and empty.
fn answered(session: Option<SessionInputs>, key: SessionKey, asked: Asked) -> SessionInputs {
    session.unwrap_or_else(|| {
        SessionInputs::absent(key, asked, Absent::Unreachable("no answer".into()))
    })
}

/// Each place to read and what to ask it, one per entry, in the parts'
/// order. Parts no entry holds aren't merged: each says its own why.
fn targets<R>(sources: &Sources, open: Option<&Open<'_, R>>) -> Vec<Target> {
    let kargo = Asked {
        kargo: true,
        ..Asked::NONE
    };
    // Argo CD's own cluster is a destination of its own, which no mapping
    // names: its labelled workloads are read with it.
    let argo_cd = Asked {
        argo_cd: true,
        workloads: true,
        ..Asked::NONE
    };
    let workloads = Asked {
        workloads: true,
        ..Asked::NONE
    };
    let asks = [
        (sources.get(Part::Kargo), kargo, Some(Part::Kargo)),
        (sources.get(Part::ArgoCd), argo_cd, Some(Part::ArgoCd)),
    ]
    .into_iter()
    .chain(
        sources
            .workloads()
            .iter()
            .map(|chosen| (chosen, workloads, None)),
    );
    let mut targets: Vec<Target> = Vec::new();
    for (chosen, asked, part) in asks {
        let bounded = !matches!(chosen, Chosen::Open);
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
                part.map_or_else(|| SessionKey::new(WORKSPACE), missing_key),
                Place::Missing(chosen.missing().unwrap_or_default()),
            ),
            Chosen::Gone(id) => (
                SessionKey::new(id),
                Place::Missing(chosen.missing().unwrap_or_default()),
            ),
        };
        // A part no entry holds is missing for what it is alone.
        let asked = match (&place, asked == argo_cd) {
            (Place::Missing(_), true) => Asked {
                workloads: false,
                ..asked
            },
            _ => asked,
        };
        let merges = !matches!(place, Place::Missing(_));
        match targets.iter_mut().find(|target| {
            merges && target.key == key && !matches!(target.place, Place::Missing(_))
        }) {
            Some(target) => target.asked = target.asked.and(asked),
            None => targets.push(Target {
                key,
                place,
                asked,
                bounded,
                first: part.is_some(),
            }),
        }
    }
    targets
}

#[cfg(test)]
mod tests;
