use super::{
    Check, Field, Pilot, Resolved, SettingsPage, SettingsSection, Verdict, form, setup, ui,
};
use crate::{
    GpuiOptions,
    desktop::{
        layout_check,
        testing::{fixture, mount},
    },
};
use freshkube_ui::real_io::RealIo;
use gpui_kit::{AnyWindowHandle, TestAppContext, test::TestWindowExt};
use gpui_kit::{App, AppContext, Entity, Focusable, Window, px};
use std::sync::Arc;

struct NeverClient;
impl freshkube_core::cluster_source::KubeClientSource for NeverClient {
    fn client(
        &self,
    ) -> futures::future::BoxFuture<'_, Result<kube::Client, freshkube_core::failure::Failure>>
    {
        Box::pin(async {
            Err(freshkube_core::failure::Failure::new(
                freshkube_core::failure::FailureKind::Unreachable,
                "The invented source is unreachable",
            ))
        })
    }
}

const SETUP: &str = "\
version: 1
clusters:
- {id: cluster-a, role: core, context: cluster-a, future_entry: {keep: true}}
sources: {argocd: cluster-a}
destinations: [{name: example, entry: cluster-a}]
metrics:
  mode: central
  store:
    entry: cluster-a
    service: {namespace: metrics, name: prometheus, port: 9090, future_service: [7, null]}
    future_store: {keep: true}
  future_metrics: [example, false]
coroot: {server: 'https://coroot.example.test', sign_in: anonymous, future_coroot: true}
supply_chain: {transparency_log: false, future_chain: true}
future: {keep: [7, null, true]}
";

