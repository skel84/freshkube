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
    incident_mode: Arc<AtomicUsize>,
    incident_gate: Arc<tokio::sync::Notify>,
    /// The status the logs path answers with while the rest answer 200.
    logs_status: Arc<AtomicU16>,
    /// 1: the application's page lists two revisions; 2: also, the window
    /// around one answers Coroot's 404; 3: the window answers 500; 4: the
    /// newest revision's ±30 min window waits for `window_gate`; 5: the
    /// window answers 403.
    deploy_mode: Arc<AtomicUsize>,
    window_gate: Arc<tokio::sync::Notify>,
    /// The paths answered, once the answer is written.
    responded: Arc<Mutex<Vec<String>>>,
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
        let incident_mode = Arc::new(AtomicUsize::new(0));
        let incident_gate = Arc::new(tokio::sync::Notify::new());
        let (mode, gate) = (incident_mode.clone(), incident_gate.clone());
        let logs_status = Arc::new(AtomicU16::new(200));
        let logs_state = logs_status.clone();
        let deploy_mode = Arc::new(AtomicUsize::new(0));
        let deploys = deploy_mode.clone();
        let window_gate = Arc::new(tokio::sync::Notify::new());
        let held = window_gate.clone();
        let responded = Arc::new(Mutex::new(Vec::new()));
        let written = responded.clone();
        let task=runtime.spawn(async move {
            loop {
                let (mut socket,_)=listener.accept().await.unwrap();
                let (state,reads,captured,mode,gate,logs_state,deploys,held,written)=(state.clone(),reads.clone(),captured.clone(),mode.clone(),gate.clone(),logs_state.clone(),deploys.clone(),held.clone(),written.clone());
                tokio::spawn(async move {
                    let mut buf=vec![0;16384]; let Ok(n)=socket.read(&mut buf).await else{return;};
                    let request=String::from_utf8_lossy(&buf[..n]);
                    reads.fetch_add(1,Ordering::SeqCst);
                    let path=request.split_whitespace().nth(1).unwrap_or("");
                    captured.lock().unwrap().push(path.to_string());
                    // Like Coroot, any API key but the right one, exactly, is refused.
                    let key=request.lines().find_map(|line| line.strip_prefix("authorization: ").or_else(|| line.strip_prefix("Authorization: ")));
                    let status=if key.is_some_and(|key| key!="Bearer sanitized-key") {401} else if path.contains("mcp") {404} else {state.load(Ordering::SeqCst)};
                    let status=if path.contains("/logs") && status==200 {logs_state.load(Ordering::SeqCst)} else {status};
                    let selected_mode=mode.load(Ordering::SeqCst);
                    if path.contains("/incident/k1") && selected_mode==5 {gate.notified().await;}
                    if path.contains(NEWEST_WINDOW) && deploys.load(Ordering::SeqCst)==4 {held.notified().await;}
                    let body=if path.contains("/incidents") {
                        let a=incident_wire("k1"); let b=incident_wire("k2");
                        match selected_mode {
                            1=>serde_json::json!([b,a]),2=>serde_json::json!([]),3=>serde_json::json!({}),4=>serde_json::json!([b]),
                            _=>serde_json::json!([a,b]),
                        }
                    } else if path.contains("/incident/") {
                        let mut v=incident_wire(if path.contains("/incident/k2"){"k2"}else{"k1"});
                        v["availability_slo"]=serde_json::json!({"objective":"99% of requests should not fail","compliance":"97.2%","violated":true,"threshold":0});
                        v["details"]=serde_json::json!({"availability_burn_rates":[{"severity":"critical","long_window":3600000,"short_window":300000,"long_window_burn_rate":18.4,"short_window_burn_rate":21,"threshold":6}]});
                        v["rca"]=serde_json::json!({"status":"OK","root_cause":"Source-reported cause","propagation_map":{"applications":[{"id":"cluster-a:prod:Deployment:worker","status":"critical","issues":["Reported problem"]}]}});
                        v
                    } else if path.contains("api/user") {
                        serde_json::json!({"projects":[{"id":"p1","name":"Same name"},{"id":"p2","name":"Same name"}]})
                    } else if path.contains("/tracing") {
                        tracing_wire(path)
                    } else if path.contains("/profiling") {
                        profiling_wire(path)
                    } else if path.contains("/logs") {
                        logs_wire(path)
                    } else if path.contains("/app/") {
                        // The application the path names, as Coroot answers for it.
                        let id=path.split("/app/").nth(1).unwrap_or("").split('?').next().unwrap_or("").replace("%3A",":");
                        let mut page=serde_json::json!({"app_map":{"application":{"id":id,"status":"warning"},"dependencies":[{"id":"cluster-a:prod:Deployment:db","status":"ok","link":{"status":"critical"}}]},"reports":[{"name":"CPU","status":"unknown","checks":[{"id":"check","title":"Measured by Coroot","status":"unknown","message":"No measurement available","threshold":80,"unit":"percent","condition_format_template":"the CPU usage > <threshold>"}],"widgets":[{"table":{"header":["Container","Usage"],"rows":[{"cells":[{"value":"api"},{"value":"0.2","unit":"cores"}]}]},"width":"100%"}]},{"name":"Logs","status":"ok","checks":[],"widgets":[{"logs":{},"width":"100%"}]}]});
                        if deploys.load(Ordering::SeqCst)>0 {
                            page["reports"].as_array_mut().unwrap().push(deployments_wire());
                        }
                        page
                    } else if path.contains("map") {
                        serde_json::json!({"map":[]})
                    } else if status==204 {
                        serde_json::json!({"applications":[]})
                    } else {
                        serde_json::json!({"applications":[{"id":"cluster-a:prod:Deployment:api","category":"application","status":"warning","cpu":{"status":"ok","value":""},"memory":{"status":"unknown","value":""}}]})
                    };
                    let body = if path.contains("api/user") {body} else {serde_json::json!({"context":{},"data":body})}.to_string();
                    let code=if status==204 {200} else {status};
                    // The window around the revision at 1789999000 s.
                    let window=path.contains("/app/") && path.contains("from=17899");
                    let (code,body,kind)=match deploys.load(Ordering::SeqCst) {
                        2 if window=>(404,"Application not found\n".to_owned(),"text/plain"),
                        3 if window=>(500,"internal error".to_owned(),"text/plain"),
                        5 if window=>(403,"forbidden".to_owned(),"text/plain"),
                        _=>(code,body,"application/json"),
                    };
                    let response=format!("HTTP/1.1 {code} Test\r\nContent-Type: {kind}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",body.len());
                    _=socket.write_all(response.as_bytes()).await;
                    written.lock().unwrap().push(path.to_string());
                });
            }
        });
        Self {
            url,
            status,
            requests,
            paths,
            incident_mode,
            incident_gate,
            logs_status,
            deploy_mode,
            window_gate,
            responded,
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
        !page.live.view.is_loading()
    })
    .await;
    cx.update_window(handle, |_, window, cx| {
        let current = page.read(cx);
        assert!(current.live.view.data().is_some());
        // The page shows Coroot's view alone: one read of the application,
        // and no report from the REST API or MCP beside it.
        let paths = server.paths.lock().unwrap();
        let app_reads = paths.iter().filter(|path| {
            let path = path.split('?').next().unwrap_or_default();
            path.rsplit_once("/app/")
                .is_some_and(|(_, id)| !id.contains('/'))
        });
        assert_eq!(app_reads.count(), 1, "{paths:?}");
        assert!(paths.iter().all(|path| !path.contains("mcp")), "{paths:?}");
        drop(paths);
        window.render_frame(cx);
        assert!(window.try_find("obs-threshold").is_none());
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
async fn a_connected_page_keeps_connection_settings_behind_the_sidebar_action(
    cx: &mut TestAppContext,
) {
    cx.executor().allow_parking();
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
            page.live.provider = Some(provider);
            page.live.source = Some(source);
            page.live.access = Some("access:a".into());
            page.settings_open = false;
        });
        window.render_frame(cx);
        assert!(window.try_find("obs-refresh").is_some());
        assert!(window.try_find("obs-disconnect").is_none());
        assert!(window.try_find("obs-unmap").is_none());
        assert!(window.try_find("obs-connect-settings").is_none());
        page.update(cx, |page, cx| page.show_connection(cx));
        window.render_frame(cx);
        assert!(window.try_find("obs-disconnect").is_some());
        assert!(window.try_find("obs-unmap").is_some());
        window.click("obs-connect-settings", cx);
        window.render_frame(cx);
        assert!(window.try_find("obs-disconnect").is_none());
    })
    .unwrap();
}

