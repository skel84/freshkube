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
pub(crate) mod store;
pub use store::{saved_token_urls, token_account};

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

#[cfg(test)]
mod tests {
    use super::*;
    use freshkube_reads::Loader;
    use freshkube_ui::real_io::until_io;
    use gpui_kit::{Entity, TestAppContext};

    #[derive(Default)]
    struct Reader {
        request: Option<Loader<(), ()>>,
        answer: Option<Result<u32, QueryError>>,
    }

    fn read(
        cx: &mut TestAppContext,
        example: bool,
        runtime: &Handle,
        deadline: Duration,
        work: impl Future<Output = Result<u32, QueryError>> + Send + 'static,
    ) -> Entity<Reader> {
        let reader = cx.new(|_| Reader::default());
        cx.update(|cx| {
            reader.update(cx, |reader, cx| {
                let mut loader = Loader::default();
                let request = loader.start(());
                let (job, task) = dispatch(
                    example,
                    runtime,
                    deadline,
                    work,
                    cx,
                    move |reader, result, cx| {
                        let completion = result
                            .as_ref()
                            .map(|_| ())
                            .map_err(|error| error.message.clone());
                        if reader
                            .request
                            .as_mut()
                            .is_some_and(|loader| loader.land(&request, completion, cx))
                        {
                            reader.answer = Some(result);
                        }
                    },
                );
                loader.run(job, task);
                reader.request = Some(loader);
            })
        });
        reader
    }

    fn answered(
        cx: &mut TestAppContext,
        reader: &Entity<Reader>,
    ) -> Option<Result<u32, QueryError>> {
        until_io(cx, "the dispatched answer", |cx| {
            cx.read(|cx| reader.read(cx).answer.is_some())
        });
        cx.read(|cx| reader.read(cx).answer.clone())
    }

    #[gpui_kit::test]
    fn prometheus_answers_on_the_view(cx: &mut TestAppContext) {
        cx.executor().allow_parking();
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let reader = read(cx, false, runtime.handle(), Duration::from_secs(5), async {
            Ok(7)
        });
        assert_eq!(answered(cx, &reader), Some(Ok(7)));
    }

    #[gpui_kit::test]
    fn a_read_past_its_deadline_times_out(cx: &mut TestAppContext) {
        cx.executor().allow_parking();
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let never = std::future::pending::<Result<u32, QueryError>>();
        let reader = read(
            cx,
            false,
            runtime.handle(),
            Duration::from_millis(20),
            never,
        );
        assert_eq!(
            answered(cx, &reader),
            Some(Err(QueryError::new(ErrorKind::TimedOut, TIMED_OUT)))
        );
    }

    #[gpui_kit::test]
    fn a_failure_reaches_the_view_as_it_is(cx: &mut TestAppContext) {
        cx.executor().allow_parking();
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let reader = read(cx, false, runtime.handle(), Duration::from_secs(5), async {
            Err(QueryError::refused())
        });
        assert_eq!(answered(cx, &reader), Some(Err(QueryError::refused())));
    }

    #[gpui_kit::test]
    fn a_dropped_request_answers_nothing(cx: &mut TestAppContext) {
        cx.executor().allow_parking();
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let (sender, receiver) = tokio::sync::oneshot::channel::<()>();
        let reader = read(
            cx,
            false,
            runtime.handle(),
            Duration::from_secs(5),
            async move {
                _ = receiver.await;
                Ok(7)
            },
        );
        cx.update(|cx| reader.update(cx, |reader, _| reader.request = None));
        until_io(cx, "the dropped dispatched work", |_| sender.is_closed());
        _ = sender.send(());
        cx.run_until_parked();
        assert_eq!(cx.read(|cx| reader.read(cx).answer.clone()), None);
    }

    #[gpui_kit::test]
    fn example_data_answers_without_tokio(cx: &mut TestAppContext) {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let reader = read(cx, true, runtime.handle(), Duration::from_secs(5), async {
            Ok(u32::from(Handle::try_current().is_err()))
        });
        cx.run_until_parked();
        assert_eq!(cx.read(|cx| reader.read(cx).answer.clone()), Some(Ok(1)));
    }
}