fn open(
    cx: &mut TestAppContext,
    handle: AnyWindowHandle,
    pilot: &Entity<Pilot>,
) -> Entity<SettingsPage> {
    cx.update_window(handle, |_, window, cx| {
        pilot.update(cx, |pilot, cx| {
            pilot.show_settings(SettingsSection::Monitoring, window, cx)
        });
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| window.render_frame(cx))
        .unwrap();
    cx.read(|cx| pilot.read(cx).settings_page.clone())
}

async fn live(
    cx: &mut TestAppContext,
    directory: &std::path::Path,
    context: &str,
) -> (
    tokio::runtime::Runtime,
    AnyWindowHandle,
    Entity<Pilot>,
    Entity<SettingsPage>,
) {
    cx.executor().allow_parking();
    let options = GpuiOptions::kubernetes_only(
        Some(directory.join("missing-kubeconfig")),
        Some(context.into()),
        100,
    )
    .with_preferences(Some(directory.join("preferences.json")));
    let (runtime, handle, pilot) = mount(cx, options, 1280., 1600.);
    cx.wait_for_io(handle, |_, cx| !pilot.read(cx).config_loading)
        .await;
    cx.update_window(handle, |_, window, cx| {
        pilot.update(cx, |pilot, cx| {
            pilot.adopt_launched_entry(
                &crate::desktop::switch::Launched {
                    kubernetes_only: true,
                    kubeconfig: None,
                    kube_context: Some(context.into()),
                    talosconfig: None,
                    talos_context: None,
                },
                cx,
            );
            pilot.pages.monitoring.update(cx, |page, cx| {
                page.set_source(
                    Some(freshkube_core::cluster_source::ClusterSource {
                        id: format!("fake-{context}"),
                        context: context.into(),
                        access: freshkube_core::cluster_source::ClusterAccess::Live(Arc::new(
                            NeverClient,
                        )),
                    }),
                    cx,
                )
            });
        });
        window.activate_window();
    })
    .unwrap();
    cx.run_until_parked();
    let page = open(cx, handle, &pilot);
    (runtime, handle, pilot, page)
}

fn replace(window: &mut Window, cx: &mut App, id: &'static str, value: &str) {
    window.click(id, cx);
    window.press("secondary-a", cx);
    window.input(value, cx);
}

async fn saved(cx: &mut TestAppContext, handle: AnyWindowHandle, page: &Entity<SettingsPage>) {
    cx.wait_for_io(handle, |_, cx| {
        page.read(cx).monitoring.as_ref().unwrap().saving.is_none()
    })
    .await;
    cx.run_until_parked();
}

async fn reload(cx: &mut TestAppContext, handle: AnyWindowHandle, page: &Entity<SettingsPage>) {
    cx.update(|cx| {
        let file = page.read(cx).monitoring.as_ref().unwrap().file.clone();
        file.update(cx, |file, cx| file.reload(cx)).detach();
    });
    cx.wait_for_io(handle, |_, cx| !page.read(cx).setup_refusal)
        .await;
    cx.run_until_parked();
}

#[gpui_kit::test]
fn monitoring_modes_revert_and_fixture_controls(cx: &mut TestAppContext) {
    let directory = tempfile::tempdir().unwrap();
    std::fs::write(directory.path().join("freshkube.yaml"), SETUP).unwrap();
    let options =
        GpuiOptions::fixture().with_preferences(Some(directory.path().join("preferences.json")));
    let (_runtime, handle, pilot) = mount(cx, options, 1280., 1600.);
    let page = open(cx, handle, &pilot);
    cx.update_window(handle, |_, window, cx| {
        assert!(window.find("metrics-entry").visible());
        assert!(window.find("metrics-label").visible());
        window.click("metrics-automatic", cx);
        window.render_frame(cx);
        assert!(window.try_find("metrics-entry").is_none());
        assert!(window.try_find("metrics-url").is_none());
        window.click("metrics-per-entry", cx);
        window.render_frame(cx);
        assert!(window.find("metrics-per-entry-row").visible());
        window.click("metrics-central", cx);
        window.render_frame(cx);
        window.click("metrics-store-url", cx);
        window.render_frame(cx);
        assert!(window.find("metrics-url").visible());
        assert!(window.try_find("metrics-entry").is_none());
        window.click("metrics-save", cx);
        window.click("metrics-test", cx);
        window.click("monitoring-folder-choose", cx);
        assert!(page.read(cx).monitoring.as_ref().unwrap().saving.is_none());
        assert!(!page.read(cx).monitoring.as_ref().unwrap().read.is_loading());
        window.click("metrics-revert", cx);
        window.render_frame(cx);
        assert!(window.find("metrics-entry").visible());
        assert!(!page.read(cx).monitoring.as_ref().unwrap().dirty);
        assert!(window.find("metrics-mode-row-hint").visible());
    })
    .unwrap();
    assert!(!cx.did_prompt_for_paths());
    assert_eq!(
        std::fs::read_to_string(directory.path().join("freshkube.yaml")).unwrap(),
        SETUP
    );
    assert!(!directory.path().join("monitoring.json").exists());
}

#[gpui_kit::test]
async fn monitoring_saves_only_metrics_and_keeps_every_unknown_key(cx: &mut TestAppContext) {
    let directory = tempfile::tempdir().unwrap();
    let file = directory.path().join("freshkube.yaml");
    std::fs::write(&file, SETUP).unwrap();
    let (_runtime, handle, _pilot, page) = live(cx, directory.path(), "cluster-a").await;
    let mut expected = setup::parse(SETUP.as_bytes()).unwrap();
    expected
        .metrics
        .store
        .as_mut()
        .unwrap()
        .service
        .as_mut()
        .unwrap()
        .port = 9091;
    cx.update_window(handle, |_, window, cx| {
        replace(window, cx, "metrics-port", "9091");
        window.press("enter", cx);
    })
    .unwrap();
    saved(cx, handle, &page).await;
    assert_eq!(
        setup::parse(&std::fs::read(&file).unwrap()).unwrap(),
        expected
    );
    assert_eq!(
        std::fs::read_to_string(file.with_extension("yaml.bak")).unwrap(),
        SETUP
    );
    cx.update_window(handle, |_, window, cx| {
        assert!(page.read(cx).monitoring.as_ref().unwrap().error.is_none());
        assert!(!page.read(cx).monitoring.as_ref().unwrap().dirty);
        replace(window, cx, "metrics-label", "example_cluster");
        window.press("escape", cx);
        window.render_frame(cx);
        assert_eq!(
            page.read(cx)
                .monitoring
                .as_ref()
                .unwrap()
                .label
                .read(cx)
                .value(),
            "cluster"
        );
        assert!(!page.read(cx).monitoring.as_ref().unwrap().dirty);
    })
    .unwrap();
}

#[gpui_kit::test]
async fn monitoring_fields_report_port_label_and_missing_entry_errors(cx: &mut TestAppContext) {
    let directory = tempfile::tempdir().unwrap();
    let file = directory.path().join("freshkube.yaml");
    std::fs::write(&file, SETUP).unwrap();
    let (_runtime, handle, _pilot, page) = live(cx, directory.path(), "cluster-a").await;
    cx.update_window(handle, |_, window, cx| {
        replace(window, cx, "metrics-port", "wrong");
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.click("metrics-label", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(window.find("metrics-port-row-error").visible());
        window.click("metrics-save", cx);
        window.render_frame(cx);
        assert!(
            page.read(cx)
                .monitoring
                .as_ref()
                .unwrap()
                .port
                .read(cx)
                .focus_handle(cx)
                .is_focused(window)
        );
        replace(window, cx, "metrics-port", "9090");
        replace(window, cx, "metrics-label", "not a label");
        window.click("metrics-save", cx);
        window.render_frame(cx);
        assert!(window.find("metrics-label-row-error").visible());
        window.click("metrics-revert", cx);
        window.render_frame(cx);
        page.update(cx, |page, cx| {
            let draft = page.monitoring.as_mut().unwrap();
            draft
                .entry
                .update(cx, |state, cx| state.set_selected_index(None, window, cx));
            draft.changed(cx);
        });
        window.render_frame(cx);
        window.click("metrics-save", cx);
        window.render_frame(cx);
        assert!(window.find("metrics-entry-row-error").visible());
        page.update(cx, |page, cx| {
            let draft = page.monitoring.as_mut().unwrap();
            draft.missing_entry = Some("cluster-gone".into());
            draft.changed(cx);
        });
        window.render_frame(cx);
        window.click("metrics-save", cx);
        window.render_frame(cx);
        assert!(
            page.read(cx).monitoring.as_ref().unwrap().errors[&Field::Entry]
                .contains("in the workspace")
        );
        assert!(page.read(cx).monitoring.as_ref().unwrap().saving.is_none());
    })
    .unwrap();
    assert_eq!(std::fs::read_to_string(file).unwrap(), SETUP);
}

#[gpui_kit::test]
async fn monitoring_refuses_broken_and_changed_files_and_revert_reads_the_latest(
    cx: &mut TestAppContext,
) {
    let directory = tempfile::tempdir().unwrap();
    let file = directory.path().join("freshkube.yaml");
    std::fs::write(&file, SETUP).unwrap();
    let (_runtime, handle, _pilot, page) = live(cx, directory.path(), "cluster-a").await;
    cx.update_window(handle, |_, window, cx| {
        replace(window, cx, "metrics-port", "9091")
    })
    .unwrap();
    std::fs::write(&file, "version: [").unwrap();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("metrics-save", cx);
    })
    .unwrap();
    saved(cx, handle, &page).await;
    assert_eq!(std::fs::read_to_string(&file).unwrap(), "version: [");
    cx.read(|cx| {
        assert_eq!(
            page.read(cx).monitoring.as_ref().unwrap().error.as_deref(),
            Some(
                "freshkube.yaml isn’t usable. Nothing was written."
                    .replace('’', "'")
                    .as_str()
            )
        )
    });
    let changed = SETUP.replace("port: 9090", "port: 9092");
    std::fs::write(&file, &changed).unwrap();
    reload(cx, handle, &page).await;
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("metrics-save", cx);
    })
    .unwrap();
    saved(cx, handle, &page).await;
    assert_eq!(std::fs::read_to_string(&file).unwrap(), changed);
    cx.read(|cx| {
        assert_eq!(
            page.read(cx).monitoring.as_ref().unwrap().error.as_deref(),
            Some(crate::setup_file::SaveError::Changed.to_string().as_str())
        )
    });
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("metrics-revert", cx);
        window.render_frame(cx);
        assert_eq!(
            page.read(cx)
                .monitoring
                .as_ref()
                .unwrap()
                .port
                .read(cx)
                .value(),
            "9092"
        );
    })
    .unwrap();
}