#[gpui_kit::test]
async fn editing_credentials_invalidates_connection_and_object_links(cx: &mut TestAppContext) {
    cx.executor().allow_parking();
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
        assert_eq!(page.secret.read(cx).value(), "replacement");
    });
    cx.update_window(handle, |_, window, cx| {
        window.click("obs-disconnect", cx);
        assert!(page.read(cx).secret.read(cx).value().is_empty());
        window.click("obs-credential", cx);
        window.input("retained-after-failure", cx);
        window.click("obs-url", cx);
        window.input("not-a-url", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| window.click("obs-connect", cx))
        .unwrap();
    let observed = page.clone();
    cx.wait_for(handle, std::time::Duration::from_secs(5), move |_, cx| {
        observed.read(cx).live.error.is_some()
    })
    .await;
    cx.update_window(handle, |_, window, cx| {
        assert!(page.read(cx).live.provider.is_none());
        assert_eq!(
            page.read(cx).secret.read(cx).value(),
            "retained-after-failure"
        );
        window.click("obs-disconnect", cx);
        assert!(page.read(cx).secret.read(cx).value().is_empty());
        assert!(page.read(cx).live.error.is_none());
        window.click("obs-credential", cx);
        window.input("pending-credential", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        // Hold the same owned delivery path used by initial connection reads.
        // No provider exists yet, and the request must remain cancellable.
        page.update(cx, |page, cx| {
            page.live.connecting = true;
            page.spawn_read(
                std::future::pending::<Result<(), api::ReadError>>(),
                |_, _, _| {
                    panic!("cancelled connection must not deliver");
                },
                cx,
            );
        });
        let generation = page.read(cx).live.generation;
        assert_eq!(page.read(cx).live.jobs.len(), 1);
        window.click("obs-disconnect", cx);
        let current = page.read(cx);
        assert!(current.secret.read(cx).value().is_empty());
        assert!(!current.live.connecting);
        assert!(current.live.jobs.is_empty());
        assert!(current.live.generation > generation);
    })
    .unwrap();
}

fn incident_wire(key: &str) -> serde_json::Value {
    serde_json::json!({"key":key,"application_id":"cluster-a:prod:Deployment:api","cluster":"Production","severity":"critical","opened_at":1790000000000_i64,"resolved_at":null,"impact":2.8,"duration":120000,"short_description":"Source SLO violation"})
}
#[gpui_kit::test]
async fn incidents_keep_identity_stale_evidence_and_read_only_links(cx: &mut TestAppContext) {
    cx.executor().allow_parking();
    let (runtime, handle, page) = mount(cx, false);
    let server = Server::new(&runtime);
    cx.update(|cx| {
        page.update(cx, |page, cx| {
            let provider = api::Provider::new(&server.url, api::Credentials::None).unwrap();
            page.live.source = Some(provider.source(&api::ProjectInfo {
                id: "p1".into(),
                name: "Production".into(),
            }));
            page.live.provider = Some(provider);
            page.open(Destination::Incidents, cx);
        })
    });
    incident_loaded(cx, handle, &page).await;
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(
            page.read(cx).live.incident.data().unwrap().incident().key,
            "k1"
        );
        assert_eq!(page.read(cx).incident_count(), Some("2 sampled"));
        for id in [
            "obs-incident-mute",
            "obs-incident-fix",
            "obs-incident-traces",
        ] {
            assert!(window.try_find(id).is_none());
        }
        window.click("obs-live-incident-k2", cx);
    })
    .unwrap();
    incident_loaded(cx, handle, &page).await;
    server.incident_mode.store(1, Ordering::SeqCst);
    cx.update(|cx| page.update(cx, |page, cx| page.refresh(cx)));
    incident_loaded(cx, handle, &page).await;
    cx.update(|cx| {
        assert_eq!(
            page.read(cx).live.incident.data().unwrap().incident().key,
            "k2"
        )
    });
    server.status.store(403, Ordering::SeqCst);
    cx.update(|cx| page.update(cx, |page, cx| page.refresh(cx)));
    incident_loaded(cx, handle, &page).await;
    cx.update_window(handle, |_, window, cx| {
        assert!(page.read(cx).live.incidents.is_stale());
        assert!(page.read(cx).live.incident.is_stale());
        window.render_frame(cx);
        window.click("obs-incident-primary-app", cx);
        assert_eq!(
            page.read(cx).destination,
            Destination::Incidents,
            "stale source cannot navigate"
        );
    })
    .unwrap();
    server.status.store(200, Ordering::SeqCst);
    cx.update_window(handle, |_, window, cx| {
        window.click("obs-incident-retry", cx)
    })
    .unwrap();
    incident_loaded(cx, handle, &page).await;
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        for _ in 0..20 {
            // Wholly inside the scroll area, so the click lands on it.
            let link = window.find("obs-incident-app-cluster-a:prod:Deployment:worker");
            if link.visible()
                && link.bounds().bottom() <= window.find("obs-scroll").bounds().bottom()
            {
                break;
            }
            window.scroll(
                "obs-scroll",
                gpui_kit::ScrollDelta::Pixels(gpui_kit::point(
                    gpui_kit::px(0.),
                    gpui_kit::px(-100.),
                )),
                cx,
            );
        }
        window.click("obs-incident-app-cluster-a:prod:Deployment:worker", cx);
        assert_eq!(
            page.read(cx).selected_app.as_ref().unwrap().as_str(),
            "cluster-a:prod:Deployment:worker"
        );
        assert_eq!(page.read(cx).destination, Destination::Application);
    })
    .unwrap();
    server.incident_mode.store(2, Ordering::SeqCst);
    cx.update(|cx| page.update(cx, |page, cx| page.open(Destination::Incidents, cx)));
    incident_loaded(cx, handle, &page).await;
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(window.find("obs-incidents-empty").visible());
        assert!(page.read(cx).live.incident.data().is_none());
    })
    .unwrap();
}
async fn incident_loaded(
    cx: &mut TestAppContext,
    handle: gpui_kit::AnyWindowHandle,
    page: &gpui_kit::Entity<ObservabilityPage>,
) {
    let observed = page.clone();
    cx.wait_for(handle, std::time::Duration::from_secs(5), move |_, cx| {
        let live = &observed.read(cx).live;
        !live.incidents.is_loading() && !live.incident.is_loading()
    })
    .await;
}
#[gpui_kit::test]
async fn incidents_drop_delayed_replaced_hidden_and_project_answers(cx: &mut TestAppContext) {
    cx.executor().allow_parking();
    let (runtime, handle, page) = mount(cx, false);
    let server = Server::new(&runtime);
    server.incident_mode.store(5, Ordering::SeqCst);
    cx.update(|cx| {
        page.update(cx, |page, cx| {
            let provider = api::Provider::new(&server.url, api::Credentials::None).unwrap();
            page.live.source = Some(provider.source(&api::ProjectInfo {
                id: "p1".into(),
                name: "Production".into(),
            }));
            page.live.provider = Some(provider);
            page.open(Destination::Incidents, cx);
        })
    });
    let paths = server.paths.clone();
    cx.wait_for(handle, std::time::Duration::from_secs(5), move |_, _| {
        paths
            .lock()
            .unwrap()
            .iter()
            .any(|p| p.contains("/incident/k1"))
    })
    .await;
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("obs-live-incident-k2", cx);
    })
    .unwrap();
    incident_loaded(cx, handle, &page).await;
    server.incident_gate.notify_one();
    cx.run_until_parked();
    cx.update(|cx| {
        assert_eq!(
            page.read(cx).live.incident.data().unwrap().incident().key,
            "k2"
        )
    });
    let previous = server
        .paths
        .lock()
        .unwrap()
        .iter()
        .filter(|p| p.contains("/incident/k1"))
        .count();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("obs-live-incident-k1", cx);
    })
    .unwrap();
    let paths = server.paths.clone();
    cx.wait_for(handle, std::time::Duration::from_secs(5), move |_, _| {
        paths
            .lock()
            .unwrap()
            .iter()
            .filter(|p| p.contains("/incident/k1"))
            .count()
            > previous
    })
    .await;
    cx.update(|cx| {
        page.update(cx, |page, cx| {
            page.set_visible(false, cx);
            assert!(page.live.jobs.is_empty());
            assert!(page.live.incident_job.is_none());
        })
    });
    let reads = server.requests.load(Ordering::SeqCst);
    cx.update(|cx| page.update(cx, |page, cx| page.refresh(cx)));
    cx.run_until_parked();
    assert_eq!(server.requests.load(Ordering::SeqCst), reads);
    server.incident_gate.notify_one();
    cx.run_until_parked();
    cx.update(|cx| {
        page.update(cx, |page, cx| {
            page.select_project(
                &api::ProjectInfo {
                    id: "p2".into(),
                    name: "Production".into(),
                },
                cx,
            );
            assert!(page.live.incidents.data().is_none());
            assert!(page.live.incident.data().is_none());
            assert_eq!(page.incident_count(), None);
        })
    });
}

