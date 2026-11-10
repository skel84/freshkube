//! The explorer on example data: every state of the connect step, the lazy
//! schema browser, and a capped table. The example session is
//! `freshkube_db::fake`; no forward, Secret or socket is used.
use super::super::tests::mount;
use super::{Example, Fail, Phase};
use crate::screens::ScreenPanel;
use freshkube_ui::real_io::RealIo;
use gpui_kit::test::TestWindowExt;
use gpui_kit::{AppContext, Entity, TestAppContext, WindowHandle};

type Page = Entity<super::super::DatabasesScreen>;

/// Mounts the databases page, chooses `orders-db` and presses Explore.
async fn open(
    cx: &mut TestAppContext,
    shape: Example,
) -> (
    tokio::runtime::Runtime,
    Page,
    WindowHandle<gpui_kit::component::Root>,
) {
    cx.executor().allow_parking();
    let (runtime, page, window) = mount(cx, 1280.);
    cx.update_window(window.into(), |_, window, cx| {
        page.update(cx, |page, _| page.explore_example = Some(shape));
        window.render_frame(cx);
        let id = page
            .read(cx)
            .display
            .rows
            .iter()
            .find(|row| row.identity.name == "orders-db")
            .unwrap()
            .id
            .clone();
        window.click(id, cx);
        window.render_frame(cx);
        window.click("databases-explore", cx);
        window.render_frame(cx);
    })
    .unwrap();
    cx.run_until_parked();
    (runtime, page, window)
}

fn explorer(page: &Page, cx: &gpui_kit::App) -> Entity<super::ExploreView> {
    page.read(cx).explorers[0].clone()
}

async fn connected(
    cx: &mut TestAppContext,
    page: &Page,
    window: &WindowHandle<gpui_kit::component::Root>,
) {
    cx.wait_for_io((*window).into(), |_, cx| {
        let view = explorer(page, cx);
        let view = view.read(cx);
        view.phase() == &Phase::Connected && view.browser.read(cx).tables_of("public") == Some(true)
    })
    .await;
}

#[gpui_kit::test]
async fn connects_read_only_and_shows_the_writes_gate_as_a_later_slice(cx: &mut TestAppContext) {
    let (_runtime, page, window) = open(cx, Example::Connected).await;
    connected(cx, &page, &window).await;
    cx.update_window(window.into(), |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(
            window.find("explore-read-only").label(),
            Some("Read-only session")
        );
        assert_eq!(
            window.find("explore-writes").label(),
            Some("Writes: later slice")
        );
        assert!(
            window
                .find("explore-who")
                .label()
                .unwrap()
                .contains("reader Secret")
        );
        // The inventory's own controls give way to the tab.
        assert!(window.try_find("databases-open").is_none());
        assert!(window.try_find("databases-explore-tab-orders-db").is_some());
    })
    .unwrap();
}

#[gpui_kit::test]
async fn schemas_are_read_lazily_and_the_first_opens_alone(cx: &mut TestAppContext) {
    let (_runtime, page, window) = open(cx, Example::Connected).await;
    connected(cx, &page, &window).await;
    cx.update_window(window.into(), |_, window, cx| {
        window.render_frame(cx);
        let view = explorer(&page, cx);
        let browser = view.read(cx).browser.read(cx);
        assert!(browser.tables_of("public").is_some());
        assert!(
            browser.tables_of("audit").is_none(),
            "audit is not read until opened"
        );
        assert!(window.try_find("explore-table-public-orders").is_some());
        window.click("explore-schema-audit", cx);
    })
    .unwrap();
    cx.wait_for_io(window.into(), |_, cx| {
        let view = explorer(&page, cx);
        let browser = view.read(cx).browser.read(cx);
        browser.tables_of("audit") == Some(true)
    })
    .await;
    cx.update_window(window.into(), |_, window, cx| {
        window.render_frame(cx);
        assert!(window.try_find("explore-table-audit-events").is_some());
        // Closing a schema hides its tables and keeps what was read.
        window.click("explore-schema-audit", cx);
        window.render_frame(cx);
        assert!(window.try_find("explore-table-audit-events").is_none());
    })
    .unwrap();
}

#[gpui_kit::test]
async fn a_table_larger_than_the_cap_shows_its_first_rows_and_says_so(cx: &mut TestAppContext) {
    let (_runtime, page, window) = open(cx, Example::Connected).await;
    connected(cx, &page, &window).await;
    cx.update_window(window.into(), |_, window, cx| {
        window.render_frame(cx);
        window.click("explore-table-public-orders", cx);
    })
    .unwrap();
    cx.wait_for_io(window.into(), |_, cx| {
        let view = explorer(&page, cx);
        let data = view.read(cx).data.read(cx);
        data.summary().is_some()
    })
    .await;
    cx.update_window(window.into(), |_, window, cx| {
        window.render_frame(cx);
        let summary = window.find("explore-summary").label().unwrap().to_owned();
        assert!(
            summary.contains("public.orders") && summary.contains("first 200 rows"),
            "{summary}"
        );
        assert!(window.try_find("explore-capped").is_some());
        // A row opens its record in the Inspector, with NULL as such.
        window.click("explore-row-1", cx);
        window.render_frame(cx);
        assert!(window.try_find("explore-record").is_some());
    })
    .unwrap();
}