#[gpui_kit::test]
async fn monitoring_tests_unsaved_values_with_pass_failure_and_unknown(cx: &mut TestAppContext) {
    let directory = tempfile::tempdir().unwrap();
    let file = directory.path().join("freshkube.yaml");
    std::fs::write(&file, SETUP).unwrap();
    let (_runtime, handle, _pilot, page) = live(cx, directory.path(), "cluster-a").await;
    for (verdict, state) in [
        (Verdict::Pass, "Passed"),
        (Verdict::Fail("The store is unreachable".into()), "Failed"),
        (
            Verdict::Unknown("No series match this entry".into()),
            "Unknown",
        ),
    ] {
        let seen = Arc::new(std::sync::Mutex::new(None));
        let observed = seen.clone();
        cx.update_window(handle, |_, window, cx| {
            page.update(cx, |page, _| {
                page.monitoring.as_mut().unwrap().checker = Arc::new(move |resolved, _, _| {
                    *observed.lock().unwrap() = Some(resolved);
                    let verdict = verdict.clone();
                    Box::pin(async move {
                        vec![Check {
                            what: "The store answers",
                            verdict,
                        }]
                    })
                })
            });
            replace(window, cx, "metrics-label", "example_cluster");
        })
        .unwrap();
        cx.run_until_parked();
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            window.click("metrics-test", cx);
        })
        .unwrap();
        cx.wait_for_io(handle, |_, cx| {
            page.read(cx).monitoring.as_ref().unwrap().outcome.is_some()
        })
        .await;
        cx.read(|cx| {
            assert!(
                page.read(cx)
                    .monitoring
                    .as_ref()
                    .unwrap()
                    .outcome
                    .as_ref()
                    .unwrap()
                    .1
                    .starts_with(state)
            )
        });
        assert!(
            matches!(seen.lock().unwrap().as_ref(), Some(Resolved::Central { matcher, .. }) if matcher.label() == "example_cluster" && matcher.value() == "cluster-a")
        );
        assert_eq!(std::fs::read_to_string(&file).unwrap(), SETUP);
    }
}