#[gpui_kit::test]
async fn a_remembered_connection_returns_on_the_next_launch_with_its_key_in_the_store(
    cx: &mut TestAppContext,
) {
    use super::remember::account;
    use crate::secrets::MemoryStore;
    cx.executor().allow_parking();
    let directory = tempfile::tempdir().unwrap();
    let preferences = directory.path().join("preferences.json");
    let file = directory.path().join("coroot.json");
    let store = Arc::new(MemoryStore::default());
    let (runtime, handle, page) = super::tests::mount_remembering(cx, &preferences, store.clone());
    let server = Server::new(&runtime);
    let key_account = account(&server.url);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("obs-remember", cx);
        page.update(cx, |page, cx| {
            assert!(page.memory.as_ref().unwrap().saved.remember);
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
        observed.read(cx).live.provider.is_some()
    })
    .await;
    cx.update(|cx| {
        page.update(cx, |page, cx| {
            let project = page.live.projects[1].clone();
            page.select_project(&project, cx);
        })
    });
    let (saved, account_name) = (store.clone(), key_account.clone());
    let path = file.clone();
    cx.wait_for(handle, std::time::Duration::from_secs(5), move |_, _| {
        saved.keys.lock().unwrap().contains_key(&account_name)
            && std::fs::read_to_string(&path).is_ok_and(|text| text.contains("p2"))
    })
    .await;
    let text = std::fs::read_to_string(&file).unwrap();
    assert!(
        !text.contains("sanitized-key"),
        "the key never reaches the file"
    );
    assert_eq!(
        store
            .keys
            .lock()
            .unwrap()
            .get(&key_account)
            .map(String::as_str),
        Some("sanitized-key")
    );

    // The next launch connects again with the stored key and opens the project.
    let (_second_runtime, second, reopened) =
        super::tests::mount_remembering(cx, &preferences, store.clone());
    let observed = reopened.clone();
    cx.wait_for(second, std::time::Duration::from_secs(5), move |_, cx| {
        observed
            .read(cx)
            .live
            .source
            .as_ref()
            .is_some_and(|source| source.project() == "p2")
    })
    .await;
    cx.update(|cx| {
        let page = reopened.read(cx);
        assert_eq!(page.url.read(cx).value(), server.url.as_str());
        assert!(
            page.secret.read(cx).value().is_empty(),
            "the stored key is never put in the field"
        );
    });

    // Disconnect forgets the key and stops reconnecting.
    cx.update_window(second, |_, window, cx| {
        reopened.update(cx, |page, cx| page.disconnect(window, cx))
    })
    .unwrap();
    let (saved, account_name, path) = (store.clone(), key_account.clone(), file.clone());
    cx.wait_for(second, std::time::Duration::from_secs(5), move |_, _| {
        !saved.keys.lock().unwrap().contains_key(&account_name)
            && std::fs::read_to_string(&path)
                .is_ok_and(|text| text.contains("\"reconnect\": false"))
    })
    .await;

    // A store that can't save turns Remember off and says so.
    store
        .unavailable
        .store(true, std::sync::atomic::Ordering::SeqCst);
    cx.update_window(second, |_, window, cx| {
        reopened.update(cx, |page, cx| {
            page.secret
                .update(cx, |input, cx| input.set_value("sanitized-key", window, cx));
            page.connect(cx);
        })
    })
    .unwrap();
    let observed = reopened.clone();
    cx.wait_for(second, std::time::Duration::from_secs(5), move |_, cx| {
        observed
            .read(cx)
            .memory
            .as_ref()
            .is_some_and(|memory| !memory.saved.remember && memory.error.is_some())
    })
    .await;
    cx.update(|cx| {
        let error = reopened
            .read(cx)
            .memory
            .as_ref()
            .unwrap()
            .error
            .clone()
            .unwrap();
        assert!(error.contains("stays in memory"), "{error}");
        assert!(!error.contains("sanitized-key"));
    });
}

