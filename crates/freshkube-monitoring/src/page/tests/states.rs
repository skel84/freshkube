use super::*;

#[gpui_kit::test]
fn without_a_source_the_page_says_it_is_not_connected(cx: &mut TestAppContext) {
    let (_runtime, handle, page) = mount(cx, None);
    show(cx, handle, &page);
    assert!(shown(cx, handle, "monitoring-page"));
    cx.read(|cx| assert!(matches!(page.read(cx).connection, Connection::None)));
    assert!(ready(cx, &page).is_empty());
    assert!(!shown(cx, handle, "monitoring-grid"));
}

#[gpui_kit::test]
fn discovery_states_show_what_was_looked_for_and_offer_retry(cx: &mut TestAppContext) {
    let (_runtime, handle, page) = mount(cx, Some(example_source()));
    show(cx, handle, &page);
    let service = PrometheusService::new("monitoring", "metrics", 9090);
    let missing = Discovery::Missing {
        candidates: vec![Candidate {
            service: service.clone(),
            rank: Rank::Named,
        }],
        tried: vec![Tried {
            service: PrometheusService::new("monitoring", "prometheus", 9090),
            error: QueryError::new(ErrorKind::NotFound, "Not found"),
        }],
    };
    cx.update(|cx| page.update(cx, |page, cx| page.discovered(Ok(missing), cx)));
    assert!(shown(cx, handle, "monitoring-candidate-0"));
    assert!(shown(cx, handle, "monitoring-retry"));
    assert!(!shown(cx, handle, "monitoring-grid"));

    cx.update(|cx| {
        page.update(cx, |page, cx| {
            page.discovered(Err(QueryError::refused()), cx)
        })
    });
    assert!(matches!(
        cx.read(|cx| page.read(cx).connection_name()),
        "refused"
    ));
    assert!(shown(cx, handle, "monitoring-retry"));

    let failed = QueryError::new(ErrorKind::Unavailable, "Service unavailable");
    cx.update(|cx| page.update(cx, |page, cx| page.discovered(Err(failed), cx)));
    assert_eq!(cx.read(|cx| page.read(cx).connection_name()), "failed");

    // Try again starts over: the example source answers at once.
    cx.update_window(handle, |_, window, cx| window.click("monitoring-retry", cx))
        .unwrap();
    cx.run_until_parked();
    frame(cx, handle);
    assert_eq!(cx.read(|cx| page.read(cx).connection_name()), "example");
    assert!(shown(cx, handle, "monitoring-grid"));
}

#[gpui_kit::test]
fn a_file_that_is_not_a_dashboard_shows_why(cx: &mut TestAppContext) {
    let folder = std::env::temp_dir().join(format!(
        "freshkube-monitoring-folder-{}",
        std::process::id()
    ));
    _ = std::fs::remove_dir_all(&folder);
    std::fs::create_dir_all(&folder).unwrap();
    std::fs::write(folder.join("broken.json"), "{ not json").unwrap();
    let (_runtime, handle, page) = mount(cx, Some(example_source()));
    cx.update(|cx| {
        page.update(cx, |page, cx| {
            page.saved.folder = Some(folder.clone());
            page.read_folder(cx);
        })
    });
    cx.run_until_parked();
    let id = cx.read(|cx| match &page.read(cx).catalog.folder {
        FolderState::Read { entries, .. } => entries[0].id.clone(),
        _ => panic!("the folder wasn't read"),
    });
    show(cx, handle, &page);
    cx.update(|cx| page.update(cx, |page, cx| page.open(id, cx)));
    cx.run_until_parked();
    frame(cx, handle);
    cx.read(|cx| {
        let board = page.read(cx).board.as_ref().unwrap();
        assert!(board.error.is_some());
        assert!(board.slots.is_empty());
    });
    assert!(!shown(cx, handle, "monitoring-grid"));
    assert!(!shown(cx, handle, "monitoring-range"));
    std::fs::remove_dir_all(&folder).unwrap();
}

/// A live cluster whose client never builds, counting the asks and what it
/// was told to forget.
#[derive(Default)]
struct Forgetting {
    asked: std::sync::atomic::AtomicUsize,
    forgot: std::sync::atomic::AtomicUsize,
}

impl KubeClientSource for Forgetting {
    fn client(&self) -> futures::future::BoxFuture<'_, Result<kube::Client, Failure>> {
        self.asked.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        Box::pin(async { Err(Failure::new(FailureKind::Config, "No kubeconfig")) })
    }

    fn forget(&self) {
        self.forgot
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    }
}