#[gpui_kit::test]
async fn a_small_table_is_not_capped(cx: &mut TestAppContext) {
    let (_runtime, page, window) = open(cx, Example::Connected).await;
    connected(cx, &page, &window).await;
    cx.update_window(window.into(), |_, window, cx| {
        window.render_frame(cx);
        window.click("explore-table-public-customers", cx);
    })
    .unwrap();
    cx.wait_for_io(window.into(), |_, cx| {
        explorer(&page, cx)
            .read(cx)
            .data
            .read(cx)
            .summary()
            .is_some()
    })
    .await;
    cx.update_window(window.into(), |_, window, cx| {
        window.render_frame(cx);
        assert!(
            window
                .find("explore-summary")
                .label()
                .unwrap()
                .ends_with("12 rows")
        );
        assert!(window.try_find("explore-capped").is_none());
    })
    .unwrap();
}

#[gpui_kit::test]
async fn an_empty_table_says_it_has_no_rows(cx: &mut TestAppContext) {
    let (_runtime, page, window) = open(cx, Example::Connected).await;
    connected(cx, &page, &window).await;
    cx.update_window(window.into(), |_, window, cx| {
        window.render_frame(cx);
        window.click("explore-table-public-products", cx);
    })
    .unwrap();
    cx.wait_for_io(window.into(), |_, cx| {
        explorer(&page, cx)
            .read(cx)
            .data
            .read(cx)
            .summary()
            .is_some()
    })
    .await;
    cx.update_window(window.into(), |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(
            window.find("explore-data-note").label(),
            Some("This table has no rows.")
        );
    })
    .unwrap();
}

async fn state(shape: Example, id: &'static str, cx: &mut TestAppContext) {
    let (_runtime, page, window) = open(cx, shape).await;
    // Trying again can't change a setting the server keeps off.
    let retry = shape != Example::NotReadOnly;
    cx.update_window(window.into(), |_, window, cx| {
        window.render_frame(cx);
        assert!(window.try_find(id).is_some(), "{id}");
        assert_eq!(window.try_find("explore-retry").is_some(), retry);
        if retry {
            // A failed connection offers to try again and stays in its state.
            window.click("explore-retry", cx);
            window.render_frame(cx);
            assert!(window.try_find(id).is_some(), "{id} after Try again");
        }
        assert!(window.try_find("explore-read-only").is_none());
    })
    .unwrap();
    assert!(matches!(explorer_phase(&page, cx), Phase::Failed(_)));
}

fn explorer_phase(page: &Page, cx: &TestAppContext) -> Phase {
    cx.read(|cx| explorer(page, cx).read(cx).phase().clone())
}

#[gpui_kit::test]
async fn no_reader_secret_is_a_state(cx: &mut TestAppContext) {
    state(Example::NoSecret, "explore-state-no-secret", cx).await;
}

#[gpui_kit::test]
async fn a_failed_forward_is_a_state(cx: &mut TestAppContext) {
    state(Example::ForwardFailed, "explore-state-forward-failed", cx).await;
}

#[gpui_kit::test]
async fn a_refused_login_is_a_state(cx: &mut TestAppContext) {
    state(Example::Refused, "explore-state-refused", cx).await;
}

#[gpui_kit::test]
async fn a_timeout_is_a_state(cx: &mut TestAppContext) {
    state(Example::TimedOut, "explore-state-timed-out", cx).await;
}

#[gpui_kit::test]
async fn a_session_that_is_not_read_only_is_refused(cx: &mut TestAppContext) {
    state(Example::NotReadOnly, "explore-state-not-read-only", cx).await;
}

#[gpui_kit::test]
async fn the_first_step_shows_while_connecting_and_has_no_retry(cx: &mut TestAppContext) {
    let (_runtime, _page, window) = open(cx, Example::Loading).await;
    cx.update_window(window.into(), |_, window, cx| {
        window.render_frame(cx);
        assert!(window.try_find("explore-state-connecting").is_some());
        assert!(window.try_find("explore-retry").is_none());
    })
    .unwrap();
}

#[gpui_kit::test]
async fn closing_the_tab_returns_to_the_inventory_and_drops_the_explorer(cx: &mut TestAppContext) {
    let (_runtime, page, window) = open(cx, Example::Connected).await;
    connected(cx, &page, &window).await;
    cx.update_window(window.into(), |_, window, cx| {
        window.render_frame(cx);
        window.click("databases-explore-close-orders-db", cx);
        window.render_frame(cx);
        assert!(page.read(cx).explorers.is_empty());
        assert!(window.try_find("databases-open").is_some());
    })
    .unwrap();
}

#[gpui_kit::test]
async fn hiding_the_page_stops_reading_and_showing_it_resumes(cx: &mut TestAppContext) {
    let (_runtime, page, window) = open(cx, Example::Connected).await;
    connected(cx, &page, &window).await;
    cx.update_window(window.into(), |_, window, cx| {
        page.update(cx, |page, cx| page.set_visible(false, window, cx));
        assert!(!explorer(&page, cx).read(cx).visible);
        page.update(cx, |page, cx| page.set_visible(true, window, cx));
        assert!(explorer(&page, cx).read(cx).visible);
    })
    .unwrap();
    connected(cx, &page, &window).await;
}