#[gpui_kit::test]
async fn a_pasted_key_is_trimmed_and_a_refused_one_says_which_key_coroot_wants(
    cx: &mut TestAppContext,
) {
    cx.executor().allow_parking();
    let (runtime, handle, page) = mount(cx, false);
    let server = Server::new(&runtime);
    let connect = |cx: &mut TestAppContext, url: String, key: &'static str| {
        cx.update_window(handle, |_, window, cx| {
            page.update(cx, |page, cx| {
                page.url
                    .update(cx, |input, cx| input.set_value(url, window, cx));
                page.secret
                    .update(cx, |input, cx| input.set_value(key, window, cx));
                page.connect(cx);
            })
        })
        .unwrap();
    };
    connect(cx, server.url.clone(), "crt_wrong");
    settled(cx, handle, &page).await;
    cx.update(|cx| {
        let page = page.read(cx);
        assert!(page.live.provider.is_none());
        let error = page.live.error.clone().unwrap();
        assert!(error.contains("user API key"), "{error}");
        assert!(!error.contains("crt_wrong"), "{error}");
    });

    // Spaces around a pasted URL and key are not part of them.
    connect(cx, format!("  {}  ", server.url), "  sanitized-key  ");
    settled(cx, handle, &page).await;
    cx.update(|cx| {
        let page = page.read(cx);
        assert!(page.live.provider.is_some(), "{:?}", page.live.error);
    });
}

async fn settled(
    cx: &mut TestAppContext,
    handle: gpui_kit::AnyWindowHandle,
    page: &gpui_kit::Entity<ObservabilityPage>,
) {
    let observed = page.clone();
    cx.wait_for(handle, std::time::Duration::from_secs(5), move |_, cx| {
        !observed.read(cx).live.connecting
    })
    .await;
}

fn span_wire(id: &str, parent: &str, at: i64, ms: f64, error: bool) -> serde_json::Value {
    serde_json::json!({"service":"api","trace_id":"t1","id":id,"parent_id":parent,
        "name":format!("op {id}"),"timestamp":at,"duration":ms,"client":"web",
        "status":{"error":error,"message":if error {"HTTP 503"} else {""}},
        "details":{"text":"","lang":""},"attributes":{"http.route":"/cart"},"events":null})
}
/// A healthy request in another trace, listed after the failed one.
fn other_trace(at: i64) -> serde_json::Value {
    let mut span = span_wire("other", "", at - 60_000, 40., false);
    span["trace_id"] = "t2".into();
    span
}
fn tracing_wire(path: &str) -> serde_json::Value {
    let at = 1_789_999_000_000_i64;
    let sources = serde_json::json!([
        {"type":"otel","name":"OpenTelemetry","selected":!path.contains("agent")},
        {"type":"agent","name":"OpenTelemetry (eBPF)","selected":path.contains("agent")}]);
    if path.contains("t1%3A%3A") {
        return serde_json::json!({"status":"ok","message":"","sources":sources,"heatmap":null,
            "spans":[span_wire("root","",at,100.,false),span_wire("child","root",at+40,20.,true)],"limit":0});
    }
    serde_json::json!({"status":"ok","message":"Using traces of <i>api</i>","sources":sources,
        "heatmap":{"ctx":{"from":at-3_600_000,"to":at,"step":60_000},"title":"",
            "series":[
                {"name":"5ms","value":"0.005","data":[1.5,null,2]},
                {"name":">5s","value":"inf","data":[null,null,0.1]},
                {"name":"errors","value":"err","data":[0,0.2,null]}],"annotations":null},
        "spans":[span_wire("root","",at,100.,true),other_trace(at)],"limit":0})
}
fn profiling_wire(path: &str) -> serde_json::Value {
    let diff = path.contains("diff");
    serde_json::json!({"status":"ok","message":"OK","services":[],
        "profiles":[{"type":"go:profile_cpu:nanoseconds","name":"CPU"},{"type":"go:heap_inuse_space:bytes","name":"Memory"}],
        "profile":{"type":"go:profile_cpu:nanoseconds","diff":diff,"flamegraph":
            {"name":"total","total":200,"self":0,"comp":100,"children":[
                {"name":"gc","total":50,"self":50,"comp":10,"children":null},
                {"name":"main","total":150,"self":50,"comp":90,"children":[
                    {"name":"handle","total":100,"self":100,"comp":70,"children":[]}]}]}},
        "chart":null,"instances":["api-0","api-1"]})
}

async fn connected(
    cx: &mut TestAppContext,
    handle: gpui_kit::AnyWindowHandle,
    page: &gpui_kit::Entity<ObservabilityPage>,
    url: String,
) {
    cx.update_window(handle, |_, window, cx| {
        page.update(cx, |page, cx| {
            page.url
                .update(cx, |input, cx| input.set_value(url, window, cx));
            page.secret
                .update(cx, |input, cx| input.set_value("sanitized-key", window, cx));
            page.connect(cx);
        })
    })
    .unwrap();
    settled(cx, handle, page).await;
    cx.update(|cx| {
        page.update(cx, |page, cx| {
            let project = page.live.projects[0].clone();
            page.select_project(&project, cx);
        })
    });
    loaded(cx, handle, page).await;
}

async fn traced(
    cx: &mut TestAppContext,
    handle: gpui_kit::AnyWindowHandle,
    page: &gpui_kit::Entity<ObservabilityPage>,
) {
    let page = page.clone();
    cx.wait_for(handle, std::time::Duration::from_secs(5), move |_, cx| {
        let live = &page.read(cx).live;
        !live.tracing.is_loading() && !live.trace.is_loading()
    })
    .await;
}

fn last_path(server: &Server, part: &str) -> String {
    server
        .paths
        .lock()
        .unwrap()
        .iter()
        .rev()
        // The selected trace is read after its list; skip it.
        .find(|path| path.contains(part) && !path.contains("t1%3A%3A"))
        .cloned()
        .unwrap_or_default()
}

#[gpui_kit::test]
async fn traces_list_requests_open_a_trace_and_narrow_to_a_cell(cx: &mut TestAppContext) {
    cx.executor().allow_parking();
    let (runtime, handle, page) = mount(cx, false);
    let server = Server::new(&runtime);
    connected(cx, handle, &page, server.url.clone()).await;
    cx.update_window(handle, |_, window, cx| {
        page.update(cx, |page, cx| {
            let app = page.applications[0].id.clone();
            page.open_app(app, Report::Cpu, cx);
        });
        window.render_frame(cx);
        window.click("obs-app-traces", cx);
    })
    .unwrap();
    traced(cx, handle, &page).await;
    cx.update_window(handle, |_, window, cx| {
        let current = page.read(cx);
        assert_eq!(current.destination, Destination::Traces);
        assert_eq!(current.live.trace.data().unwrap().spans.len(), 2);
        window.render_frame(cx);
        // Failed requests first, then slowest: errors, >5s, 5ms.
        assert!(window.try_find("obs-live-bucket-2-2").is_some());
        assert!(window.try_find("obs-live-span-t1-root").is_some());
        assert!(window.try_find("obs-live-trace-span-1").is_some());
        assert!(window.try_find("obs-live-trace-span-2").is_none());
        window.click("obs-live-trace-span-1", cx);
        window.render_frame(cx);
        assert!(window.try_find("obs-live-span-detail").is_some());
        window.click("obs-live-bucket-1-0", cx);
    })
    .unwrap();
    traced(cx, handle, &page).await;
    let path = last_path(&server, "/tracing");
    assert!(path.contains("0.005-inf"), "{path}");
    assert!(!path.contains("t1%3A%3A"), "{path}");

    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("obs-trace-failed", cx);
    })
    .unwrap();
    traced(cx, handle, &page).await;
    let path = last_path(&server, "/tracing");
    assert!(path.contains("inf-err"), "{path}");

    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("obs-trace-source-agent", cx);
    })
    .unwrap();
    traced(cx, handle, &page).await;
    assert!(last_path(&server, "/tracing").contains("trace=agent%3A"));

    // Hidden, the page asks nothing more.
    cx.update(|cx| page.update(cx, |page, cx| page.set_visible(false, cx)));
    let reads = server.requests.load(Ordering::SeqCst);
    cx.update(|cx| page.update(cx, |page, cx| page.refresh(cx)));
    cx.run_until_parked();
    assert_eq!(server.requests.load(Ordering::SeqCst), reads);
}

