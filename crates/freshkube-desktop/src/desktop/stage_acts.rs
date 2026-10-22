//! A Stage action of the change page (#855): Refresh, Reverify, Promote,
//! Abort. The page asked; this describes the action through
//! `freshkube_delivery::acts`, takes the levels (the stricter of the Kargo
//! cluster's and the destination's), connects to Kargo's cluster, and hands
//! the write to `write_flow`, where core's `write` module checks and sends it.
//! Nothing here builds a request or decides a level's requirement.

use std::sync::Arc;
use std::time::Duration;

use super::*;
use crate::applications::{ActKind, KargoAt, StageAct};
use crate::backend::spawn_job;
use crate::mutation::{entry_level_cell, level_cell, write_audit_store};
use crate::write_flow::{self, WriteRequest};
use freshkube_core::workspace::{Entry, Role, Workspace};
use freshkube_core::write::{ActionDescription, AuditStore, LevelCell, Protection, Runner, Who};
use freshkube_delivery::acts::{self, Pick, Slot, StageActs};
use freshkube_delivery::change::Destination as StageDestination;
use freshkube_delivery::entry;
use gpui_kit::component::WindowExt;

/// How long connecting to Kargo's cluster may take before nothing is sent.
const CONNECT: Duration = Duration::from_secs(30);

/// A refresh token that differs for every press, even two in one instant:
/// Kargo reconciles when the annotation's value changes, so a repeated value
/// would register once.
fn unique_token(now: chrono::DateTime<chrono::Utc>) -> String {
    use std::sync::atomic::{AtomicU64, Ordering};
    static PRESSES: AtomicU64 = AtomicU64::new(0);
    format!(
        "{}-{}",
        now.to_rfc3339_opts(chrono::SecondsFormat::Nanos, true),
        PRESSES.fetch_add(1, Ordering::Relaxed)
    )
}

#[cfg(test)]
mod tests {
    use super::unique_token;

    #[test]
    fn two_presses_in_the_same_instant_get_different_tokens() {
        let now = chrono::Utc::now();
        assert_ne!(unique_token(now), unique_token(now));
    }
}

/// Where the writes go and what holds them: the Kargo cluster's entry, its
/// level now, and the cell a runner follows.
struct KargoCluster {
    entry: Entry,
    level: Protection,
    cell: LevelCell,
}

impl Pilot {
    /// The level an entry works under: what it sets, with the stricter of any
    /// entry naming the same context and what a skipped row locked it to.
    fn level_of_entry(&self, entry: &Entry, cx: &App) -> Protection {
        let settings = self.settings_page.read(cx);
        settings
            .workspace()
            .level_of(&entry.context, &settings.source_recovery)
            .stricter(entry.level())
    }

    /// The workspace entry a destination names, by id or by context.
    fn entry_named(&self, name: &str, cx: &App) -> Option<Entry> {
        let workspace: &Workspace = self.settings_page.read(cx).workspace();
        workspace
            .clusters
            .iter()
            .find(|entry| entry.id == name || entry.context == name)
            .cloned()
    }

    fn kargo_cluster(&self, act: &StageAct, cx: &mut App) -> KargoCluster {
        let active = self.switch.active_cluster().map(str::to_owned);
        let entry = match &act.kargo {
            KargoAt::Entry(entry) => (**entry).clone(),
            KargoAt::Open => {
                let id = active.clone().unwrap_or_else(|| "open".into());
                self.entry_named(&id, cx)
                    .unwrap_or_else(|| Entry::new(id, Role::Cicd, self.display_context(cx)))
            }
            KargoAt::Example => self
                .entry_named(&act.change.kargo_cluster, cx)
                .unwrap_or_else(|| {
                    Entry::new(
                        act.change.kargo_cluster.clone(),
                        Role::Cicd,
                        act.change.kargo_cluster.clone(),
                    )
                }),
        };
        let level = self.level_of_entry(&entry, cx);
        // The window's own cell when Kargo is the cluster it shows, so a
        // level changed in Settings reaches a dialog already open.
        let cell = if active.as_deref() == Some(entry.id.as_str()) {
            level_cell(cx)
        } else {
            entry_level_cell(&entry.id, level, cx)
        };
        KargoCluster { entry, level, cell }
    }

