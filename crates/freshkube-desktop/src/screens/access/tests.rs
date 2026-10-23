use super::{AccessScreen, PageSource, ScreenPanel, display, ui};
use crate::desktop::layout_check;
use freshkube_core::{
    access::example::NAMESPACES,
    cluster_source::{ClusterAccess, ClusterSource, KubeClientSource},
    failure::Failure,
};
use freshkube_ui::real_io::RealIo;
use gpui_kit::component::Root;
use gpui_kit::test::TestWindowExt;
use gpui_kit::{AppContext, Entity, TestAppContext, WindowHandle, px, size};
use std::sync::Arc;

fn source(access: ClusterAccess) -> PageSource {
    PageSource {
        kube: Some(ClusterSource {
            id: "invented-access".into(),
            context: "invented-access-context".into(),
            access,
        }),
        ..PageSource::default()
    }
}

fn mount_with(
    cx: &mut TestAppContext,
    width: f32,
    height: f32,
    shape: &str,
    access: impl FnOnce(&tokio::runtime::Runtime) -> ClusterAccess,
) -> (
    tokio::runtime::Runtime,
    Entity<AccessScreen>,
    WindowHandle<Root>,
) {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(1)
        .enable_all()
        .build()
        .unwrap();
    let access = access(&runtime);
    cx.update(|cx| {
        gpui_kit::init(cx);
        cx.set_reduce_motion(true);
        crate::theme::install(cx)
    });
    let mut page = None;
    let window = cx.open_window(size(px(width), px(height)), |window, cx| {
        let view = cx.new(|cx| {
            let mut view =
                AccessScreen::new(runtime.handle().clone(), window, cx).with_example(shape);
            view.set_page_source(source(access), window, cx);
            view.set_visible(true, window, cx);
            view.activate(window, cx);
            view
        });
        page = Some(view.clone());
        Root::new(view, window, cx)
    });
    cx.run_until_parked();
    (runtime, page.unwrap(), window)
}

fn mount(
    cx: &mut TestAppContext,
    width: f32,
    height: f32,
    shape: &str,
) -> (
    tokio::runtime::Runtime,
    Entity<AccessScreen>,
    WindowHandle<Root>,
) {
    mount_with(cx, width, height, shape, |_| ClusterAccess::EXAMPLE)
}

/// Draws until the rows a frame shows have been asked: each frame notes its
/// rows, and the review answers between frames.
fn settle(cx: &mut TestAppContext, window: WindowHandle<Root>) {
    for _ in 0..6 {
        cx.update_window(window.into(), |_, window, cx| window.render_frame(cx))
            .unwrap();
        cx.run_until_parked();
    }
}

fn row(page: &Entity<AccessScreen>, name: &str, cx: &gpui_kit::App) -> display::Row {
    page.read(cx)
        .display
        .rows
        .iter()
        .find(|row| row.label == name)
        .unwrap_or_else(|| panic!("no row {name}"))
        .clone()
}

/// The row's words, space apart: `yes yes no …`.
fn words(row: &display::Row) -> String {
    row.cells
        .iter()
        .map(|cell| cell.word)
        .collect::<Vec<_>>()
        .join(" ")
}

fn status_text(page: &Entity<AccessScreen>, cx: &mut gpui_kit::App) -> String {
    let segment = page.update(cx, |page, cx| page.status(cx)).unwrap();
    segment.lines(None).join(" | ")
}

#[gpui_kit::test]
fn the_page_says_what_the_rules_grant_in_words(cx: &mut TestAppContext) {
    let (_runtime, page, window) = mount(cx, 1280., 880., "limited");
    settle(cx, window);
    cx.update_window(window.into(), |_, _, cx| {
        let all = |name| words(&row(&page, name, cx));
        assert_eq!(all("shop"), "yes yes yes no yes yes yes no");
        assert_eq!(all("default"), "yes yes no no no no no no");
        assert_eq!(all("kube-system"), "no no no no no no no no");
        // The cluster-wide row is answered apart, and means nothing for most
        // columns.
        assert_eq!(all("Nodes"), "yes – – – – no – –");
        let shop = row(&page, "shop", cx);
        assert_eq!(shop.tone, ui::Tone::Info);
        assert_eq!(
            shop.cells[3].tip.as_ref(),
            "needs create on pods/portforward in shop"
        );
        assert_eq!(row(&page, "kube-system", cx).tone, ui::Tone::Info);
    })
    .unwrap();
}

