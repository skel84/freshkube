//! The real reader/lifecycle against an in-memory GET-only Kubernetes API.
use super::{ClusterSource, PageSource, ScreenPanel};
use freshkube_core::cluster_source::{ClusterAccess, KubeClientSource};
use freshkube_ui::real_io::RealIo;
use gpui_kit::test::TestWindowExt;
use gpui_kit::{AppContext, TestAppContext};
use serde_json::json;
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicU16, Ordering},
};
struct Fake {
    client: kube::Client,
    calls: Arc<Mutex<Vec<String>>>,
    backup_status: Arc<AtomicU16>,
}
impl KubeClientSource for Fake {
    fn client(
        &self,
    ) -> futures::future::BoxFuture<'_, Result<kube::Client, freshkube_core::resources::Failure>>
    {
        let client = self.client.clone();
        Box::pin(async move { Ok(client) })
    }
}
impl Fake {
    fn assert_read(&self, namespace: Option<&str>) {
        let mut expected = vec![
            "GET /apis/postgresql.cnpg.io".to_owned(),
            "GET /apis/postgresql.cnpg.io/v1".into(),
        ];
        for kind in ["clusters", "backups", "scheduledbackups"] {
            let scope = namespace
                .map(|ns| format!("/namespaces/{ns}"))
                .unwrap_or_default();
            expected.push(format!(
                "GET /apis/postgresql.cnpg.io/v1{scope}/{kind}?limit=200"
            ));
        }
        self.assert_requests(expected);
    }
    fn assert_requests(&self, mut expected: Vec<String>) {
        let mut actual = std::mem::take(&mut *self.calls.lock().unwrap());
        actual.sort();
        expected.sort();
        assert_eq!(actual, expected);
    }
    fn new() -> Arc<Self> {
        let calls = Arc::new(Mutex::new(Vec::new()));
        let seen = calls.clone();
        let backup_status = Arc::new(AtomicU16::new(200));
        let status = backup_status.clone();
        let service = tower::service_fn(move |request: http::Request<kube::client::Body>| {
            seen.lock()
                .unwrap()
                .push(format!("{} {}", request.method(), request.uri()));
            let path = request.uri().path();
            let mut code = 200;
            let data = match path {
                "/apis/postgresql.cnpg.io" => json!({"versions":[{"version":"v1"}]}),
                "/apis/postgresql.cnpg.io/v1" => {
                    json!({"resources":[{"name":"clusters","kind":"Cluster","namespaced":true,"verbs":["list"]},{"name":"backups","kind":"Backup","namespaced":true,"verbs":["list"]},{"name":"scheduledbackups","kind":"ScheduledBackup","namespaced":true,"verbs":["list"]}]})
                }
                "/apis/postgresql.cnpg.io/v1/clusters"
                | "/apis/postgresql.cnpg.io/v1/namespaces/shop/clusters" => {
                    json!({"items":[{"metadata":{"name":"orders-db","namespace":"shop","uid":"fake-orders","creationTimestamp":"2026-01-01T00:00:00Z"},"spec":{"instances":2},"status":{"currentPrimary":"orders-db-1","readyInstances":2}}]})
                }
                "/apis/postgresql.cnpg.io/v1/backups"
                | "/apis/postgresql.cnpg.io/v1/namespaces/shop/backups" => {
                    code = status.load(Ordering::SeqCst);
                    if code == 200 {
                        json!({"items":[{"metadata":{"name":"orders-backup","namespace":"shop","uid":"fake-backup","creationTimestamp":"2026-10-20T00:00:00Z"},"spec":{"cluster":{"name":"orders-db"},"method":"plugin"},"status":{"phase":"completed","stoppedAt":"2026-10-20T01:00:00Z","s3Credentials":{"secret":"sensitive-canary"}}}]})
                    } else {
                        json!({"kind":"Status","status":"Failure","reason":"Forbidden","message":"invented refusal","code":code})
                    }
                }
                "/apis/postgresql.cnpg.io/v1/scheduledbackups"
                | "/apis/postgresql.cnpg.io/v1/namespaces/shop/scheduledbackups" => {
                    json!({"items":[]})
                }
                _ => {
                    code = 500;
                    json!({"kind":"Status","status":"Failure","reason":"UnexpectedRequest","code":500})
                }
            };
            async move {
                Ok::<_, std::convert::Infallible>(
                    http::Response::builder()
                        .status(code)
                        .body(kube::client::Body::from(data.to_string().into_bytes()))
                        .unwrap(),
                )
            }
        });
        Arc::new(Self {
            client: kube::Client::new(service, "shop"),
            calls,
            backup_status,
        })
    }
}
#[gpui_kit::test]
async fn real_reader_keeps_per_kind_failures_stale_and_never_reads_while_hidden(
    cx: &mut TestAppContext,
) {
    cx.executor().allow_parking();
    let (_runtime, page, window) = super::tests::mount(cx, 1280.);
    let fake = {
        let _entered = _runtime.enter();
        Fake::new()
    };
    cx.update_window(window.into(), |_, window, cx| {
        page.update(cx, |page, cx| {
            page.set_page_source(
                PageSource {
                    kube: Some(ClusterSource {
                        id: "fake-db-access".into(),
                        context: "fake".into(),
                        access: ClusterAccess::Live(fake.clone()),
                    }),
                    ..PageSource::default()
                },
                window,
                cx,
            );
            page.activate(window, cx);
        })
    })
    .unwrap();
    cx.wait_for_io(window.into(), |_, cx| {
        page.read(cx).loader.data().is_some() && !page.read(cx).loader.is_loading()
    })
    .await;
    fake.assert_read(None);
    cx.update_window(window.into(), |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(page.read(cx).display.rows.len(), 1);
        let key = page.read(cx).display.rows[0].id.clone();
        window.click(key, cx);
        window.render_frame(cx);
        fake.backup_status.store(403, Ordering::SeqCst);
        window.click("databases-refresh", cx);
    })
    .unwrap();
    cx.wait_for_io(window.into(), |_, cx| {
        page.read(cx)
            .loader
            .data()
            .is_some_and(|inventory| inventory.backups.stale)
    })
    .await;
    fake.assert_read(None);
    cx.update_window(window.into(), |_, window, cx| {
        window.render_frame(cx);
        window.find("databases-detail");
        let inventory = page.read(cx).loader.data().unwrap();
        assert_eq!(inventory.backups.items.len(), 1);
        assert!(!format!("{inventory:?}").contains("sensitive-canary"));
        assert!(
            window
                .find("databases-coverage")
                .label()
                .unwrap()
                .contains("last read")
        );
        page.update(cx, |page, cx| page.set_visible(false, window, cx));
        page.update(cx, |page, cx| page.refresh(window, cx));
    })
    .unwrap();
    cx.executor()
        .advance_clock(std::time::Duration::from_secs(61));
    cx.run_until_parked();
    // A real fake-API round trip drains queued work before asserting absence.
    let client = fake.client.clone();
    let barrier = _runtime.spawn(async move {
        client
            .request::<serde_json::Value>(
                http::Request::builder()
                    .uri("/apis/postgresql.cnpg.io")
                    .body(Vec::new())
                    .unwrap(),
            )
            .await
    });
    cx.wait_for_io(window.into(), |_, _| barrier.is_finished())
        .await;
    _runtime.block_on(barrier).unwrap().unwrap();
    fake.assert_requests(vec!["GET /apis/postgresql.cnpg.io".into()]);
    cx.update_window(window.into(), |_, window, cx| {
        assert!(!page.read(cx).loader.is_loading());
        page.update(cx, |page, cx| page.set_visible(true, window, cx));
    })
    .unwrap();
    cx.wait_for_io(window.into(), |_, cx| !page.read(cx).loader.is_loading())
        .await;
    fake.assert_read(None);
    cx.update_window(window.into(), |_, window, cx| {
        window.click("databases-namespace", cx);
        window.input("shop", cx);
        window.press("enter", cx);
    })
    .unwrap();
    cx.wait_for_io(window.into(), |_, cx| {
        page.read(cx).namespace.as_deref() == Some("shop") && !page.read(cx).loader.is_loading()
    })
    .await;
    fake.assert_read(Some("shop"));
}
