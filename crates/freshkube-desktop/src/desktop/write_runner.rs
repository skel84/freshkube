//! What every page that starts a write needs to build its runner (#855, #857):
//! who is acting, and the runner example data gets. One copy, so the Stage
//! actions and the Resources actions cannot drift.

use std::sync::Arc;

use freshkube_core::write::{AuditStore, LevelCell, Runner, Who};
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
    let scratch = std::env::temp_dir().join("freshkube-example-audit");
    let _guard = runtime.enter();
    Arc::new(Runner::example_offline(AuditStore::new(scratch), who).following(cell))
}