#[gpui_kit::test]
fn only_the_rows_a_frame_draws_are_asked(cx: &mut TestAppContext) {
    // A window too short for all the rows.
    let (_runtime, page, window) = mount(cx, 1280., 420., "limited");
    settle(cx, window);
    let total = NAMESPACES.len() + 1;
    let first = cx.update_window(window.into(), |_, _, cx| page.read(cx).board.answered());
    let first = first.unwrap();
    assert!(first > 0 && first < total, "{first} of {total}");
    // Asking for the last row scrolls it into view, and it alone is asked.
    cx.update_window(window.into(), |_, _, cx| {
        page.update(cx, |page, cx| page.step(isize::MAX, cx))
    })
    .unwrap();
    settle(cx, window);
    cx.update_window(window.into(), |_, _, cx| {
        let board = &page.read(cx).board;
        assert!(board.answered() > first);
        let last = row(&page, NAMESPACES[NAMESPACES.len() - 1], cx);
        assert_ne!(last.cells[0].word, "…", "the row in view is answered");
    })
    .unwrap();
}

#[gpui_kit::test]
fn a_row_not_asked_yet_reads_checking(cx: &mut TestAppContext) {
    let board = freshkube_core::access::Board::default();
    let names = vec!["shop".to_owned()];
    let display = display::Display::new(&names, &board);
    let shop = display.rows.iter().find(|row| row.label == "shop").unwrap();
    assert_eq!(shop.tone, ui::Tone::Waiting);
    assert_eq!(shop.state.as_ref(), "Checking");
    assert!(shop.cells.iter().all(|cell| cell.word == "…"));
    // The page itself starts there: the first frame has no answer.
    let (_runtime, page, window) = mount(cx, 1280., 880., "limited");
    cx.update_window(window.into(), |_, _, cx| {
        assert_eq!(page.read(cx).board.answered(), 0);
    })
    .unwrap();
}

#[gpui_kit::test]
fn an_incomplete_review_reads_ask_and_says_so(cx: &mut TestAppContext) {
    let (_runtime, page, window) = mount(cx, 1280., 880., "incomplete");
    settle(cx, window);
    cx.update_window(window.into(), |_, window, cx| {
        let shop = row(&page, "shop", cx);
        assert_eq!(words(&shop), "yes yes ask ask ask ask ask ask");
        assert_eq!(shop.tone, ui::Tone::Unknown);
        window.render_frame(cx);
        assert!(window.find("access-incomplete").visible());
        assert!(status_text(&page, cx).contains("Rules incomplete"));
        // An ask is not requested as a no.
        assert!(shop.detail.denied.is_empty());
        assert!(
            shop.cells[2]
                .tip
                .contains("The check before the action decides")
        );
    })
    .unwrap();
}

#[gpui_kit::test]
fn another_person_drops_the_old_answers_at_once(cx: &mut TestAppContext) {
    let (_runtime, page, window) = mount(cx, 1280., 880., "identity-changed");
    settle(cx, window);
    cx.update_window(window.into(), |_, window, cx| {
        let changed = page.read(cx).board.changed().cloned().unwrap();
        assert_eq!(changed.from.as_deref(), Some("alice@example.test"));
        assert_eq!(changed.to.as_deref(), Some("bob@example.test"));
        window.render_frame(cx);
        assert!(window.find("access-changed").visible());
        assert!(status_text(&page, cx).contains("bob@example.test"));
        assert!(status_text(&page, cx).contains("Identity changed"));
        // What shows now was read as the new person, not kept from before.
        assert_eq!(words(&row(&page, "shop", cx)), "yes yes no no no no no no");
        window.click("access-changed-dismiss", cx);
        window.render_frame(cx);
        assert!(window.try_find("access-changed").is_none());
    })
    .unwrap();
}