#[gpui_kit::test]
async fn the_explorer_keeps_the_shared_frame_and_table_sizes_at_each_text_size(
    cx: &mut TestAppContext,
) {
    use crate::desktop::layout_check::{self, PageFrame, Table};
    let (_runtime, page, window) = open(cx, Example::Connected).await;
    connected(cx, &page, &window).await;
    cx.update_window(window.into(), |_, window, cx| {
        window.render_frame(cx);
        window.click("explore-table-public-orders", cx);
    })
    .unwrap();
    cx.wait_for_io(window.into(), |_, cx| {
        explorer(&page, cx)
            .read(cx)
            .data
            .read(cx)
            .summary()
            .is_some()
    })
    .await;
    for text in [13., 16., 20.] {
        cx.update(|cx| crate::text_size::set(text, cx));
        cx.update_window(window.into(), |_, window, cx| {
            layout_check::settle_header(window, cx);
            window.render_frame(cx);
            layout_check::assert_edge_frame(
                window,
                cx,
                &PageFrame {
                    page: "databases-page",
                    title: "databases-title",
                    title_text: "Databases",
                    content: "explore",
                },
            );
            layout_check::assert_table(
                window,
                cx,
                &Table {
                    table: Some("explore-browser-table-scroll"),
                    list: "explore-browser-list",
                },
            );
            layout_check::assert_table(
                window,
                cx,
                &Table {
                    table: Some("explore-data-table-scroll"),
                    list: "explore-data-list",
                },
            );
        })
        .unwrap();
    }
}

#[gpui_kit::test]
async fn arrow_keys_step_the_browser_and_open_the_table_under_them(cx: &mut TestAppContext) {
    let (_runtime, page, window) = open(cx, Example::Connected).await;
    connected(cx, &page, &window).await;
    cx.update_window(window.into(), |_, window, cx| {
        window.render_frame(cx);
        window.click("explore-table-public-orders", cx);
        window.render_frame(cx);
        window.press("down", cx);
    })
    .unwrap();
    cx.wait_for_io(window.into(), |_, cx| {
        explorer(&page, cx)
            .read(cx)
            .data
            .read(cx)
            .summary()
            .is_some_and(|s| s.starts_with("public.products"))
    })
    .await;
}

#[gpui_kit::test]
async fn the_strip_says_what_protects_the_connection(cx: &mut TestAppContext) {
    for (shape, words) in [
        (Example::Connected, "Encrypted, certificate verified"),
        (Example::Unverified, "Encrypted, certificate not verified"),
        (Example::Plain, "Not encrypted"),
    ] {
        let (_runtime, page, window) = open(cx, shape).await;
        connected(cx, &page, &window).await;
        cx.update_window(window.into(), |_, window, cx| {
            window.render_frame(cx);
            assert_eq!(window.find("explore-encryption").label(), Some(words));
            window.click("databases-explore-close-orders-db", cx);
        })
        .unwrap();
    }
}

/// Nothing a failed or finished connection prints holds the password.
#[gpui_kit::test]
async fn no_state_prints_a_password(cx: &mut TestAppContext) {
    use super::{Connected, Fail};
    use freshkube_db::{ConnectError, wire::Encryption};
    let secret = "hunter2";
    // The real error of a connect with that password to a closed port.
    let real = {
        use freshkube_db::{
            Limits,
            credentials::Password,
            wire::{Target, TlsPlan, TokioPostgres},
        };
        let port = std::net::TcpListener::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap()
            .port();
        let password = Password::new(secret);
        let limits = Limits {
            connect_timeout: std::time::Duration::from_secs(2),
            ..Limits::default()
        };
        let plan = TlsPlan {
            server_name: "orders-db-ro.staging.svc".into(),
            ca_pem: None,
        };
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let error = runtime
            .block_on(TokioPostgres::connect(
                Target {
                    address: "127.0.0.1".parse().unwrap(),
                    port,
                    user: "shop_reader",
                    database: "shop",
                },
                &password,
                &limits,
                &plan,
            ))
            .err()
            .expect("nothing listens there");
        assert!(!format!("{error:?} {error}").contains(secret));
        Fail::of_connect(error)
    };
    let fails = [
        real,
        Fail::of_connect(ConnectError::Refused(
            "password authentication failed for \"u\"".to_owned(),
        )),
        Fail::of_connect(ConnectError::TimedOut),
        Fail::of_connect(ConnectError::Untrusted("invalid peer certificate".into())),
        Fail::NoSecret("orders-db-app".into()),
        Fail::Superuser("orders-db-superuser".into()),
    ];
    for fail in fails {
        let phase = Phase::Failed(fail.clone());
        for text in [
            format!("{fail:?}"),
            fail.words(),
            fail.title().to_owned(),
            format!("{phase:?}"),
        ] {
            assert!(!text.contains(secret), "{text}");
        }
    }
    let (_runtime, page, window) = open(cx, Example::Connected).await;
    connected(cx, &page, &window).await;
    cx.update(|cx| {
        let view = explorer(&page, cx);
        let connected: Connected = view.read(cx).connect.data().unwrap().clone().unwrap();
        assert!(!format!("{connected:?}").contains(secret));
        assert_eq!(connected.encryption, Encryption::Verified);
    });
}

