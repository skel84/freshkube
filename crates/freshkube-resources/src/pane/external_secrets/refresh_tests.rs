use crate::pane::{DetailPane, tests::mount};
use crate::{detail::DetailTarget, example, external_refresh::Phase, screen::ClusterAccess};
use freshkube_core::{cluster_source::KubeClientSource, resources::Failure};
use freshkube_ui::real_io::RealIo;
use gpui_kit::test::TestWindowExt;
use gpui_kit::{AnyWindowHandle, AppContext, Entity, TestAppContext};
use http::{Request, Response};
use kube::client::Body;
use std::{
    sync::{Arc, Mutex},
    time::Duration,
};

const OBJECT: &str =
    "/apis/external-secrets.io/v1/namespaces/paper-kites/externalsecrets/lantern-api";
const TOKEN: i64 = 1_767_355_200;

struct Source(kube::Client);
impl KubeClientSource for Source {
    fn client(&self) -> futures::future::BoxFuture<'_, Result<kube::Client, Failure>> {
        let client = self.0.clone();
        Box::pin(async move { Ok(client) })
    }
}
struct Fake {
    seen: Arc<Mutex<Vec<(String, String, serde_json::Value)>>>,
    release: Arc<tokio::sync::Notify>,
}
impl Fake {
    fn patches(&self) -> usize {
        self.seen
            .lock()
            .unwrap()
            .iter()
            .filter(|(verb, _, _)| verb == "PATCH")
            .count()
    }
    fn assert_requests(&self) {
        let mut actual: Vec<_> = self
            .seen
            .lock()
            .unwrap()
            .iter()
            .map(|(verb, path, _)| (verb.clone(), path.split('?').next().unwrap().to_owned()))
            .collect();
        actual.sort();
        let mut expected = [
            ("GET", OBJECT),
            ("PATCH", OBJECT),
            ("GET", "/apis"),
            ("GET", "/apis/external-secrets.io/v1"),
            (
                "GET",
                "/apis/external-secrets.io/v1/namespaces/paper-kites/secretstores/meadow-store",
            ),
            ("GET", "/api/v1/namespaces/paper-kites/events"),
        ]
        .map(|(verb, path)| (verb.to_owned(), path.to_owned()));
        expected.sort();
        assert_eq!(
            actual, expected,
            "forbidden/ignored requests must still fail the test"
        );
    }
}
fn target() -> DetailTarget {
    let key = example::external_secrets::KEY;
    let (_, rows) = example::read("invented-context", key, None, TOKEN).unwrap();
    DetailTarget {
        identity: rows
            .into_iter()
            .find(|r| r.identity.name == "lantern-api")
            .unwrap()
            .identity,
        kind: example::kind(key).unwrap(),
    }
}
fn fake(runtime: &tokio::runtime::Runtime, code: u16) -> (ClusterAccess, Fake) {
    let target = target();
    let object: serde_json::Value =
        serde_yaml::from_str(&example::document(&target.identity, TOKEN).unwrap().yaml).unwrap();
    let seen = Arc::new(Mutex::new(Vec::new()));
    let requests = seen.clone();
    let release = Arc::new(tokio::sync::Notify::new());
    let gate = release.clone();
    let api = tower::service_fn(move |request: Request<Body>| {
        let requests = requests.clone();
        let object = object.clone();
        let gate = gate.clone();
        async move {
            let (parts, body) = request.into_parts();
            let bytes = body.collect_bytes().await.unwrap();
            let json = if bytes.is_empty() {
                serde_json::Value::Null
            } else {
                serde_json::from_slice(&bytes).unwrap()
            };
            let verb = parts.method.to_string();
            let path = parts.uri.path().to_owned();
            // Record first: an ignored error may never hide a forbidden request.
            requests
                .lock()
                .unwrap()
                .push((verb.clone(), parts.uri.to_string(), json.clone()));
            if verb == "PATCH" {
                assert_eq!(path, OBJECT, "any other object write is forbidden");
                assert_eq!(parts.uri.query(), Some("fieldManager=freshkube"));
                assert_eq!(
                    parts.headers[http::header::CONTENT_TYPE],
                    "application/merge-patch+json"
                );
                assert_eq!(
                    json,
                    serde_json::json!({"metadata":{"annotations":{"force-sync":TOKEN.to_string()}}})
                );
                assert_eq!(
                    requests
                        .lock()
                        .unwrap()
                        .iter()
                        .filter(|(verb, _, _)| verb == "PATCH")
                        .count(),
                    1,
                    "no replay or bulk refresh"
                );
                gate.notified().await;
                return Ok::<_, std::convert::Infallible>(Response::builder().status(code).body(Body::from(if code == 403 {
                    serde_json::json!({"kind":"Status","status":"Failure","code":403,"reason":"Forbidden","message":"Invented viewer may read but not patch lantern-api"}).to_string()
                } else { "{}".into() }.into_bytes())).unwrap());
            }
            assert_eq!(verb, "GET", "only one PATCH is permitted");
            let response = match path.as_str() {
                OBJECT => object,
                "/apis" => {
                    serde_json::json!({"groups":[{"name":"external-secrets.io","versions":[{"version":"v1"}]}]})
                }
                "/apis/external-secrets.io/v1" => {
                    serde_json::json!({"resources":[{"name":"secretstores","kind":"SecretStore","namespaced":true,"verbs":["get","list","watch"]}]})
                }
                "/apis/external-secrets.io/v1/namespaces/paper-kites/secretstores/meadow-store" => {
                    serde_json::json!({"status":{"conditions":[{"type":"Ready","status":"True"}]}})
                }
                "/api/v1/namespaces/paper-kites/events" => return futures::future::pending().await,
                _ => panic!("Secret, credential and unexpected reads are forbidden: {path}"),
            };
            Ok(Response::new(Body::from(response.to_string().into_bytes())))
        }
    });
    let client = {
        let _entered = runtime.enter();
        kube::Client::new(api, "paper-kites")
    };
    (
        ClusterAccess::Live(Arc::new(Source(client))),
        Fake { seen, release },
    )
}
fn open(pane: &Entity<DetailPane>, access: ClusterAccess, cx: &mut gpui_kit::App) {
    pane.update(cx, |pane, cx| {
        pane.set_context("invented-context".into(), cx);
        pane.external_refresh.update(cx, |owner, cx| {
            owner.set_clock(chrono::DateTime::from_timestamp(TOKEN, 0).unwrap(), cx)
        });
        pane.open(target(), access, "1", Duration::ZERO, cx);
    });
}
async fn loaded(pane: &Entity<DetailPane>, handle: AnyWindowHandle, cx: &mut TestAppContext) {
    cx.wait_for_io(handle, |_, cx| {
        pane.read(cx)
            .external
            .stores
            .first()
            .is_some_and(|line| line.status == "Ready True")
    })
    .await;
}
fn dialog(pane: &Entity<DetailPane>, handle: AnyWindowHandle, cx: &mut TestAppContext) {
    cx.update_window(handle, |_, window, cx| {
        pane.update(cx, |pane, cx| pane.request_external_refresh(window, cx));
        window.render_frame(cx);
        assert!(
            window
                .find("external-refresh-confirmation")
                .label()
                .unwrap()
                .contains("invented-context")
        );
        assert!(
            window
                .find("external-refresh-confirmation")
                .label()
                .unwrap()
                .contains("paper-kites/lantern-api")
        );
    })
    .unwrap();
}

