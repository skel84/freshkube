//! A shell's transport for tests here and in crates that embed the view.

use freshkube_core::job::OwnedJob;
use gpui_kit::{Entity, Task, TestAppContext};

use super::{Session, ShellState, ShellView};

/// A real Tokio stream whose Control-C flush pauses until the test releases
/// it. Recording the bytes and dropping the writer are independent of GPUI.
pub struct ClosingTransport {
    bytes: std::sync::Arc<std::sync::Mutex<Vec<u8>>>,
    dropped: std::sync::Arc<std::sync::atomic::AtomicBool>,
    release: Option<tokio::sync::oneshot::Sender<()>>,
    pub(super) stdout: Option<tokio::io::DuplexStream>,
}

struct ClosingWriter {
    bytes: std::sync::Arc<std::sync::Mutex<Vec<u8>>>,
    dropped: std::sync::Arc<std::sync::atomic::AtomicBool>,
    release: tokio::sync::oneshot::Receiver<()>,
}

impl tokio::io::AsyncWrite for ClosingWriter {
    fn poll_write(
        self: std::pin::Pin<&mut Self>,
        _: &mut std::task::Context<'_>,
        bytes: &[u8],
    ) -> std::task::Poll<std::io::Result<usize>> {
        self.bytes.lock().unwrap().extend_from_slice(bytes);
        std::task::Poll::Ready(Ok(bytes.len()))
    }

    fn poll_flush(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        let interrupted = self.bytes.lock().unwrap().last() == Some(&3);
        if interrupted && std::task::ready!(std::pin::Pin::new(&mut self.release).poll(cx)).is_err()
        {
            return std::task::Poll::Ready(Err(std::io::ErrorKind::BrokenPipe.into()));
        }
        std::task::Poll::Ready(Ok(()))
    }

    fn poll_shutdown(
        self: std::pin::Pin<&mut Self>,
        _: &mut std::task::Context<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        std::task::Poll::Ready(Ok(()))
    }
}

impl Drop for ClosingWriter {
    fn drop(&mut self) {
        self.dropped
            .store(true, std::sync::atomic::Ordering::SeqCst);
    }
}

impl ClosingTransport {
    pub fn install(shell: &Entity<ShellView>, cx: &mut TestAppContext) -> Self {
        cx.executor().allow_parking();
        let bytes = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let dropped = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let (release, gate) = tokio::sync::oneshot::channel();
        let writer = ClosingWriter {
            bytes: bytes.clone(),
            dropped: dropped.clone(),
            release: gate,
        };
        let (stdout, reader) = tokio::io::duplex(64);
        cx.update(|cx| {
            shell.update(cx, |view, cx| {
                view.drop_session(cx);
                let runtime = view.runtime.clone();
                let _entered = runtime.enter();
                let session =
                    freshkube_core::resources::ExecSession::from_test_streams(writer, reader);
                view.session = Some(Session::Connecting {
                    size: view.terminal.read(cx).size(),
                    _job: OwnedJob::new(view.runtime.spawn(std::future::pending())),
                    _task: Task::ready(()),
                });
                view.state = ShellState::Connecting;
                view.used = true;
                view.connected(view.seq, Ok(session), cx);
            })
        });
        cx.run_until_parked();
        Self {
            bytes,
            dropped,
            release: Some(release),
            stdout: Some(stdout),
        }
    }

    pub fn wait_for_interrupt(&self, cx: &mut TestAppContext) {
        freshkube_ui::real_io::until_io(cx, "Control-C", |_| self.bytes() == [3]);
    }

    pub fn release(&mut self) {
        self.release.take().unwrap().send(()).unwrap();
    }

    pub fn wait_for_eof(&self, cx: &mut TestAppContext) {
        freshkube_ui::real_io::until_io(cx, "Control-D", |_| self.bytes() == [3, 4]);
    }

    pub fn exit(&mut self) {
        drop(self.stdout.take());
    }

    pub fn wait_for_completion(&self, cx: &mut TestAppContext) {
        freshkube_ui::real_io::until_io(cx, "closing worker", |cx| {
            self.dropped.load(std::sync::atomic::Ordering::SeqCst)
                && cx.read(|cx| {
                    cx.try_global::<super::Shells>().is_none_or(|shells| {
                        shells
                            .closing
                            .iter()
                            .all(tokio::task::JoinHandle::is_finished)
                    })
                })
        });
        assert_eq!(self.bytes(), [3, 4], "one interrupt and one EOF");
    }

    fn bytes(&self) -> Vec<u8> {
        self.bytes.lock().unwrap().clone()
    }
}