#[gpui_kit::test]
async fn closing_the_tab_removes_the_forward_it_started(cx: &mut TestAppContext) {
    use freshkube_core::resources::ForwardTarget;
    use freshkube_resources::forwards::{self, ForwardSpec};
    let (runtime, page, window) = open(cx, Example::Connected).await;
    connected(cx, &page, &window).await;
    cx.update_window(window.into(), |_, _, cx| {
        let spec = ForwardSpec {
            runtime: runtime.handle().clone(),
            access: freshkube_core::cluster_source::ClusterAccess::EXAMPLE,
            identity: freshkube_nav::ResourceIdentity {
                connection: "invented-access".into(),
                resource: "pods".into(),
                namespace: "staging".into(),
                name: "orders-db-2".into(),
                uid: "invented-uid".into(),
            },
            target: ForwardTarget::Pod {
                name: "orders-db-2".into(),
                uid: "invented-uid".into(),
            },
            context: "invented-db-context".into(),
            port: 8080,
            local_port: None,
        };
        let list = forwards::list(cx);
        let view = list.update(cx, |list, cx| list.start(spec, cx));
        explorer(&page, cx).update(cx, |explorer, _| explorer.adopt_forward(view));
        assert_eq!(list.read(cx).len(), 1);
    })
    .unwrap();
    cx.update_window(window.into(), |_, window, cx| {
        window.render_frame(cx);
        window.click("databases-explore-close-orders-db", cx);
        window.render_frame(cx);
    })
    .unwrap();
    // The tab's release runs once the update that dropped it ends.
    cx.run_until_parked();
    cx.update(|cx| {
        assert_eq!(
            forwards::list(cx).read(cx).len(),
            0,
            "the forward outlived its tab"
        )
    });
}

#[gpui_kit::test]
async fn a_narrow_window_with_large_text_stacks_the_panes_and_keeps_the_rows_in_reach(
    cx: &mut TestAppContext,
) {
    cx.executor().allow_parking();
    let (_runtime, page, window) = super::super::tests::mount_sized(cx, 760., 560.);
    cx.update(|cx| crate::text_size::set(20., cx));
    cx.update_window(window.into(), |_, window, cx| {
        page.update(cx, |page, _| {
            page.explore_example = Some(Example::Connected)
        });
        window.render_frame(cx);
        let id = page
            .read(cx)
            .display
            .rows
            .iter()
            .find(|r| r.identity.name == "orders-db")
            .unwrap()
            .id
            .clone();
        window.click(id, cx);
        window.render_frame(cx);
        window.click("databases-explore", cx);
        window.render_frame(cx);
    })
    .unwrap();
    connected(cx, &page, &window).await;
    cx.update_window(window.into(), |_, window, cx| {
        window.render_frame(cx);
        window.click("explore-table-public-orders", cx);
    })
    .unwrap();
    cx.wait_for_io(window.into(), |_, cx| {
        explorer(&page, cx)
            .read(cx)
            .data
            .read(cx)
            .summary()
            .is_some()
    })
    .await;
    cx.update_window(window.into(), |_, window, cx| {
        window.render_frame(cx);
        window.click("explore-row-1", cx);
        window.render_frame(cx);
        let browser = window.find("explore-browser").bounds();
        let rows = window.find("explore-data-table").bounds();
        assert!(
            rows.top() >= browser.bottom(),
            "stacked: {browser:?} above {rows:?}"
        );
        assert!(rows.size.width > gpui_kit::px(500.), "{rows:?}");
        // The record starts out below the window, and the page scrolls to it.
        let before = window.find("explore-record").bounds();
        assert!(
            before.bottom() > window.viewport_size().height,
            "nothing needed scrolling: {before:?}"
        );
        window.scroll(
            "databases-page",
            gpui_kit::ScrollDelta::Pixels(gpui_kit::point(gpui_kit::px(0.), gpui_kit::px(-4000.))),
            cx,
        );
        window.render_frame(cx);
        let record = window.find("explore-record").bounds();
        let viewport = window.viewport_size();
        assert!(
            record.top() >= gpui_kit::px(0.) && record.bottom() <= viewport.height,
            "the record is out of reach: {record:?} in {viewport:?}"
        );
        // The strip wraps instead of cutting its actions off.
        let gate = window.find("explore-writes-gate").bounds();
        assert!(gate.right() <= viewport.width, "{gate:?}");
    })
    .unwrap();
}

#[gpui_kit::test]
async fn the_query_button_hands_the_dock_the_explorers_session_and_closing_ends_it(
    cx: &mut TestAppContext,
) {
    use super::super::query_link::QueryRequest;
    use std::{cell::RefCell, rc::Rc};
    let (_runtime, page, window) = open(cx, Example::Connected).await;
    let requests: Rc<RefCell<Vec<QueryRequest>>> = Rc::default();
    let heard = requests.clone();
    let _subscription = cx.update(|cx| {
        cx.subscribe(&page, move |_, request: &QueryRequest, _| {
            heard.borrow_mut().push(request.clone())
        })
    });
    connected(cx, &page, &window).await;
    cx.update_window(window.into(), |_, window, cx| {
        window.render_frame(cx);
        window.click("explore-query", cx);
        window.render_frame(cx);
    })
    .unwrap();
    cx.run_until_parked();
    let request = requests.borrow().first().cloned().expect("a request");
    assert_eq!(request.title, "orders-db");
    assert!(request.start.is_none());
    // It hands out the explorer's own session, never a credential.
    let live = cx.update(|cx| (request.live)(cx)).expect("connected");
    assert!(live.who.contains("reader Secret"));
    assert!(!format!("{request:?}").contains("pass"));
    // Closing the explorer's tab ends it: the request finds nothing.
    cx.update_window(window.into(), |_, window, cx| {
        window.click("databases-explore-close-orders-db", cx);
        window.render_frame(cx);
    })
    .unwrap();
    cx.run_until_parked();
    assert!(cx.update(|cx| (request.live)(cx)).is_none());
}