#[gpui_kit::test]
async fn monitoring_context_outside_the_workspace_saves_monitoring_json(cx: &mut TestAppContext) {
    let directory = tempfile::tempdir().unwrap();
    std::fs::write(directory.path().join("freshkube.yaml"), SETUP).unwrap();
    let (_runtime, handle, _pilot, _page) = live(cx, directory.path(), "cluster-outside").await;
    cx.update_window(handle, |_, window, cx| {
        window.click("monitoring-source-url", cx);
        window.render_frame(cx);
        replace(
            window,
            cx,
            "monitoring-source-address",
            "https://metrics.example.test",
        );
        window.render_frame(cx);
        window.click("monitoring-source-save", cx);
    })
    .unwrap();
    let file = directory.path().join("monitoring.json");
    cx.wait_for_io(handle, |_, _| {
        std::fs::read_to_string(&file)
            .is_ok_and(|text| text.contains("https://metrics.example.test/"))
    })
    .await;
    let value: serde_json::Value = serde_json::from_slice(&std::fs::read(file).unwrap()).unwrap();
    assert_eq!(value["sources"]["cluster-outside"]["kind"], "url");
    assert_eq!(
        std::fs::read_to_string(directory.path().join("freshkube.yaml")).unwrap(),
        SETUP
    );
}

