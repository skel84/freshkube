mod chrome;
mod column;
mod custom;
mod header;
mod keys;
mod kubernetes_only;
mod layout;
mod motion;
mod observability;
mod operations;
mod overview;
mod rail;
mod screens;
mod services;
mod settings;
mod shells;
mod status;
mod target;
mod visible;

use super::{Area, ColumnReveal, GpuiOptions, NodeView, Page, Pilot, probe};
use crate::logs::TalosPanel;
pub(crate) use freshkube_ui::layout_check::settle_header;
use gpui_kit::test::{TestAppContextExt, TestWindowExt};
use gpui_kit::{
    AnyWindowHandle, AppContext, Entity, SharedString, TestAppContext,
    component::{ActiveTheme, Theme, ThemeMode},
    point, px, size,
};
use std::cell::Cell;
use std::rc::Rc;

pub(super) use super::testing::{area, expand_healthy_nodes, mount, root_pilot};
pub(crate) use super::testing::{
    fixture, open_kind, open_node_tab, pick_target, screen, start_shell,
};

/// Opens the node's Services tab and expands the inspector, so a selected
/// service's details sit beside the list rather than under it.
fn open_services_with_room(window: &mut gpui_kit::Window, cx: &mut gpui_kit::App) {
    open_node_tab(window, cx, super::nodes::NodeTab::Services);
    window.render_frame(cx);
    if !root_pilot(window, cx).read(cx).node_workspace.expanded {
        window.click("node-expand", cx);
        window.render_frame(cx);
    }
}

fn contexts(window: &mut gpui_kit::Window, cx: &mut gpui_kit::App) {
    if !root_pilot(window, cx).read(cx).context_display.open {
        window.click("context-switcher", cx);
        window.render_frame(cx);
    }
}

const FIRST_NODE: &str = "talos-cp-fra1-01";

const DEGRADED_NODE: &str = "talos-wk-fra1-02";

const SILENT_NODE: &str = "talos-wk-fra1-03";

/// Scrolls the navigation column until `id` shows in full.
fn reveal(window: &mut gpui_kit::Window, cx: &mut gpui_kit::App, id: &str) {
    for _ in 0..60 {
        if shown_in_column(window, id) {
            return;
        }
        let area = column_area(window);
        let above = window
            .try_find(SharedString::from(id.to_owned()))
            .is_some_and(|element| element.bounds().top() < area.top());
        window.scroll(
            "nav-column",
            gpui_kit::ScrollDelta::Pixels(gpui_kit::point(
                px(0.),
                px(if above { 40. } else { -40. }),
            )),
            cx,
        );
        window.render_frame(cx);
    }
    panic!("{id} never scrolled into view");
}

/// Whether `id` shows in full inside the scrolled navigation column.
/// Where the column's rows show: the source list under the title, or the
/// collapsed column's strip.
fn column_area(window: &mut gpui_kit::Window) -> gpui_kit::Bounds<gpui_kit::Pixels> {
    window
        .try_find("nav-column-list")
        .unwrap_or_else(|| window.find("nav-column"))
        .bounds()
}

fn shown_in_column(window: &mut gpui_kit::Window, id: &str) -> bool {
    let area = column_area(window);
    window
        .try_find(SharedString::from(id.to_owned()))
        .is_some_and(|element| {
            let bounds = element.bounds();
            element.visible() && bounds.top() >= area.top() && bounds.bottom() <= area.bottom()
        })
}

#[gpui_kit::test]
async fn a_talosconfig_that_fails_to_load_ends_the_services_wait(cx: &mut TestAppContext) {
    cx.executor().allow_parking();
    // Never the user's default talosconfig: a missing file.
    let options = GpuiOptions::new(Some(scratch("no-such-talosconfig-services")), None, 100);
    let (_runtime, handle, view) = mount(cx, options, 1280., 820.);
    cx.wait_for(handle, std::time::Duration::from_secs(2), |_, cx| {
        view.read(cx).config_error.is_some()
    })
    .await;
    cx.update(|cx| {
        let pilot = view.read(cx);
        // Neither page waits on an overview that will never be asked.
        let reading = pilot.system_services.read(cx).reading().clone();
        assert!(
            matches!(reading, super::system_services::Reading::Failed(_)),
            "{reading:?}"
        );
        assert!(!pilot.nodes_wait());
    });
}

/// Lets the loading motion run, as the platform would: one 16 ms frame.
/// Returns how many next-frame callbacks the frame delivered.
fn motion_frame(cx: &mut TestAppContext, handle: AnyWindowHandle) -> usize {
    cx.background_executor
        .advance_clock(std::time::Duration::from_millis(16));
    let asked = cx
        .update_window(handle, |_, window, cx| window.simulate_next_frame(cx))
        .unwrap();
    cx.run_until_parked();
    asked
}

/// The shell's cached parts, by their probes.
const CHROME: [&str; 3] = ["chrome.header", "chrome.rail", "chrome.column"];

