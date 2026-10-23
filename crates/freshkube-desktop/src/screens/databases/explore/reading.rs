//! One read of the session, the way the other pages run theirs: a Tokio job
//! owned by a `Loader`, delivered by a GPUI task, landed only if its
//! request is still the latest for its identity.
use freshkube_core::job::spawn_job;
use freshkube_reads::Loader;
use gpui_kit::*;
use std::{future::Future, time::Duration};
use tokio::runtime::Handle;

/// How long one read of the database may take, a little over the statement
/// timeout so the server's own answer arrives first.
pub(super) const READ_DEADLINE: Duration = Duration::from_secs(40);

#[allow(clippy::too_many_arguments)]
pub(super) fn read<V, T, I>(
    view: &mut V,
    slot: impl Fn(&mut V) -> &mut Loader<T, I> + Clone + 'static,
    identity: I,
    runtime: &Handle,
    what: &'static str,
    work: impl Future<Output = Result<T, String>> + Send + 'static,
    then: impl Fn(&mut V, &mut Context<V>) + 'static,
    cx: &mut Context<V>,
) where
    V: 'static,
    T: Send + 'static,
    I: Clone + Eq + 'static,
{
    let request = slot(view).start(identity);
    let (job, receiver) = spawn_job(
        runtime,
        READ_DEADLINE,
        format!("Reading {what} timed out"),
        work,
    );
    let stopped = format!("Reading {what} stopped unexpectedly");
    let landing = slot.clone();
    let task = cx.spawn(async move |this, cx| {
        let result = receiver.await.unwrap_or_else(|_| Err(stopped));
        _ = this.update(cx, |view, cx| {
            if landing(view).land(&request, result, cx) {
                then(view, cx);
            }
            cx.notify();
        });
    });
    slot(view).run(Some(job), task);
    cx.notify();
}
