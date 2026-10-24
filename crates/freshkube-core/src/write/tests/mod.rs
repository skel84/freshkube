//! The framework end to end on the recording fake: what is sent, in what
//! order, and what is never sent.

use std::time::Duration;

use serde_json::json;

use super::testing::{self as fake, Cluster};
use super::*;

mod access;
mod apply;
mod audit;
mod create;
mod dependencies;
mod dialogs;
mod dry_run;
mod example;
mod identity;
mod levels;
mod outcome;
mod rollback;
mod run_now;
mod undo;

fn tmp() -> tempfile::TempDir {
    tempfile::tempdir().unwrap()
}

async fn ready(runner: &Runner, action: ActionDescription) -> Box<Prepared> {
    match runner.preflight(action).await {
        Verdict::Ready(prepared) => prepared,
        Verdict::Blocked(block) => panic!("blocked: {block:?}"),
    }
}

async fn run(runner: &Runner, prepared: Box<Prepared>, confirmation: Confirmation) -> Finished {
    runner
        .execute(*prepared, confirmation, || false, |_| {})
        .await
}

fn phases(store: &AuditStore) -> Vec<Phase> {
    store
        .read()
        .unwrap()
        .records
        .iter()
        .map(|r| r.phase)
        .collect()
}

async fn dialog_for(level: Protection, action: ActionDescription) -> Dialog {
    let (cluster, dir) = (Cluster::new(), tmp());
    let runner = fake::runner(&cluster, dir.path(), level);
    Dialog::for_verdict(&runner.preflight(action).await, "prod-fra")
}