fn chrome_counts() -> [usize; 3] {
    CHROME.map(probe::count)
}

/// Runs 60 frames, about a second, and returns how many of them redrew
/// a part of the chrome or `page`. The header's countdown ring and a
/// page's ages tick once a second; nothing else should draw them.
fn frames_redrawing(cx: &mut TestAppContext, handle: AnyWindowHandle, page: &'static str) -> usize {
    let counts = || (chrome_counts(), probe::count(page));
    (0..60)
        .filter(|_| {
            let before = counts();
            assert!(
                motion_frame(cx, handle) > 0,
                "the bars move while the page loads"
            );
            counts() != before
        })
        .count()
}

/// Opens Pods with example data that never answers, so it stays on its
/// loading rows, and lets the motion start.
fn loading_pods(
    cx: &mut TestAppContext,
) -> (tokio::runtime::Runtime, AnyWindowHandle, Entity<Pilot>) {
    let (runtime, handle, view) = fixture(cx, 1280., 880.);
    cx.update(|cx| cx.set_reduce_motion(false));
    cx.update_window(handle, |_, window, cx| {
        let resources = view.read(cx).resources.clone();
        resources.update(cx, |resources, _| resources.hold = true);
        open_kind(window, cx, "pods");
    })
    .unwrap();
    cx.run_until_parked();
    for _ in 0..3 {
        motion_frame(cx, handle);
    }
    cx.update_window(handle, |_, window, _| {
        assert!(window.find("resource-loading").visible());
    })
    .unwrap();
    (runtime, handle, view)
}

/// Applies `change` and returns which parts of the chrome drew again.
pub(super) fn chrome_redrawn(
    cx: &mut TestAppContext,
    handle: AnyWindowHandle,
    change: impl FnOnce(&mut gpui_kit::Window, &mut gpui_kit::App),
) -> Vec<&'static str> {
    let before = chrome_counts();
    cx.update_window(handle, |_, window, cx| change(window, cx))
        .unwrap();
    cx.run_until_parked();
    let after = chrome_counts();
    CHROME
        .into_iter()
        .zip(before.into_iter().zip(after))
        .filter(|(_, (before, after))| after > before)
        .map(|(part, _)| part)
        .collect()
}

#[gpui_kit::test]
async fn browse_picks_a_talosconfig_and_reloads_contexts(cx: &mut TestAppContext) {
    // Configuration loads on Tokio; let GPUI park for its completion.
    cx.executor().allow_parking();
    let guard = tempfile::tempdir().unwrap();
    let directory = guard.path().to_path_buf();
    let chosen = directory.join("config");
    std::fs::write(
        &chosen,
        "context: alpha\ncontexts:\n  alpha: &entry\n    endpoints: [192.0.2.1]\n    ca: YQ==\n    crt: Yg==\n    key: Yw==\n  beta: *entry\n",
    )
    .unwrap();
    // Never the user's default talosconfig: start from a missing file.
    let options = GpuiOptions::new(Some(directory.join("missing")), None, 100);
    let (_runtime, handle, view) = mount(cx, options, 1280., 820.);
    cx.wait_for(handle, std::time::Duration::from_secs(2), |_, cx| {
        view.read(cx).config_error.is_some()
    })
    .await;
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        // The error screen offers Browse directly; cancelling changes nothing.
        window.click("browse-config-empty", cx);
    })
    .unwrap();
    assert!(cx.did_prompt_for_paths());
    cx.simulate_path_prompt_response(|_| None);
    cx.run_until_parked();
    cx.update(|cx| assert!(view.read(cx).config_error.is_some()));
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("settings", cx);
        window.render_frame(cx);
        window.click("browse-config", cx);
    })
    .unwrap();
    assert!(cx.did_prompt_for_paths());
    cx.simulate_path_prompt_response(|options| {
        assert!(options.files && !options.directories && !options.multiple);
        Some(vec![chosen.clone()])
    });
    cx.wait_for(handle, std::time::Duration::from_secs(2), |_, cx| {
        view.read(cx).contexts == ["alpha", "beta"]
    })
    .await;
    cx.update(|cx| {
        let pilot = view.read(cx);
        assert_eq!(pilot.applied.path.as_deref(), Some(chosen.as_path()));
        assert_eq!(pilot.path.read(cx).value(), chosen.display().to_string());
        assert!(pilot.config_error.is_none());
    });
}

/// Draws what changed since the last frame. Unlike `render_frame` this keeps
/// GPUI's view cache, which is what these tests are about.
fn draw(window: &mut gpui_kit::Window, cx: &mut gpui_kit::App) {
    window.draw(cx).clear(cx);
}

fn count(name: &'static str) -> usize {
    super::probe::count(name)
}

/// One countdown tick as the shell's timer applies it.
fn tick(view: &Entity<Pilot>, cx: &mut TestAppContext) {
    cx.update(|cx| {
        view.update(cx, |pilot, cx| {
            pilot.elapsed += std::time::Duration::from_secs(1);
            pilot.tick_countdown(cx);
        });
    });
    cx.run_until_parked();
}

