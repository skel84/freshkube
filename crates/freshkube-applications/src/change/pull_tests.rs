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
    open_at(cx, shape, 1280., 880.)
}

fn open_at(
    cx: &mut TestAppContext,
    shape: Shape,
    width: f32,
    height: f32,
) -> (AnyWindowHandle, Entity<ChangePage>) {
    cx.update(|cx| {
        gpui_kit::init(cx);
        freshkube_ui::theme::install(cx);
        freshkube_ui::text_size::install(None, cx);
        cx.set_reduce_motion(true);
        // These tests are about the hop table's rows.
        crate::change_view::use_session(crate::change_view::ChangeView::Hops, cx);
    });
    let mut page = None;
    let handle = cx.open_window(size(px(width), px(height)), |window, cx| {
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
        Some("Pull request #418 · Merged · 1 approved, 1 commented · checks passing (3)")
    );
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let row = window.find("change-pull");
        assert_eq!(
            row.label(),
            Some("Pull request #418 · Merged · 1 approved, 1 commented · checks passing (3)"),
        );
    })
    .unwrap();
}

#[gpui_kit::test]
fn kept_answers_say_why_they_are_old_and_a_refused_token_leaves_no_row(cx: &mut TestAppContext) {
    for (shape, why) in [
        (
            Shape::GitHubLimited,
            "GitHub's rate limit is reached until ",
        ),
        (
            Shape::GitHubOffline,
            "GitHub didn't answer: could not connect",
        ),
    ] {
        let (handle, page) = open(cx, shape);
        let text = line(cx, &page).expect("the kept answer");
        assert!(text.starts_with("Pull request #418 · Merged · "), "{text}");
        // The banner of what may be missing gives the reason; the line says
        // only that it is old, so the two never repeat each other.
        assert!(text.ends_with(" · stale, read at 13:12 UTC"), "{text}");
        assert!(!text.contains(why) && !text.contains("GitHub"), "{text}");
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            assert!(
                window
                    .find("change-missing")
                    .label()
                    .is_some_and(|label| label.contains(why)),
                "the banner gives the reason"
            );
        })
        .unwrap();
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

/// The replies of a GitHub that knows pull request 418 on `acme/shop`, and
/// every request it was asked.
fn github(
    runtime: &tokio::runtime::Runtime,
    failing: Arc<std::sync::atomic::AtomicBool>,
) -> freshkube_delivery::testing::github::Server {
    use freshkube_delivery::testing::github::{Reply, Server};
    use serde_json::json;
    let head = format!("a1f3c9e{}", "0".repeat(33));
    let _enter = runtime.enter();
    Server::new(move |request| {
        if failing.load(Ordering::SeqCst) {
            return Reply::empty(500, "");
        }
        let path = request
            .path
            .split('?')
            .next()
            .unwrap_or_default()
            .to_owned();
        let review = |login: &str, state: &str, commit: &str| {
            json!({"user": {"login": login}, "state": state, "commit_id": commit,
                   "submitted_at": "2026-10-01T09:00:00Z"})
        };
        match path.as_str() {
            "/repos/acme/shop/pulls/418" => Reply::json(
                200,
                "",
                &json!({"number": 418, "title": "Retry idempotency keys", "state": "closed",
                        "merged_at": "2026-10-01T10:00:00Z", "head": {"sha": head},
                        "user": {"login": "ken"}, "additions": 84, "deletions": 12,
                        "changed_files": 5}),
            ),
            "/repos/acme/shop/pulls/418/reviews" => Reply::json(
                200,
                "",
                &json!([
                    review("mira", "APPROVED", &head),
                    review("zoe", "COMMENTED", &head)
                ]),
            ),
            p if p == format!("/repos/acme/shop/commits/{head}/check-runs") => Reply::json(
                200,
                "",
                &json!({"total_count": 2, "check_runs": [
                    {"name": "build", "status": "completed", "conclusion": "success"},
                    {"name": "lint", "status": "completed", "conclusion": "success"}]}),
            ),
            p if p == format!("/repos/acme/shop/commits/{head}/status") => Reply::json(
                200,
                "",
                &json!({"state": "success", "total_count": 0, "statuses": []}),
            ),
            _ => Reply::empty(404, ""),
        }
    })
}

/// The page reading live through a client pointed at `server`, showing the
/// pull request the example change names.
fn live(
    cx: &mut TestAppContext,
    runtime: &tokio::runtime::Runtime,
    server: &freshkube_delivery::testing::github::Server,
) -> (AnyWindowHandle, Entity<ChangePage>) {
    let (handle, page) = open(cx, Shape::Whole);
    let github = Handle::anonymous_at(&server.address);
    cx.update_window(handle, |_, _, cx| {
        page.update(cx, |page, _| {
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
                    github: Some(github),
                }),
                kubeconfig: None,
            };
            // What the example's own read left is not what this client read.
            page.pull_reset();
        });
    })
    .unwrap();
    (handle, page)
}

fn answer(cx: &mut TestAppContext, handle: AnyWindowHandle, page: &Entity<ChangePage>) {
    let change = cx.read(|cx| page.read(cx).shown().clone());
    cx.update_window(handle, |_, window, cx| {
        page.update(cx, |page, cx| page.read_answer(change, window, cx));
    })
    .unwrap();
}