    /// Describes the action `act` asks for, or `None` when it isn't on offer.
    fn describe_stage_act(
        &self,
        act: &StageAct,
        stage_acts: &StageActs,
        cx: &App,
    ) -> Option<ActionDescription> {
        let now = chrono::Utc::now();
        let token = unique_token(now);
        Some(match act.kind {
            ActKind::RefreshStage => acts::refresh_stage(stage_acts, &token),
            ActKind::RefreshWarehouse => acts::refresh_warehouse(stage_acts, &token)?,
            ActKind::Reverify => acts::reverify(stage_acts)?,
            ActKind::Abort => match stage_acts.slot() {
                Slot::Abort(running) => acts::abort(stage_acts, running, &act.change),
                _ => return None,
            },
            ActKind::Promote => {
                if stage_acts.slot() != Slot::Promote {
                    return None;
                }
                let stage = act.change.stages.iter().find(|s| s.name == act.stage)?;
                let destination = match &stage.cluster {
                    StageDestination::Cluster(name)
                    | StageDestination::Entry { entry: name, .. } => self
                        .entry_named(name, cx)
                        .map(|entry| (entry.id.clone(), self.level_of_entry(&entry, cx))),
                    StageDestination::Unknown(_) => None,
                };
                let suffix = format!("{:x}", now.timestamp_millis());
                acts::promote(
                    &act.change,
                    &[Pick {
                        acts: stage_acts,
                        destination,
                    }],
                    &suffix,
                )
            }
        })
    }

    pub(super) fn stage_act(
        &mut self,
        act: &StageAct,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(stage) = act.change.stages.iter().find(|s| s.name == act.stage) else {
            return;
        };
        let Some(stage_acts) = stage.acts.clone() else {
            return;
        };
        let Some(action) = self.describe_stage_act(act, &stage_acts, cx) else {
            return;
        };
        let kargo = self.kargo_cluster(act, cx);
        let who = Who {
            entry: kargo.entry.id.clone(),
            context: kargo.entry.context.clone(),
            protection: kargo.level,
            actor: freshkube_core::ambient::get()
                .user
                .clone()
                .unwrap_or_else(|| "unknown".into()),
        };
        let runtime = self.runtime.clone();
        if matches!(act.kargo, KargoAt::Example) {
            // Example data: a runner that sends nothing, so a capture shows
            // each dialog at each level. Its audit goes to a scratch place.
            let scratch = std::env::temp_dir().join("freshkube-example-audit");
            let runner = {
                let _guard = runtime.enter();
                Runner::example_offline(AuditStore::new(scratch), who).following(kargo.cell)
            };
            self.start_stage_write(Arc::new(runner), action, window, cx);
            return;
        }
        let Some(audit) = write_audit_store() else {
            window.push_notification("The audit log can't be located, so nothing was sent.", cx);
            return;
        };
        let kubeconfigs = freshkube_core::resources::kubeconfig_sources(act.kubeconfig.as_deref());
        let name = kargo.entry.id.clone();
        let (job, receiver) = spawn_job(
            &runtime,
            CONNECT,
            "Kargo's cluster did not answer in time.".to_owned(),
            async move {
                entry::connect_writer(kubeconfigs, kargo.entry)
                    .await
                    .map_err(|failure| failure.message)
            },
        );
        let cell = kargo.cell;
        cx.spawn_in(window, async move |this, cx| {
            let _job = job;
            let connected = receiver
                .await
                .unwrap_or_else(|_| Err("Connecting stopped unexpectedly.".to_owned()));
            _ = this.update_in(cx, |this, window, cx| match connected {
                Ok(client) => {
                    let runner = Runner::new(client, audit, who).following(cell);
                    this.start_stage_write(Arc::new(runner), action, window, cx);
                }
                Err(why) => window.push_notification(
                    format!("Couldn't connect to {name}: {why}. Nothing was sent."),
                    cx,
                ),
            });
        })
        .detach();
    }

    fn start_stage_write(
        &mut self,
        runner: Arc<Runner>,
        action: ActionDescription,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let request = WriteRequest {
            runner,
            runtime: self.runtime.clone(),
            action,
            undo: None,
        };
        let flow = write_flow::start(request, window, cx);
        write_flow::retain(flow, cx);
    }
}