/// A kubeconfig whose server refuses at once, so nothing leaves this
/// machine, with a fake token.
const KUBECONFIG: &str = "apiVersion: v1
kind: Config
current-context: lab
clusters:
- name: lab
  cluster:
    server: https://127.0.0.1:1
contexts:
- name: lab
  context:
    cluster: lab
    user: lab
- name: staging
  context:
    cluster: lab
    user: lab
    namespace: web
users:
- name: lab
  user:
    token: not-a-real-token
";

fn scratch(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("freshkube-tests-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    dir.join(name)
}

/// The window `--kubernetes-only --kubeconfig <path>` opens. Never the
/// ambient kubeconfig: the path is always explicit.
fn kubernetes_only(
    cx: &mut TestAppContext,
    kubeconfig: std::path::PathBuf,
    context: Option<&str>,
) -> (tokio::runtime::Runtime, AnyWindowHandle, Entity<Pilot>) {
    // Reading and connecting run on Tokio and wake GPUI from there.
    cx.executor().allow_parking();
    let options = GpuiOptions::kubernetes_only(Some(kubeconfig), context.map(Into::into), 100);
    mount(cx, options, 1280., 860.)
}

#[gpui_kit::test]
async fn object_metadata_completion_rechecks_access_even_without_an_epoch_change(
    cx: &mut TestAppContext,
) {
    use std::sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    };
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let server_runtime = tokio::runtime::Runtime::new().unwrap();
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    listener.set_nonblocking(true).unwrap();
    let received = Arc::new(AtomicBool::new(false));
    let release = Arc::new(tokio::sync::Notify::new());
    let (seen, gate) = (received.clone(), release.clone());
    let server = server_runtime.spawn(async move {
        let listener = tokio::net::TcpListener::from_std(listener).unwrap();
        while let Ok((mut socket, _)) = listener.accept().await {
            let (seen, gate) = (seen.clone(), gate.clone());
            tokio::spawn(async move {
                let mut request = vec![0u8; 16384];
                let n = socket.read(&mut request).await.unwrap();
                let request = String::from_utf8_lossy(&request[..n]);
                let (status, body) = if request.starts_with("GET /api/v1/namespaces/prod/pods/later ") {
                    assert!(request.contains("PartialObjectMetadata"));
                    seen.store(true, Ordering::SeqCst);
                    gate.notified().await;
                    (200, serde_json::json!({"apiVersion":"meta.k8s.io/v1","kind":"PartialObjectMetadata","metadata":{"namespace":"prod","name":"later","uid":"old-access-uid"}}))
                } else if request.starts_with("GET /version ") {
                    (200, serde_json::json!({"major":"1","minor":"32","gitVersion":"v1.32.0","gitCommit":"fixture","gitTreeState":"clean","buildDate":"2026-01-01T00:00:00Z","goVersion":"go1.24","compiler":"gc","platform":"test"}))
                } else {
                    (404, serde_json::json!({"apiVersion":"v1","kind":"Status","status":"Failure","reason":"NotFound","code":404,"message":"No fixture for this endpoint"}))
                };
                let body = body.to_string();
                let response = format!("HTTP/1.1 {status} Test\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
                _ = socket.write_all(response.as_bytes()).await;
            });
        }
    });
    let directory = tempfile::tempdir().unwrap();
    let config = directory.path().join("config");
    std::fs::write(
        &config,
        KUBECONFIG.replace("https://127.0.0.1:1", &format!("http://{address}")),
    )
    .unwrap();
    let (_runtime, handle, view) = kubernetes_only(cx, config, Some("lab"));
    let observed = view.clone();
    cx.wait_for(handle, std::time::Duration::from_secs(5), move |_, cx| {
        observed.read(cx).kube_source().is_some()
    })
    .await;
    cx.update_window(handle, |_, window, cx| {
        view.update(cx, |view, cx| {
            view.open_object(
                crate::resources::example::kind("pods").unwrap(),
                crate::resources::model::ObjectRef {
                    connection: None,
                    namespace: "prod".into(),
                    name: "later".into(),
                    uid: String::new(),
                },
                crate::resources::Tab::Overview,
                window,
                cx,
            );
        })
    })
    .unwrap();
    cx.wait_for(handle, std::time::Duration::from_secs(5), move |_, _| {
        received.load(Ordering::SeqCst)
    })
    .await;
    let completion = cx.update(|cx| {
        view.update(cx, |view, _| {
            // Simulate a replacement access independently of the existing
            // epoch/sequence guards, leaving the delayed worker alive.
            view.fixture = true;
            view.applied.context = Some("replacement".into());
            view.object_open_task.take().unwrap()
        })
    });
    release.notify_one();
    completion.await;
    cx.read(|cx| {
        assert!(
            view.read(cx)
                .resources
                .read(cx)
                .detail_identity(cx)
                .is_none()
        )
    });
    server.abort();
}
