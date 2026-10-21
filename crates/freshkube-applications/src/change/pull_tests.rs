//! The pull request card's placeholder row: what it says in example data and
//! in each way GitHub can fail, and that a read lives only while the page
//! shows.
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use chrono::DateTime;
use freshkube_delivery::change::example::Shape;
use freshkube_delivery::change::live::Place;
use freshkube_delivery::github::Handle;
use freshkube_ui::real_io::RealIo;
use gpui_kit::component::Root;
use gpui_kit::test::TestWindowExt;
use gpui_kit::{AnyWindowHandle, AppContext, Entity, TestAppContext, px, size};
use tokio::io::AsyncReadExt;

use super::{ChangePage, Connections, Fetch, SessionKey};
use crate::resources::ClusterAccess;

fn open(cx: &mut TestAppContext, shape: Shape) -> (AnyWindowHandle, Entity<ChangePage>) {
    cx.update(|cx| {
        gpui_kit::init(cx);
        freshkube_ui::theme::install(cx);
        freshkube_ui::text_size::install(None, cx);
        cx.set_reduce_motion(true);
    });
    let mut page = None;
    let handle = cx.open_window(size(px(1280.), px(880.)), |window, cx| {
        let entity = cx.new(|cx| {
            ChangePage::new(
                Fetch::Example {
                    project: "checkout".into(),
                    freight: "wonky-otter".into(),
                    now: DateTime::from_timestamp(1_760_000_000, 0).unwrap(),
                    shape,
                    delay: Duration::ZERO,
                },
                None,
                Connections::new("example", [(SessionKey::new("core-fra"), "example".into())]),
                true,
                window,
                cx,
            )
        });
        page = Some(entity.clone());
        Root::new(entity, window, cx)
    });
    cx.run_until_parked();
    (handle.into(), page.unwrap())
}

fn line(cx: &mut TestAppContext, page: &Entity<ChangePage>) -> Option<String> {
    cx.read(|cx| page.read(cx).pull_line().map(str::to_owned))
}

#[gpui_kit::test]
fn the_row_says_where_the_pull_request_stands_its_reviews_and_its_checks(cx: &mut TestAppContext) {
    let (handle, page) = open(cx, Shape::Whole);
    assert_eq!(
        line(cx, &page).as_deref(),
        Some("Pull request #418 · Merged · 1 approved · checks passing (3)")
    );
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let row = window.find("change-pull");
        assert_eq!(
            row.label(),
            Some("Pull request: Pull request #418 · Merged · 1 approved · checks passing (3)")
        );
    })
    .unwrap();
}

#[gpui_kit::test]
fn kept_answers_say_why_they_are_old_and_a_refused_token_leaves_no_row(cx: &mut TestAppContext) {
    for (shape, why) in [
        (
            Shape::GitHubLimited,
            "GitHub's rate limit is reached until 14:05 UTC",
        ),
        (
            Shape::GitHubOffline,
            "GitHub didn't answer: could not connect",
        ),
    ] {
        let (_, page) = open(cx, shape);
        let text = line(cx, &page).expect("the kept answer");
        assert!(text.starts_with("Pull request #418 · Merged · "), "{text}");
        assert!(
            text.ends_with(&format!("{why}; showing what was read at 13:12 UTC")),
            "{text}"
        );
    }
    // Nothing was ever read, so there is no pull request to ask about; the
    // page's banner of what may be missing says why.
    let (handle, page) = open(cx, Shape::GitHubRefused);
    assert_eq!(line(cx, &page), None);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(window.try_find("change-pull").is_none());
        assert!(
            window
                .find("change-missing")
                .label()
                .is_some_and(|label| label.contains("GitHub refused the token (401)"))
        );
    })
    .unwrap();
}

/// A TCP server that takes a connection and says nothing, so a read made
/// to it stays in flight, and that counts how many it was asked and how
/// many the other side then closed.
struct Silent {
    address: String,
    accepted: Arc<AtomicUsize>,
    closed: Arc<AtomicUsize>,
    task: tokio::task::JoinHandle<()>,
}

