//! The explorer on example data: every state of the connect step, the lazy
//! schema browser, and a capped table. The example session is
//! `freshkube_db::fake`; no forward, Secret or socket is used.
use super::super::tests::mount;
use super::{Example, Phase};
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
