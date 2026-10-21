//! The app constructor, setup hand-off and both delivery read jobs on
//! invented data. The signature is the existing offline cosign fixture.
mod fake;
mod results_forward;

use super::{ApplicationsPage, ChangesPage, Clusters, Variant, change::ChangePage};
use freshkube_core::secrets::{MemoryStore, SecretStore, Secrets};
use freshkube_core::setup::{KeysJson, Setup, SupplyChain};
use freshkube_core::workspace::{Entry, Role, Workspace};
use freshkube_delivery::change::live::Mapping;
use freshkube_delivery::join::Confidence;
use freshkube_ui::real_io::{RealIo, until_io};
use gpui_kit::{AnyWindowHandle, AppContext, Entity, TestAppContext, component::Root, px, size};
use std::sync::{Arc, atomic::Ordering};

fn setup(
    key: Option<std::path::PathBuf>,
    results: Option<&fake::Results>,
    evidence: bool,
) -> Setup {
    let mut entry = Entry::new("cluster-b", Role::Cicd, "cluster-b");
    if let Some(results) = results {
        let mut settings = freshkube_core::setup::Results::default();
        settings.address = Some(results.address.clone());
        settings.token = Some(KeysJson);
        entry.results = Some(Box::new(settings));
    }
    let mut setup = Setup::new(Workspace::new(vec![entry]));
    if key.is_some() || evidence {
        let mut chain = SupplyChain::default();
        chain.public_key_file = key;
        if evidence {
            chain.evidence = Some(
                serde_json::from_value(serde_json::json!({
                    "result":"ACME_EVIDENCE", "commit":"/source/commit", "digest":"/output/digest"
                }))
                .unwrap(),
            );
        }
        setup.supply_chain = Some(chain);
    }
    setup
}

fn mount(
    cx: &mut TestAppContext,
    runtime: &tokio::runtime::Runtime,
    setup: &Setup,
    secrets: Option<Secrets>,
    live_runs: bool,
    gate: Arc<fake::Gate>,
) -> (AnyWindowHandle, Entity<ApplicationsPage>) {
    mount_access(cx, runtime, setup, secrets, live_runs, gate, None)
}

fn mount_access(
    cx: &mut TestAppContext,
    runtime: &tokio::runtime::Runtime,
    setup: &Setup,
    secrets: Option<Secrets>,
    live_runs: bool,
    gate: Arc<fake::Gate>,
    access: Option<freshkube_core::cluster_source::ClusterAccess>,
) -> (AnyWindowHandle, Entity<ApplicationsPage>) {
    cx.executor().allow_parking();
    cx.update(|cx| {
        gpui_kit::init(cx);
        freshkube_ui::theme::install(cx);
        freshkube_ui::text_size::install(None, cx);
        cx.set_reduce_motion(true);
        cx.bind_keys(super::key_bindings());
    });
    let access = {
        let _entered = runtime.enter();
        access.unwrap_or_else(|| fake::access(live_runs, gate))
    };
    let mut app = None;
    let window = cx.open_window(size(px(1280.), px(880.)), |window, cx| {
        let page = cx.new(|cx| {
            let mut page =
                ApplicationsPage::new(runtime.handle().clone(), Variant::Acme, secrets, window, cx);
            page.set_setup(setup, cx);
            page.set_clusters(
                Clusters {
                    entries: vec!["cluster-b".into()],
                    active: Some("cluster-b".into()),
                    mapping: Mapping {
                        open: Some("cluster-b".into()),
                        ..Default::default()
                    },
                    sources: freshkube_core::workspace::Sources::of(&setup.workspace),
                    kubeconfig: None,
                },
                cx,
            );
            page.set_source(
                Some(freshkube_core::cluster_source::ClusterSource {
                    id: "invented-connection".into(),
                    context: "cluster-b".into(),
                    access,
                }),
                Some("cluster-b".into()),
                cx,
            );
            page.set_visible(true, cx);
            page.show_changes(window, cx);
            page
        });
        app = Some(page.clone());
        Root::new(page, window, cx)
    });
    (window.into(), app.unwrap())
}

