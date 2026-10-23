//! `read`: the gate, the deadline and the dispatch to a live or example
//! source.

use super::answer::Answer;
use super::call::Request;
use super::gate::AgentAccess;
use super::refusal::{Code, Refusal};
use super::target::{AgentTarget, Source};
use super::{example, live};
use crate::constants::agent::READ_DEADLINE;

/// Answers one parsed request on a target.
///
/// The gate is asked first and builds nothing: with no grant for the target's
/// entry and context, the answer is `not_enabled` and no client exists. A
/// grant that ends while the read runs ends the read, and what it had read is
/// dropped. The read has [`READ_DEADLINE`], and the session has a budget of
/// reads that only a read holding a lease spends.
pub async fn read(
    access: &AgentAccess,
    target: &AgentTarget,
    request: &Request,
) -> Result<Answer, Refusal> {
    let mut lease = access
        .lease(&target.entry, &target.context)
        .ok_or_else(Refusal::not_enabled)?;
    access.spend()?;
    let work = async {
        match &target.source {
            Source::Live(cluster) => live::run(cluster, &target.entry, request).await,
            Source::Example(objects) => example::run(objects.as_ref(), &target.entry, request),
        }
    };
    let outcome = tokio::select! {
        biased;
        () = lease.ended() => Err(Refusal::not_enabled()),
        done = tokio::time::timeout(READ_DEADLINE, work) => done.unwrap_or_else(|_| {
            Err(Refusal::new(Code::TimedOut, "the read took longer than its deadline"))
        }),
    };
    // A grant that ended while the answer was being made takes it back.
    if !lease.is_live() {
        return Err(Refusal::not_enabled());
    }
    outcome
}
