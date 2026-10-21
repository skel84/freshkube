use super::super::tests::mount;
use super::{
    ClusterAccess, DetailPane, DetailTarget, Duration, Failure, FailureKind, StoreRead, Tone,
    example,
};
use crate::example::external_secrets::KEY;
use freshkube_core::resources::StoreStatus;
use gpui_kit::test::TestWindowExt;
use gpui_kit::{App, AppContext, Entity, TestAppContext};

fn target(name: &str) -> DetailTarget {
    let (_, rows) = example::read("homelab", KEY, None, 1_767_312_000).unwrap();
    DetailTarget {
        identity: rows
            .into_iter()
            .find(|row| row.identity.name == name)
            .unwrap()
            .identity,
        kind: example::kind(KEY).unwrap(),
    }
}
fn open(pane: &Entity<DetailPane>, target: &DetailTarget, cx: &mut App) {
    pane.update(cx, |pane, cx| {
        pane.open(
            target.clone(),
            ClusterAccess::EXAMPLE,
            "1",
            Duration::ZERO,
            cx,
        )
    });
}

#[gpui_kit::test]
fn external_secrets_show_the_reported_cause_first_and_store_health(cx: &mut TestAppContext) {
    let (_runtime, pane, handle, _) = mount(cx);
    for (name, cause) in [
        ("lantern-api", "SecretSynced"),
        ("old-refresh", "Last reported sync"),
        ("broken-store", "not ready"),
        ("missing-entry", "not found"),
        ("access-denied", "Access denied"),
        ("owned-elsewhere", "another controller"),
        ("refused-store", "SecretSynced"),
        ("missing-store", "missing"),
        ("cluster-store", "SecretSynced"),
        ("on-change", "SecretSynced"),
        ("created-once", "SecretSynced"),
    ] {
        cx.update_window(handle, |_, window, cx| {
            open(&pane, &target(name), cx);
            window.render_frame(cx);
            let summary = window.find("external-secret-summary");
            assert!(summary.visible(), "{name}");
            assert_eq!(
                summary.bounds().top(),
                window.find("detail-overview").bounds().top()
            );
            assert!(
                window
                    .find("external-condition-Ready")
                    .label()
                    .unwrap()
                    .contains(cause),
                "{name}"
            );
            assert!(window.try_find("external-secret-refresh").is_none());
            assert!(window.try_find("detail-secret-reveal").is_none());
            let view = &pane.read(cx).external;
            assert_eq!(view.stores.len(), 1);
            if name == "broken-store" {
                assert_eq!(view.stores[0].tone, Tone::Crit);
            }
            if name == "refused-store" {
                assert!(view.stores[0].detail.contains("Not permitted"));
            }
            if name == "missing-store" {
                assert!(view.stores[0].detail.contains("meadow-missing not found"));
            }
            if name == "cluster-store" {
                assert!(
                    view.stores[0]
                        .label
                        .contains("ClusterSecretStore shared-meadow")
                );
            }
            if name == "on-change" {
                assert_eq!(view.policy, "OnChange");
            }
            if name == "created-once" {
                assert_eq!(view.policy, "CreatedOnce");
            }
            // Ordinary details/YAML navigation reads no target Secret.
            window.click("detail-tab-yaml", cx);
            window.render_frame(cx);
            assert!(window.try_find("external-secret-summary").is_none());
            window.click("detail-tab-details", cx);
            window.render_frame(cx);
            assert!(window.find("external-secret-summary").visible());
        })
        .unwrap();
    }
    cx.update_window(handle, |_, window, cx| {
        open(&pane, &target("unknown-status"), cx);
        window.render_frame(cx);
        assert!(window.find("external-unknown").visible());
        assert_eq!(pane.read(cx).external.refreshed, "Not refreshed yet");
    })
    .unwrap();
}

