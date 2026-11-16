//! What every page that starts a write needs to build its runner (#855, #857):
//! who is acting, and the runner example data gets. One copy, so the Stage
//! actions and the Resources actions cannot drift.

use std::sync::Arc;

use freshkube_core::write::{AuditStore, LevelCell, Runner, Who};
use gpui_kit::App;
use tokio::runtime::Handle;

/// The user the audit log names as the actor.
pub(super) fn actor() -> String {
    freshkube_core::ambient::get()
        .user
        .clone()
        .unwrap_or_else(|| "unknown".into())
}

/// A runner for example data: it shows each dialog at its level and sends
/// nothing. Its audit goes to a scratch place.
pub(super) fn example_runner(runtime: &Handle, who: Who, cell: LevelCell) -> Arc<Runner> {
    example_runner_of(runtime, who, cell, vec![])
}

/// [`example_runner`] that knows the example objects its dialog's diff
/// compares against, so a changed field reads Changed, as a live diff does.
pub(super) fn example_runner_of(
    runtime: &Handle,
    who: Who,
    cell: LevelCell,
    objects: Vec<serde_json::Value>,
) -> Arc<Runner> {
    let scratch = std::env::temp_dir().join("freshkube-example-audit");
    let _guard = runtime.enter();
    Arc::new(
        Runner::example_offline(AuditStore::new(scratch), who)
            .with_example_objects(objects)
            .following(cell),
    )
}

/// A runner for a live cluster, one copy for every site that starts a write:
/// it follows `cell` and reads the setup file again at its send step, for the
/// entry and context `who` names. There is no way to build a live runner here
/// without the guard ([`Runner::new`] takes it), and `mutation::runner_guard`
/// refuses, rather than skips, when there is no file.
pub(super) fn live_runner(
    client: kube::Client,
    audit: AuditStore,
    who: Who,
    cell: LevelCell,
    cx: &App,
) -> Runner {
    let setup = crate::mutation::runner_guard(&who.entry, &who.context, &cell, cx);
    Runner::new(client, audit, who, setup).following(cell)
}
