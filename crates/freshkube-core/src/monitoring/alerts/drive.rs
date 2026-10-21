//! Connecting to the sources and the session's loop: find them once, then
//! read the active alerts every [`POLL`](super::state::POLL).
use std::time::Duration;

use super::super::{cluster_client, forget_after};
use super::discover::{Found, LOOKED_FOR, Remembered, find};
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
    let own = cluster_client(access).await.map_err(unknown)?;
    let objects = rules::any_objects(&own).await.unwrap_or(false);
    Err(if objects && scope == &Scope::Cluster {
        Unknown::RulesOnly
    } else {
        Unknown::NotFound(match found.tried.first() {
            Some((label, error)) => format!("{LOOKED_FOR}; {label} said: {}", error.message),
            None => LOOKED_FOR.to_owned(),
        })
    })
}

/// The session's read, run on Tokio until dropped: connect, then read every
/// minute and hand each state to `publish`. It starts polling only once a
/// source was found; when none is, it publishes that and ends, and a refusal
/// ends it too, since the same ask would be refused again. A source that
/// stops answering is found again on the next tick.
pub async fn drive(
    access: ClusterAccess,
    host: Option<ClusterAccess>,
    scope: Scope,
    remembered: Remembered,
    mut publish: impl FnMut(State),
) {
    let mut state = State::Reading;
    let mut found: Option<Found> = None;
    loop {
        if found.is_none() {
            match connect(&access, host.as_ref(), &scope, &remembered).await {
                Ok(connected) => found = Some(connected),
                Err(unknown @ Unknown::Unread(_)) => {
                    state = state.after(Err(unknown));
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
        if matches!(read, Err(Unknown::Unread(_))) {
            access.forget();
            found = None;
        }
        if matches!(read, Err(Unknown::Refused(_))) {
            publish(state.after(read));
            return;
        }
        state = state.after(read);
        publish(state.clone());
        tokio::time::sleep(POLL).await;
    }
}
