//! Connecting to the sources and the session's loop: find them once, then
//! read the active alerts every [`POLL`](super::state::POLL).
use std::time::Duration;

use super::super::{ErrorKind, cluster_client, forget_after};
use super::discover::{Found, Remembered, find};
use super::read::{Scope, active, unknown};
use super::rules;
use super::state::{POLL, State, Unknown};
use crate::cluster_source::ClusterAccess;

/// How long one read of the active alerts may take in all.
const READ_DEADLINE: Duration = Duration::from_secs(40);

/// Finds the sources for `scope`. `access` is the open cluster; a central
/// store's Service is reached through `host`'s, when it isn't the open
/// cluster's. Finding neither source is [`Unknown::NotFound`], or
/// [`Unknown::RulesOnly`] when the cluster holds PrometheusRule objects, which
/// are definitions and no source of alerts.
pub async fn connect(
    access: &ClusterAccess,
    host: Option<&ClusterAccess>,
    scope: &Scope,
    remembered: &Remembered,
) -> Result<Found, Unknown> {
    if let Scope::Unsupported(why) = scope {
        return Err(Unknown::NotFound(why.clone()));
    }
    let reach = host.unwrap_or(access);
    let client = cluster_client(reach).await.map_err(super::read::unknown)?;
    let found = find(&client, remembered, scope.beside())
        .await
        .inspect_err(|error| forget_after(reach, error))
        .map_err(unknown)?;
    if !found.sources.is_empty() {
        return Ok(found);
    }
    // A Service that looks right and doesn't answer is a source that can't
    // be read, not one that isn't there.
    if let Some((label, error)) = found
        .tried
        .iter()
        .find(|(_, error)| matches!(error.kind, ErrorKind::Unavailable | ErrorKind::TimedOut))
    {
        return Err(Unknown::Unread(format!("{label}: {}", error.message)));
    }
    let own = cluster_client(access).await.map_err(unknown)?;
    let objects = rules::any_objects(&own).await.unwrap_or(false);
    Err(if objects && scope == &Scope::Cluster {
        Unknown::RulesOnly
    } else {
        Unknown::NotFound(match found.tried.first() {
            Some((label, error)) => format!("{label} said: {}", error.message),
            None => String::new(),
        })
    })
}

/// Failed reads in a row after which the sources are looked for again.
const REDISCOVER_AFTER: u32 = 3;

/// The session's read, run on Tokio until dropped: connect, then read every
/// minute and hand each state to `publish`. It starts polling only once a
/// source was found; when none is, it publishes that and ends, and a refusal
/// ends it too, since the same ask would be refused again. Once a source was
/// found, nothing ends it: a source that stops answering leaves the last read
/// as stale evidence, is found again after a few failed reads, and a read
/// that works again is current.
pub async fn drive(
    access: ClusterAccess,
    host: Option<ClusterAccess>,
    scope: Scope,
    remembered: Remembered,
    publish: impl FnMut(State),
) {
    drive_from(State::Reading, access, host, scope, remembered, publish).await
}

/// [`drive`] that starts from what an earlier read of the same source
/// knew, so a refresh that fails leaves that as stale evidence rather than
/// forgetting it.
pub async fn drive_from(
    previous: State,
    access: ClusterAccess,
    host: Option<ClusterAccess>,
    scope: Scope,
    remembered: Remembered,
    mut publish: impl FnMut(State),
) {
    let mut ever_found = previous.evidence().is_some();
    let mut state = previous;
    let mut found: Option<Found> = None;
    let mut failures = 0;
    loop {
        if found.is_none() {
            match connect(&access, host.as_ref(), &scope, &remembered).await {
                Ok(connected) => {
                    found = Some(connected);
                    ever_found = true;
                }
                Err(unknown) if ever_found || matches!(unknown, Unknown::Unread(_)) => {
                    // Not there now, or not answering: what was known stays
                    // as stale evidence, and the next tick looks again.
                    let why = match unknown {
                        Unknown::Unread(why) => why,
                        other => other.words(),
                    };
                    state = state.after(Err(Unknown::Unread(why)));
                    publish(state.clone());
                    tokio::time::sleep(POLL).await;
                    continue;
                }
                Err(unknown) => {
                    publish(State::Unknown(unknown));
                    return;
                }
            }
        }
        let Some(connected) = &found else { continue };
        let read = tokio::time::timeout(
            READ_DEADLINE,
            active(&connected.sources, &connected.others, &scope),
        )
        .await
        .unwrap_or_else(|_| Err(Unknown::Unread("the read timed out".into())));
        match &read {
            Err(Unknown::Unread(_)) => {
                failures += 1;
                if failures >= REDISCOVER_AFTER {
                    access.forget();
                    found = None;
                    failures = 0;
                }
            }
            Err(Unknown::Refused(_)) => {
                publish(state.after(read));
                return;
            }
            _ => failures = 0,
        }
        state = state.after(read);
        publish(state.clone());
        tokio::time::sleep(POLL).await;
    }
}
