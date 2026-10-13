use super::*;

async fn logs_read(
    cx: &mut TestAppContext,
    handle: gpui_kit::AnyWindowHandle,
    page: &gpui_kit::Entity<ObservabilityPage>,
) {
    let page = page.clone();
    cx.wait_for(handle, std::time::Duration::from_secs(5), move |_, cx| {
        let page = page.read(cx);
        page.live.logs_job.is_some() && !page.live.logs.is_loading() && !page.live.view.is_loading()
    })
    .await;
}

#[gpui_kit::test]
async fn logs_are_read_then_refreshed_with_since_and_a_failure_keeps_them(cx: &mut TestAppContext) {
    cx.executor().allow_parking();
    let (runtime, handle, page) = mount(cx, false);
    let server = Server::new(&runtime);
    connected(cx, handle, &page, server.url.clone()).await;
    cx.update(|cx| {
        page.update(cx, |page, cx| {
            let app = page.applications[0].id.clone();
            page.open_app(app, Report::Logs, cx);
        })
    });
    logs_read(cx, handle, &page).await;
    let path = last_path(&server, "/logs");
    assert!(
        path.contains("/project/p1/app/cluster-a%3Aprod%3ADeployment%3Aapi/logs?"),
        "{path}"
    );
    assert!(path.contains("since%22%3A%22%22"), "{path}");
    assert_eq!(retained(cx, &page).len(), 2);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        for id in ["obs-logs-histogram", "obs-logs-list", "obs-logs-limit"] {
            assert!(window.try_find(id).is_some(), "{id}");
        }
        assert!(window.try_find("obs-logs-failed").is_none());
    })
    .unwrap();
    // A refresh asks from the newest message's nanosecond and adds only
    // what is new.
    cx.update(|cx| page.update(cx, |page, cx| page.refresh_current(cx)));
    logs_read(cx, handle, &page).await;
    // One read a window, though the page asks again when the view
    // answers; the second asks from the newest message's nanosecond.
    let paths: Vec<String> = server.paths.lock().unwrap().clone();
    let reads: Vec<&String> = paths.iter().filter(|p| p.contains("/logs?")).collect();
    assert_eq!(reads.len(), 2, "{reads:#?}");
    assert!(
        reads[1].contains("since%22%3A%221789999000999999999%22"),
        "{reads:#?}"
    );
    let lines = retained(cx, &page);
    assert_eq!(lines.len(), 3, "{lines:#?}");
    assert!(lines[2].ends_with(" third"), "{lines:#?}");
    // A failed read keeps the lines and offers Retry.
    server.logs_status.store(500, Ordering::SeqCst);
    cx.update(|cx| page.update(cx, |page, cx| page.refresh_current(cx)));
    // The banner itself: the page keeps the report away while it reads its
    // applications again, so the failure can be in before there is a banner.
    cx.wait_for(handle, std::time::Duration::from_secs(5), |window, _| {
        window.try_find("obs-logs-failed").is_some()
    })
    .await;
    assert_eq!(retained(cx, &page).len(), 3);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let banner = window.find("obs-logs-failed");
        assert!(
            banner
                .label()
                .is_some_and(|l| l.contains("from the last read")),
            "{:?}",
            banner.label()
        );
        server.logs_status.store(200, Ordering::SeqCst);
        window.click("obs-logs-retry", cx);
    })
    .unwrap();
    let observed = page.clone();
    cx.wait_for(handle, std::time::Duration::from_secs(5), move |_, cx| {
        observed.read(cx).live_logs.failure().is_none()
    })
    .await;
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(window.try_find("obs-logs-failed").is_none());
    })
    .unwrap();
    assert_eq!(retained(cx, &page).len(), 3);
}

#[gpui_kit::test]
async fn a_first_read_that_fails_says_so_with_retry(cx: &mut TestAppContext) {
    cx.executor().allow_parking();
    let (runtime, handle, page) = mount(cx, false);
    let server = Server::new(&runtime);
    connected(cx, handle, &page, server.url.clone()).await;
    for status in [403, 500] {
        server.logs_status.store(status, Ordering::SeqCst);
        cx.update(|cx| {
            page.update(cx, |page, cx| {
                let app = page.applications[0].id.clone();
                page.live_logs.forget();
                page.open_app(app, Report::Logs, cx);
            })
        });
        let observed = page.clone();
        cx.wait_for(handle, std::time::Duration::from_secs(5), move |_, cx| {
            let page = observed.read(cx);
            !page.live.view.is_loading()
                && !page.live.logs.is_loading()
                && page.live_logs.failure().is_some()
        })
        .await;
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            let banner = window.find("obs-logs-failed");
            let label = banner.label().unwrap_or_default();
            assert!(!label.contains("last read"), "{status}: {label}");
            if status == 403 {
                assert!(label.contains("refused"), "{label}");
            }
            assert!(window.try_find("obs-logs-list").is_none());
        })
        .unwrap();
    }
}