#[gpui_kit::test]
async fn a_connection_that_did_not_open_has_no_query_button(cx: &mut TestAppContext) {
    let (_runtime, _page, window) = open(cx, Example::Refused).await;
    cx.update_window(window.into(), |_, window, cx| {
        window.render_frame(cx);
        assert!(window.try_find("explore-query").is_none());
        assert!(window.try_find("explore-retry").is_some());
    })
    .unwrap();
}

/// The Secret is read once per connection, however the Query tab is used
/// afterwards. This drives the explorer's own live connect (`connect_live`,
/// the only caller of `read_login`) against a recording fake of the Kubernetes
/// API and a recording fake Postgres server, then opens Query, runs a
/// statement, cancels one that hangs, and runs another. Each of them goes over
/// the real wire; none of them asks the API for anything.
#[gpui_kit::test]
async fn the_login_secret_is_read_once_whatever_the_query_tab_runs_and_cancels(
    cx: &mut TestAppContext,
) {
    use crate::desktop::dock::QueryView;
    use crate::screens::{Live, QueryRequest};
    use base64::Engine as _;
    use freshkube_core::{
        cluster_source::{ClusterAccess, KubeClientSource},
        databases::example,
        fake_api::{self, json},
    };
    use freshkube_db::fake::FakeServer;
    use freshkube_ui::level::Mark;
    use gpui_kit::{px, size};
    use std::sync::Arc;

    cx.executor().allow_parking();
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .unwrap();
    // The fake client's worker runs on this runtime.
    let _entered = runtime.enter();
    let database = example::inventory(chrono::Utc::now())
        .databases
        .items
        .into_iter()
        .find(|d| d.identity.name == "orders-db")
        .expect("the example has orders-db");
    let namespace = database.identity.namespace.clone();
    let b64 = |text: &str| base64::engine::general_purpose::STANDARD.encode(text);
    let secret = serde_json::json!({
        "apiVersion": "v1", "kind": "Secret",
        "metadata": {"name": "orders-db-app", "namespace": namespace},
        "data": {"username": b64("shop_app"), "password": b64("hunter2"), "dbname": b64("shop")},
    });
    let not_found = serde_json::json!({
        "kind": "Status", "apiVersion": "v1", "status": "Failure", "code": 404,
        "reason": "NotFound", "message": "not found",
    });
    let login_path = format!("/api/v1/namespaces/{namespace}/secrets/orders-db-app");
    let (client, recorder) = fake_api::client(move |request| {
        let secrets = format!("/api/v1/namespaces/{namespace}/secrets/");
        if request.method != http::Method::GET {
            return None;
        }
        if request.path == login_path {
            Some(json(200, &secret))
        } else if request.path.starts_with(&secrets) || request.path.contains("/clusters/") {
            // The CA Secret and the Cluster object: not there.
            Some(json(404, &not_found))
        } else {
            None
        }
    });
    struct Source(kube::Client);
    impl KubeClientSource for Source {
        fn client(
            &self,
        ) -> futures::future::BoxFuture<'_, Result<kube::Client, freshkube_core::failure::Failure>>
        {
            let client = self.0.clone();
            Box::pin(async move { Ok(client) })
        }
    }
    let access = ClusterAccess::Live(Arc::new(Source(client)));
    let server = runtime.block_on(FakeServer::start());
    let port = server.port();
    let connected = runtime
        .block_on(super::connect_live(access, database, port, true))
        .map_err(|f| format!("{f:?}"))
        .expect("the explorer connects to the fake server");
    assert!(connected.app_user);
    let login_reads = |lines: &[String]| {
        lines
            .iter()
            .filter(|l| l.contains("/secrets/orders-db-app"))
            .count()
    };
    let after_connect = recorder.lines();
    assert_eq!(login_reads(&after_connect), 1, "{after_connect:?}");

    // Open Query on that session.
    let session = connected.session.clone();
    let (who, encryption, app_user) = (
        connected.who.clone(),
        connected.encryption,
        connected.app_user,
    );
    let request = QueryRequest {
        start: None,
        key: "uid".into(),
        title: "orders-db".into(),
        live: std::rc::Rc::new(move |_| {
            Some(Live {
                session: session.clone(),
                who: who.clone(),
                route: None,
                encryption,
                app_user,
            })
        }),
    };
    cx.update(|cx| {
        gpui_kit::init(cx);
        cx.set_reduce_motion(true);
        crate::theme::install(cx)
    });
    let handle = runtime.handle().clone();
    let mut view = None;
    let window = cx.open_window(size(px(1100.), px(700.)), |window, cx| {
        let made = cx.new(|cx| QueryView::new(handle, request, Mark::Guarded, window, cx));
        view = Some(made.clone());
        gpui_kit::component::Root::new(made, window, cx)
    });
    let view = view.unwrap();
    let run = |cx: &mut TestAppContext, text: &str| {
        let text = text.to_owned();
        cx.update_window(window.into(), |_, window, cx| {
            view.update(cx, |v, cx| v.start_example(text, window, cx))
        })
        .unwrap();
    };

    run(cx, "select 1");
    cx.wait_for_io(window.into(), |_, cx| !view.read(cx).running())
        .await;
    // A statement the server holds, then Cancel.
    run(cx, "select pg_sleep(60)");
    cx.wait_for_io(window.into(), |_, cx| view.read(cx).cancellable())
        .await;
    cx.update_window(window.into(), |_, window, cx| {
        window.render_frame(cx);
        window.click("query-cancel", cx);
    })
    .unwrap();
    cx.wait_for_io(window.into(), |_, cx| !view.read(cx).running())
        .await;
    cx.update_window(window.into(), |_, window, cx| {
        window.render_frame(cx);
        assert!(window.try_find("query-state-cancelled").is_some());
    })
    .unwrap();
    // And run again, on the same session, after the cancel.
    run(cx, "select 2");
    cx.wait_for_io(window.into(), |_, cx| !view.read(cx).running())
        .await;
    cx.update_window(window.into(), |_, window, cx| {
        window.render_frame(cx);
        assert!(window.try_find("query-summary").is_some());
        assert!(window.try_find("query-state-cancelled").is_none());
    })
    .unwrap();

    // The server saw one cancel request, on its own connection; and the
    // Kubernetes API was asked nothing since the connect: still one read of
    // the login Secret.
    assert_eq!(server.cancels(), 1);
    let lines = recorder.lines();
    assert_eq!(lines, after_connect, "the API was asked again: {lines:?}");
    assert_eq!(login_reads(&lines), 1);
}

