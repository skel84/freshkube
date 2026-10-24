//! The everyday actions of the Resources page (#857): Scale, Restart, a
//! Deployment's Roll back and a CronJob's Suspend or Resume and Run now. The list asked; this reads the object as it
//! is, describes the action through `freshkube_core::write::everyday`, takes
//! the window's level and hands the write to `write_flow`, where core's
//! `write` module checks and sends it. Nothing here builds a request or
//! decides a level's requirement.

mod rollback;
mod scale;

use std::collections::BTreeSet;
use std::sync::Arc;
use std::time::Duration;

use super::*;
use crate::backend::spawn_job;
use crate::mutation::{Operations, entry_level_cell, write_audit_store};
use crate::resources::model::ResourceIdentity;
use crate::resources::{ClusterAccess, WorkloadAct};
use crate::write_flow::{self, WriteRequest};
use freshkube_core::write::everyday::{self, Autoscaler, Everyday, Revisions, Row, Subject};
use freshkube_core::write::{ActionDescription, Runner, Who};
use gpui_kit::component::WindowExt;

/// How long reading the object may take before nothing is sent.
const READ: Duration = Duration::from_secs(30);

/// The objects an action is in flight for, from the press until its dialog
/// closes: a repeated press on one is ignored, so a stray key cannot start a
/// second read, dialog or send for it.
#[derive(Default)]
struct InFlight(BTreeSet<ResourceIdentity>);

impl gpui_kit::Global for InFlight {}

/// Takes the object for an action. False when it already has one in flight,
/// or another write holds the app's one write slot.
fn claim(identity: &ResourceIdentity, cx: &mut App) -> bool {
    if Operations::current(cx).is_some() {
        return false;
    }
    cx.default_global::<InFlight>().0.insert(identity.clone())
}

pub(super) fn release(identity: &ResourceIdentity, cx: &mut App) {
    cx.default_global::<InFlight>().0.remove(identity);
}

/// Where the press happened: the entry the window works under and the context
/// it shows. A read that returns after either changed is for another cluster
/// and is dropped.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Where {
    entry: String,
    context: String,
}

/// The object an example row stands for: example data has no cluster to read.
fn example_object(act: &WorkloadAct) -> serde_json::Value {
    let api_version = if act.kind.group.is_empty() {
        act.kind.version.clone()
    } else {
        format!("{}/{}", act.kind.group, act.kind.version)
    };
    let mut metadata = serde_json::json!({
        "name": act.identity.name,
        "uid": act.identity.uid,
        "resourceVersion": "1",
    });
    if !act.identity.namespace.is_empty() {
        metadata["namespace"] = serde_json::json!(act.identity.namespace);
    }
    serde_json::json!({
        "apiVersion": api_version,
        "kind": act.kind.kind,
        "metadata": metadata,
        "spec": {"replicas": 3, "suspend": false},
    })
}

fn row_of(act: &WorkloadAct) -> Row {
    Row {
        group: act.kind.group.clone(),
        version: act.kind.version.clone(),
        kind: act.kind.kind.clone(),
        resource: act.kind.plural.clone(),
        namespace: (!act.identity.namespace.is_empty()).then(|| act.identity.namespace.clone()),
        name: act.identity.name.clone(),
        uid: act.identity.uid.clone(),
    }
}

impl Pilot {
    /// The entry and context the window is on now.
    fn everyday_where(&self, act: &WorkloadAct, cx: &App) -> Where {
        Where {
            entry: self
                .switch
                .active_cluster()
                .map_or_else(|| act.source.context.clone(), str::to_owned),
            context: self.display_context(cx),
        }
    }

    /// Who is writing, and under what level, as of the press.
    fn everyday_who(&self, act: &WorkloadAct, at: &Where, cx: &App) -> Who {
        Who {
            entry: at.entry.clone(),
            context: act.source.context.clone(),
            protection: self.current_level(cx),
            actor: super::write_runner::actor(),
        }
    }

