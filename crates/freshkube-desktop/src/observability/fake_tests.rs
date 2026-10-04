//! Exercises the real Tokio provider → GPUI publication path with sanitized HTTP.
use super::{Destination, ObservabilityPage, Report, Status, tests::mount};
use freshkube_core::coroot as api;
use gpui_kit::test::{TestAppContextExt, TestWindowExt};
use gpui_kit::{AppContext, TestAppContext};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicU16, AtomicUsize, Ordering},
};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

struct Server {
    url: String,
    status: Arc<AtomicU16>,
    requests: Arc<AtomicUsize>,
    paths: Arc<Mutex<Vec<String>>>,
    task: tokio::task::JoinHandle<()>,
}
impl Drop for Server {
    fn drop(&mut self) {
        self.task.abort();
    }
}
impl Server {
    fn new(runtime: &tokio::runtime::Runtime) -> Self {
        let _enter = runtime.enter();
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        listener.set_nonblocking(true).unwrap();
        let listener = tokio::net::TcpListener::from_std(listener).unwrap();
        let status = Arc::new(AtomicU16::new(200));
        let requests = Arc::new(AtomicUsize::new(0));
        let paths = Arc::new(Mutex::new(Vec::new()));
        let (state, reads, captured) = (status.clone(), requests.clone(), paths.clone());
        let task=runtime.spawn(async move {
            loop {
                let (mut socket,_)=listener.accept().await.unwrap();
                let (state,reads,captured)=(state.clone(),reads.clone(),captured.clone());
                tokio::spawn(async move {
                    let mut buf=vec![0;16384]; let Ok(n)=socket.read(&mut buf).await else{return;};
                    let request=String::from_utf8_lossy(&buf[..n]);
                    reads.fetch_add(1,Ordering::SeqCst);
                    let path=request.split_whitespace().nth(1).unwrap_or("");
                    captured.lock().unwrap().push(path.to_string());
                    let status=if path.contains("mcp") {404} else {state.load(Ordering::SeqCst)};
                    let body=if path.contains("api/user") {
                        serde_json::json!({"projects":[{"id":"p1","name":"Same name"},{"id":"p2","name":"Same name"}]})
                    } else if path.contains("/app/") {
                        serde_json::json!({"app_map":{"application":{"status":"warning"}},"reports":[{"name":"CPU","status":"unknown","checks":[{"id":"check","title":"Measured by Coroot","status":"unknown","message":"No measurement available"}]}]})
                    } else if path.contains("map") {
                        serde_json::json!({"map":[]})
                    } else if status==204 {
                        serde_json::json!({"applications":[]})
                    } else {
                        serde_json::json!({"applications":[{"id":"cluster-a:prod:Deployment:api","category":"Apps","status":"warning","cpu":{"status":"ok","value":""},"memory":{"status":"unknown","value":""}}]})
                    };
                    let body = if path.contains("api/user") {body} else {serde_json::json!({"context":{},"data":body})}.to_string();
                    let code=if status==204 {200} else {status};
                    let response=format!("HTTP/1.1 {code} Test\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",body.len());
                    _=socket.write_all(response.as_bytes()).await;
                });
            }
        });
        Self {
            url,
            status,
            requests,
            paths,
            task,
        }
    }
}