#[gpui_kit::test]
async fn confirmation_is_required_and_403_displays_its_cause_without_retry(
    cx: &mut TestAppContext,
) {
    let (runtime, pane, handle, _) = mount(cx);
    cx.executor().allow_parking();
    let (access, fake) = fake(&runtime, 403);
    cx.update(|cx| open(&pane, access, cx));
    loaded(&pane, handle, cx).await;
    // Exercise the same handler Enter uses before any dialog frame is drawn.
    // TestWindowExt::press itself renders first, so it cannot test this boundary.
    cx.update_window(handle, |_, window, cx| {
        pane.update(cx, |pane, cx| pane.request_external_refresh(window, cx));
        let dialog = pane
            .read(cx)
            .refresh_dialog
            .as_ref()
            .unwrap()
            .upgrade()
            .unwrap();
        dialog.update(cx, |dialog, cx| dialog.confirm(window, cx));
    })
    .unwrap();
    cx.run_until_parked();
    assert_eq!(fake.patches(), 0);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("external-refresh-cancel", cx);
    })
    .unwrap();
    cx.run_until_parked();
    assert_eq!(fake.patches(), 0);
    dialog(&pane, handle, cx);
    cx.update_window(handle, |_, window, cx| window.press("escape", cx))
        .unwrap();
    cx.run_until_parked();
    assert_eq!(fake.patches(), 0);
    dialog(&pane, handle, cx);
    cx.update_window(handle, |_, window, cx| {
        window.click("external-refresh-confirm", cx)
    })
    .unwrap();
    cx.wait_for_io(handle, |_, _| fake.patches() == 1).await;
    // A second invocation while submitted does not open another confirmation.
    cx.update_window(handle, |_, window, cx| {
        pane.update(cx, |pane, cx| pane.request_external_refresh(window, cx));
        window.render_frame(cx);
        assert!(window.try_find("external-refresh-confirmation").is_none());
    })
    .unwrap();
    fake.release.notify_one();
    cx.wait_for_io(handle, |_, cx| {
        pane.read(cx)
            .external_refresh
            .read(cx)
            .attempt
            .as_ref()
            .is_some_and(|a| a.phase == Phase::Finished)
    })
    .await;
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let expected = "Not permitted to refresh: Invented viewer may read but not patch";
        let owner = pane.read(cx).external_refresh.read(cx);
        assert!(
            owner.attempt.as_ref().unwrap().text.contains(expected),
            "{}",
            owner.attempt.as_ref().unwrap().text
        );
        assert!(
            window
                .find("external-refresh-result")
                .label()
                .unwrap()
                .contains(expected)
        );
    })
    .unwrap();
    cx.executor().advance_clock(Duration::from_secs(60));
    cx.run_until_parked();
    assert_eq!(fake.patches(), 1);
    fake.assert_requests();
}