#[gpui_kit::test]
fn a_review_that_fails_reads_couldnt_check_and_rechecks(cx: &mut TestAppContext) {
    let (_runtime, page, window) = mount(cx, 1280., 880., "failed");
    settle(cx, window);
    cx.update_window(window.into(), |_, window, cx| {
        let shop = row(&page, "shop", cx);
        assert_eq!(words(&shop), "? ? ? ? ? ? ? ?");
        assert_eq!(shop.tone, ui::Tone::Warn);
        assert_eq!(shop.state.as_ref(), "Couldn't check");
        assert!(status_text(&page, cx).contains("couldn't check"));
        // A row beside it that answered is not affected.
        assert_eq!(row(&page, "default", cx).cells[0].word, "yes");
        window.click("access-row-ns-shop", cx);
        window.render_frame(cx);
        assert!(window.find("access-detail-failure").visible());
        // Recheck asks again; the example fails again, and the row says so
        // rather than keeping a stale answer.
        window.click("access-detail-recheck", cx);
        window.render_frame(cx);
    })
    .unwrap();
    settle(cx, window);
    cx.update_window(window.into(), |_, _, cx| {
        assert_eq!(row(&page, "shop", cx).state.as_ref(), "Couldn't check");
    })
    .unwrap();
}

#[gpui_kit::test]
fn copy_puts_an_access_request_on_the_clipboard(cx: &mut TestAppContext) {
    let (_runtime, _page, window) = mount(cx, 1280., 880., "limited");
    settle(cx, window);
    cx.update_window(window.into(), |_, window, cx| {
        cx.write_to_clipboard(gpui_kit::ClipboardItem::new_string("unchanged".into()));
        window.click("access-row-ns-default", cx);
        window.render_frame(cx);
        window.click("access-detail-copy", cx);
        let text = cx.read_from_clipboard().and_then(|item| item.text());
        let text = text.expect("something was copied");
        assert!(
            text.contains("Cluster context: invented-access-context"),
            "{text}"
        );
        assert!(text.contains("Who: alice@example.test"), "{text}");
        assert!(text.contains("Namespace: default"), "{text}");
        assert!(text.contains("- create on pods/exec"), "{text}");
        assert!(!text.contains("Bearer"), "{text}");
        // The detail names each no in the short form.
        assert!(window.find("access-detail-shell").visible());
    })
    .unwrap();
}

#[gpui_kit::test]
fn copy_has_nothing_to_ask_for_when_everything_is_allowed(cx: &mut TestAppContext) {
    let (_runtime, page, window) = mount(cx, 1280., 880., "full");
    settle(cx, window);
    cx.update_window(window.into(), |_, window, cx| {
        cx.write_to_clipboard(gpui_kit::ClipboardItem::new_string("unchanged".into()));
        window.click("access-row-ns-shop", cx);
        window.render_frame(cx);
        assert_eq!(row(&page, "shop", cx).tone, ui::Tone::Good);
        window.click("access-detail-copy", cx);
        assert_eq!(
            cx.read_from_clipboard()
                .and_then(|item| item.text())
                .as_deref(),
            Some("unchanged")
        );
    })
    .unwrap();
}

#[gpui_kit::test]
fn the_table_and_inspector_follow_the_shared_layout(cx: &mut TestAppContext) {
    for width in [1280., 760.] {
        let (_runtime, _page, window) = mount(cx, width, 880., "limited");
        settle(cx, window);
        for text in [13., 20.] {
            cx.update_window(window.into(), |_, window, cx| {
                crate::text_size::set(text, cx);
                window.render_frame(cx);
            })
            .unwrap();
            cx.run_until_parked();
            cx.update_window(window.into(), |_, window, cx| {
                layout_check::settle_header(window, cx);
                window.render_frame(cx);
                let layout = layout_check::assert_table_page(
                    window,
                    cx,
                    &layout_check::TablePage {
                        page: "access-page",
                        title: "access-title",
                        title_text: "Your access",
                        table: "access-table-scroll",
                        list: "access-list",
                    },
                );
                assert!(layout.group.is_some(), "{layout:#?}");
                window.click("access-row-ns-shop", cx);
                window.render_frame(cx);
                layout_check::assert_inspector(
                    window,
                    cx,
                    "access-split",
                    "access-table",
                    "access-detail",
                    "access-detail-title",
                );
                window.press("escape", cx);
                window.render_frame(cx);
            })
            .unwrap();
        }
    }
}