#[gpui_kit::test]
fn monitoring_has_the_padded_frame_at_each_window_and_text_size(cx: &mut TestAppContext) {
    for (width, height) in [(1280., 880.), (760., 560.)] {
        let (_runtime, handle, pilot) = fixture(cx, width, height);
        open(cx, handle, &pilot);
        for size in [13., 20.] {
            cx.update_window(handle, |_, window, cx| {
                crate::text_size::set(size, cx);
                window.render_frame(cx);
                layout_check::assert_page_frame(
                    window,
                    cx,
                    &layout_check::PageFrame {
                        page: "settings-page",
                        title: "settings-monitoring-title",
                        title_text: "Monitoring",
                        content: "settings-monitoring-form",
                    },
                );
                let group = window.find("monitoring-metrics").bounds();
                assert!(group.size.width <= ui::dp_px(form::MAX_WIDTH, window) + px(1.));
                for id in ["metrics-automatic", "metrics-central", "metrics-per-entry"] {
                    let control = window.find(id).bounds();
                    assert!(
                        (control.size.height - ui::dp_px(ui::CONTROL_HEIGHT, window)).abs()
                            < px(1.)
                    );
                    assert!(
                        control.right() <= group.right() + px(1.),
                        "{width} x {height}, text {size}: {id} exceeds the group"
                    );
                }
            })
            .unwrap();
        }
    }
}

#[gpui_kit::test]
async fn monitoring_folder_changes_save_at_once_and_keep_other_preferences(
    cx: &mut TestAppContext,
) {
    let directory = tempfile::tempdir().unwrap();
    std::fs::write(directory.path().join("freshkube.yaml"), SETUP).unwrap();
    let file = directory.path().join("monitoring.json");
    let mut expected = serde_json::json!({"dashboards": null, "prometheus": {}, "sources": {}, "future": {"keep": [true, null, 7]}});
    std::fs::write(&file, serde_json::to_vec(&expected).unwrap()).unwrap();
    let folder = directory.path().join("dashboards");
    std::fs::create_dir(&folder).unwrap();
    let (_runtime, handle, _pilot, _page) = live(cx, directory.path(), "cluster-outside").await;
    cx.update_window(handle, |_, window, cx| {
        window.click("monitoring-folder-choose", cx);
    })
    .unwrap();
    assert!(cx.did_prompt_for_paths());
    cx.simulate_path_prompt_response(|options| {
        assert!(options.directories);
        assert!(!options.files);
        Some(vec![folder.clone()])
    });
    cx.wait_for_io(handle, |_, _| {
        std::fs::read_to_string(&file).is_ok_and(|text| text.contains("dashboards\""))
    })
    .await;
    cx.wait_for_io(handle, |_, _| {
        serde_json::from_slice::<serde_json::Value>(&std::fs::read(&file).unwrap())
            .is_ok_and(|value| value["dashboards"].is_string())
    })
    .await;
    expected["dashboards"] = serde_json::to_value(&folder).unwrap();
    let read =
        || serde_json::from_slice::<serde_json::Value>(&std::fs::read(&file).unwrap()).unwrap();
    assert_eq!(read(), expected);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("monitoring-folder-forget", cx);
    })
    .unwrap();
    cx.wait_for_io(handle, |_, _| read()["dashboards"].is_null())
        .await;
    expected["dashboards"] = serde_json::Value::Null;
    assert_eq!(read(), expected);
}