fn list(app: &Entity<ApplicationsPage>, cx: &gpui_kit::App) -> Entity<ChangesPage> {
    app.read(cx).changes.clone()
}
fn supply(page: &ChangePage) -> &freshkube_delivery::change::Hop {
    page.change
        .hops
        .iter()
        .find(|hop| hop.name == "Supply chain")
        .expect("a supply-chain hop")
}
async fn wait_list(
    cx: &mut TestAppContext,
    window: AnyWindowHandle,
    app: &Entity<ApplicationsPage>,
) {
    cx.wait_for_io(window, |_, cx| {
        let list = list(app, cx);
        !list.read(cx).is_reading() && !list.read(cx).display.rows.is_empty()
    })
    .await;
}
fn open_change(
    cx: &mut TestAppContext,
    window: AnyWindowHandle,
    app: &Entity<ApplicationsPage>,
) -> Entity<ChangePage> {
    cx.update_window(window, |_, window, cx| {
        let list = list(app, cx);
        let key = list.read(cx).display.rows[0].key.clone();
        list.update(cx, |page, cx| {
            page.select(&key, window, cx);
            page.focus(window, cx);
        });
        window.dispatch_action(Box::new(super::changes::OpenChange), cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(window, |_, _, cx| {
        list(app, cx)
            .read(cx)
            .change_page()
            .cloned()
            .expect("the selected change opened")
    })
    .unwrap()
}

#[gpui_kit::test]
async fn setup_key_and_evidence_reach_both_delivery_reads(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    let key = dir.path().join("checkout.pub");
    std::fs::write(&key, fake::KEY).unwrap();
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let configured = setup(Some(key), None, true);
    let (window, app) = mount(cx, &runtime, &configured, None, true, Arc::default());
    wait_list(cx, window, &app).await;
    cx.update_window(window, |_, _, cx| {
        let list = list(&app, cx);
        let trail = list.read(cx).display.rows[0].trail.as_ref().unwrap();
        let hop = trail
            .iter()
            .flat_map(|group| &group.hops)
            .find(|hop| hop.name == "Supply chain")
            .unwrap();
        assert_eq!(hop.link, Some("Confirmed"));
        assert!(hop.why.contains("evidence record agrees with"));
    })
    .unwrap();
    let change = open_change(cx, window, &app);
    cx.wait_for_io(window, |_, cx| change.read(cx).answered())
        .await;
    cx.update_window(window, |_, _, cx| {
        let hop = supply(change.read(cx));
        assert_eq!(hop.link, Some(Confidence::Confirmed));
        assert!(hop.detail.contains("evidence record agrees with"));
    })
    .unwrap();
}

#[gpui_kit::test]
async fn results_token_from_the_app_store_reaches_both_read_jobs(cx: &mut TestAppContext) {
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let results = fake::Results::new(runtime.handle());
    let store = Arc::new(MemoryStore::default());
    store
        .write(&format!("Tekton Results {}/", results.address), fake::TOKEN)
        .unwrap();
    let configured = setup(None, Some(&results), false);
    let (window, app) = mount(
        cx,
        &runtime,
        &configured,
        Some(store.clone()),
        false,
        Arc::default(),
    );
    wait_list(cx, window, &app).await;
    assert_eq!(results.authenticated.load(Ordering::SeqCst), 3);
    let change = open_change(cx, window, &app);
    cx.wait_for_io(window, |_, cx| change.read(cx).answered())
        .await;
    assert_eq!(results.authenticated.load(Ordering::SeqCst), 6);
    let replacement = "invented-replacement-token";
    store
        .write(&format!("Tekton Results {}/", results.address), replacement)
        .unwrap();
    results.replace_token(replacement);
    cx.update_window(window, |_, _, cx| {
        app.update(cx, |page, cx| page.set_setup(&configured, cx))
    })
    .unwrap();
    wait_list(cx, window, &app).await;
    cx.wait_for_io(window, |_, cx| change.read(cx).answered())
        .await;
    assert_eq!(results.authenticated.load(Ordering::SeqCst), 12);

    cx.update_window(window, |_, _, cx| {
        assert!(
            change
                .read(cx)
                .change
                .hops
                .iter()
                .any(|hop| hop.name == "Run checkout-7")
        );
        assert!(
            !change
                .read(cx)
                .change
                .missing
                .iter()
                .any(|gap| gap.text.contains("key store"))
        );
    })
    .unwrap();
}

#[gpui_kit::test]
async fn reload_replaces_the_key_and_cancels_the_old_read(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    let key = dir.path().join("checkout.pub");
    std::fs::write(&key, fake::KEY).unwrap();
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let gate = Arc::new(fake::Gate::default());
    let mut configured = setup(Some(key.clone()), None, false);
    let (window, app) = mount(cx, &runtime, &configured, None, true, gate.clone());
    wait_list(cx, window, &app).await;
    let change = open_change(cx, window, &app);
    cx.wait_for_io(window, |_, cx| change.read(cx).answered())
        .await;
    cx.update_window(window, |_, _, cx| {
        assert_eq!(supply(change.read(cx)).link, Some(Confidence::Confirmed))
    })
    .unwrap();
    // Hold an old-setup refresh after it has started. The replacement read
    // must use new trust, even if the cancelled request later answers.
    let previous = gate.asked.load(Ordering::SeqCst);
    gate.hold.store(true, Ordering::SeqCst);
    cx.update_window(window, |_, _, cx| {
        change.update(cx, |page, cx| page.refresh(cx))
    })
    .unwrap();
    until_io(cx, "old setup's run request", |_| {
        gate.asked.load(Ordering::SeqCst) > previous
    });
    let other = dir.path().join("other.pub");
    std::fs::write(&other, fake::OTHER_KEY).unwrap();
    configured.supply_chain.as_mut().unwrap().public_key_file = Some(other);
    gate.hold.store(false, Ordering::SeqCst);
    cx.update_window(window, |_, _, cx| {
        app.update(cx, |page, cx| page.set_setup(&configured, cx))
    })
    .unwrap();
    cx.wait_for_io(window, |_, cx| change.read(cx).answered())
        .await;
    gate.release.notify_waiters();
    cx.run_until_parked();
    cx.update_window(window, |_, _, cx| {
        assert_eq!(supply(change.read(cx)).link, Some(Confidence::Claimed))
    })
    .unwrap();
    // Reload the same path after replacing its contents too.
    std::fs::write(&key, fake::OTHER_KEY).unwrap();
    configured.supply_chain.as_mut().unwrap().public_key_file = Some(key.clone());
    cx.update_window(window, |_, _, cx| {
        app.update(cx, |page, cx| page.set_setup(&configured, cx))
    })
    .unwrap();
    cx.wait_for_io(window, |_, cx| change.read(cx).answered())
        .await;
    cx.update_window(window, |_, _, cx| {
        assert_eq!(supply(change.read(cx)).link, Some(Confidence::Claimed));
    })
    .unwrap();
    std::fs::write(&key, fake::KEY).unwrap();
    cx.update_window(window, |_, _, cx| {
        app.update(cx, |page, cx| page.set_setup(&configured, cx))
    })
    .unwrap();
    cx.wait_for_io(window, |_, cx| change.read(cx).answered())
        .await;
    cx.update_window(window, |_, _, cx| {
        assert_eq!(supply(change.read(cx)).link, Some(Confidence::Confirmed))
    })
    .unwrap();
}

#[gpui_kit::test]
async fn the_stage_naming_reaches_the_list_and_the_open_change_on_reload(cx: &mut TestAppContext) {
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let mut configured = setup(None, None, false);
    let (window, app) = mount(cx, &runtime, &configured, None, false, Arc::default());
    wait_list(cx, window, &app).await;
    let change = open_change(cx, window, &app);
    cx.wait_for_io(window, |_, cx| change.read(cx).answered())
        .await;
    configured.delivery = Some(freshkube_core::setup::Delivery::naming(
        freshkube_core::setup::StageNaming::new("{stage}-{project}"),
    ));
    cx.update_window(window, |_, _, cx| {
        app.update(cx, |page, cx| page.set_setup(&configured, cx))
    })
    .unwrap();
    // The list derives its projects from the naming, so it reads again.
    cx.update_window(window, |_, _, cx| {
        assert!(app.read(cx).loader.is_loading());
    })
    .unwrap();
    wait_list(cx, window, &app).await;
    cx.update_window(window, |_, _, cx| {
        let expected = Some("{stage}-{project}");
        let page = app.read(cx);
        let of = |naming: &Option<freshkube_core::setup::StageNaming>| {
            naming.as_ref().map(|naming| naming.name_template.clone())
        };
        assert_eq!(of(&page.stage_naming).as_deref(), expected);
        // The change page's own read.
        match &change.read(cx).fetch {
            crate::change::Fetch::Live { place, .. } => {
                assert_eq!(of(&place.stage_naming).as_deref(), expected)
            }
            _ => panic!("the change reads live"),
        }
    })
    .unwrap();
    // Removing the key turns it off again.
    configured.delivery = None;
    cx.update_window(window, |_, _, cx| {
        app.update(cx, |page, cx| page.set_setup(&configured, cx))
    })
    .unwrap();
    cx.update_window(window, |_, _, cx| {
        let page = app.read(cx);
        assert!(page.stage_naming.is_none());
        assert!(page.loader.is_loading());
    })
    .unwrap();
    wait_list(cx, window, &app).await;
}

#[gpui_kit::test]
async fn a_configured_token_without_a_store_keeps_the_named_error(cx: &mut TestAppContext) {
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let results = fake::Results::new(runtime.handle());
    let configured = setup(None, Some(&results), false);
    let (window, app) = mount(cx, &runtime, &configured, None, false, Arc::default());
    wait_list(cx, window, &app).await;
    let change = open_change(cx, window, &app);
    cx.wait_for_io(window, |_, cx| change.read(cx).answered())
        .await;
    cx.update_window(window, |_, _, cx| {
        assert!(change.read(cx).change.missing.iter().any(|gap| {
            gap.text
                .contains("the key store isn't available to this read")
        }));
    })
    .unwrap();
    assert_eq!(results.authenticated.load(Ordering::SeqCst), 0);
}

#[gpui_kit::test]
async fn the_application_stage_hands_its_key_and_store_to_the_change(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    let key = dir.path().join("checkout.pub");
    std::fs::write(&key, fake::KEY).unwrap();
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let results = fake::Results::new(runtime.handle());
    let store = Arc::new(MemoryStore::default());
    store
        .write(&format!("Tekton Results {}/", results.address), fake::TOKEN)
        .unwrap();
    let configured = setup(Some(key), Some(&results), true);
    let (window, app) = mount(
        cx,
        &runtime,
        &configured,
        Some(store),
        false,
        Arc::default(),
    );
    cx.wait_for_io(window, |_, cx| app.read(cx).loader.data().is_some())
        .await;
    let change = cx
        .update_window(window, |_, window, cx| {
            app.update(cx, |page, cx| {
                page.open_named("checkout", window, cx);
                page.follow("dev", window, cx);
                page.change
                    .as_ref()
                    .map(|(change, _)| change.clone())
                    .expect("the application's Stage opened its change")
            })
        })
        .unwrap();
    cx.wait_for_io(window, |_, cx| change.read(cx).answered())
        .await;
    cx.update_window(window, |_, _, cx| {
        assert_eq!(supply(change.read(cx)).link, Some(Confidence::Confirmed));
        assert!(
            supply(change.read(cx))
                .detail
                .contains("evidence record agrees with")
        );
    })
    .unwrap();
    assert!(results.authenticated.load(Ordering::SeqCst) >= 3);
}

#[gpui_kit::test]
async fn a_hidden_old_setup_ticket_cannot_publish_after_reload(cx: &mut TestAppContext) {
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let configured = setup(None, None, false);
    let (window, app) = mount(cx, &runtime, &configured, None, true, Arc::default());
    wait_list(cx, window, &app).await;
    let change = open_change(cx, window, &app);
    cx.wait_for_io(window, |_, cx| change.read(cx).answered())
        .await;
    cx.update_window(window, |_, _, cx| {
        app.update(cx, |page, cx| page.set_visible(false, cx));
        let old = change.read(cx).change.clone();
        // A completion queued by an old read, with the same change and the
        // first request generation. Reset alone would reuse that generation.
        let old_request = change.update(cx, |page, _| {
            page.loader.reset();
            page.loader.start(page.fetch.identity())
        });
        app.update(cx, |page, cx| page.set_setup(&configured, cx));
        change.update(cx, |page, cx| {
            page.loader.start(page.fetch.identity());
            assert!(!page.loader.land(&old_request, Ok(old), cx));
        });
    })
    .unwrap();
}

#[gpui_kit::test]
async fn a_remap_keeps_the_last_good_change_on_refresh_failure(cx: &mut TestAppContext) {
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let gate = Arc::new(fake::Gate::default());
    let configured = setup(None, None, false);
    let (window, app) = mount(cx, &runtime, &configured, None, true, gate.clone());
    wait_list(cx, window, &app).await;
    let change = open_change(cx, window, &app);
    cx.wait_for_io(window, |_, cx| change.read(cx).answered())
        .await;
    gate.hold.store(true, Ordering::SeqCst);
    cx.update_window(window, |_, window, cx| {
        let before = change.read(cx).fetch.identity();
        let connections = app.read(cx).base_connections();
        let mut across = app.read(cx).across();
        across.mapping.entries.push("cluster-c".into());
        change.update(cx, |page, cx| {
            page.set_clusters(across, connections, cx);
            assert_eq!(
                page.fetch.identity(),
                before,
                "a remap keeps the setup identity"
            );
            assert!(page.is_reading());
            assert!(
                page.answered(),
                "the previous read stays available during refresh"
            );
            page.fail_read("invented refresh failure", window, cx);
            assert!(page.answered());
            assert!(page.loader.is_stale());
            assert_eq!(page.loader.error(), Some("invented refresh failure"));
        });
    })
    .unwrap();
}

#[gpui_kit::test]
async fn setup_build_namespace_reaches_every_delivery_read(cx: &mut TestAppContext) {
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let mut configured = setup(None, None, false);
    let (window, app) = mount(cx, &runtime, &configured, None, true, Arc::default());
    wait_list(cx, window, &app).await;
    let change = open_change(cx, window, &app);
    cx.update_window(window, |_, _, cx| {
        assert_eq!(app.read(cx).across().build_namespace, None);
        assert_eq!(change.read(cx).build_namespace(), None);
    })
    .unwrap();
    let mut delivery = freshkube_core::setup::Delivery::default();
    delivery.build_namespace = Some("acme-builds".into());
    configured.delivery = Some(delivery);
    cx.update_window(window, |_, _, cx| {
        app.update(cx, |page, cx| page.set_setup(&configured, cx));
        assert_eq!(
            app.read(cx).across().build_namespace.as_deref(),
            Some("acme-builds")
        );
        assert_eq!(change.read(cx).build_namespace(), Some("acme-builds"));
    })
    .unwrap();
    // A reload that drops the key drops it from the reads too.
    configured.delivery = None;
    cx.update_window(window, |_, _, cx| {
        app.update(cx, |page, cx| page.set_setup(&configured, cx));
        assert_eq!(app.read(cx).across().build_namespace, None);
        assert_eq!(change.read(cx).build_namespace(), None);
    })
    .unwrap();
}

fn assert_log_handoff(
    app: &Entity<ApplicationsPage>,
    change: &Entity<ChangePage>,
    enabled: bool,
    url: Option<&str>,
    cx: &gpui_kit::App,
) {
    let source = app.read(cx).source.as_ref().unwrap();
    let at = list(app, cx).read(cx).where_to_read(source);
    assert_eq!(at.transparency_log, enabled);
    assert_eq!(at.transparency_url.as_deref(), url);
    let super::change::Fetch::Live { place, .. } = &change.read(cx).fetch else {
        panic!("the invented live read");
    };
    assert_eq!(place.transparency_log, enabled);
    assert_eq!(place.transparency_url.as_deref(), url);
}

#[gpui_kit::test]
async fn log_setup_reaches_both_reads_and_reloads_replace_their_requests(cx: &mut TestAppContext) {
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let gate = Arc::new(fake::Gate::default());
    let mut configured = setup(None, None, false);
    let (window, app) = mount(cx, &runtime, &configured, None, true, gate.clone());
    wait_list(cx, window, &app).await;
    let change = open_change(cx, window, &app);
    cx.wait_for_io(window, |_, cx| change.read(cx).answered())
        .await;
    cx.update_window(window, |_, _, cx| {
        assert_log_handoff(&app, &change, false, None, cx)
    })
    .unwrap();

    for (enabled, url) in [
        (true, Some("https://rekor.example.test/log")),
        (true, Some("https://other-log.example.test")),
        (true, None),
        (false, None),
    ] {
        // Parse the actual file shape. No key means no attestation can
        // verify, so the application test never contacts either address.
        let text = format!(
            "version: 1\nsupply_chain:\n  transparency_log: {enabled}\n{}",
            url.map(|url| format!("  transparency_url: {url}\n"))
                .unwrap_or_default()
        );
        configured.supply_chain = freshkube_core::setup::parse(text.as_bytes())
            .unwrap()
            .supply_chain;
        gate.hold.store(true, Ordering::SeqCst);
        let previous = gate.asked.load(Ordering::SeqCst);
        cx.update_window(window, |_, _, cx| {
            let before = change.read(cx).fetch.identity();
            app.update(cx, |page, cx| page.set_setup(&configured, cx));
            assert_log_handoff(&app, &change, enabled, url, cx);
            assert_ne!(change.read(cx).fetch.identity(), before);
            assert!(change.read(cx).is_reading());
            assert!(list(&app, cx).read(cx).is_reading());
        })
        .unwrap();
        until_io(cx, "both replacement delivery reads", |_| {
            gate.asked.load(Ordering::SeqCst) >= previous + 2
        });
        gate.hold.store(false, Ordering::SeqCst);
        gate.release.notify_waiters();
        wait_list(cx, window, &app).await;
        cx.wait_for_io(window, |_, cx| change.read(cx).answered())
            .await;
    }
}

#[gpui_kit::test]
async fn an_applications_stage_hands_the_log_setup_to_its_change(cx: &mut TestAppContext) {
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let mut configured = setup(None, None, false);
    configured.supply_chain = Some(freshkube_core::setup::parse(
        b"version: 1\nsupply_chain: {transparency_log: true, transparency_url: 'https://rekor.example.test'}\n"
    ).unwrap().supply_chain.unwrap());
    let (window, app) = mount(cx, &runtime, &configured, None, true, Arc::default());
    cx.wait_for_io(window, |_, cx| app.read(cx).loader.data().is_some())
        .await;
    let change = cx
        .update_window(window, |_, window, cx| {
            app.update(cx, |page, cx| {
                page.open_named("checkout", window, cx);
                page.follow("dev", window, cx);
                page.change.as_ref().unwrap().0.clone()
            })
        })
        .unwrap();
    cx.wait_for_io(window, |_, cx| change.read(cx).answered())
        .await;
    cx.update_window(window, |_, _, cx| {
        assert_log_handoff(&app, &change, true, Some("https://rekor.example.test"), cx);
    })
    .unwrap();
}