    pub(super) fn everyday_act(
        &mut self,
        act: &WorkloadAct,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let at = self.everyday_where(act, cx);
        let who = self.everyday_who(act, &at, cx);
        if let Some(reason) = who.protection.refusal() {
            window.push_notification(format!("{reason} Nothing was sent."), cx);
            return;
        }
        if !claim(&act.identity, cx) {
            return;
        }
        // The cell of the entry the press fixed, not the window's: a switch to
        // another cluster while the dialog is open must not change the level
        // the write is held to or audited under.
        let runtime = self.runtime.clone();
        let cell = entry_level_cell(&at.entry, who.protection, cx);
        if matches!(act.source.access, ClusterAccess::Example(_)) {
            // Example data: a runner that sends nothing. The press keeps the
            // object until `begin_everyday` is done with it, on every path.
            // The example cluster's own object when it has one, so what the
            // dialog says (an owner, an autoscaler, Argo CD) is what the
            // page shows; an identity it doesn't know gets a plain one.
            let now = chrono::Utc::now().timestamp();
            let object = crate::resources::example::object_json(&act.identity, now)
                .unwrap_or_else(|| example_object(act));
            let Some(subject) = Subject::from_object(&object) else {
                release(&act.identity, cx);
                return;
            };
            let subject = subject.with_autoscaler(
                crate::resources::example::autoscaler_of(&act.identity)
                    .map_or(Autoscaler::None, |name| Autoscaler::Found(name.into())),
            );
            let revisions = crate::resources::example::replicasets_of(&act.identity, now);
            let subject = if act.act == Everyday::Rollback {
                let found = Revisions::from_replicasets(&object, &act.identity.uid, &revisions);
                subject.with_revisions(found)
            } else {
                subject
            };
            let runner = super::write_runner::example_runner(&runtime, who, cell);
            self.begin_everyday(runner, subject, act.act, &act.identity, window, cx);
            return;
        }
        let Some(audit) = write_audit_store() else {
            release(&act.identity, cx);
            window.push_notification("The audit log can't be located, so nothing was sent.", cx);
            return;
        };
        let (access, row, wants_revisions) = (
            act.source.access.clone(),
            row_of(act),
            act.act == Everyday::Rollback,
        );
        let (job, receiver) = spawn_job(
            &runtime,
            READ,
            "The cluster did not answer in time. Nothing was sent.".to_owned(),
            async move {
                let client = access.client().await.map_err(|failure| failure.message)?;
                let mut subject = Subject::read(&client, &row).await?;
                if wants_revisions {
                    subject = subject.read_revisions(&client).await;
                }
                Ok::<_, String>((client, subject))
            },
        );
        let (identity, kind, pressed) = (act.identity.clone(), act.act, act.clone());
        cx.spawn_in(window, async move |this, cx| {
            let _job = job;
            let read = receiver
                .await
                .unwrap_or_else(|_| Err("Reading stopped unexpectedly.".to_owned()));
            _ = this.update_in(cx, |this, window, cx| {
                let what = identity.name.clone();
                // The window may have moved to another cluster while the
                // object was read: the level it works under is that
                // cluster's now, so what was asked for is not sent.
                if this.everyday_where(&pressed, cx) != at {
                    release(&identity, cx);
                    window.push_notification(
                        format!(
                            "The cluster changed while {what} was read. Nothing was sent; ask again."
                        ),
                        cx,
                    );
                    return;
                }
                match read {
                    Ok((client, subject)) => {
                        let runner = Runner::new(client, audit, who).following(cell);
                        this.begin_everyday(Arc::new(runner), subject, kind, &identity, window, cx);
                    }
                    Err(why) => {
                        release(&identity, cx);
                        window.push_notification(
                            format!("Couldn't start the action on {what}: {why} Nothing was sent."),
                            cx,
                        );
                    }
                }
            });
        })
        .detach();
    }

    /// Describes the action on the object as read, or asks for what it needs
    /// first (Scale's count).
    pub(super) fn begin_everyday(
        &mut self,
        runner: Arc<Runner>,
        subject: Subject,
        act: Everyday,
        identity: &ResourceIdentity,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // Refused here, before any dialog, whatever the menu offered.
        if act == Everyday::Scale
            && let Some(why) = subject.scale_refusal()
        {
            release(identity, cx);
            window.push_notification(why, cx);
            return;
        }
        if act == Everyday::Rollback {
            // The picker keeps the object claimed until it is gone.
            if let Some(why) = subject.rollback_refusal() {
                release(identity, cx);
                window.push_notification(why, cx);
                return;
            }
            rollback::open(
                runner,
                self.runtime.clone(),
                subject,
                identity.clone(),
                window,
                cx,
            );
            return;
        }
        let described = match act {
            // The count dialog keeps the object claimed until it is gone.
            Everyday::Scale => {
                scale::open(
                    runner,
                    self.runtime.clone(),
                    subject,
                    identity.clone(),
                    window,
                    cx,
                );
                return;
            }
            Everyday::Restart => everyday::restart(&subject, &chrono::Utc::now().to_rfc3339()),
            Everyday::Suspend => everyday::suspend(&subject),
            Everyday::RunNow => everyday::run_now(&subject, &job_suffix()),
            Everyday::Rollback => unreachable!("handled above"),
        };
        match described {
            Ok(action) => {
                // The press is over; `start` takes the object for the flow.
                release(identity, cx);
                start(runner, self.runtime.clone(), action, identity, window, cx);
            }
            Err(everyday::NotOffered(why)) => {
                release(identity, cx);
                window.push_notification(why, cx);
            }
        }
    }
}

/// Five lowercase letters and digits, new for every press, so a second Run
/// now makes a second Job; a name that is taken anyway is refused by the dry
/// run.
fn job_suffix() -> String {
    (0..5)
        .map(|_| char::from_digit(fastrand::u32(0..36), 36).unwrap_or('x'))
        .collect()
}

/// Starts the write flow for a described action.
pub(super) fn start(
    runner: Arc<Runner>,
    runtime: tokio::runtime::Handle,
    action: ActionDescription,
    identity: &ResourceIdentity,
    window: &mut Window,
    cx: &mut App,
) {
    if !claim(identity, cx) {
        return;
    }
    let request = WriteRequest {
        runner,
        runtime,
        action,
        undo: None,
    };
    let flow = write_flow::start(request, window, cx);
    // Held until its dialog closes, however it ends.
    let identity = identity.clone();
    cx.observe(&flow, move |flow, cx| {
        if flow.read(cx).is_closed() {
            release(&identity, cx);
        }
    })
    .detach();
    write_flow::retain(flow, cx);
}