#[gpui_kit::test]
async fn profiling_draws_a_flame_graph_and_compares_with_the_window_before(
    cx: &mut TestAppContext,
) {
    cx.executor().allow_parking();
    let (runtime, handle, page) = mount(cx, false);
    let server = Server::new(&runtime);
    connected(cx, handle, &page, server.url.clone()).await;
    let profiled = |page: &gpui_kit::Entity<ObservabilityPage>| {
        let page = page.clone();
        move |_: &mut gpui_kit::Window, cx: &mut gpui_kit::App| {
            !page.read(cx).live.profiling.is_loading()
        }
    };
    // The picker finds an application by part of its name.
    let wanted = cx.update(|cx| {
        page.update(cx, |page, cx| {
            page.selected_app = None;
            page.open(Destination::Profiling, cx);
            page.applications.last().unwrap().id.clone()
        })
    });
    let asked = wanted.clone();
    for act in [
        &(|window: &mut gpui_kit::Window, cx: &mut gpui_kit::App| {
            window.within("obs-profile-app").click("input", cx)
        }) as &dyn Fn(&mut gpui_kit::Window, &mut gpui_kit::App),
        &|window, cx| window.input(wanted.name(), cx),
        &|window, cx| window.press("enter", cx),
    ] {
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            act(window, cx);
        })
        .unwrap();
        cx.run_until_parked();
    }
    assert_eq!(
        cx.update(|cx| page.read(cx).selected_app.clone()),
        Some(wanted)
    );
    cx.update(|cx| {
        page.update(cx, |page, cx| {
            let app = page.applications[0].id.clone();
            page.selected_app = Some(app);
            page.refresh(cx);
        })
    });
    cx.wait_for(handle, std::time::Duration::from_secs(5), profiled(&page))
        .await;
    let path = last_path(&server, "/profiling");
    assert!(path.contains("query=cpu"));
    // The profile asked for is the page's application's.
    assert_eq!(
        cx.read(|cx| page.read(cx).selected_app.clone()),
        Some(asked.clone())
    );
    let app = asked.as_str().replace(':', "%3A");
    assert!(path.contains(&format!("/app/{app}/profiling")), "{path}");
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        for ix in 0..4 {
            assert!(
                window
                    .try_find(gpui_kit::SharedString::from(format!("obs-live-flame-{ix}")))
                    .is_some()
            );
        }
        assert!(window.try_find("obs-live-increase-2").is_none());
        // A frame is a data mark: 3 px corners, like a meter (DESIGN.md).
        let scale = window.scale_factor();
        let frame = window.find("obs-live-flame-0").bounds().scale(scale);
        let corners: Vec<f32> = window
            .painted_quads()
            .into_iter()
            .filter(|quad| {
                let b = quad.bounds;
                (b.left() - frame.left()).0.abs() < 1.
                    && (b.top() - frame.top()).0.abs() < 1.
                    && (b.right() - frame.right()).0.abs() < 1.
            })
            .map(|quad| quad.corner_radii.top_left.0 / scale)
            .collect();
        assert!(
            corners.iter().any(|radius| (radius - 3.).abs() < 0.01),
            "the frame's quads round at {corners:?}"
        );
        window.click("obs-live-flame-2", cx);
        window.render_frame(cx);
        assert!(window.try_find("obs-live-frame-detail").is_some());
        window.click("obs-live-profile-compare", cx);
    })
    .unwrap();
    cx.wait_for(handle, std::time::Duration::from_secs(5), profiled(&page))
        .await;
    let path = last_path(&server, "/profiling");
    assert!(
        path.contains("diff") && path.contains("profile_cpu"),
        "{path}"
    );
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        // main gained 30 points of the total; gc lost them.
        assert!(window.try_find("obs-live-increase-2").is_some());
        assert!(window.try_find("obs-live-increase-1").is_none());
    })
    .unwrap();
}

#[gpui_kit::test]
async fn the_trace_pane_names_its_read_and_retries_a_failed_trace(cx: &mut TestAppContext) {
    cx.executor().allow_parking();
    let (runtime, handle, page) = mount(cx, false);
    let server = Server::new(&runtime);
    connected(cx, handle, &page, server.url.clone()).await;
    cx.update_window(handle, |_, window, cx| {
        page.update(cx, |page, cx| {
            let app = page.applications[0].id.clone();
            page.open_app(app, Report::Cpu, cx);
        });
        window.render_frame(cx);
        window.click("obs-app-traces", cx);
    })
    .unwrap();
    traced(cx, handle, &page).await;
    let read = |window: &mut gpui_kit::Window| {
        window
            .try_find("obs-trace-read")
            .map(|tag| tag.label().unwrap_or_default().to_string())
    };
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(window.find("obs-trace-detail").visible());
        assert!(window.find("obs-live-waterfall").visible());
        assert_eq!(read(window), None, "a current trace shows no read tag");

        // A refresh reads the trace again; its tag says so until it answers.
        server.status.store(500, Ordering::SeqCst);
        page.update(cx, |page, cx| page.refresh(cx));
        window.render_frame(cx);
        assert_eq!(read(window).as_deref(), Some("Reading"));
    })
    .unwrap();
    traced(cx, handle, &page).await;
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        // The last trace stays, marked stale, with the error and Retry.
        assert_eq!(read(window).as_deref(), Some("Stale"));
        assert!(window.find("obs-live-waterfall").visible());
        assert!(window.find("obs-trace-retry").visible());

        // Another trace has nothing to keep, so its read failed.
        window.click("obs-live-span-t2-other", cx);
    })
    .unwrap();
    traced(cx, handle, &page).await;
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(read(window).as_deref(), Some("Failed"));
        assert!(window.try_find("obs-live-waterfall").is_none());
        server.status.store(200, Ordering::SeqCst);
        window.click("obs-trace-retry", cx);
    })
    .unwrap();
    traced(cx, handle, &page).await;
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(read(window), None);
        assert!(window.try_find("obs-trace-retry").is_none());
        assert!(window.find("obs-live-waterfall").visible());
    })
    .unwrap();
}

