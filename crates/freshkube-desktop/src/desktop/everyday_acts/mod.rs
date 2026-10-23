//! The everyday actions of the Resources page (#857): Scale, Restart and a
//! CronJob's Suspend or Resume. The list asked; this reads the object as it
//! is, describes the action through `freshkube_core::write::everyday`, takes
//! the window's level and hands the write to `write_flow`, where core's
//! `write` module checks and sends it. Nothing here builds a request or
//! decides a level's requirement.

mod scale;

use std::sync::Arc;
use std::time::Duration;

use super::*;
use crate::backend::spawn_job;
use crate::mutation::{level_cell, write_audit_store};
use crate::resources::{ClusterAccess, WorkloadAct};
use crate::write_flow::{self, WriteRequest};
use freshkube_core::write::everyday::{self, Everyday, Row, Subject};
use freshkube_core::write::{ActionDescription, AuditStore, Runner, Who};
use gpui_kit::component::WindowExt;

/// How long reading the object may take before nothing is sent.
const READ: Duration = Duration::from_secs(30);

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
    /// Who is writing, and under what level: the active entry, else the
    /// context the list was read from.
    fn everyday_who(&self, act: &WorkloadAct, cx: &App) -> Who {
        Who {
            entry: self
                .switch
                .active_cluster()
                .map_or_else(|| act.source.context.clone(), str::to_owned),
            context: act.source.context.clone(),
            protection: self.current_level(cx),
            actor: freshkube_core::ambient::get()
                .user
                .clone()
                .unwrap_or_else(|| "unknown".into()),
        }
    }

    pub(super) fn everyday_act(
        &mut self,
        act: &WorkloadAct,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let who = self.everyday_who(act, cx);
        if let Some(reason) = who.protection.refusal() {
            window.push_notification(format!("{reason} Nothing was sent."), cx);
            return;
        }
        let (runtime, cell) = (self.runtime.clone(), level_cell(cx));
        if matches!(act.source.access, ClusterAccess::Example(_)) {
            // Example data: a runner that sends nothing, so a capture shows
            // each dialog at each level. Its audit goes to a scratch place.
            let Some(subject) = Subject::from_object(&example_object(act)) else {
                return;
            };
            let scratch = std::env::temp_dir().join("freshkube-example-audit");
            let runner = {
                let _guard = runtime.enter();
                Runner::example_offline(AuditStore::new(scratch), who).following(cell)
            };
            self.begin_everyday(Arc::new(runner), subject, act.act, window, cx);
            return;
        }
        let Some(audit) = write_audit_store() else {
            window.push_notification("The audit log can't be located, so nothing was sent.", cx);
            return;
        };
        let (access, row) = (act.source.access.clone(), row_of(act));
        let (job, receiver) = spawn_job(
            &runtime,
            READ,
            "The cluster did not answer in time. Nothing was sent.".to_owned(),
            async move {
                let client = access.client().await.map_err(|failure| failure.message)?;
                let subject = Subject::read(&client, &row).await?;
                Ok::<_, String>((client, subject))
            },
        );
        let (what, kind) = (act.identity.name.clone(), act.act);
        cx.spawn_in(window, async move |this, cx| {
            let _job = job;
            let read = receiver
                .await
                .unwrap_or_else(|_| Err("Reading stopped unexpectedly.".to_owned()));
            _ = this.update_in(cx, |this, window, cx| match read {
                Ok((client, subject)) => {
                    let runner = Runner::new(client, audit, who).following(cell);
                    this.begin_everyday(Arc::new(runner), subject, kind, window, cx);
                }
                Err(why) => window.push_notification(
                    format!("Couldn't start the action on {what}: {why} Nothing was sent."),
                    cx,
                ),
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
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let described = match act {
            Everyday::Scale => {
                scale::open(runner, self.runtime.clone(), subject, window, cx);
                return;
            }
            Everyday::Restart => everyday::restart(&subject, &chrono::Utc::now().to_rfc3339()),
            Everyday::Suspend => everyday::suspend(&subject),
        };
        match described {
            Ok(action) => start(runner, self.runtime.clone(), action, window, cx),
            Err(everyday::NotOffered(why)) => window.push_notification(why, cx),
        }
    }
}

/// Starts the write flow for a described action.
pub(super) fn start(
    runner: Arc<Runner>,
    runtime: tokio::runtime::Handle,
    action: ActionDescription,
    window: &mut Window,
    cx: &mut App,
) {
    let request = WriteRequest {
        runner,
        runtime,
        action,
        undo: None,
    };
    let flow = write_flow::start(request, window, cx);
    write_flow::retain(flow, cx);
}