// Teleport (E3).

#[gpui_kit::test]
async fn each_teleport_state_is_shown_with_its_command(cx: &mut TestAppContext) {
    for (shape, id, command) in [
        (
            Example::TeleportNoTsh,
            "explore-state-teleport-not-installed",
            "brew install teleport",
        ),
        (
            Example::TeleportNotLoggedIn,
            "explore-state-teleport-not-logged-in",
            "tsh login --proxy=teleport.example.com",
        ),
        (
            Example::TeleportNoRole,
            "explore-state-teleport-no-role",
            "tsh status --proxy=teleport.example.com",
        ),
        (
            Example::TeleportNotFound,
            "explore-state-teleport-not-found",
            "tsh db ls --proxy=teleport.example.com",
        ),
        (
            Example::TeleportProxyFailed,
            "explore-state-teleport-proxy-failed",
            "tsh --proxy=teleport.example.com proxy db --tunnel",
        ),
    ] {
        let (_runtime, page, window) = open(cx, shape).await;
        cx.update_window(window.into(), |_, window, cx| {
            window.render_frame(cx);
            let state = window.try_find(id).unwrap_or_else(|| panic!("{id}"));
            let words = state.label().unwrap_or_default().to_owned();
            assert!(words.contains(command), "{id}: {words}");
            // The person copies the command; Freshkube runs nothing.
            assert!(window.try_find("explore-copy-command").is_some(), "{id}");
            assert!(window.try_find("explore-retry").is_some(), "{id}");
            assert!(window.try_find("explore-read-only").is_none());
        })
        .unwrap();
        cx.update(|cx| {
            let view = explorer(&page, cx);
            let Phase::Failed(fail) = view.read(cx).phase().clone() else {
                panic!("{id}: not failed");
            };
            assert!(fail.command().unwrap().contains(command));
        });
    }
}

#[gpui_kit::test]
async fn teleport_connects_read_only_and_says_it_is_a_tunnel_not_unencrypted(
    cx: &mut TestAppContext,
) {
    let (_runtime, page, window) = open(cx, Example::Teleport).await;
    connected(cx, &page, &window).await;
    cx.update_window(window.into(), |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(
            window.find("explore-read-only").label(),
            Some("Read-only session")
        );
        assert_eq!(
            window.find("explore-encryption").label(),
            Some("Through Teleport (tsh tunnel)")
        );
        let who = window
            .find("explore-who")
            .label()
            .unwrap_or_default()
            .to_owned();
        assert!(who.contains("Teleport") && !who.contains("Secret"), "{who}");
        let route = window
            .find("explore-route")
            .label()
            .unwrap_or_default()
            .to_owned();
        assert!(route.contains("orders-db-ro"), "{route}");
    })
    .unwrap();
}

/// A fake `tsh` that signs in, lists `orders-db-ro` and announces the proxy on
/// `port`. Each proxy it starts adds its pid to `pids`.
#[cfg(unix)]
fn fake_tsh(dir: &std::path::Path, port: u16) -> std::path::PathBuf {
    use std::os::unix::fs::PermissionsExt;
    let script = format!(
        "#!/bin/sh\n[ \"$1\" = --warmup ] && exit 0\ncase \"$2\" in\n  status) echo '{{\"active\":{{}}}}' ;;\n  db) echo '[{{\"metadata\":{{\"name\":\"orders-db-ro\"}}}}]' ;;\n  proxy) echo \"$$\" >> \"$(dirname \"$0\")/pids\"; echo 'Started authenticated tunnel for the PostgreSQL database \"orders-db-ro\" in cluster \"teleport.example.com\" on 127.0.0.1:{port}.'; exec sleep 300 ;;\nesac\n"
    );
    let path = dir.join("tsh");
    std::fs::write(&path, script).unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
    // Another test's fork may still hold the file open for writing, which
    // makes running it fail with "text file busy" for a moment.
    for _ in 0..200 {
        match std::process::Command::new(&path).arg("--warmup").status() {
            Err(e) if e.raw_os_error() == Some(26) => {
                std::thread::sleep(std::time::Duration::from_millis(10))
            }
            _ => break,
        }
    }
    path
}

