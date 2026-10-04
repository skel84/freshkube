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
        let task=runtime.spawn(async move {
            loop {
                let (mut socket,_)=listener.accept().await.unwrap();
                let (state,reads,captured,mode,gate)=(state.clone(),reads.clone(),captured.clone(),mode.clone(),gate.clone());
                tokio::spawn(async move {
                    let mut buf=vec![0;16384]; let Ok(n)=socket.read(&mut buf).await else{return;};
                    let request=String::from_utf8_lossy(&buf[..n]);
                    reads.fetch_add(1,Ordering::SeqCst);
                    let path=request.split_whitespace().nth(1).unwrap_or("");
                    captured.lock().unwrap().push(path.to_string());
                    let status=if path.contains("mcp") {404} else {state.load(Ordering::SeqCst)};
                    let selected_mode=mode.load(Ordering::SeqCst);
                    if path.contains("/incident/k1") && selected_mode==5 {gate.notified().await;}
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
            incident_mode,
            incident_gate,
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
            if window
                .find("obs-incident-app-cluster-a:prod:Deployment:worker")
                .visible()
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