impl Forgetting {
    /// The asks and the forgets.
    fn counts(&self) -> (usize, usize) {
        use std::sync::atomic::Ordering::SeqCst;
        (self.asked.load(SeqCst), self.forgot.load(SeqCst))
    }
}

#[gpui_kit::test]
async fn a_failed_client_is_forgotten_so_try_again_builds_it_again(cx: &mut TestAppContext) {
    cx.executor().allow_parking();
    let counting = std::sync::Arc::new(Forgetting::default());
    let source = ClusterSource {
        id: "live".into(),
        context: "prod-ams".into(),
        access: ClusterAccess::Live(counting.clone()),
    };
    let (_runtime, handle, page) = mount(cx, Some(source));
    show(cx, handle, &page);
    settled(cx, handle, &page).await;
    assert_eq!(cx.read(|cx| page.read(cx).connection_name()), "failed");
    assert_eq!(counting.counts(), (1, 1));

    cx.update_window(handle, |_, window, cx| window.click("monitoring-retry", cx))
        .unwrap();
    settled(cx, handle, &page).await;
    assert_eq!(cx.read(|cx| page.read(cx).connection_name()), "failed");
    assert_eq!(counting.counts(), (2, 2));
}

#[gpui_kit::test]
fn credentials_refused_show_as_refused(cx: &mut TestAppContext) {
    let (_runtime, handle, page) = mount(cx, Some(example_source()));
    show(cx, handle, &page);
    cx.update(|cx| {
        page.update(cx, |page, cx| {
            page.discovered(Err(QueryError::unauthorized()), cx)
        })
    });
    assert_eq!(cx.read(|cx| page.read(cx).connection_name()), "refused");
    assert!(shown(cx, handle, "monitoring-retry"));
    assert!(!shown(cx, handle, "monitoring-grid"));
}

async fn discovery_failure_is_shown(cx: &mut TestAppContext, code: u16, kind: ErrorKind) {
    use super::connection::Connection;
    use freshkube_core::monitoring::discover;
    cx.executor().allow_parking();
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let cluster = FakeCluster::new(fake_api_with_discovery(&runtime, code, true));
    let result = runtime.block_on(async {
        let client = cluster.client().await.unwrap();
        discover(&client, None).await
    });
    match result {
        Err(error) => {
            assert_eq!(code, 403);
            assert_eq!(error.kind, kind);
            assert_eq!(
                error.message,
                "Not allowed to query Prometheus (services/proxy)"
            );
        }
        Ok(Discovery::Missing { tried, .. }) => {
            assert_eq!(code, 404);
            assert_eq!(tried.len(), 1);
            assert_eq!(tried[0].error.kind, kind);
            assert_eq!(tried[0].error.message, "Not found: the proxy says no");
        }
        _ => panic!("a failed proxy must not discover a ready Prometheus"),
    }
    let source = ClusterSource {
        id: "cluster-a".into(),
        context: "cluster-a".into(),
        access: ClusterAccess::Live(cluster),
    };
    let (handle, page) = mount_with(cx, &runtime, Some(source), None, None);
    show(cx, handle, &page);
    settled(cx, handle, &page).await;
    cx.read(|cx| match &page.read(cx).connection {
        Connection::Refused(words) if code == 403 => assert_eq!(
            words.as_ref(),
            "Not allowed to query Prometheus (services/proxy)"
        ),
        Connection::Missing(missing) if code == 404 => assert_eq!(
            missing.tried.as_deref(),
            Some("monitoring/prometheus-operated:9090: Not found: the proxy says no")
        ),
        _ => panic!("discovery must retain a refusal or the failed candidate's words"),
    });
    assert!(shown(cx, handle, "monitoring-retry"));
    assert!(!shown(cx, handle, "monitoring-grid"));
    if code == 404 {
        assert!(shown(cx, handle, "monitoring-candidate-0"));
    }
}

#[gpui_kit::test]
async fn a_discovery_403_shows_the_refusal_words(cx: &mut TestAppContext) {
    discovery_failure_is_shown(cx, 403, ErrorKind::Refused).await;
}

#[gpui_kit::test]
async fn a_discovery_404_shows_the_failed_candidate_words(cx: &mut TestAppContext) {
    discovery_failure_is_shown(cx, 404, ErrorKind::NotFound).await;
}
