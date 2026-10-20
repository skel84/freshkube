use super::*;
use crate::probe;
use freshkube_core::cluster_source::{ClusterAccess, ClusterSource, Hold, KubeClientSource};
use freshkube_core::failure::{Failure, FailureKind};
use freshkube_core::monitoring::{
    Candidate, Discovery, ErrorKind, PrometheusService, QueryError, Rank, Tried,
};
use freshkube_ui::real_io::RealIo;
use gpui_kit::component::{Root, Theme, ThemeMode};
use gpui_kit::test::TestWindowExt;
use gpui_kit::{
    AnyWindowHandle, AppContext, Bounds, Entity, Pixels, Point, ScrollDelta, SharedString,
    TestAppContext, point, px, size,
};

mod alerts;
mod answers;
mod client;
mod frame;
mod history;
mod history_recovery;
mod markers;
mod reads;
mod scope;
mod settings;
mod states;

const NOW: i64 = 1_700_003_600;

fn example_source() -> ClusterSource {
    ClusterSource {
        id: "example".into(),
        context: "prod-fra".into(),
        access: ClusterAccess::EXAMPLE,
    }
}

/// The page alone in a window, its clock fixed and its source example data.
fn mount(
    cx: &mut TestAppContext,
    source: Option<ClusterSource>,
) -> (
    tokio::runtime::Runtime,
    AnyWindowHandle,
    Entity<MonitoringPage>,
) {
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let (handle, page) = mount_with(cx, &runtime, source, None, None);
    (runtime, handle, page)
}

fn mount_with(
    cx: &mut TestAppContext,
    runtime: &tokio::runtime::Runtime,
    source: Option<ClusterSource>,
    preferences: Option<&std::path::Path>,
    secrets: Option<freshkube_core::secrets::Secrets>,
) -> (AnyWindowHandle, Entity<MonitoringPage>) {
    cx.update(|cx| {
        gpui_kit::init(cx);
        crate::theme::install(cx);
        crate::text_size::install(None, cx);
        Theme::change(ThemeMode::Dark, None, cx);
        cx.set_reduce_motion(true);
    });
    let mut page = None;
    let window = cx.open_window(size(px(1200.), px(900.)), |window, cx| {
        let view = cx.new(|cx| {
            let mut view =
                MonitoringPage::new(runtime.handle().clone(), preferences, secrets.clone(), cx);
            view.now = || NOW;
            view.set_source(source, cx);
            view
        });
        page = Some(view.clone());
        // Uncached, as the shell holds it.
        Root::new(view, window, cx)
    });
    let refresh = page.as_ref().unwrap().downgrade();
    cx.update(|cx| {
        cx.on_action(move |_: &freshkube_ui::page::Refresh, cx| {
            _ = refresh.update(cx, |page, cx| page.refresh(cx));
        });
    });
    cx.run_until_parked();
    (window.into(), page.unwrap())
}

fn frame(cx: &mut TestAppContext, handle: AnyWindowHandle) {
    cx.update_window(handle, |_, window, cx| window.render_frame(cx))
        .unwrap();
    cx.run_until_parked();
}

/// The wheel over the top of the grid, where it shows on a short page whose
/// grid runs below the window; `TestWindowExt::scroll` aims at its centre.
fn wheel(window: &mut gpui_kit::Window, delta: ScrollDelta, cx: &mut gpui_kit::App) {
    window.render_frame(cx);
    let grid = window.find("monitoring-grid").bounds();
    let position = point(grid.center().x, grid.top() + px(20.));
    window.dispatch_event(
        gpui_kit::PlatformInput::MouseMove(gpui_kit::MouseMoveEvent {
            position,
            pressed_button: None,
            modifiers: Default::default(),
        }),
        cx,
    );
    window.dispatch_event(
        gpui_kit::PlatformInput::ScrollWheel(gpui_kit::ScrollWheelEvent {
            position,
            delta,
            ..Default::default()
        }),
        cx,
    );
    window.render_frame(cx);
}

fn shown(cx: &mut TestAppContext, handle: AnyWindowHandle, id: &str) -> bool {
    let id = SharedString::from(id.to_owned());
    cx.update_window(handle, move |_, window, cx| {
        window.render_frame(cx);
        window.try_find(id).is_some()
    })
    .unwrap()
}

fn show(cx: &mut TestAppContext, handle: AnyWindowHandle, page: &Entity<MonitoringPage>) {
    cx.update(|cx| page.update(cx, |page, cx| page.set_visible(true, cx)));
    cx.run_until_parked();
    frame(cx, handle);
}

/// The slots answered, by title.
fn ready(cx: &mut TestAppContext, page: &Entity<MonitoringPage>) -> Vec<String> {
    cx.read(|cx| {
        let page = page.read(cx);
        page.board
            .as_ref()
            .map(|board| {
                board
                    .slots
                    .iter()
                    .filter(|slot| slot.view.read(cx).is_ready())
                    .map(|slot| slot.spec.title.clone())
                    .collect()
            })
            .unwrap_or_default()
    })
}

/// Where slot `n` is drawn: its panel, or the empty card in its place.
fn slot_bounds(window: &mut gpui_kit::Window, n: usize) -> Bounds<Pixels> {
    window
        .try_find(SharedString::from(format!("monitoring-panel-{n}")))
        .unwrap_or_else(|| window.find(format!("monitoring-placeholder-{n}")))
        .bounds()
}

fn slot_count(cx: &mut TestAppContext, page: &Entity<MonitoringPage>) -> usize {
    cx.read(|cx| {
        page.read(cx)
            .board
            .as_ref()
            .map_or(0, |board| board.slots.len())
    })
}

const CPU: &str = "CPU usage by node";