impl Silent {
    fn new(runtime: &tokio::runtime::Runtime) -> Self {
        let accepted = Arc::new(AtomicUsize::new(0));
        let closed = Arc::new(AtomicUsize::new(0));
        let listener = runtime
            .block_on(tokio::net::TcpListener::bind("127.0.0.1:0"))
            .unwrap();
        let address = format!(
            "https://127.0.0.1:{}",
            listener.local_addr().unwrap().port()
        );
        let task = runtime.spawn({
            let (accepted, closed) = (accepted.clone(), closed.clone());
            async move {
                loop {
                    let Ok((mut stream, _)) = listener.accept().await else {
                        return;
                    };
                    accepted.fetch_add(1, Ordering::SeqCst);
                    let closed = closed.clone();
                    tokio::spawn(async move {
                        let mut buffer = [0u8; 4096];
                        // The client's hello is read; nothing answers it, until
                        // the client hangs up.
                        while stream.read(&mut buffer).await.is_ok_and(|read| read > 0) {}
                        closed.fetch_add(1, Ordering::SeqCst);
                    });
                }
            }
        });
        Self {
            address,
            accepted,
            closed,
            task,
        }
    }
}

impl Drop for Silent {
    fn drop(&mut self) {
        self.task.abort();
    }
}

#[gpui_kit::test]
async fn a_hide_drops_the_read_and_showing_again_asks_again(cx: &mut TestAppContext) {
    cx.executor().allow_parking();
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let server = Silent::new(&runtime);
    let (handle, page) = open(cx, Shape::Whole);
    // The page reads live, through a client that points at the silent server,
    // and shows the pull request the change names.
    let change = cx.read(|cx| page.read(cx).shown().clone());
    cx.update_window(handle, |_, window, cx| {
        page.update(cx, |page, cx| {
            page.fetch = Fetch::Live {
                runtime: runtime.handle().clone(),
                access: ClusterAccess::EXAMPLE,
                secrets: None,
                setup_revision: 0,
                place: Box::new(Place {
                    cluster: "core-fra".into(),
                    label: "home-lab".into(),
                    project: "checkout".into(),
                    freight: "wonky-otter".into(),
                    argocd_namespace: "argocd".into(),
                    mapping: Default::default(),
                    sources: Default::default(),
                    public_key_file: None,
                    transparency_log: false,
                    transparency_url: None,
                    evidence_result: None,
                    build_namespace: None,
                    stage_naming: None,
                    github: Some(Handle::anonymous_at(&server.address)),
                }),
                kubeconfig: None,
            };
            page.read_answer(change, window, cx);
        });
    })
    .unwrap();
    cx.wait_for_io(handle, |_, _| server.accepted.load(Ordering::SeqCst) == 1)
        .await;
    assert!(cx.read(|cx| page.read(cx).pull_reading()));
    assert_eq!(
        line(cx, &page).as_deref(),
        Some("Pull request #418 · reading GitHub…")
    );
    // Hidden, the read is dropped: nothing is loading and the other side
    // sees the connection go.
    cx.update(|cx| page.update(cx, |page, cx| page.set_visible(false, cx)));
    assert!(!cx.read(|cx| page.read(cx).pull_reading()));
    cx.wait_for_io(handle, |_, _| server.closed.load(Ordering::SeqCst) == 1)
        .await;
    // Shown again, it asks again.
    cx.update(|cx| page.update(cx, |page, cx| page.set_visible(true, cx)));
    cx.wait_for_io(handle, |_, _| server.accepted.load(Ordering::SeqCst) == 2)
        .await;
    assert!(cx.read(|cx| page.read(cx).pull_reading()));
    // A change of client drops what was read for the last one.
    cx.update(|cx| page.update(cx, |page, _| page.pull_reset()));
    assert_eq!(cx.read(|cx| page.read(cx).pull_key().cloned()), None);
}
