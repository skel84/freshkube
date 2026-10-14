//! One read for a Monitoring view, the page's or a history's: example data
//! on GPUI's background executor, Prometheus on Tokio under a deadline. Both
//! hand their answer back on the view, so every read times out and fails the
//! same way, and dropping the `Request` drops the answer.
use std::future::Future;
use std::time::Duration;

use freshkube_core::monitoring::QueryError;
use gpui_kit::{Context, Task};
use tokio::runtime::Handle;

use freshkube_core::job::OwnedJob;

/// A read in flight. Dropping it cancels the work and its answer.
pub(crate) struct Request {
    _job: Option<OwnedJob>,
    _task: Task<()>,
}

/// Runs `work` where its source answers (Tokio for Prometheus, within
/// `deadline`; the background executor for example data) and hands the
/// result to `done` on the view, unless the request is dropped first.
pub(crate) fn run<V: 'static, T: Send + 'static>(
    example: bool,
    runtime: &Handle,
    deadline: Duration,
    work: impl Future<Output = Result<T, QueryError>> + Send + 'static,
    cx: &mut Context<V>,
    done: impl FnOnce(&mut V, Result<T, QueryError>, &mut Context<V>) + 'static,
) -> Request {
    let (job, task) = crate::dispatch(example, runtime, deadline, work, cx, done);
    Request {
        _job: job,
        _task: task,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::TIMED_OUT;
    use freshkube_core::monitoring::ErrorKind;
    use gpui_kit::AppContext;
    use gpui_kit::{Entity, TestAppContext};

    /// A view that keeps its request and what it was told.
    #[derive(Default)]
    struct Reader {
        request: Option<Request>,
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
                reader.request = Some(run(
                    example,
                    runtime,
                    deadline,
                    work,
                    cx,
                    |reader, result, _| reader.answer = Some(result),
                ));
            })
        });
        reader
    }

    /// Steps GPUI until the reader has an answer or a second passes, since
    /// Tokio answers from its own threads.
    fn answered(
        cx: &mut TestAppContext,
        reader: &Entity<Reader>,
    ) -> Option<Result<u32, QueryError>> {
        for _ in 0..100 {
            cx.run_until_parked();
            if let Some(answer) = cx.read(|cx| reader.read(cx).answer.clone()) {
                return Some(answer);
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        None
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
        _ = sender.send(());
        assert_eq!(answered(cx, &reader), None);
    }

    #[gpui_kit::test]
    fn example_data_answers_without_tokio(cx: &mut TestAppContext) {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        // 1 when the work ran outside Tokio.
        let reader = read(cx, true, runtime.handle(), Duration::from_secs(5), async {
            Ok(u32::from(Handle::try_current().is_err()))
        });
        cx.run_until_parked();
        assert_eq!(cx.read(|cx| reader.read(cx).answer.clone()), Some(Ok(1)));
    }
}
