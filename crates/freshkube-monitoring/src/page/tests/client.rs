use super::*;

#[tokio::test]
async fn the_fake_client_gate_ignores_other_calls_before_and_after_the_rebuild() {
    use std::sync::atomic::Ordering::SeqCst;

    let cluster = FakeCluster::new("http://127.0.0.1:1".into());
    let (release, receive) = tokio::sync::oneshot::channel();
    *cluster.gate.lock().unwrap() = Some(ClientGate {
        after_forget: 1,
        receive,
    });
    // Ordinary reads before eviction must neither block nor take the gate.
    cluster.client().await.unwrap();
    cluster.client().await.unwrap();
    assert!(!cluster.gate_started.load(SeqCst));
    cluster.forget();
    let mut rebuild = cluster.client();
    assert!(!cluster.gate_started.load(SeqCst));
    assert!(futures::poll!(rebuild.as_mut()).is_pending());
    assert!(cluster.gate_started.load(SeqCst));
    // A concurrent marker read doesn't block on the held rebuild's call.
    cluster.client().await.unwrap();
    assert!(!release.is_closed());
    drop(rebuild);
    assert!(release.is_closed());
}

/// The page Ready on a Service of `cluster`, as discovery leaves it, with
/// the generation it was at.
async fn ready_on(
    cx: &mut TestAppContext,
    runtime: &tokio::runtime::Runtime,
    cluster: &std::sync::Arc<FakeCluster>,
) -> (AnyWindowHandle, Entity<MonitoringPage>, u64) {
    let source = ClusterSource {
        id: "live".into(),
        context: "prod-ams".into(),
        access: ClusterAccess::Live(cluster.clone()),
    };
    let (handle, page) = mount_with(cx, runtime, Some(source), None, None);
    show(cx, handle, &page);
    settled(cx, handle, &page).await;
    assert_eq!(cx.read(|cx| page.read(cx).connection_name()), "missing");
    let client = runtime.block_on(cluster.client()).unwrap();
    let service = PrometheusService::new("monitoring", "prometheus-operated", 9090);
    let found = Discovery::Found {
        prometheus: freshkube_core::monitoring::Prometheus::new(client, service),
        version: None,
        tried: Vec::new(),
    };
    let generation = cx.update(|cx| {
        page.update(cx, |page, cx| {
            let generation = page.generation;
            page.discovered(Ok(found), cx);
            generation
        })
    });
    (handle, page, generation)
}

/// Waits until every panel asked in this generation has answered, the
/// client isn't being built again, and `forgot` forgets were counted.
async fn panels_answered(
    cx: &mut TestAppContext,
    handle: AnyWindowHandle,
    page: &Entity<MonitoringPage>,
    cluster: &std::sync::Arc<FakeCluster>,
    forgot: usize,
) {
    let (observed, cluster) = (page.clone(), cluster.clone());
    cx.wait_for_io(handle, move |_, cx| {
        let page = observed.read(cx);
        let Some(board) = &page.board else {
            return false;
        };
        cluster.forgot() == forgot
            && page.rebuild_name() != "building"
            && board.resolving.is_none()
            && board
                .slots
                .iter()
                .any(|slot| slot.asked == Some(page.generation))
            && board.slots.iter().all(|slot| slot.request.is_none())
    })
    .await;
}

#[gpui_kit::test]
async fn panels_that_lose_the_cluster_build_its_client_again_once(cx: &mut TestAppContext) {
    cx.executor().allow_parking();
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let cluster = FakeCluster::new(fake_api(&runtime, 503));
    let (handle, page, generation) = ready_on(cx, &runtime, &cluster).await;

    // The panels fail through the proxy: the client is forgotten and built
    // again, and the variables and panels are read again through it, which
    // history takes too.
    panels_answered(cx, handle, &page, &cluster, 1).await;
    cx.read(|cx| {
        let page = page.read(cx);
        assert_eq!(page.rebuild_name(), "used");
        assert_eq!(page.generation, generation + 2);
        assert_eq!(page.history().unwrap().client, 1);
        // Every panel in reach failed, and they built it again only once.
        let failed = page
            .board
            .as_ref()
            .unwrap()
            .slots
            .iter()
            .filter(|slot| slot.failed);
        assert!(failed.count() >= 2);
    });

    // Failing again stays a failure, as on the next automatic read.
    cx.update(|cx| page.update(cx, |page, cx| page.ask_again(cx)));
    panels_answered(cx, handle, &page, &cluster, 1).await;
    assert_eq!(cx.read(|cx| page.read(cx).rebuild_name()), "used");
    assert_eq!(cluster.forgot(), 1);

    // Refresh lets the next failure build it again.
    cx.update(|cx| page.update(cx, |page, cx| page.refresh(cx)));
    panels_answered(cx, handle, &page, &cluster, 2).await;
    cx.read(|cx| {
        let page = page.read(cx);
        assert_eq!(page.rebuild_name(), "used");
        assert_eq!(page.history().unwrap().client, 2);
    });
}

#[gpui_kit::test]
async fn panels_refused_by_rbac_keep_the_client(cx: &mut TestAppContext) {
    cx.executor().allow_parking();
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let cluster = FakeCluster::new(fake_api(&runtime, 403));
    let (handle, page, generation) = ready_on(cx, &runtime, &cluster).await;

    panels_answered(cx, handle, &page, &cluster, 0).await;
    cx.read(|cx| {
        let page = page.read(cx);
        assert_eq!(page.rebuild_name(), "unused");
        assert_eq!(page.generation, generation + 1);
        assert_eq!(page.history().unwrap().client, 0);
        assert!(
            page.board
                .as_ref()
                .unwrap()
                .slots
                .iter()
                .any(|slot| slot.failed)
        );
    });
    assert_eq!(cluster.forgot(), 0);
}