/// A live cluster whose client never builds, as a kubeconfig without
/// files: nothing in these tests reaches the cluster.
struct NoClient;

impl KubeClientSource for NoClient {
    fn client(&self) -> futures::future::BoxFuture<'_, Result<kube::Client, Failure>> {
        Box::pin(async { Err(Failure::new(FailureKind::Config, "No kubeconfig")) })
    }
}

fn live_source() -> ClusterSource {
    ClusterSource {
        id: "live".into(),
        context: "prod-ams".into(),
        access: ClusterAccess::Live(std::sync::Arc::new(NoClient)),
    }
}

async fn settled(cx: &mut TestAppContext, handle: AnyWindowHandle, page: &Entity<MonitoringPage>) {
    let observed = page.clone();
    cx.wait_for_io(handle, move |_, cx| {
        !matches!(
            observed.read(cx).connection,
            Connection::Looking { .. } | Connection::None
        )
    })
    .await;
}

/// A fake API server on loopback, through which every client of
/// `FakeCluster` reaches it. Lists are empty, a variable's values are
/// `a`, and every other read through the proxy answers `panels`.
fn fake_api(runtime: &tokio::runtime::Runtime, panels: u16) -> String {
    fake_api_with_discovery(runtime, panels, false)
}

fn fake_api_with_discovery(
    runtime: &tokio::runtime::Runtime,
    panels: u16,
    advertise_prometheus: bool,
) -> String {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let listener = runtime
        .block_on(tokio::net::TcpListener::bind("127.0.0.1:0"))
        .unwrap();
    let address = listener.local_addr().unwrap();
    runtime.spawn(async move {
        while let Ok((mut socket, _)) = listener.accept().await {
            tokio::spawn(async move {
                let mut buffer = vec![0; 16 * 1024];
                let mut read = 0;
                while !buffer[..read].windows(4).any(|w| w == b"\r\n\r\n") {
                    match socket.read(&mut buffer[read..]).await {
                        Ok(0) | Err(_) => return,
                        Ok(n) => read += n,
                    }
                }
                let head = String::from_utf8_lossy(&buffer[..read]);
                let path = head.split_whitespace().nth(1).unwrap_or("").to_owned();
                let (status, body) = if advertise_prometheus && path.starts_with("/api/v1/services?") {
                    (200, r#"{"apiVersion":"v1","kind":"ServiceList","metadata":{},"items":[{"metadata":{"namespace":"monitoring","name":"prometheus-operated"},"spec":{"ports":[{"name":"web","port":9090}]}}]}"#)
                } else if path.contains("/proxy/api/v1/label/") {
                    (200, r#"{"status":"success","data":["a"]}"#)
                } else if path.contains("/proxy/api/v1/series") {
                    (
                        200,
                        r#"{"status":"success","data":[{"node":"a","namespace":"a"}]}"#,
                    )
                } else if path.contains("/proxy/") {
                    (panels, "the proxy says no")
                } else {
                    (
                        200,
                        r#"{"apiVersion":"v1","kind":"List","metadata":{"resourceVersion":"1"},"items":[]}"#,
                    )
                };
                let answer = format!(
                    "HTTP/1.1 {status} X\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
                    body.len()
                );
                let _ = socket.write_all(answer.as_bytes()).await;
            });
        }
    });
    format!("http://{address}")
}

/// Holds the first provider call after a particular client eviction.
struct ClientGate {
    after_forget: usize,
    receive: tokio::sync::oneshot::Receiver<()>,
}

/// A live cluster at `fake_api`, counting what it was told to forget.
struct FakeCluster {
    url: String,
    forgot: std::sync::atomic::AtomicUsize,
    asked: std::sync::atomic::AtomicUsize,
    replacement: std::sync::Mutex<Option<String>>,
    gate: std::sync::Mutex<Option<ClientGate>>,
    gate_started: std::sync::Arc<std::sync::atomic::AtomicBool>,
    fail_rebuild: std::sync::atomic::AtomicBool,
}

impl FakeCluster {
    fn new(url: String) -> std::sync::Arc<Self> {
        std::sync::Arc::new(Self {
            url,
            forgot: Default::default(),
            asked: Default::default(),
            replacement: Default::default(),
            gate: Default::default(),
            gate_started: Default::default(),
            fail_rebuild: Default::default(),
        })
    }

    fn forgot(&self) -> usize {
        self.forgot.load(std::sync::atomic::Ordering::SeqCst)
    }
}

impl KubeClientSource for FakeCluster {
    fn client(&self) -> futures::future::BoxFuture<'_, Result<kube::Client, Failure>> {
        self.asked.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let forgotten = self.forgot();
        let replacement = self.replacement.lock().unwrap();
        let url = if forgotten > 0 {
            replacement.as_ref().unwrap_or(&self.url)
        } else {
            &self.url
        };
        let config = kube::Config::new(url.parse().unwrap());
        let gate = {
            let mut gate = self.gate.lock().unwrap();
            if gate
                .as_ref()
                .is_some_and(|gate| forgotten >= gate.after_forget)
            {
                gate.take().map(|gate| gate.receive)
            } else {
                None
            }
        };
        let started = self.gate_started.clone();
        let fail = forgotten > 0 && self.fail_rebuild.load(std::sync::atomic::Ordering::SeqCst);
        Box::pin(async move {
            if let Some(gate) = gate {
                started.store(true, std::sync::atomic::Ordering::SeqCst);
                _ = gate.await;
            }
            if fail {
                return Err(Failure::new(
                    FailureKind::Other,
                    "The replacement client did not build",
                ));
            }
            kube::Client::try_from(config).map_err(|error| {
                let message = error.to_string();
                Failure::new(Failure::from_kube(error).kind, message)
            })
        })
    }

    fn forget(&self) {
        self.forgot
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    }
}
