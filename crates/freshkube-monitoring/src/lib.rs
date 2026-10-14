//! Monitoring: Grafana dashboards drawn in the Console look from the
//! cluster's own Prometheus ([docs/MONITORING.md](../../../docs/MONITORING.md)).
//! `derive` turns an answer into display data once; `panel` draws it;
//! `page` is the rail area's page that reads and lays out a dashboard;
//! `dispatch` runs the page and history work for their completion loaders.
pub mod colors;
pub(crate) mod derive;
pub mod history;
pub mod page;
pub mod panel;
mod request;
pub(crate) mod store;

use std::{future::Future, time::Duration};

use freshkube_core::job::{OwnedJob, spawn_job};
use freshkube_core::monitoring::{ErrorKind, QueryError};
use freshkube_probe::{perf, probe};
use gpui_kit::{AppContext, Context, Task};
use tokio::runtime::Handle;
// The look lives in freshkube-ui; the moved code reaches it by its old paths.
use freshkube_ui::{palette, ui};
#[cfg(test)]
use freshkube_ui::{text_size, theme};

/// What a live read past its owner's deadline says.
const TIMED_OUT: &str = "Prometheus didn't answer in time";

/// Dispatches work and typed delivery only. The caller's loader owns both
/// returned handles and decides whether the answer may publish.
fn dispatch<V: 'static, T: Send + 'static>(
    example: bool,
    runtime: &Handle,
    deadline: Duration,
    work: impl Future<Output = Result<T, QueryError>> + Send + 'static,
    cx: &mut Context<V>,
    done: impl FnOnce(&mut V, Result<T, QueryError>, &mut Context<V>) + 'static,
) -> (Option<OwnedJob>, Task<()>) {
    if example {
        let work = cx.background_spawn(work);
        let task = cx.spawn(async move |this, cx| {
            let result = work.await;
            _ = this.update(cx, |this, cx| done(this, result, cx));
        });
        return (None, task);
    }
    let (job, receiver) = spawn_job(
        runtime,
        deadline,
        QueryError::new(ErrorKind::TimedOut, TIMED_OUT),
        work,
    );
    let task = cx.spawn(async move |this, cx| {
        let Ok(result) = receiver.await else {
            return;
        };
        _ = this.update(cx, |this, cx| done(this, result, cx));
    });
    (Some(job), task)
}
