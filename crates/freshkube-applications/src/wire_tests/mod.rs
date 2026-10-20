//! The app constructor, setup hand-off and both delivery read jobs on
//! invented data. The signature is the existing offline cosign fixture.
mod fake;

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
        fake::access(live_runs, gate)
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
        assert!(hop.why.contains("agrees with"));
    })
    .unwrap();
    let change = open_change(cx, window, &app);
    cx.wait_for_io(window, |_, cx| change.read(cx).answered())
        .await;
    cx.update_window(window, |_, _, cx| {
        let hop = supply(change.read(cx));
        assert_eq!(hop.link, Some(Confidence::Confirmed));
        assert!(hop.detail.contains("agrees with"));
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
        assert!(supply(change.read(cx)).detail.contains("agrees with"));
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
            page.loader
                .start("invented-connection/checkout/freight-a/setup/1".into())
        });
        app.update(cx, |page, cx| page.set_setup(&configured, cx));
        change.update(cx, |page, cx| {
            let super::change::Fetch::Live { setup_revision, .. } = &page.fetch else {
                panic!()
            };
            page.loader.start(format!(
                "invented-connection/checkout/freight-a/setup/{setup_revision}"
            ));
            assert!(!page.loader.land(&old_request, Ok(old), cx));
        });
    })
    .unwrap();
}