#[gpui_kit::test]
async fn connection_project_partial_reports_refusal_and_recovery(cx: &mut TestAppContext) {
    cx.executor().allow_parking();
    let (runtime, handle, page) = mount(cx, false);
    let server = Server::new(&runtime);
    cx.update_window(handle, |_, window, cx| {
        page.update(cx, |page, cx| {
            page.url.update(cx, |input, cx| {
                input.set_value(server.url.clone(), window, cx)
            });
            page.secret
                .update(cx, |input, cx| input.set_value("sanitized-key", window, cx));
            page.connect(cx);
        });
    })
    .unwrap();
    let observed = page.clone();
    cx.wait_for(handle, std::time::Duration::from_secs(5), move |_, cx| {
        !observed.read(cx).live.connecting
    })
    .await;
    cx.update(|cx| {
        page.update(cx, |page, cx| {
            assert!(page.live.provider.is_some(), "{:?}", page.live.error);
            assert!(
                page.live.source.is_none(),
                "never select a project implicitly"
            );
            let project = page.live.projects[1].clone();
            page.select_project(&project, cx);
        })
    });
    loaded(cx, handle, &page).await;
    cx.update(|cx| {
        page.update(cx, |page, cx| {
            assert_eq!(page.live.source.as_ref().unwrap().project(), "p2");
            let app = &page.applications[0];
            assert_eq!(app.check(Report::Cpu).status, Status::Ok);
            assert_eq!(app.check(Report::Memory).status, Status::Unknown);
            assert_eq!(app.check(Report::Logs).status, Status::Absent);
            page.open_app(app.id.clone(), Report::Cpu, cx);
        })
    });
    let observed = page.clone();
    cx.wait_for(handle, std::time::Duration::from_secs(5), move |_, cx| {
        let page = observed.read(cx);
        !page.live.rest.is_loading() && !page.live.extended.is_loading()
    })
    .await;
    cx.update_window(handle, |_, window, cx| {
        let current = page.read(cx);
        assert!(current.live.rest.data().is_some());
        assert!(current.live.extended.data().is_none());
        assert_eq!(
            current.live.capabilities[3],
            api::Capability::Unavailable(api::ReadError::Unsupported)
        );
        window.render_frame(cx);
        assert!(window.try_find("obs-threshold").is_none());
        assert!(window.try_find("obs-app-rollback").is_none());
        page.update(cx, |page, cx| page.open(Destination::Applications, cx));
    })
    .unwrap();
    loaded(cx, handle, &page).await;
    for status in [403, 500, 200, 204] {
        server.status.store(status, Ordering::SeqCst);
        cx.update(|cx| page.update(cx, |page, cx| page.refresh(cx)));
        loaded(cx, handle, &page).await;
        cx.update(|cx| {
            let page = page.read(cx);
            if matches!(status, 403 | 500) {
                assert!(page.live.apps.is_stale());
                assert_eq!(page.applications.len(), 1);
                let expected = if status == 403 {
                    api::ReadError::Refused
                } else {
                    api::ReadError::Failed
                };
                assert_eq!(
                    page.live.capabilities[0],
                    api::Capability::Unavailable(expected)
                );
            } else {
                assert!(!page.live.apps.is_stale());
                assert!(page.live.apps.error().is_none());
                assert_eq!(page.applications.len(), usize::from(status != 204));
            }
        });
    }
    let previous = cx.read(|cx| page.read(cx).live.range);
    cx.executor()
        .advance_clock(std::time::Duration::from_secs(3600));
    cx.update(|cx| page.update(cx, |page, cx| page.refresh_current(cx)));
    loaded(cx, handle, &page).await;
    let current = cx.read(|cx| page.read(cx).live.range);
    assert!(current.to > previous.to);
    let paths = server.paths.lock().unwrap();
    let last = paths.last().unwrap();
    assert!(last.contains("/project/p2/overview/applications"));
    assert!(last.contains(&format!(
        "from={}",
        current.from.unwrap().timestamp_millis()
    )));
    assert!(last.contains(&format!("to={}", current.to.unwrap().timestamp_millis())));
    drop(paths);
    cx.update(|cx| page.update(cx, |page, cx| page.set_visible(false, cx)));
    let reads = server.requests.load(Ordering::SeqCst);
    cx.update(|cx| {
        page.update(cx, |page, cx| {
            page.refresh(cx);
            page.connect(cx);
        })
    });
    assert_eq!(server.requests.load(Ordering::SeqCst), reads);
}
async fn loaded(
    cx: &mut TestAppContext,
    handle: gpui_kit::AnyWindowHandle,
    page: &gpui_kit::Entity<ObservabilityPage>,
) {
    let page = page.clone();
    cx.wait_for(handle, std::time::Duration::from_secs(5), move |_, cx| {
        !page.read(cx).live.apps.is_loading()
    })
    .await;
}

#[gpui_kit::test]
fn editing_credentials_invalidates_connection_and_object_links(cx: &mut TestAppContext) {
    let (_runtime, handle, page) = mount(cx, false);
    cx.update_window(handle, |_, window, cx| {
        page.update(cx, |page, _| {
            let provider =
                api::Provider::new("http://127.0.0.1:1", api::Credentials::None).unwrap();
            let source = provider
                .source(&api::ProjectInfo {
                    id: "p".into(),
                    name: "Project".into(),
                })
                .with_association(Some(api::Association::new(
                    "access:a".into(),
                    "cluster-a".into(),
                )));
            let app = api::AppId::new("cluster-a:prod:Deployment:api");
            let subject = api::ObjectSubject::for_app(&source, &app).unwrap();
            page.live.provider = Some(provider);
            page.live.source = Some(source.clone());
            page.live.access = Some("access:a".into());
            page.selected_app = Some(app.clone());
            assert!(page.link_is_current(&source, &app, &subject));
            page.live.visible = false;
            assert!(!page.link_is_current(&source, &app, &subject));
            page.live.visible = true;
            page.settings_open = true;
        });
        window.render_frame(cx);
        window.click("obs-credential", cx);
        window.input("replacement", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update(|cx| {
        let page = page.read(cx);
        assert!(page.live.provider.is_none());
        assert!(page.live.source.is_none());
        assert!(page.selected_app.is_none());
    });
}