/// A client that hands out one fake cluster.
struct Fake(kube::Client);

impl KubeClientSource for Fake {
    fn client(&self) -> futures::future::BoxFuture<'_, Result<kube::Client, Failure>> {
        let client = self.0.clone();
        Box::pin(async move { Ok(client) })
    }
}

fn cluster() -> (kube::Client, freshkube_core::fake_api::Recorder) {
    use freshkube_core::fake_api::{self, Recorded};
    use serde_json::json;
    fake_api::client(|request: &Recorded| {
        let path = request.path.as_str();
        Some(match path {
            "/api/v1/namespaces" => fake_api::json(
                200,
                json!({"apiVersion":"v1","kind":"NamespaceList","metadata":{},
                    "items": NAMESPACES.iter().map(|name| json!({"metadata":{"name":name}})).collect::<Vec<_>>()}),
            ),
            "/apis/authentication.k8s.io/v1/selfsubjectreviews" => fake_api::json(
                201,
                json!({"apiVersion":"authentication.k8s.io/v1","kind":"SelfSubjectReview",
                    "status":{"userInfo":{"username":"alice","groups":["devs"]}}}),
            ),
            "/apis/authorization.k8s.io/v1/selfsubjectrulesreviews" => fake_api::json(
                201,
                json!({"apiVersion":"authorization.k8s.io/v1","kind":"SelfSubjectRulesReview",
                    "status":{"resourceRules":[{"verbs":"get list watch","apiGroups":[""],"resources":["pods"]}],
                        "nonResourceRules":[],"incomplete":false}}),
            ),
            "/apis/authorization.k8s.io/v1/selfsubjectaccessreviews" => fake_api::json(
                201,
                json!({"apiVersion":"authorization.k8s.io/v1","kind":"SelfSubjectAccessReview",
                    "status":{"allowed":true}}),
            ),
            _ => return None,
        })
    })
}

#[gpui_kit::test]
async fn the_live_page_asks_only_for_the_rows_in_view_and_nothing_when_hidden(
    cx: &mut TestAppContext,
) {
    cx.executor().allow_parking();
    let mut recorder = None;
    let (_runtime, page, window) = mount_with(cx, 1280., 420., "", |runtime| {
        let _entered = runtime.enter();
        let (client, seen) = cluster();
        recorder = Some(seen);
        ClusterAccess::Live(Arc::new(Fake(client)))
    });
    let recorder = recorder.unwrap();
    cx.wait_for_io(window.into(), |_, cx| page.read(cx).board.answered() > 0)
        .await;
    // Let anything started finish.
    cx.wait_for_io(window.into(), |_, cx| page.read(cx).board.in_flight() == 0)
        .await;
    let total = NAMESPACES.len() + 1;
    let answered = cx
        .update_window(window.into(), |_, _, cx| page.read(cx).board.answered())
        .unwrap();
    assert!(answered < total, "{answered} of {total}");
    // Nothing but the namespace list, who you are, and reviews.
    recorder.assert_all(
        "the page only lists namespaces and creates reviews",
        |request| {
            let review = request
                .path
                .starts_with("/apis/authorization.k8s.io/v1/selfsubject")
                || request.path == "/apis/authentication.k8s.io/v1/selfsubjectreviews";
            (request.method == http::Method::POST && review)
                || (request.method == http::Method::GET && request.path == "/api/v1/namespaces")
        },
    );
    let lines = recorder.lines();
    let rules = lines
        .iter()
        .filter(|line| line.ends_with("selfsubjectrulesreviews"))
        .count();
    let nodes = lines
        .iter()
        .filter(|line| line.ends_with("selfsubjectaccessreviews"))
        .count();
    assert!(
        rules > 0 && rules + usize::from(nodes > 0) <= answered,
        "{lines:?}"
    );
    // The namespaces asked are exactly those of the rows answered.
    let mut asked: Vec<String> = recorder
        .requests()
        .iter()
        .filter(|request| request.path.ends_with("selfsubjectrulesreviews"))
        .map(|request| {
            let body: serde_json::Value = serde_json::from_str(&request.body).unwrap();
            body["spec"]["namespace"].as_str().unwrap().to_owned()
        })
        .collect();
    asked.sort();
    asked.dedup();
    assert_eq!(asked.len(), rules, "each namespace is asked once");

    // Hidden, the page asks nothing more, however it is drawn.
    cx.update_window(window.into(), |_, window, cx| {
        page.update(cx, |page, cx| page.set_visible(false, window, cx));
    })
    .unwrap();
    let before = recorder.lines().len();
    settle(cx, window);
    cx.update_window(window.into(), |_, _, cx| {
        page.update(cx, |page, cx| page.step(isize::MAX, cx))
    })
    .unwrap();
    settle(cx, window);
    assert_eq!(recorder.lines().len(), before, "{:?}", recorder.lines());
}