#[gpui_kit::test]
async fn submitted_patch_survives_hide_and_close_and_never_publishes_to_another_identity(
    cx: &mut TestAppContext,
) {
    let (runtime, pane, handle, _) = mount(cx);
    cx.executor().allow_parking();
    let (access, fake) = fake(&runtime, 200);
    cx.update(|cx| open(&pane, access, cx));
    loaded(&pane, handle, cx).await;
    dialog(&pane, handle, cx);
    cx.update_window(handle, |_, window, cx| {
        window.click("external-refresh-confirm", cx)
    })
    .unwrap();
    cx.wait_for_io(handle, |_, _| fake.patches() == 1).await;
    cx.update(|cx| {
        pane.update(cx, |pane, cx| {
            pane.set_active(false, cx);
            pane.close(cx);
        })
    });
    cx.run_until_parked();
    let read_count = fake.seen.lock().unwrap().len();
    fake.release.notify_one();
    cx.wait_for_io(handle, |_, cx| {
        pane.read(cx)
            .external_refresh
            .read(cx)
            .attempt
            .as_ref()
            .is_some_and(|a| a.phase == Phase::Waiting)
    })
    .await;
    cx.executor().advance_clock(Duration::from_secs(31));
    cx.run_until_parked();
    cx.update(|cx| {
        let owner = pane.read(cx).external_refresh.read(cx);
        assert_eq!(owner.attempt.as_ref().unwrap().phase, Phase::Finished);
        assert!(
            owner
                .attempt
                .as_ref()
                .unwrap()
                .text
                .contains("no resync observed")
        );
    });
    assert_eq!(fake.patches(), 1);
    assert_eq!(
        fake.seen.lock().unwrap().len(),
        read_count,
        "hidden/closed pane makes no observation reads"
    );
    fake.assert_requests();
    cx.update_window(handle, |_, window, cx| {
        pane.update(cx, |pane, cx| {
            pane.set_active(true, cx);
            let mut replacement = target();
            replacement.identity.connection = "new-access".into();
            pane.open(replacement, ClusterAccess::EXAMPLE, "1", Duration::ZERO, cx);
        });
        window.render_frame(cx);
        assert!(window.try_find("external-refresh-result").is_none());
    })
    .unwrap();
}

#[gpui_kit::test]
fn a_confirmation_cannot_retarget_changed_access_selection_or_deleted_objects(
    cx: &mut TestAppContext,
) {
    for change in 0..6 {
        let (_runtime, pane, handle, _) = mount(cx);
        cx.update(|cx| open(&pane, ClusterAccess::EXAMPLE, cx));
        dialog(&pane, handle, cx);
        cx.update_window(handle, |_, window, cx| {
            pane.update(cx, |pane, cx| match change {
                0 => pane.set_active(false, cx),
                1 => {
                    let mut changed = target();
                    changed.identity.connection = "changed-access".into();
                    pane.open(changed, ClusterAccess::EXAMPLE, "2", Duration::ZERO, cx);
                }
                2 => pane.gone(None, cx),
                3 => {
                    let mut replacement = target();
                    replacement.identity.uid = "invented-replacement".into();
                    pane.open(replacement, ClusterAccess::EXAMPLE, "2", Duration::ZERO, cx);
                }
                4 => pane.close(cx),
                _ => pane.set_context("different-context".into(), cx),
            });
            window.render_frame(cx);
            window.press("enter", cx);
            assert!(
                pane.read(cx).external_refresh.read(cx).attempt.is_none(),
                "change {change}"
            );
        })
        .unwrap();
    }
}

#[gpui_kit::test]
fn shift_r_does_not_open_refresh_from_yaml_search(cx: &mut TestAppContext) {
    let (_runtime, pane, handle, _) = mount(cx);
    cx.update(|cx| open(&pane, ClusterAccess::EXAMPLE, cx));
    cx.update_window(handle, |_, window, cx| {
        pane.update(cx, |pane, cx| pane.set_tab(crate::pane::Tab::Yaml, cx));
        window.render_frame(cx);
        window.click("detail-find", cx);
        window.press("shift-r", cx);
        window.render_frame(cx);
        assert!(window.try_find("external-refresh-confirmation").is_none());
        assert!(pane.read(cx).external_refresh.read(cx).attempt.is_none());
    })
    .unwrap();
}

#[gpui_kit::test]
fn same_identity_client_handle_renewal_keeps_an_open_confirmation_valid(cx: &mut TestAppContext) {
    let (_runtime, pane, handle, _) = mount(cx);
    cx.update(|cx| open(&pane, ClusterAccess::EXAMPLE, cx));
    dialog(&pane, handle, cx);
    cx.executor().advance_clock(Duration::from_secs(16));
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        pane.update(cx, |pane, cx| pane.set_access(ClusterAccess::EXAMPLE, cx));
        window.render_frame(cx);
        window.click("external-refresh-confirm", cx);
        assert!(
            pane.read(cx).external_refresh.read(cx).attempt.is_some(),
            "fresh handles for unchanged access must preserve confirmation"
        );
    })
    .unwrap();
}