#[gpui_kit::test]
fn store_results_are_owned_by_identity_generation_and_visibility(cx: &mut TestAppContext) {
    let (_runtime, pane, handle, _) = mount(cx);
    let first = target("lantern-api");
    let second = target("broken-store");
    let (seq, reads) = cx.update(|cx| {
        open(&pane, &first, cx);
        (
            pane.read(cx).external.seq,
            pane.read(cx).external.previous.clone(),
        )
    });
    cx.update(|cx| {
        open(&pane, &second, cx);
        pane.update(cx, |pane, cx| {
            pane.finish_external_stores(&first.identity, seq, reads.clone(), cx)
        });
        assert_eq!(pane.read(cx).external.stores[0].tone, Tone::Crit);
        open(&pane, &first, cx);
        let before = pane.read(cx).external.seq;
        assert!(before > seq);
        let store = reads[0].store.clone();
        // An old generation for the SAME identity cannot overwrite it.
        pane.update(cx, |pane, cx| {
            pane.finish_external_stores(
                &first.identity,
                seq,
                vec![StoreRead::new(
                    store.clone(),
                    Err(Failure::new(FailureKind::Forbidden, "old refusal")),
                )],
                cx,
            )
        });
        assert_eq!(pane.read(cx).external.stores[0].tone, Tone::Good);
        // A current failure retains last-known Ready, visibly stale.
        pane.update(cx, |pane, cx| {
            pane.finish_external_stores(
                &first.identity,
                before,
                vec![StoreRead::new(
                    store.clone(),
                    Err(Failure::new(FailureKind::Forbidden, "current refusal")),
                )],
                cx,
            )
        });
        assert_eq!(pane.read(cx).external.stores[0].tone, Tone::Warn);
        assert!(pane.read(cx).external.stores[0].status.contains("stale"));
        assert!(
            pane.read(cx).external.stores[0]
                .detail
                .contains("current refusal")
        );
        pane.update(cx, |pane, cx| pane.set_active(false, cx));
        assert!(pane.read(cx).external.job.is_none());
        pane.update(cx, |pane, cx| {
            pane.finish_external_stores(&first.identity, before, reads.clone(), cx)
        });
        assert_eq!(pane.read(cx).external.stores[0].tone, Tone::Warn);
    });
    cx.executor().advance_clock(Duration::from_secs(60));
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        pane.update(cx, |pane, cx| pane.set_active(true, cx));
        window.render_frame(cx);
        assert_eq!(pane.read(cx).external.stores[0].tone, Tone::Good);
        let seq = pane.read(cx).external.seq;
        pane.update(cx, |pane, cx| pane.close(cx));
        pane.update(cx, |pane, cx| {
            pane.finish_external_stores(&first.identity, seq, reads, cx)
        });
        assert!(pane.read(cx).external.secret.is_none());
    })
    .unwrap();
}

#[gpui_kit::test]
fn store_loading_and_unknown_conditions_keep_sync_visible(cx: &mut TestAppContext) {
    let (_runtime, pane, handle, _) = mount(cx);
    cx.update_window(handle, |_, window, cx| {
        let target = target("lantern-api");
        open(&pane, &target, cx);
        pane.update(cx, |pane, _| {
            let secret = pane.external.secret.clone().unwrap();
            pane.external.reset();
            pane.external.set_secret(secret);
        });
        window.render_frame(cx);
        assert!(window.find("external-condition-Ready").visible());
        assert_eq!(pane.read(cx).external.stores[0].status, "Reading");
        let store = pane.read(cx).external.secret.as_ref().unwrap().stores[0].clone();
        pane.update(cx, |pane, cx| {
            let seq = pane.external.seq;
            pane.finish_external_stores(
                &target.identity,
                seq,
                vec![StoreRead::new(store, Ok(StoreStatus::new(None)))],
                cx,
            );
        });
        assert_eq!(pane.read(cx).external.stores[0].status, "Unknown");
        assert_eq!(pane.read(cx).external.stores[0].tone, Tone::Unknown);
    })
    .unwrap();
}