#[gpui_kit::test]
async fn monitoring_url_tokens_save_separately_and_a_refused_file_restores_them(
    cx: &mut TestAppContext,
) {
    use freshkube_core::secrets::MemoryStore;
    let directory = tempfile::tempdir().unwrap();
    let file = directory.path().join("freshkube.yaml");
    std::fs::write(&file, SETUP).unwrap();
    let (runtime, handle, _pilot, page) = live(cx, directory.path(), "cluster-a").await;
    let secrets = Arc::new(MemoryStore::default());
    cx.update_window(handle, |_, window, cx| {
        let monitoring = cx.new(|cx| {
            super::MonitoringPage::new(
                runtime.handle().clone(),
                Some(&directory.path().join("preferences.json")),
                Some(secrets.clone()),
                cx,
            )
        });
        page.update(cx, |page, _| {
            page.monitoring.as_mut().unwrap().page = monitoring
        });
        window.click("metrics-store-url", cx);
        window.render_frame(cx);
        replace(window, cx, "metrics-url", "https://metrics.example.test");
        replace(window, cx, "metrics-token-input", "example-token");
    })
    .unwrap();
    cx.run_until_parked();
    std::fs::write(&file, "version: [").unwrap();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("metrics-save", cx);
    })
    .unwrap();
    saved(cx, handle, &page).await;
    assert!(secrets.keys.lock().unwrap().is_empty());
    assert_eq!(std::fs::read_to_string(&file).unwrap(), "version: [");
    std::fs::write(&file, SETUP).unwrap();
    reload(cx, handle, &page).await;
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("metrics-save", cx);
    })
    .unwrap();
    saved(cx, handle, &page).await;
    let text = std::fs::read_to_string(&file).unwrap();
    let setup = setup::parse(text.as_bytes()).unwrap();
    assert!(setup.metrics.store.unwrap().token.is_some());
    assert!(!text.contains("example-token"));
    assert_eq!(
        secrets
            .keys
            .lock()
            .unwrap()
            .get("Prometheus https://metrics.example.test/")
            .map(String::as_str),
        Some("example-token")
    );
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("metrics-token-forget", cx);
        window.render_frame(cx);
        assert!(!secrets.keys.lock().unwrap().is_empty());
        window.click("metrics-revert", cx);
        window.render_frame(cx);
        assert!(!secrets.keys.lock().unwrap().is_empty());
        window.click("metrics-token-forget", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("metrics-save", cx);
    })
    .unwrap();
    saved(cx, handle, &page).await;
    assert!(secrets.keys.lock().unwrap().is_empty());
    assert!(
        setup::parse(&std::fs::read(&file).unwrap())
            .unwrap()
            .metrics
            .store
            .unwrap()
            .token
            .is_none()
    );
}

#[gpui_kit::test]
async fn monitoring_discards_a_check_when_the_draft_changes_or_the_section_hides(
    cx: &mut TestAppContext,
) {
    let directory = tempfile::tempdir().unwrap();
    std::fs::write(directory.path().join("freshkube.yaml"), SETUP).unwrap();
    let (_runtime, handle, pilot, page) = live(cx, directory.path(), "cluster-a").await;
    for hide in [false, true] {
        let (sender, receiver) = tokio::sync::oneshot::channel::<()>();
        let held = Arc::new(std::sync::Mutex::new(Some(receiver)));
        let entered = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let started = entered.clone();
        cx.update_window(handle, |_, window, cx| {
            page.update(cx, |page, _| {
                page.monitoring.as_mut().unwrap().checker = Arc::new(move |_, _, _| {
                    let receiver = held.lock().unwrap().take().unwrap();
                    let started = started.clone();
                    Box::pin(async move {
                        started.store(true, std::sync::atomic::Ordering::SeqCst);
                        _ = receiver.await;
                        vec![Check {
                            what: "The store answers",
                            verdict: Verdict::Pass,
                        }]
                    })
                })
            });
            window.render_frame(cx);
            window.click("metrics-test", cx);
        })
        .unwrap();
        cx.wait_for_io(handle, |_, _| {
            entered.load(std::sync::atomic::Ordering::SeqCst)
        })
        .await;
        cx.update_window(handle, |_, window, cx| {
            if hide {
                pilot.update(cx, |pilot, cx| {
                    pilot.show_settings(SettingsSection::General, window, cx)
                });
            } else {
                replace(window, cx, "metrics-label", "example_cluster");
            }
        })
        .unwrap();
        cx.wait_for_io(handle, |_, _| sender.is_closed()).await;
        cx.read(|cx| {
            let draft = page.read(cx).monitoring.as_ref().unwrap();
            assert!(!draft.read.is_loading());
            assert!(draft.outcome.is_none());
        });
        if !hide {
            cx.update_window(handle, |_, window, cx| {
                window.render_frame(cx);
                window.click("metrics-revert", cx);
            })
            .unwrap();
        }
    }
}