#[cfg(unix)]
fn pid_alive(pid: u32) -> bool {
    std::process::Command::new("kill")
        .args(["-0", &pid.to_string()])
        .stderr(std::process::Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

#[cfg(unix)]
async fn gone(pid: u32) {
    for _ in 0..100 {
        if !pid_alive(pid) {
            return;
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
    panic!("the proxy child {pid} outlived its owner");
}

/// The pids the fake `tsh` has started proxies as, once there are `count`.
#[cfg(unix)]
fn proxy_pids(dir: &std::path::Path, count: usize) -> Vec<u32> {
    for _ in 0..400 {
        let text = std::fs::read_to_string(dir.join("pids")).unwrap_or_default();
        let pids: Vec<u32> = text.lines().filter_map(|l| l.trim().parse().ok()).collect();
        if pids.len() >= count {
            return pids;
        }
        std::thread::sleep(std::time::Duration::from_millis(25));
    }
    panic!("the fake tsh did not start {count} proxies");
}

/// A listener that accepts and never answers, so a connect over the proxy
/// stays busy while the test looks at the child.
#[cfg(unix)]
fn silent_port() -> (std::net::TcpListener, u16) {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    (listener, port)
}

#[cfg(unix)]
fn teleport_access(dir: &std::path::Path, port: u16) -> freshkube_db::teleport::TeleportAccess {
    freshkube_db::teleport::TeleportAccess {
        tsh: Some(fake_tsh(dir, port)),
        ..super::example_teleport()
    }
}

#[cfg(unix)]
#[test]
fn a_failed_connect_over_the_proxy_leaves_no_child() {
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let tmp = tempfile::tempdir().unwrap();
    // Nothing serves port 9, so the connection is refused; the job's end
    // drops the proxy and with it the child.
    let access = teleport_access(tmp.path(), 9);
    let result = runtime.block_on(super::connect_teleport(access, "orders-db".into()));
    assert!(
        matches!(
            result,
            Err(Fail::Unreachable(_) | Fail::Refused(_) | Fail::TimedOut)
        ),
        "{result:?}"
    );
    let pid = proxy_pids(tmp.path(), 1)[0];
    runtime.block_on(gone(pid));
}

#[cfg(unix)]
#[test]
fn the_tunnel_route_reads_as_a_tunnel_whatever_the_server_says_of_ssl() {
    use freshkube_db::{fake::FakeServer, wire::Encryption};
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let tmp = tempfile::tempdir().unwrap();
    // The fake server offers no TLS, as a plain Postgres would; behind
    // Teleport that would be the agent's hop, not ours, so the strip must
    // not depend on it.
    let server = runtime.block_on(FakeServer::start());
    let access = teleport_access(tmp.path(), server.port());
    let connected = runtime
        .block_on(super::connect_teleport(access, "orders-db".into()))
        .expect("connects over the fake proxy");
    assert_eq!(connected.encryption, Encryption::Tunnel);
    assert!(connected.who.contains("Teleport") && !connected.who.contains("Secret"));
    let route = connected.route.clone().unwrap();
    assert!(route.contains("orders-db-ro"), "{route}");
    assert!(!connected.app_user);
    let pid = proxy_pids(tmp.path(), 1)[0];
    assert!(pid_alive(pid));
    drop(connected);
    runtime.block_on(gone(pid));
}

/// An explorer tab that really runs the Teleport route (no example), over
/// the fake `tsh`, beside the page's example one.
#[cfg(unix)]
fn teleport_view(
    runtime: &tokio::runtime::Runtime,
    page: &Page,
    access: freshkube_db::teleport::TeleportAccess,
    cx: &mut TestAppContext,
) -> Entity<super::ExploreView> {
    cx.update(|cx| {
        let (source, database) = {
            let view = explorer(page, cx);
            let view = view.read(cx);
            (view.source.clone(), view.database.clone())
        };
        let handle = runtime.handle().clone();
        cx.new(|cx| super::ExploreView::new(handle, source, database, None, Some(access), cx))
    })
}

#[cfg(unix)]
#[gpui_kit::test]
async fn try_again_kills_the_previous_child_and_closing_kills_the_last(cx: &mut TestAppContext) {
    let (runtime, page, _window) = open(cx, Example::Teleport).await;
    let tmp = tempfile::tempdir().unwrap();
    let (_listener, port) = silent_port();
    let view = teleport_view(&runtime, &page, teleport_access(tmp.path(), port), cx);
    cx.update(|cx| view.update(cx, |v, cx| v.set_visible(true, cx)));
    let first = proxy_pids(tmp.path(), 1)[0];
    assert!(pid_alive(first));
    // Try again connects again: the first job is dropped, and its child with it.
    cx.update(|cx| view.update(cx, |v, cx| v.connect(cx)));
    let second = proxy_pids(tmp.path(), 2)[1];
    runtime.block_on(gone(first));
    assert!(pid_alive(second));
    // Dropping the tab drops its connection job and the child.
    drop(view);
    // The entity is released when the next update ends.
    cx.update(|_| {});
    cx.run_until_parked();
    runtime.block_on(gone(second));
}

#[cfg(unix)]
#[gpui_kit::test]
async fn hiding_the_tab_mid_connect_leaves_no_child(cx: &mut TestAppContext) {
    let (runtime, page, _window) = open(cx, Example::Teleport).await;
    let tmp = tempfile::tempdir().unwrap();
    let (_listener, port) = silent_port();
    let view = teleport_view(&runtime, &page, teleport_access(tmp.path(), port), cx);
    cx.update(|cx| view.update(cx, |v, cx| v.set_visible(true, cx)));
    let pid = proxy_pids(tmp.path(), 1)[0];
    assert!(pid_alive(pid));
    cx.update(|cx| view.update(cx, |v, cx| v.set_visible(false, cx)));
    runtime.block_on(gone(pid));
}

/// A connected Example tab whose connection holds a real proxy over the fake
/// `tsh`; returns the child's pid.
#[cfg(unix)]
async fn connected_with_a_proxy(
    cx: &mut TestAppContext,
    runtime: &tokio::runtime::Runtime,
    page: &Page,
    dir: &std::path::Path,
) -> u32 {
    use freshkube_db::teleport::Proxy;
    let proxy = runtime
        .block_on(Proxy::start(&teleport_access(dir, 9), "orders-db"))
        .expect("the fake tsh listens");
    let pid = proxy.pid().unwrap();
    assert!(pid_alive(pid));
    // The tab owns the connection that holds the proxy.
    cx.update(|cx| {
        explorer(page, cx).update(cx, |view, _| {
            let mut connected: super::Connected = view.connect.data().unwrap().clone().unwrap();
            connected.proxy = Some(std::sync::Arc::new(proxy));
            view.connect.resolve(view.epoch, Ok(Ok(connected)));
        })
    });
    pid
}

#[cfg(unix)]
#[gpui_kit::test]
async fn safety_g21_closing_the_tab_kills_the_teleport_child(cx: &mut TestAppContext) {
    let (runtime, page, window) = open(cx, Example::Teleport).await;
    connected(cx, &page, &window).await;
    let tmp = tempfile::tempdir().unwrap();
    let pid = connected_with_a_proxy(cx, &runtime, &page, tmp.path()).await;
    cx.update_window(window.into(), |_, window, cx| {
        window.render_frame(cx);
        window.click("databases-explore-close-orders-db", cx);
        window.render_frame(cx);
    })
    .unwrap();
    cx.run_until_parked();
    assert!(cx.read(|cx| page.read(cx).explorers.is_empty()));
    runtime.block_on(gone(pid));
}

#[cfg(unix)]
#[gpui_kit::test]
async fn safety_g21_quitting_the_app_ends_the_teleport_child(cx: &mut TestAppContext) {
    let (runtime, page, window) = open(cx, Example::Teleport).await;
    connected(cx, &page, &window).await;
    let tmp = tempfile::tempdir().unwrap();
    let pid = connected_with_a_proxy(cx, &runtime, &page, tmp.path()).await;
    cx.update(|cx| cx.shutdown());
    cx.run_until_parked();
    runtime.block_on(gone(pid));
}

#[test]
fn nothing_secret_is_in_teleports_words_or_debug() {
    use freshkube_db::teleport::TeleportError;
    let hostile = "token=sekrit password=hunter2";
    let proxy_failed = Fail::Teleport(TeleportError::ProxyFailed {
        why: freshkube_db::teleport::redact(hostile, &[]),
        command: "tsh --proxy=teleport.example.com proxy db --tunnel".into(),
    });
    for text in [
        format!("{proxy_failed:?}"),
        proxy_failed.words(),
        proxy_failed.title().to_owned(),
    ] {
        assert!(
            !text.contains("sekrit") && !text.contains("hunter2"),
            "{text}"
        );
    }
}

#[test]
fn a_tsh_found_nowhere_names_the_places_and_keeps_the_install_help() {
    use freshkube_db::teleport::TeleportError;
    let fail = Fail::TshNotFound(
        "Teleport database access needs the login program 'tsh', which was not found in \
         /opt/homebrew/bin, /usr/local/bin. Install it in one of those places"
            .into(),
    );
    assert_eq!(fail.slug(), "teleport-not-installed");
    let not_installed = Fail::Teleport(TeleportError::NotInstalled);
    assert_eq!(fail.title(), not_installed.title());
    assert_eq!(fail.command().as_deref(), Some("brew install teleport"));
    let words = fail.words();
    assert!(words.contains("/opt/homebrew/bin, /usr/local/bin"));
    assert!(words.contains("On macOS, run `brew install teleport`"));
    assert!(fail.retry());
}

#[test]
fn a_chosen_tsh_is_kept_and_a_found_one_replaces_none() {
    use std::path::PathBuf;
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let chosen = freshkube_db::teleport::TeleportAccess {
        tsh: Some(PathBuf::from("/chosen/tsh")),
        ..super::example_teleport()
    };
    let kept = runtime
        .block_on(super::with_resolved_tsh(chosen, || async {
            panic!("a chosen tsh is never looked for")
        }))
        .unwrap();
    assert_eq!(kept.tsh, Some(PathBuf::from("/chosen/tsh")));

    let found = runtime
        .block_on(super::with_resolved_tsh(
            super::example_teleport(),
            || async { Ok(PathBuf::from("/opt/homebrew/bin/tsh")) },
        ))
        .unwrap();
    assert_eq!(found.tsh, Some(PathBuf::from("/opt/homebrew/bin/tsh")));

    let missing = runtime.block_on(super::with_resolved_tsh(
        super::example_teleport(),
        || async {
            Err(freshkube_core::failure::Failure::new(
                freshkube_core::failure::FailureKind::Config,
                "not found in /x".to_owned(),
            ))
        },
    ));
    assert_eq!(
        missing.unwrap_err(),
        Fail::TshNotFound("not found in /x".into())
    );
}