#[gpui_kit::test]
async fn traces_and_profiling_first_read_failures_show_retry_and_recover(cx: &mut TestAppContext) {
    cx.executor().allow_parking();
    let (runtime, handle, page) = mount(cx, false);
    let server = Server::new(&runtime);
    cx.update(|cx| {
        page.update(cx, |page, _| {
            let provider = api::Provider::new(&server.url, api::Credentials::None).unwrap();
            page.live.source = Some(provider.source(&api::ProjectInfo {
                id: "p1".into(),
                name: "Test project".into(),
            }));
            page.live.provider = Some(provider);
            let app = api::Application {
                id: api::AppId::new("cluster-a:prod:Deployment:api"),
                cluster: "Test cluster".into(),
                category: "application".into(),
                app_type: "Go".into(),
                status: api::Status::Ok,
                signals: Default::default(),
            };
            page.selected_app = Some(app.id.clone());
            page.apply_applications(&[app]);
        })
    });
    for destination in [Destination::Traces, Destination::Profiling] {
        for status in [403, 500] {
            server.status.store(status, Ordering::SeqCst);
            cx.update(|cx| {
                page.update(cx, |page, cx| {
                    page.live.tracing = Default::default();
                    page.live.profiling = Default::default();
                    page.open(destination, cx);
                })
            });
            let observed = page.clone();
            cx.wait_for(handle, std::time::Duration::from_secs(5), move |_, cx| {
                let page = observed.read(cx);
                match destination {
                    Destination::Traces => page.live.tracing.error().is_some(),
                    _ => page.live.profiling.error().is_some(),
                }
            })
            .await;
            cx.update_window(handle, |_, window, cx| {
                window.render_frame(cx);
                assert!(
                    window.find("obs-failed").visible(),
                    "{destination:?} first {status}"
                );
                assert!(window.find("obs-retry").visible());
                assert!(window.try_find("obs-stale").is_none());
                server.status.store(200, Ordering::SeqCst);
                window.click("obs-retry", cx);
            })
            .unwrap();
            let observed = page.clone();
            cx.wait_for(handle, std::time::Duration::from_secs(5), move |_, cx| {
                let page = observed.read(cx);
                match destination {
                    Destination::Traces => page.live.tracing.data().is_some(),
                    _ => page.live.profiling.data().is_some(),
                }
            })
            .await;
            cx.update_window(handle, |_, window, cx| {
                window.render_frame(cx);
                assert!(window.try_find("obs-failed").is_none());
            })
            .unwrap();
        }
    }
}

#[gpui_kit::test]
async fn heatmap_arrows_read_nothing_until_enter(cx: &mut TestAppContext) {
    cx.executor().allow_parking();
    let (runtime, handle, page) = mount(cx, false);
    let server = Server::new(&runtime);
    connected(cx, handle, &page, server.url.clone()).await;
    cx.update_window(handle, |_, window, cx| {
        page.update(cx, |page, cx| {
            let app = page.applications[0].id.clone();
            page.open_app(app, Report::Cpu, cx);
        });
        window.render_frame(cx);
        window.click("obs-app-traces", cx);
    })
    .unwrap();
    traced(cx, handle, &page).await;
    let cursor = |window: &mut gpui_kit::Window| {
        window
            .try_find("obs-heatmap-cursor")
            .map(|caption| caption.label().unwrap_or_default().to_string())
    };
    let before = cx
        .update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            assert_eq!(
                cursor(window),
                None,
                "no cursor until the heatmap has focus"
            );
            // Tab reaches the heatmap; its cursor starts on the latest failures.
            let focus = page.read(cx).heat_focus.clone();
            window.focus(&focus, cx);
            window.render_frame(cx);
            let label = cursor(window).expect("a focused heatmap shows its cursor");
            assert!(label.starts_with("Failed requests · "), "{label}");
            assert!(label.ends_with(" · no failures"), "{label}");
            assert_eq!(
                window.find("obs-live-bucket-0-2").label(),
                Some(label.as_str())
            );
            server.requests.load(Ordering::SeqCst)
        })
        .unwrap();
    cx.update_window(handle, |_, window, cx| {
        // Moves clamp at the edges and read nothing.
        window.press("up", cx);
        window.press("right", cx);
        window.press("left", cx);
        window.press("down", cx);
        window.press("down", cx);
        window.press("down", cx);
        window.press("home", cx);
        window.press("right", cx);
        window.render_frame(cx);
        assert_eq!(
            page.read(cx).live_traces.selection,
            api::TraceSelection::Recent
        );
        let label = cursor(window).unwrap();
        assert!(label.starts_with("Up to 5ms · "), "{label}");
        assert_eq!(
            window.find("obs-live-bucket-2-1").label(),
            Some(label.as_str())
        );
    })
    .unwrap();
    cx.run_until_parked();
    assert_eq!(server.requests.load(Ordering::SeqCst), before);
    // Enter lists the cell's requests: one read.
    cx.update_window(handle, |_, window, cx| window.press("enter", cx))
        .unwrap();
    traced(cx, handle, &page).await;
    cx.update_window(handle, |_, window, cx| {
        let api::TraceSelection::Latency { above, up_to, .. } =
            page.read(cx).live_traces.selection.clone()
        else {
            panic!("a latency cell");
        };
        assert_eq!((above.as_str(), up_to.as_str()), ("0", "0.005"));
        assert!(server.requests.load(Ordering::SeqCst) > before);
        // Escape leaves the heatmap for the page.
        window.press("escape", cx);
        window.render_frame(cx);
        assert!(!page.read(cx).heat_focus.is_focused(window));
        assert!(page.read(cx).focus.is_focused(window));
        assert_eq!(cursor(window), None);
    })
    .unwrap();
}

#[gpui_kit::test]
async fn an_application_reads_coroots_own_view(cx: &mut TestAppContext) {
    cx.executor().allow_parking();
    let (runtime, handle, page) = mount(cx, false);
    let server = Server::new(&runtime);
    connected(cx, handle, &page, server.url.clone()).await;
    cx.update(|cx| {
        page.update(cx, |page, cx| {
            let app = page.applications[0].id.clone();
            page.open_app(app, Report::Cpu, cx);
        })
    });
    let observed = page.clone();
    cx.wait_for(handle, std::time::Duration::from_secs(5), move |_, cx| {
        !observed.read(cx).live.view.is_loading()
    })
    .await;
    assert!(
        last_path(&server, "/app/").contains("/project/p1/app/cluster-a%3Aprod%3ADeployment%3Aapi")
    );
    cx.update_window(handle, |_, window, cx| {
        assert!(page.read(cx).live.view.error().is_none());
        window.render_frame(cx);
        assert!(
            window
                .find("obs-app-map-cluster-a:prod:Deployment:db")
                .visible()
        );
        assert!(window.find("obs-report-cpu").visible());
        assert!(window.find("obs-app-checks").visible());
        assert!(window.find("obs-app-table-0-row-0").visible());
    })
    .unwrap();
}

/// Coroot's logs answer: two messages, or on a refresh the newest again
/// with one after it, as Coroot's `since` returns the newest message's own
/// millisecond.
fn logs_wire(path: &str) -> serde_json::Value {
    let refresh = !path.contains("since%22%3A%22%22");
    let t = 1_789_999_000_000_i64;
    let entry = |ms: i64, message: &str| serde_json::json!({"timestamp":ms,"severity":"info","message":message,"attributes":{"host.name":"app-a"},"trace_id":""});
    let (entries, newest) = if refresh {
        (
            vec![
                entry(t + 2000, "third"),
                entry(t + 1000, "second\n  continued"),
            ],
            t + 2000,
        )
    } else {
        (
            vec![entry(t + 1000, "second\n  continued"), entry(t, "first")],
            t + 1000,
        )
    };
    serde_json::json!({"status":"ok","message":"Using container logs","sources":["agent"],"source":"agent",
        "view":"messages","entries":entries,"patterns":null,"limit":0,"max_ts":(newest * 1_000_000).to_string(),
        "chart":{"ctx":{"from":t - 3_600_000,"to":t + 3_600_000,"step":60_000},"title":"","column":true,
            "series":[{"name":"info","color":"blue-lighten2","data":[1,2]}]}})
}

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

