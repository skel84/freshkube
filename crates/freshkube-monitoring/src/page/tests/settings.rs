use super::*;

/// A Prometheus API on loopback that wants `Bearer s3cret` and answers
/// only the confirming query.
fn metrics_server(runtime: &tokio::runtime::Runtime) -> String {
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
                let head = String::from_utf8_lossy(&buffer[..read]).to_lowercase();
                let target = head.split_whitespace().nth(1).unwrap_or("").to_owned();
                let (status, body) = if !head.contains("authorization: bearer s3cret") {
                    (401, "unauthorized".to_owned())
                } else if target == "/vm/api/v1/query?query=1" {
                    (
                        200,
                        r#"{"status":"success","data":{"resultType":"scalar","result":[1,"1"]}}"#
                            .to_owned(),
                    )
                } else {
                    (404, "not found".to_owned())
                };
                let answer = format!(
                    "HTTP/1.1 {status} X\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
                    body.len()
                );
                let _ = socket.write_all(answer.as_bytes()).await;
            });
        }
    });
    format!("http://{address}/vm")
}

fn test_said(cx: &mut TestAppContext, page: &Entity<MonitoringPage>) -> Option<String> {
    cx.read(|cx| match &page.read(cx).form.as_ref()?.test {
        source::Test::Passed(message) => Some(format!("passed: {message}")),
        source::Test::Failed(message) => Some(format!("failed: {message}")),
        _ => None,
    })
}

async fn tested(cx: &mut TestAppContext, handle: AnyWindowHandle, page: &Entity<MonitoringPage>) {
    let observed = page.clone();
    cx.wait_for_io(handle, move |_, cx| {
        !matches!(
            observed.read(cx).form.as_ref().map(|form| &form.test),
            Some(source::Test::Testing(_))
        )
    })
    .await;
}