#[gpui_kit::test]
async fn a_live_read_reaches_the_drawn_line_and_is_read_again_when_it_is_old(
    cx: &mut TestAppContext,
) {
    cx.executor().allow_parking();
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let server = github(&runtime, Arc::default());
    let (handle, page) = live(cx, &runtime, &server);
    answer(cx, handle, &page);
    let expected = "Pull request #418 · Merged · 1 approved, 1 commented · checks passing (2)";
    cx.wait_for_io(handle, |_, cx| page.read(cx).pull_line() == Some(expected))
        .await;
    // Four reads of the four things, and nothing else.
    let paths: Vec<String> = server.seen().iter().map(|r| r.path.clone()).collect();
    assert_eq!(paths.len(), 4, "{paths:?}");
    // Drawn, with its words once.
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(window.find("change-pull").label(), Some(expected));
    })
    .unwrap();
    // Read again, fresh: no read is started.
    answer(cx, handle, &page);
    assert!(!cx.read(|cx| page.read(cx).pull_reading()), "still fresh");
    // Over a minute old: it is read again, and the old line stands meanwhile.
    cx.executor().advance_clock(Duration::from_secs(61));
    answer(cx, handle, &page);
    assert!(
        cx.read(|cx| page.read(cx).pull_reading()),
        "old, so read again"
    );
    assert_eq!(
        line(cx, &page).as_deref(),
        Some(expected),
        "the old line stands"
    );
    cx.wait_for_io(handle, |_, cx| !page.read(cx).pull_reading())
        .await;
    assert_eq!(line(cx, &page).as_deref(), Some(expected));
}

#[gpui_kit::test]
async fn a_failed_read_backs_off_instead_of_being_asked_on_every_refresh(cx: &mut TestAppContext) {
    cx.executor().allow_parking();
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let failing = Arc::new(std::sync::atomic::AtomicBool::new(true));
    let server = github(&runtime, failing.clone());
    let (handle, page) = live(cx, &runtime, &server);
    answer(cx, handle, &page);
    cx.wait_for_io(handle, |_, cx| {
        page.read(cx)
            .pull_line()
            .is_some_and(|line| line.contains("HTTP 500"))
    })
    .await;
    assert_eq!(server.seen().len(), 1);
    // Every refresh of the change in the next minute reads nothing.
    for _ in 0..5 {
        answer(cx, handle, &page);
        assert!(!cx.read(|cx| page.read(cx).pull_reading()));
    }
    assert_eq!(server.seen().len(), 1, "the failed read is waited out");
    // After the first wait it is read again, and fails again: the wait doubles.
    cx.executor().advance_clock(Duration::from_secs(61));
    answer(cx, handle, &page);
    assert!(cx.read(|cx| page.read(cx).pull_reading()));
    cx.wait_for_io(handle, |_, cx| !page.read(cx).pull_reading())
        .await;
    assert_eq!(server.seen().len(), 2);
    cx.executor().advance_clock(Duration::from_secs(61));
    answer(cx, handle, &page);
    assert!(
        !cx.read(|cx| page.read(cx).pull_reading()),
        "two minutes now, not one"
    );
    cx.executor().advance_clock(Duration::from_secs(60));
    failing.store(false, Ordering::SeqCst);
    answer(cx, handle, &page);
    cx.wait_for_io(handle, |_, cx| {
        page.read(cx)
            .pull_line()
            .is_some_and(|line| line.contains("checks passing"))
    })
    .await;
    // And what answered is what was kept: no more waiting.
    answer(cx, handle, &page);
    assert!(
        !cx.read(|cx| page.read(cx).pull_reading()),
        "answered, so fresh"
    );
}

#[gpui_kit::test]
fn the_row_doesnt_move_the_trail_or_the_inspector_sideways(cx: &mut TestAppContext) {
    let mut extras = Vec::new();
    for (width, height) in [(1280., 880.), (760., 560.)] {
        let (handle, page) = open_at(cx, Shape::Whole, width, height);
        let measure = |cx: &mut TestAppContext| {
            cx.update_window(handle, |_, window, cx| {
                window.render_frame(cx);
                (
                    window.find("change-split").bounds(),
                    window.find("change-table").bounds(),
                )
            })
            .unwrap()
        };
        let (with_split, with_table) = measure(cx);
        let row = cx
            .update_window(handle, |_, window, _| window.find("change-pull").bounds())
            .unwrap();
        // Above them, in the page's own width.
        assert!(
            row.bottom() <= with_split.top(),
            "{width}: {row:?} above {with_split:?}"
        );
        cx.update_window(handle, |_, _, cx| {
            page.update(cx, |page, cx| {
                page.pull_reset();
                cx.notify();
            })
        })
        .unwrap();
        let (without_split, without_table) = measure(cx);
        assert_eq!(
            cx.read(|cx| page.read(cx).pull_line().map(str::to_owned)),
            None
        );
        // The same columns: the trail and Inspector stay as wide and as
        // far from the sides as they were. A short page scrolls the whole
        // split at its full height to keep the Inspector footer reachable;
        // a taller page keeps the bottom edge and gives the row its space.
        for (with, without) in [(with_split, without_split), (with_table, without_table)] {
            assert_eq!(
                (with.left(), with.right()),
                (without.left(), without.right()),
                "{width}"
            );
            if height < freshkube_ui::page::SHORT_HEIGHT {
                assert_eq!(with.size.height, without.size.height, "{width}");
            } else {
                assert_eq!(with.bottom(), without.bottom(), "{width}");
            }
            assert!(with.top() >= without.top(), "{width}");
            // Only the row, in its block's padding, was added above them: the
            // top moved down by the row and a little more, never by a whole
            // other row, and by as much at either size.
            let moved = with.top() - without.top();
            assert!(
                moved >= row.size.height,
                "{width}: moved {moved:?}, row {:?}",
                row.size.height
            );
            let extra = moved - row.size.height;
            assert!(extra <= gpui_kit::px(24.), "{width}: {extra:?} of padding");
            extras.push(extra);
        }
    }
    assert!(
        (extras[0] - extras[1]).abs() <= gpui_kit::px(1.),
        "{extras:?}"
    );
}