#[gpui_kit::test]
async fn external_secret_pane_fake_api_rejects_every_secret_credential_or_mutation_request(
    cx: &mut TestAppContext,
) {
    use freshkube_core::cluster_source::KubeClientSource;
    use freshkube_ui::real_io::RealIo;
    use http::{Request, Response};
    use kube::client::Body;
    use std::sync::{Arc, Mutex};
    struct Source(kube::Client);
    impl KubeClientSource for Source {
        fn client(&self) -> futures::future::BoxFuture<'_, Result<kube::Client, Failure>> {
            let client = self.0.clone();
            Box::pin(async move { Ok(client) })
        }
    }
    let (_runtime, pane, handle, _) = mount(cx);
    cx.executor().allow_parking();
    let target = target("lantern-api");
    let yaml = example::document(&target.identity, 1_767_312_000)
        .unwrap()
        .yaml;
    let seen = Arc::new(Mutex::new(Vec::new()));
    let requests = seen.clone();
    let api = tower::service_fn(move |request: Request<Body>| {
        let path = request.uri().path().to_owned();
        requests
            .lock()
            .unwrap()
            .push((request.method().clone(), path.clone()));
        assert_eq!(request.method(), http::Method::GET, "unexpected mutation");
        let body = match path.as_str() {
            "/apis/external-secrets.io/v1/namespaces/paper-kites/externalsecrets/lantern-api" => {
                serde_yaml::from_str::<serde_json::Value>(&yaml).unwrap()
            }
            "/apis" => {
                serde_json::json!({"groups":[{"name":"external-secrets.io","versions":[{"version":"v1"}]}]})
            }
            "/apis/external-secrets.io/v1" => {
                serde_json::json!({"resources":[{"name":"secretstores","kind":"SecretStore","namespaced":true,"verbs":["get","list","watch"]}]})
            }
            "/apis/external-secrets.io/v1/namespaces/paper-kites/secretstores/meadow-store" => {
                serde_json::json!({"spec":{"provider":{"auth":{"token":"invented-sensitive-sentinel","secretRef":{"name":"provider-auth"}}}},"status":{"conditions":[{"type":"Ready","status":"True","reason":"Valid","message":"Store validated"}]}})
            }
            "/api/v1/namespaces/paper-kites/events" => serde_json::json!({}),
            _ => panic!(
                "Unexpected request: Secret, credential and provider routes are forbidden: {path}"
            ),
        };
        async move {
            if path == "/api/v1/namespaces/paper-kites/events" {
                return futures::future::pending::<Result<Response<Body>, std::convert::Infallible>>().await;
            }
            Ok::<_, std::convert::Infallible>(Response::new(Body::from(
                body.to_string().into_bytes(),
            )))
        }
    });
    let client = {
        let _entered = _runtime.enter();
        kube::Client::new(api, "paper-kites")
    };
    cx.update(|cx| {
        pane.update(cx, |pane, cx| {
            pane.open(
                target.clone(),
                ClusterAccess::Live(Arc::new(Source(client))),
                "1",
                Duration::ZERO,
                cx,
            )
        })
    });
    cx.wait_for_io(handle, |_, cx| {
        pane.read(cx)
            .external
            .stores
            .first()
            .is_some_and(|line| line.tone == Tone::Good)
    })
    .await;
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(window.find("external-secret-summary").visible());
        let doc = &pane
            .read(cx)
            .detail
            .as_ref()
            .unwrap()
            .view
            .as_ref()
            .unwrap()
            .document;
        assert!(!doc.yaml.contains("invented-sensitive-sentinel"));
        assert!(!doc.yaml.contains("provider-auth"));
        pane.update(cx, |pane, cx| {
            pane.set_query("invented-sensitive-sentinel".into(), cx)
        });
        assert!(pane.read(cx).matches.is_empty());
        window.click("detail-copy-yaml", cx);
        assert!(
            !cx.read_from_clipboard()
                .unwrap()
                .text()
                .unwrap()
                .contains("invented-sensitive-sentinel")
        );
        assert!(window.try_find("external-secret-refresh").is_none());
        pane.update(cx, |pane, cx| pane.set_active(false, cx));
        assert!(pane.read(cx).external.job.is_none());
    })
    .unwrap();
    let count = seen.lock().unwrap().len();
    cx.executor().advance_clock(Duration::from_secs(60));
    cx.run_until_parked();
    assert_eq!(
        seen.lock().unwrap().len(),
        count,
        "hidden panes must not read"
    );
    let seen = seen.lock().unwrap();
    assert_eq!(seen.len(), 5, "only the approved read routes are allowed");
    assert!(
        seen.iter()
            .any(|(_, path)| path.ends_with("secretstores/meadow-store"))
    );
    assert!(seen.iter().all(|(verb, path)| {
        *verb == http::Method::GET
            && !path
                .split('/')
                .any(|part| matches!(part, "secrets" | "credentials" | "token"))
    }));
}

#[gpui_kit::test]
fn external_store_replies_cannot_cross_access_sessions_or_deleted_objects(cx: &mut TestAppContext) {
    use crate::detail::DocumentView;
    let (_runtime, pane, _, _) = mount(cx);
    let original = target("lantern-api");
    cx.update(|cx| {
        open(&pane, &original, cx);
        let seq = pane.read(cx).external.seq;
        let replies: Vec<_> = pane
            .read(cx)
            .external
            .previous
            .iter()
            .map(|read| {
                StoreRead::new(
                    read.store.clone(),
                    Err(Failure::new(FailureKind::Forbidden, "old access refusal")),
                )
            })
            .collect();
        let mut replacement = original.clone();
        replacement.identity.connection = "invented-other-access-session".into();
        // A changed access identity opens loading; the example has no object
        // under that identity, so deliver the same invented object explicitly.
        pane.update(cx, |pane, cx| {
            pane.open(
                replacement.clone(),
                ClusterAccess::EXAMPLE,
                "1",
                Duration::from_secs(1),
                cx,
            )
        });
        pane.update(cx, |pane, cx| {
            let current = pane.read_seq;
            pane.finish_read(
                &replacement.identity,
                current,
                Ok(DocumentView::new(
                    example::document(&original.identity, 1_767_312_000).unwrap(),
                )),
                cx,
            );
            pane.finish_external_stores(&original.identity, pane.external.seq, replies.clone(), cx);
            pane.finish_external_stores(&replacement.identity, seq, replies.clone(), cx);
            assert_eq!(pane.target_identity(), Some(&replacement.identity));
            assert_eq!(pane.external.stores[0].tone, Tone::Good);
            // Marking the document deleted drops the store generation too.
            let before = pane.external.seq;
            pane.finish_read(
                &replacement.identity,
                current,
                Err(Failure::new(
                    FailureKind::NotFound,
                    "Invented object deleted",
                )),
                cx,
            );
            assert!(pane.external.seq > before);
            assert!(pane.external.job.is_none());
            pane.finish_external_stores(&replacement.identity, before, replies.clone(), cx);
            pane.finish_external_stores(&replacement.identity, pane.external.seq, replies, cx);
            assert_eq!(pane.external.stores[0].tone, Tone::Good);
        });
    });
}