#[gpui_kit::test]
async fn a_url_chosen_in_settings_is_tested_saved_and_read_with_its_token_from_the_store(
    cx: &mut TestAppContext,
) {
    use freshkube_core::secrets::{MemoryStore, Secrets};
    cx.executor().allow_parking();
    let directory = std::env::temp_dir().join(format!(
        "freshkube-monitoring-source-{}-{}",
        std::process::id(),
        line!()
    ));
    let _ = std::fs::remove_dir_all(&directory);
    std::fs::create_dir_all(&directory).unwrap();
    let preferences = directory.join("preferences.json");
    let store = std::sync::Arc::new(MemoryStore::default());
    let secrets: Secrets = store.clone();
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let base = metrics_server(&runtime);
    let (handle, page) = mount_with(
        cx,
        &runtime,
        Some(live_source()),
        Some(&preferences),
        Some(secrets.clone()),
    );

    // Test checks the form as typed: a wrong token is refused.
    let fill = |cx: &mut TestAppContext, token: &'static str| {
        let url = base.clone();
        cx.update_window(handle, |_, window, cx| {
            page.update(cx, |page, cx| {
                page.source_form(window, cx);
                page.set_mode(source::Mode::Url, cx);
                let form = page.form.as_ref().unwrap();
                form.url
                    .update(cx, |input, cx| input.set_value(url, window, cx));
                form.token
                    .update(cx, |input, cx| input.set_value(token, window, cx));
                page.test_source(cx);
            })
        })
        .unwrap();
    };
    fill(cx, "wrong");
    tested(cx, handle, &page).await;
    let said = test_said(cx, &page).unwrap();
    assert!(said.starts_with("failed: The server refused"), "{said}");
    assert!(!said.contains("wrong"), "{said}");
    fill(cx, "s3cret");
    tested(cx, handle, &page).await;
    assert_eq!(
        test_said(cx, &page).unwrap(),
        format!("passed: Prometheus API answered at {base}")
    );
    // Testing saved nothing.
    assert!(store.keys.lock().unwrap().is_empty());
    cx.read(|cx| assert!(page.read(cx).saved.choices.is_empty()));

    // Save keeps the URL in monitoring.json and the token in the store,
    // then the page reads from it.
    cx.update(|cx| page.update(cx, |page, cx| page.set_visible(true, cx)));
    cx.update_window(handle, |_, window, cx| {
        page.update(cx, |page, cx| page.save_source(window, cx))
    })
    .unwrap();
    settled(cx, handle, &page).await;
    let url = format!("{base}/");
    cx.read(|cx| {
        let page = page.read(cx);
        assert_eq!(page.connection_name(), "ready");
        let history = page.history().unwrap();
        assert!(matches!(
            history.kind,
            crate::history::HistoryKind::Ready(ref prometheus)
                if prometheus.endpoint().label() == base
        ));
    });
    let account = crate::store::token_account(&url);
    cx.wait_for_io(handle, {
        let store = store.clone();
        let account = account.clone();
        move |_, _| store.keys.lock().unwrap().contains_key(&account)
    })
    .await;
    assert_eq!(
        store.keys.lock().unwrap().get(&account).map(String::as_str),
        Some("s3cret")
    );
    let file = directory.join("monitoring.json");
    cx.wait_for_io(handle, {
        let file = file.clone();
        move |_, _| std::fs::read_to_string(&file).is_ok_and(|text| text.contains("sources"))
    })
    .await;
    let written = std::fs::read_to_string(&file).unwrap();
    assert!(written.contains(&url), "{written}");
    assert!(!written.contains("s3cret"), "{written}");

    // The next launch reads the token from the store.
    let (relaunched_handle, relaunched) = mount_with(
        cx,
        &runtime,
        Some(live_source()),
        Some(&preferences),
        Some(secrets),
    );
    cx.update(|cx| relaunched.update(cx, |page, cx| page.set_visible(true, cx)));
    settled(cx, relaunched_handle, &relaunched).await;
    cx.read(|cx| assert_eq!(relaunched.read(cx).connection_name(), "ready"));

    // Forget drops the token, and the server refuses the page.
    cx.update_window(relaunched_handle, |_, window, cx| {
        relaunched.update(cx, |page, cx| {
            page.source_form(window, cx);
            page.forget_token(window, cx);
        })
    })
    .unwrap();
    settled(cx, relaunched_handle, &relaunched).await;
    cx.read(|cx| assert_eq!(relaunched.read(cx).connection_name(), "refused"));
    cx.wait_for_io(relaunched_handle, {
        let store = store.clone();
        move |_, _| store.keys.lock().unwrap().is_empty()
    })
    .await;
    assert!(store.keys.lock().unwrap().is_empty());
    std::fs::remove_dir_all(&directory).unwrap();
}

#[gpui_kit::test]
fn the_form_says_why_it_cannot_be_saved(cx: &mut TestAppContext) {
    let (_runtime, handle, page) = mount(cx, Some(live_source()));
    cx.update_window(handle, |_, window, cx| {
        page.update(cx, |page, cx| {
            page.source_form(window, cx);
            page.set_mode(source::Mode::Service, cx);
            page.form
                .as_ref()
                .unwrap()
                .namespace
                .update(cx, |input, cx| input.set_value("vm", window, cx));
            page.save_source(window, cx);
            assert!(page.saved.choices.is_empty());
            page.set_mode(source::Mode::Url, cx);
            page.form.as_ref().unwrap().url.update(cx, |input, cx| {
                input.set_value("https://u:p@vm.lan", window, cx)
            });
            page.save_source(window, cx);
            assert!(page.saved.choices.is_empty());
        })
    })
    .unwrap();
    // A chosen Service is the only one tried, and history reads it.
    let service = PrometheusService::new("vm", "vmselect", 8481).with_path("select/0/prometheus");
    cx.update(|cx| {
        page.update(cx, |page, cx| {
            page.saved.choices.insert(
                "prod-ams".into(),
                crate::store::Choice::Service(service.clone()),
            );
            page.saved.services.insert(
                "prod-ams".into(),
                PrometheusService::new("monitoring", "prometheus-operated", 9090),
            );
            page.set_source(None, cx);
            page.set_source(Some(live_source()), cx);
        })
    });
    cx.read(|cx| {
        assert!(matches!(
            page.read(cx).history().unwrap().kind,
            crate::history::HistoryKind::Remembered { service: ref s, .. } if *s == service
        ));
    });
}