fn retained(cx: &mut TestAppContext, page: &gpui_kit::Entity<ObservabilityPage>) -> Vec<String> {
    cx.read(|cx| {
        let view = page.read(cx).live_logs.view.read(cx);
        view.retained().iter().map(|e| e.raw.clone()).collect()
    })
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
    let observed = page.clone();
    cx.wait_for(handle, std::time::Duration::from_secs(5), move |_, cx| {
        observed.read(cx).live_logs.failure().is_some()
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

/// The newest revision's ±30 min window, as its request names it.
const NEWEST_WINDOW: &str = "from=1789997200000";

/// Waits until `count` requests named `part`, and the page read the
/// application.
async fn sent(
    cx: &mut TestAppContext,
    handle: gpui_kit::AnyWindowHandle,
    page: &gpui_kit::Entity<ObservabilityPage>,
    server: &Server,
    part: &'static str,
    count: usize,
) {
    let (observed, paths) = (page.clone(), server.paths.clone());
    cx.wait_for(handle, std::time::Duration::from_secs(5), move |_, cx| {
        let paths = paths.lock().unwrap();
        paths.iter().filter(|p| p.contains(part)).count() >= count
            && !observed.read(cx).live.view.is_loading()
    })
    .await;
}

/// Waits until the fake has written `count` answers to requests named
/// `part`, so a late answer has reached the page if it ever will.
async fn responded(
    cx: &mut TestAppContext,
    handle: gpui_kit::AnyWindowHandle,
    server: &Server,
    part: &'static str,
    count: usize,
) {
    let answered = server.responded.clone();
    cx.wait_for(handle, std::time::Duration::from_secs(5), move |_, _| {
        let answered = answered.lock().unwrap();
        answered.iter().filter(|p| p.contains(part)).count() >= count
    })
    .await;
    cx.run_until_parked();
}

/// Opens Deployments on the fake's application.
fn open_deployments(cx: &mut TestAppContext, page: &gpui_kit::Entity<ObservabilityPage>) {
    cx.update(|cx| {
        page.update(cx, |page, cx| {
            page.selected_app = Some(page.applications[0].id.clone());
            page.open(Destination::Deployments, cx);
        })
    });
}

/// The window the last answer covers, from its start in milliseconds.
fn answered_from(
    cx: &mut TestAppContext,
    page: &gpui_kit::Entity<ObservabilityPage>,
) -> Option<i64> {
    cx.read(|cx| {
        let answer = page.read(cx).live.revision.data();
        answer.map(|answer| answer.from.timestamp_millis())
    })
}

/// Coroot's Deployments report: two revisions, newest first.
fn deployments_wire() -> serde_json::Value {
    let row = |id: &str, version: &str, summaries: serde_json::Value| {
        // Without findings, Coroot sends its note as a stub.
        let summary = if summaries.as_array().is_some_and(Vec::is_empty) {
            serde_json::json!({"value":"No notable changes","is_stub":true})
        } else {
            serde_json::json!({"value":"","deployment_summaries":summaries})
        };
        serde_json::json!({"id":id,"cells":[
            {"value":version,"status":"ok"},{"value":"1h ago"},summary]})
    };
    serde_json::json!({"name":"Deployments","status":"ok","checks":null,"widgets":[{"table":{
        "header":["Deployment","Deployed","Summary"],"rows":[
            row("9c41e7:1789999000","9c41e7: example.test/shop/api:1.4.0",
                serde_json::json!([{"report":"SLO","ok":true,"message":"Availability: 100% (objective: 99%)","time":null}])),
            row("2b70aa:1789990000","2b70aa: example.test/shop/api:1.3.9",serde_json::json!([])),
        ]},"width":"100%"}]})
}

async fn revision_read(
    cx: &mut TestAppContext,
    handle: gpui_kit::AnyWindowHandle,
    page: &gpui_kit::Entity<ObservabilityPage>,
) {
    let observed = page.clone();
    cx.wait_for(handle, std::time::Duration::from_secs(5), move |_, cx| {
        let page = observed.read(cx);
        let failed = page.revision_observations.list_error().is_some();
        !page.live.view.is_loading()
            && (failed || page.live.revision_job.is_some() && !page.live.revision.is_loading())
    })
    .await;
    cx.read(|cx| {
        let error = page.read(cx).revision_observations.list_error().cloned();
        assert_eq!(error, None, "Coroot's revisions decode");
    });
}

#[gpui_kit::test]
async fn deployments_read_coroots_revisions_and_the_window_around_one(cx: &mut TestAppContext) {
    cx.executor().allow_parking();
    let (runtime, handle, page) = mount(cx, false);
    let server = Server::new(&runtime);
    server.deploy_mode.store(1, Ordering::SeqCst);
    connected(cx, handle, &page, server.url.clone()).await;
    cx.update(|cx| {
        page.update(cx, |page, cx| {
            page.selected_app = Some(page.applications[0].id.clone());
            page.open(Destination::Deployments, cx);
        })
    });
    revision_read(cx, handle, &page).await;
    let window = last_path(&server, "from=17899");
    assert!(
        window.contains("/app/cluster-a%3Aprod%3ADeployment%3Aapi"),
        "{window}"
    );
    assert!(window.contains("from=1789997200000"), "{window}");
    assert!(window.contains("to=1790000800000"), "{window}");
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(window.find("obs-revision-9c41e7-1789999000").visible());
        assert!(window.find("obs-revision-2b70aa-1789990000").visible());
        assert!(window.find("obs-revision-finding-0").visible());
        assert!(window.find("obs-revision-window").visible());
        // Coroot's page in the fake has no charts.
        assert!(window.find("obs-revision-no-charts").visible());
    })
    .unwrap();

    // Coroot's 404 for the window is no data, never a missing application.
    server.deploy_mode.store(2, Ordering::SeqCst);
    cx.update(|cx| page.update(cx, |page, cx| page.refresh(cx)));
    revision_read(cx, handle, &page).await;
    cx.update_window(handle, |_, window, cx| {
        assert!(page.read(cx).live.revision.error().is_none());
        window.render_frame(cx);
        assert!(window.find("obs-revision-no-data").visible());
        assert!(window.try_find("obs-revision-failed").is_none());
        assert!(window.find("obs-revision-9c41e7-1789999000").visible());
    })
    .unwrap();

    // Any other failure says so, with Retry.
    server.deploy_mode.store(3, Ordering::SeqCst);
    cx.update(|cx| page.update(cx, |page, cx| page.refresh(cx)));
    revision_read(cx, handle, &page).await;
    cx.update_window(handle, |_, window, cx| {
        assert!(page.read(cx).live.revision.error().is_some());
        window.render_frame(cx);
        assert!(window.find("obs-revision-failed").visible());
        assert!(window.find("obs-revision-retry").visible());
    })
    .unwrap();
    server.deploy_mode.store(1, Ordering::SeqCst);
    cx.update_window(handle, |_, window, cx| {
        window.click("obs-revision-retry", cx)
    })
    .unwrap();
    revision_read(cx, handle, &page).await;
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(window.try_find("obs-revision-failed").is_none());
        assert!(window.find("obs-revision-window").visible());
    })
    .unwrap();
}