#[test]
fn repeated_documents_keep_store_rows_until_the_references_change() {
    use super::ExternalView;
    let secret = example::document(&target("lantern-api").identity, 1_767_312_000)
        .unwrap()
        .overview
        .external_secret
        .unwrap();
    let mut view = ExternalView::default();
    view.set_secret(secret.clone());
    view.finish(example::external_secrets::stores(&secret));
    let shown = view.stores[0].accessible.clone();
    view.set_secret(secret.clone());
    assert_eq!(view.stores[0].accessible, shown);
    let store = secret.stores[0].clone();
    view.finish(vec![StoreRead::new(
        store,
        Err(Failure::new(FailureKind::Forbidden, "Invented refusal")),
    )]);
    let stale = view.stores[0].accessible.clone();
    view.set_secret(secret);
    assert_eq!(view.stores[0].accessible, stale);
    let other = example::document(&target("broken-store").identity, 1_767_312_000)
        .unwrap()
        .overview
        .external_secret
        .unwrap();
    view.set_secret(other);
    assert_eq!(view.stores[0].status, "Reading");
    assert!(view.previous.is_empty());
}

#[test]
fn missing_stores_have_a_specific_heading_and_stale_failures_stay_red() {
    use super::ExternalView;
    let mut view = ExternalView::default();
    let secret = example::document(&target("missing-store").identity, 1_767_312_000)
        .unwrap()
        .overview
        .external_secret
        .unwrap();
    view.set_secret(secret.clone());
    view.finish(example::external_secrets::stores(&secret));
    assert!(view.stores[0].detail.starts_with("Store not found:"));
    let broken = example::document(&target("broken-store").identity, 1_767_312_000)
        .unwrap()
        .overview
        .external_secret
        .unwrap();
    view.set_secret(broken.clone());
    view.finish(example::external_secrets::stores(&broken));
    view.finish(vec![StoreRead::new(
        broken.stores[0].clone(),
        Err(Failure::new(FailureKind::Forbidden, "Invented refusal")),
    )]);
    assert_eq!(view.stores[0].tone, Tone::Crit);
    assert!(view.stores[0].status.contains("stale"));
}

#[test]
fn defaults_are_labelled_and_both_incomplete_source_notes_are_shown() {
    use super::ExternalView;
    use freshkube_core::resources::object_from_yaml;
    let mut secret = object_from_yaml(
        &target("lantern-api").kind,
        "metadata:\n  namespace: paper-kites\nspec: {}\nstatus: {}\n",
    )
    .unwrap()
    .overview
    .external_secret
    .unwrap();
    secret.unsupported_sources = true;
    secret.stores_omitted = 7;
    let mut view = ExternalView::default();
    view.set_secret(secret);
    assert_eq!(view.interval, "1h (default)");
    assert_eq!(view.policy, "Periodic (default)");
    let note = view.note.as_ref().unwrap();
    assert!(note.contains("health is unknown"));
    assert!(note.contains("7 more stores not read"));
}

#[test]
fn absent_api_and_empty_store_causes_are_explicit() {
    use super::ExternalView;
    let identity = target("missing-store").identity;
    let secret = example::document(&identity, 1_767_312_000)
        .unwrap()
        .overview
        .external_secret
        .unwrap();
    let mut reads = example::external_secrets::stores(&secret);
    reads[0].api_absent = true;
    let mut view = ExternalView::default();
    view.set_secret(secret);
    view.finish(reads.clone());
    assert!(view.stores[0].detail.starts_with("Store API not served:"));
    reads[0].api_absent = false;
    reads[0].status = Ok(freshkube_core::resources::StoreStatus::new(Some(
        freshkube_core::resources::Condition {
            kind: "Ready".into(),
            status: "True".into(),
            reason: String::new(),
            message: String::new(),
            changed: None,
        },
    )));
    view.finish(reads);
    assert!(view.stores[0].detail.is_empty());
    assert!(!view.stores[0].accessible.ends_with(" · "));
}