#[gpui_kit::test]
async fn recheck_asks_the_selected_row_again_and_only_that_row(cx: &mut TestAppContext) {
    cx.executor().allow_parking();
    let mut recorder = None;
    let (_runtime, page, window) = mount_with(cx, 1280., 880., "", |runtime| {
        let _entered = runtime.enter();
        let (client, seen) = cluster();
        recorder = Some(seen);
        ClusterAccess::Live(Arc::new(Fake(client)))
    });
    let recorder = recorder.unwrap();
    let total = NAMESPACES.len() + 1;
    cx.wait_for_io(window.into(), |_, cx| {
        page.read(cx).board.answered() == total
    })
    .await;
    let rules_for = |recorder: &freshkube_core::fake_api::Recorder, name: &str| {
        recorder
            .requests()
            .iter()
            .filter(|request| {
                request.path.ends_with("selfsubjectrulesreviews")
                    && request.body.contains(&format!("\"namespace\":\"{name}\""))
            })
            .count()
    };
    assert_eq!(rules_for(&recorder, "shop"), 1);
    cx.update_window(window.into(), |_, window, cx| {
        window.click("access-row-ns-shop", cx);
        window.render_frame(cx);
        window.click("access-detail-recheck", cx);
    })
    .unwrap();
    cx.wait_for_io(window.into(), |_, _| rules_for(&recorder, "shop") == 2)
        .await;
    cx.wait_for_io(window.into(), |_, cx| page.read(cx).board.in_flight() == 0)
        .await;
    assert_eq!(rules_for(&recorder, "shop"), 2);
    assert_eq!(rules_for(&recorder, "default"), 1, "no other row was asked");
}

#[gpui_kit::test]
async fn a_new_access_drops_every_answer_before_asking_again(cx: &mut TestAppContext) {
    cx.executor().allow_parking();
    let mut recorder = None;
    let (_runtime, page, window) = mount_with(cx, 1280., 880., "", |runtime| {
        let _entered = runtime.enter();
        let (client, seen) = cluster();
        recorder = Some(seen);
        ClusterAccess::Live(Arc::new(Fake(client)))
    });
    let total = NAMESPACES.len() + 1;
    cx.wait_for_io(window.into(), |_, cx| {
        page.read(cx).board.answered() == total
    })
    .await;
    cx.update_window(window.into(), |_, window, cx| {
        let mut other = source(ClusterAccess::EXAMPLE);
        other.kube.as_mut().unwrap().id = "another-access".into();
        page.update(cx, |page, cx| page.set_page_source(other, window, cx));
        // Nothing read for the old access survives, the moment it changes.
        assert_eq!(page.read(cx).board.answered(), 0);
        assert!(page.read(cx).board.changed().is_some());
    })
    .unwrap();
    drop(recorder);
}
