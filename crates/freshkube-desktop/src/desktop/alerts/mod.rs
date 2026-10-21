//! The session alert read (#884), beside the Kubernetes summary session: one
//! read of the active alerts per live session, which feeds Overview's
//! attention rows and the rail's Monitoring mark.
//!
//! Core's `monitoring::alerts::drive` finds the Alertmanager and vmalert
//! once and then reads the active alerts every 60 s, bounded. It starts only
//! once the setup's metrics scope and the cluster are known, publishes
//! "nothing found" and ends when no source answered, and ends on a refusal.
//! The registry's `alerts_watch` is keyed by the context, the connection and
//! the scope, so a context or session change stops it and an answer for the
//! old identity never publishes. What it knows is [`State`]: only a current
//! read with nothing active is quiet; unread, stale or refused is Unknown.
//!
//! One read per live session and one per source: #846's Followed owner reads
//! [`Pilot::alert_state`] instead of polling again.
use std::sync::Arc;
use std::time::Duration;

use freshkube_core::monitoring::alerts::{
    self as alerts, Remembered, Scope, State, example,
};
use freshkube_reads::Pace;
use gpui_kit::Context;

use super::Pilot;
use crate::resources::ClusterAccess;

#[cfg(test)]
mod tests;

/// What the driver sends the shell: a pointer to the latest state, which a
/// slow shell reads instead of every state in between.
pub(super) type AlertFeed = tokio::sync::watch::Receiver<Option<Arc<State>>>;

/// One state at a time: a notification already points at the latest.
const ALERTS_PACE: Pace = Pace {
    capacity: 1,
    most: 1,
    every: Duration::ZERO,
};

/// Whose alerts these are: the context, the connection and how the setup
/// says the entry's metrics are found. A change of any is another read.
pub(super) type AlertIdentity = (String, String, Scope);

/// The active session's alert read: what it was started for, and the last
/// state it published.
pub(super) struct AlertRead {
    pub(super) identity: AlertIdentity,
    pub(super) state: Arc<State>,
}

impl Pilot {
    /// The session's alert state, for the Overview, the rail and #846's
    /// Followed owner. `None` before a read started or after it stopped.
    pub(super) fn alert_state(&self) -> Option<&Arc<State>> {
        self.switch
            .registry
            .active()
            .alerts
            .as_ref()
            .map(|read| &read.state)
    }

    /// Starts the read for the open connection and the setup's scope, or
    /// leaves the one running when it is already for them. Another
    /// identity stops the old read first.
    pub(super) fn ensure_alerts(&mut self, cx: &mut Context<Self>) {
        let Some(source) = self.source().kube else {
            return;
        };
        let monitoring = self.pages.monitoring.read(cx);
        let scope = Scope::of(&monitoring.scope().resolved);
        let host = monitoring.scope().host.clone();
        let identity = (source.context.clone(), source.id.clone(), scope.clone());
        if self
            .switch
            .registry
            .active()
            .alerts
            .as_ref()
            .is_some_and(|read| read.identity == identity)
        {
            return;
        }
        self.stop_alerts(cx);
        self.switch.registry.active_mut().alerts = Some(AlertRead {
            identity: identity.clone(),
            state: Arc::new(State::Reading),
        });
        match source.access.clone() {
            ClusterAccess::Example(hold) => {
                // Held, it stays unread; otherwise example data answers at
                // once, in the shape `FRESHKUBE_ALERTS` names.
                if !hold.monitoring {
                    let shape = self.alerts_shape;
                    self.apply_alerts(identity, example::state(shape, chrono::Utc::now()), cx);
                }
            }
            access => self.start_alerts(identity, access, host, scope, cx),
        }
        self.rebuild_joined_nodes(cx);
    }

    fn start_alerts(
        &mut self,
        identity: AlertIdentity,
        access: ClusterAccess,
        host: Option<ClusterAccess>,
        scope: Scope,
        cx: &mut Context<Self>,
    ) {
        let runtime = self.runtime.clone();
        self.switch.registry.alerts_watch.start(
            identity,
            &runtime,
            ALERTS_PACE,
            move |sender| run_alerts(access, host, scope, sender),
            |view: &mut Self| &mut view.switch.registry.alerts_watch,
            Self::deliver_alerts,
            cx,
        );
    }

    /// Drops the read and what it knew; nothing of it publishes again.
    pub(super) fn stop_alerts(&mut self, _: &mut Context<Self>) {
        self.switch.registry.alerts_watch.stop();
        self.switch.registry.active_mut().alerts = None;
    }

    /// Reads again now, as Refresh asks: a new request for the same
    /// identity, whose answer replaces the last.
    pub(super) fn refresh_alerts(&mut self, cx: &mut Context<Self>) {
        self.stop_alerts(cx);
        self.ensure_alerts(cx);
    }

    /// The latest state of a batch, taken only for the identity the watch
    /// started: the watch has already refused a stale start's.
    fn deliver_alerts(&mut self, batch: Vec<AlertFeed>, cx: &mut Context<Self>) {
        let Some(identity) = self.switch.registry.alerts_watch.identity().cloned() else {
            return;
        };
        let Some(state) = batch.last().and_then(|feed| feed.borrow().clone()) else {
            return;
        };
        self.apply_alerts(identity, (*state).clone(), cx);
    }

    /// Publishes `state` if the session still reads for `identity`.
    pub(super) fn apply_alerts(
        &mut self,
        identity: AlertIdentity,
        state: State,
        cx: &mut Context<Self>,
    ) {
        let Some(read) = self.switch.registry.active_mut().alerts.as_mut() else {
            return;
        };
        if read.identity != identity || *read.state == state {
            return;
        }
        read.state = Arc::new(state);
        crate::desktop::probe::hit("alerts.apply");
        self.rebuild_joined_nodes(cx);
        cx.notify();
    }
}

/// Runs the session's read on Tokio and sends each state down the channel.
async fn run_alerts(
    access: ClusterAccess,
    host: Option<ClusterAccess>,
    scope: Scope,
    sender: tokio::sync::mpsc::Sender<AlertFeed>,
) {
    let (latest, feed) = tokio::sync::watch::channel(None);
    alerts::drive(access, host, scope, Remembered::default(), move |state| {
        latest.send_replace(Some(Arc::new(state)));
        // A full channel already points at this latest value.
        _ = sender.try_send(feed.clone());
    })
    .await;
}