#[gpui_kit::test]
async fn coming_from_the_application_page_keeps_its_revisions_while_it_is_read_again(
    cx: &mut TestAppContext,
) {
    cx.executor().allow_parking();
    let (runtime, handle, page) = mount(cx, false);
    let server = Server::new(&runtime);
    server.deploy_mode.store(1, Ordering::SeqCst);
    connected(cx, handle, &page, server.url.clone()).await;
    cx.update(|cx| {
        page.update(cx, |page, cx| {
            page.selected_app = Some(page.applications[0].id.clone());
            page.open(Destination::Application, cx);
        })
    });
    sent(cx, handle, &page, &server, "/app/", 1).await;
    cx.update_window(handle, |_, window, cx| {
        page.update(cx, |page, cx| {
            page.open(Destination::Deployments, cx);
            // Forgets the first read's page, so only the read again can
            // derive it.
            page.app_page = None;
        });
        assert!(
            page.read(cx).live.view.is_loading(),
            "the page is read again"
        );
        window.render_frame(cx);
        assert!(window.find("obs-revision-9c41e7-1789999000").visible());
        assert!(
            window.try_find("obs-deployments-empty").is_none(),
            "no empty state while the application is read again"
        );
    })
    .unwrap();
    revision_read(cx, handle, &page).await;
    cx.read(|cx| {
        assert!(
            page.read(cx).app_page.is_some(),
            "the Application page follows the read"
        )
    });
}

#[gpui_kit::test]
async fn refresh_and_coming_back_keep_an_older_revision_chosen(cx: &mut TestAppContext) {
    cx.executor().allow_parking();
    let (runtime, handle, page) = mount(cx, false);
    let server = Server::new(&runtime);
    server.deploy_mode.store(1, Ordering::SeqCst);
    connected(cx, handle, &page, server.url.clone()).await;
    open_deployments(cx, &page);
    revision_read(cx, handle, &page).await;
    let older = crate::tables::TableKey::Revision("2b70aa:1789990000".into());
    // The ±30 min window around the older revision's start.
    let around_older = Some(1_789_988_200_000);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("obs-revision-2b70aa-1789990000", cx);
    })
    .unwrap();
    revision_read(cx, handle, &page).await;
    assert_eq!(answered_from(cx, &page), around_older);

    // While it reads, the table shows its loading rows, as after any
    // Refresh, and the choice waits for the answer.
    let chosen = |cx: &mut TestAppContext, what: &str| {
        cx.read(|cx| {
            let page = page.read(cx);
            assert!(page.live.view.is_loading());
            assert_eq!(page.revision_selected_key(), Some(&older), "{what}");
        })
    };
    let check = |cx: &mut TestAppContext, what: &str| {
        cx.update_window(handle, |_, window, cx| {
            assert_eq!(
                page.read(cx).revision_selected_key(),
                Some(&older),
                "{what} keeps the older revision"
            );
            window.render_frame(cx);
            assert!(window.find("obs-revision-2b70aa-1789990000").visible());
            assert!(window.try_find("obs-deployments-empty").is_none());
        })
        .unwrap();
    };
    cx.update(|cx| page.update(cx, |page, cx| page.refresh_current(cx)));
    chosen(cx, "reading again");
    revision_read(cx, handle, &page).await;
    check(cx, "Refresh");
    assert_eq!(answered_from(cx, &page), around_older);

    cx.update(|cx| {
        page.update(cx, |page, cx| {
            page.set_visible(false, cx);
            page.set_visible(true, cx);
        })
    });
    chosen(cx, "coming back, while reading");
    revision_read(cx, handle, &page).await;
    check(cx, "coming back");
    assert_eq!(answered_from(cx, &page), around_older);
}

#[gpui_kit::test]
async fn a_window_read_shows_loading_and_a_newer_choice_or_hiding_drops_its_answer(
    cx: &mut TestAppContext,
) {
    cx.executor().allow_parking();
    let (runtime, handle, page) = mount(cx, false);
    let server = Server::new(&runtime);
    server.deploy_mode.store(4, Ordering::SeqCst);
    connected(cx, handle, &page, server.url.clone()).await;
    open_deployments(cx, &page);
    sent(cx, handle, &page, &server, NEWEST_WINDOW, 1).await;
    cx.update_window(handle, |_, window, cx| {
        assert!(page.read(cx).live.revision.is_loading());
        window.render_frame(cx);
        assert!(window.find("obs-revision-loading").visible());
        assert!(window.find("obs-revision-read").visible(), "Reading");
        // Another revision supersedes the read in flight.
        window.click("obs-revision-2b70aa-1789990000", cx);
    })
    .unwrap();
    revision_read(cx, handle, &page).await;
    server.window_gate.notify_one();
    responded(cx, handle, &server, NEWEST_WINDOW, 1).await;
    cx.read(|cx| {
        let answer = page.read(cx).live.revision.data().unwrap();
        assert_eq!(answer.revision.id, "2b70aa:1789990000");
    });

    // So does another window.
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("obs-revision-9c41e7-1789999000", cx);
    })
    .unwrap();
    sent(cx, handle, &page, &server, NEWEST_WINDOW, 2).await;
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("obs-revision-window-1", cx);
    })
    .unwrap();
    revision_read(cx, handle, &page).await;
    server.window_gate.notify_one();
    responded(cx, handle, &server, NEWEST_WINDOW, 2).await;
    assert_eq!(answered_from(cx, &page), Some(1_789_995_400_000));

    // Hiding drops the read in flight, and its late answer never lands.
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("obs-revision-window-0", cx);
    })
    .unwrap();
    sent(cx, handle, &page, &server, NEWEST_WINDOW, 3).await;
    cx.update(|cx| {
        page.update(cx, |page, cx| {
            page.set_visible(false, cx);
            assert!(page.live.revision_job.is_none());
        })
    });
    server.window_gate.notify_one();
    responded(cx, handle, &server, NEWEST_WINDOW, 3).await;
    assert_ne!(answered_from(cx, &page), Some(1_789_997_200_000));
}

#[gpui_kit::test]
async fn a_refused_window_says_not_permitted(cx: &mut TestAppContext) {
    cx.executor().allow_parking();
    let (runtime, handle, page) = mount(cx, false);
    let server = Server::new(&runtime);
    server.deploy_mode.store(5, Ordering::SeqCst);
    connected(cx, handle, &page, server.url.clone()).await;
    open_deployments(cx, &page);
    revision_read(cx, handle, &page).await;
    cx.update_window(handle, |_, window, cx| {
        assert!(page.read(cx).live.revision_refused);
        window.render_frame(cx);
        assert!(window.find("obs-revision-refused").visible());
        assert!(window.try_find("obs-revision-failed").is_none());
        // The list stays: only the window was refused.
        assert!(window.find("obs-revision-9c41e7-1789999000").visible());
    })
    .unwrap();
}

#[gpui_kit::test]
async fn a_refused_application_page_says_not_permitted_on_deployments(cx: &mut TestAppContext) {
    cx.executor().allow_parking();
    let (runtime, handle, page) = mount(cx, false);
    let server = Server::new(&runtime);
    server.deploy_mode.store(1, Ordering::SeqCst);
    connected(cx, handle, &page, server.url.clone()).await;
    server.status.store(403, Ordering::SeqCst);
    open_deployments(cx, &page);
    sent(cx, handle, &page, &server, "/app/", 1).await;
    cx.update_window(handle, |_, window, cx| {
        assert!(page.read(cx).live.view_refused);
        window.render_frame(cx);
        assert!(window.find("obs-refused").visible());
        assert!(window.try_find("obs-failed").is_none());
        assert!(window.try_find("obs-deployments-empty").is_none());
    })
    .unwrap();
}
