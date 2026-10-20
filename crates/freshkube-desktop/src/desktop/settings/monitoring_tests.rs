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

#[gpui_kit::test]
fn per_entry_links_to_clusters_and_selects_the_first_entry(cx: &mut TestAppContext) {
    for entries in [true, false] {
        let (_runtime, handle, pilot) = fixture(cx, 1280., 1600.);
        cx.update_window(handle, |_, window, cx| {
            pilot.update(cx, |pilot, cx| {
                pilot.show_settings(SettingsSection::Clusters, window, cx);
                if !entries {
                    pilot.settings_page.update(cx, |page, cx| {
                        page.set_workspace(&freshkube_core::workspace::Loaded::Missing, false, cx)
                    });
                }
            });
            window.render_frame(cx);
            if entries {
                window.click("settings-cluster-prod-fra", cx);
            }
        })
        .unwrap();
        cx.run_until_parked();
        let page = open(cx, handle, &pilot);
        cx.update_window(handle, |_, window, cx| {
            window.click("metrics-per-entry", cx);
            window.render_frame(cx);
            assert!(window.find("metrics-per-entry-row-hint").visible());
            assert_eq!(
                window.find("metrics-per-entry-explanation").label(),
                Some("Each entry names its own source in freshkube.yaml.")
            );
            assert_eq!(
                window.find("metrics-per-entry-clusters").label(),
                Some("Edit it in Clusters")
            );
            window.click("metrics-per-entry-clusters", cx);
        })
        .unwrap();
        cx.run_until_parked();
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            assert_eq!(
                pilot.read(cx).navigation.page,
                crate::desktop::Page::Settings
            );
            assert_eq!(page.read(cx).section(), SettingsSection::Clusters);
            assert!(window.find("nav-settings-clusters").visible());
            if entries {
                assert_eq!(page.read(cx).selected.as_deref(), Some("core-fra"));
                assert_eq!(
                    window.find("settings-detail-title").label(),
                    Some("core-fra")
                );
                assert!(window.find("settings-metrics").visible());
            } else {
                assert!(page.read(cx).selected.is_none());
                assert!(window.find("settings-empty").visible());
                assert!(window.try_find("settings-metrics").is_none());
            }
        })
        .unwrap();
    }
}

#[gpui_kit::test]
fn per_entry_explanation_wraps_at_text_twenty_in_a_small_window(cx: &mut TestAppContext) {
    let (_runtime, handle, pilot) = fixture(cx, 760., 560.);
    cx.update_window(handle, |_, _, cx| crate::text_size::set(20., cx))
        .unwrap();
    let page = open(cx, handle, &pilot);
    cx.update_window(handle, |_, window, cx| {
        window.click("metrics-per-entry", cx);
        window.render_frame(cx);
        let hint = window.find("metrics-per-entry-row-hint").bounds();
        let explanation = window.find("metrics-per-entry-explanation").bounds();
        let link = window.find("metrics-per-entry-clusters");
        assert!(
            explanation.size.height > ui::dp_px(24., window),
            "the explanation must wrap: {explanation:?}"
        );
        assert!(explanation.left() >= hint.left() - px(1.));
        assert!(explanation.right() <= hint.right() + px(1.));
        assert!(link.bounds().right() <= hint.right() + px(1.));
        assert!(link.bounds().top() >= explanation.bottom() - px(1.));
        assert!(link.visible());
        window.click("metrics-per-entry-clusters", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(page.read(cx).section(), SettingsSection::Clusters);
        assert_eq!(page.read(cx).selected.as_deref(), Some("core-fra"));
    })
    .unwrap();
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
        assert_eq!(
            page.read(cx).monitoring.as_ref().unwrap().errors.get(&Field::Label).map(String::as_str),
            Some("Use a Prometheus label name: letters, digits and underscores, not starting with a digit.")
        );
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
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            assert!(
                page.read(cx)
                    .monitoring
                    .as_ref()
                    .unwrap()
                    .focus
                    .is_focused(window)
            );
            assert_ne!(window.find("metrics-test").focused(), Some(true));
        })
        .unwrap();
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
        window.click("monitoring-source-test", cx);
    })
    .unwrap();
    cx.wait_for_io(handle, |window, _| {
        window
            .find("monitoring-context-test-result")
            .label()
            .is_some_and(|label| label.starts_with("Failed"))
    })
    .await;
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(window.find("monitoring-source-test").focused(), Some(false));
    })
    .unwrap();
    cx.update_window(handle, |_, window, cx| {
        window.click("monitoring-source-service", cx);
        window.render_frame(cx);
        assert!(window.find("monitoring-context-current").visible());
        assert!(window.find("monitoring-context-name").visible());
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
        let page = open(cx, handle, &pilot);
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
                let long = "Failed · The store answers: Unreachable · couldn’t connect to the server at https://metrics.example.test while checking the selected entry. The connection attempt ended before the store answered. ".repeat(2);
                for outcome in [None, Some((ui::Tone::Crit, long))] {
                    let has_result = outcome.is_some();
                    page.update(cx, |page, _| {
                        let draft = page.monitoring.as_mut().unwrap();
                        draft.mode = setup::Mode::Automatic;
                        draft.outcome = outcome;
                    });
                    layout_check::assert_page_frame(window, cx, &layout_check::PageFrame {
                        page: "settings-page", title: "settings-monitoring-title", title_text: "Monitoring", content: "settings-monitoring-form",
                    });
                    let group = window.find("monitoring-metrics").bounds();
                    let line = window.find("metrics-test-result").bounds();
                    let text = window.find("metrics-test-text").bounds();
                    assert!(line.right() <= group.right() + px(1.));
                    assert!(text.right() <= line.right() + px(1.));
                    assert!(text.left() > window.find("metrics-test").bounds().right());
                    if has_result {
                        assert!(text.size.height > ui::dp_px(24., window), "the result wraps");
                        let glyph = window.find("metrics-test-glyph").bounds();
                        assert!(glyph.right() < text.left());
                        assert!((glyph.top() - text.top()).abs() <= ui::dp_px(4., window));
                    } else if width == 760. && size == 20. {
                        assert!(text.size.height > ui::dp_px(18., window), "the narrow hint wraps");
                    }
                }
                page.update(cx, |page, _| {
                    page.banner = Some("An invented setup warning.".into());
                    page.warning = Some("An invented unknown key.".into());
                    page.monitoring.as_mut().unwrap().error = Some("An invented save refusal.".into());
                });
                window.render_frame(cx);
                for id in ["monitoring-setup-banner", "monitoring-unknown-banner", "monitoring-save-banner"] {
                    let banner = window.find(id).bounds();
                    assert!(banner.right() <= group.right() + px(1.));
                    assert!((banner.left() - group.left()).abs() < px(1.));
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

fn url_setup() -> String {
    let mut setup = setup::parse(SETUP.as_bytes()).unwrap();
    let store = setup.metrics.store.as_mut().unwrap();
    store.entry = None;
    store.service = None;
    store.url = Some("https://metrics.example.test".into());
    store.token = Some(setup::KeysJson);
    setup::to_text(&setup)
}

fn set_secrets(
    cx: &mut TestAppContext,
    runtime: &tokio::runtime::Runtime,
    page: &Entity<SettingsPage>,
    directory: &std::path::Path,
    secrets: freshkube_core::secrets::Secrets,
) {
    cx.update(|cx| {
        let monitoring = cx.new(|cx| {
            super::MonitoringPage::new(
                runtime.handle().clone(),
                Some(&directory.join("preferences.json")),
                Some(secrets),
                cx,
            )
        });
        page.update(cx, |page, _| {
            page.monitoring.as_mut().unwrap().page = monitoring
        });
    });
}

fn write_context_source(directory: &std::path::Path, token: bool) {
    let value = serde_json::json!({"sources": {
        "outside-example": {"kind": "url", "url": "https://metrics.example.test", "token": token}
    }});
    std::fs::write(
        directory.join("monitoring.json"),
        serde_json::to_vec(&value).unwrap(),
    )
    .unwrap();
}

/// Deletion must see a committed file with no remaining reference to the
/// account. The fake never reads a key file or the system credential store.
struct TokenStore {
    memory: freshkube_core::secrets::MemoryStore,
    file: std::path::PathBuf,
    forgotten: std::sync::atomic::AtomicUsize,
}

impl TokenStore {
    fn new(file: std::path::PathBuf) -> Arc<Self> {
        let store = Arc::new(Self {
            memory: freshkube_core::secrets::MemoryStore::default(),
            file,
            forgotten: std::sync::atomic::AtomicUsize::new(0),
        });
        store.memory.keys.lock().unwrap().insert(
            "Prometheus https://metrics.example.test/".into(),
            "invented-token".into(),
        );
        store
    }

    fn assert_kept(&self, kept: bool) {
        assert_eq!(
            self.memory
                .keys
                .lock()
                .unwrap()
                .contains_key("Prometheus https://metrics.example.test/"),
            kept,
        );
        assert_eq!(
            self.forgotten.load(std::sync::atomic::Ordering::SeqCst),
            usize::from(!kept)
        );
    }
}

impl freshkube_core::secrets::SecretStore for TokenStore {
    fn read(&self, account: &str) -> Result<Option<String>, String> {
        self.memory.read(account)
    }

    fn write(&self, account: &str, value: &str) -> Result<(), String> {
        self.memory.write(account, value)
    }

    fn forget(&self, account: &str) -> Result<(), String> {
        let setup = setup::parse(&std::fs::read(&self.file).unwrap()).unwrap();
        assert!(
            !freshkube_core::monitoring::scope::token_urls(&setup)
                .iter()
                .any(|url| crate::monitoring::token_account(url) == account),
            "the YAML reference must be saved away before deleting its token"
        );
        assert!(
            !crate::monitoring::saved_token_urls(&self.file.with_file_name("preferences.json"))
                .unwrap()
                .iter()
                .any(|url| crate::monitoring::token_account(url) == account),
            "a context still naming this token must keep it"
        );
        self.forgotten
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        self.memory.forget(account)
    }
}

#[gpui_kit::test]
async fn monitoring_retires_only_unreferenced_tokens_after_a_successful_save(
    cx: &mut TestAppContext,
) {
    for action in ["service", "url", "forget", "automatic"] {
        for condition in [
            "unreferenced",
            "shared",
            "context",
            "anonymous-context",
            "unreadable-contexts",
            "refused",
        ] {
            let directory = tempfile::tempdir().unwrap();
            let file = directory.path().join("freshkube.yaml");
            let mut setup = setup::parse(url_setup().as_bytes()).unwrap();
            if condition == "shared" {
                let mut source = setup::Source::default();
                source.url = Some("https://metrics.example.test/".into());
                source.token = Some(setup::KeysJson);
                let mut metrics = setup::EntryMetrics::default();
                metrics.source = Some(source);
                setup.workspace.clusters[0].metrics = Some(Box::new(metrics));
            }
            let original = setup::to_text(&setup);
            std::fs::write(&file, &original).unwrap();
            if matches!(condition, "context" | "anonymous-context") {
                write_context_source(directory.path(), condition == "context");
            }
            let (runtime, handle, _pilot, page) = live(cx, directory.path(), "cluster-a").await;
            let store = TokenStore::new(file.clone());
            set_secrets(cx, &runtime, &page, directory.path(), store.clone());
            if condition == "unreadable-contexts" {
                std::fs::write(directory.path().join("monitoring.json"), "{").unwrap();
            }
            cx.update_window(handle, |_, window, cx| {
                window.render_frame(cx);
                match action {
                    "service" => {
                        window.click("metrics-store-service", cx);
                        window.render_frame(cx);
                        // Pick the only hosting entry using its real select.
                        window.click("metrics-entry", cx);
                        window.press("down", cx);
                        window.press("enter", cx);
                    }
                    "url" => replace(
                        window,
                        cx,
                        "metrics-url",
                        "https://other-metrics.example.test",
                    ),
                    "forget" => window.click("metrics-token-forget", cx),
                    "automatic" => window.click("metrics-automatic", cx),
                    _ => unreachable!(),
                }
            })
            .unwrap();
            cx.run_until_parked();
            if action == "service" {
                cx.update_window(handle, |_, window, cx| {
                    window.render_frame(cx);
                    replace(window, cx, "metrics-namespace", "metrics");
                    replace(window, cx, "metrics-name", "prometheus");
                    replace(window, cx, "metrics-port", "9090");
                })
                .unwrap();
                cx.run_until_parked();
            }
            cx.read(|cx| {
                let page = page.read(cx);
                page.monitoring
                    .as_ref()
                    .unwrap()
                    .metrics(&page.workspace, cx)
                    .map_err(|(_, why)| why)
                    .unwrap_or_else(|why| panic!("{action}, {condition}: {why}"));
            });
            if condition == "refused" {
                std::fs::write(&file, "version: [").unwrap();
            }
            cx.update_window(handle, |_, window, cx| {
                window.render_frame(cx);
                window.click("metrics-save", cx);
            })
            .unwrap();
            saved(cx, handle, &page).await;
            store.assert_kept(
                !matches!(condition, "unreferenced" | "anonymous-context") || action == "automatic",
            );
            if condition == "refused" {
                assert_eq!(std::fs::read_to_string(&file).unwrap(), "version: [");
                cx.read(|cx| assert!(page.read(cx).monitoring.as_ref().unwrap().error.is_some()));
            } else {
                cx.read(|cx| {
                    let draft = page.read(cx).monitoring.as_ref().unwrap();
                    assert_eq!(
                        draft.error.is_some(),
                        condition == "unreadable-contexts" && action != "automatic"
                    );
                });
                let saved = setup::parse(&std::fs::read(&file).unwrap()).unwrap();
                if action == "service" {
                    let source = saved.metrics.store.unwrap();
                    assert!(source.service.is_some());
                    assert!(source.url.is_none());
                    assert!(source.token.is_none());
                }
            }
        }
    }
}

#[gpui_kit::test]
async fn monitoring_inspector_retires_service_and_none_tokens_after_save(cx: &mut TestAppContext) {
    for service in [false, true] {
        for condition in ["unreferenced", "central", "entry", "context", "refused"] {
            let directory = tempfile::tempdir().unwrap();
            let file = directory.path().join("freshkube.yaml");
            let mut setup = setup::parse(url_setup().as_bytes()).unwrap();
            let mut source = setup::Source::default();
            source.url = Some("https://metrics.example.test/".into());
            source.token = Some(setup::KeysJson);
            let mut metrics = setup::EntryMetrics::default();
            metrics.source = Some(source);
            setup.workspace.clusters[0].metrics = Some(Box::new(metrics));
            if condition != "central" {
                setup.metrics = setup::Metrics::default();
            }
            if condition == "entry" {
                let mut other = setup.workspace.clusters[0].clone();
                other.id = "cluster-b".into();
                other.context = "cluster-b".into();
                setup.workspace.clusters.push(other);
            }
            std::fs::write(&file, setup::to_text(&setup)).unwrap();
            if condition == "context" {
                write_context_source(directory.path(), true);
            }
            let (_runtime, handle, pilot, page) = live(cx, directory.path(), "cluster-a").await;
            let store = TokenStore::new(file.clone());
            cx.update_window(handle, |_, window, cx| {
                page.update(cx, |page, _| page.metrics_secrets = Some(store.clone()));
                pilot.update(cx, |pilot, cx| {
                    pilot.show_settings(SettingsSection::Clusters, window, cx)
                });
                window.render_frame(cx);
                window.click("settings-cluster-cluster-a", cx);
                window.render_frame(cx);
                window.click(
                    if service {
                        "settings-metrics-source-service"
                    } else {
                        "settings-metrics-source-none"
                    },
                    cx,
                );
                window.render_frame(cx);
                if service {
                    replace(window, cx, "settings-metrics-namespace", "metrics");
                    replace(window, cx, "settings-metrics-service", "prometheus");
                    replace(window, cx, "settings-metrics-port", "9090");
                }
            })
            .unwrap();
            cx.run_until_parked();
            if condition == "refused" {
                std::fs::write(&file, "version: [").unwrap();
            }
            cx.update_window(handle, |_, window, cx| {
                window.render_frame(cx);
                window.click("settings-metrics-save", cx);
            })
            .unwrap();
            crate::desktop::settings::edit_tests::saved(cx, handle, &pilot).await;
            store.assert_kept(condition != "unreferenced");
            if condition == "refused" {
                assert_eq!(std::fs::read_to_string(&file).unwrap(), "version: [");
            } else {
                let saved = setup::parse(&std::fs::read(&file).unwrap()).unwrap();
                let source = saved.workspace.clusters[0]
                    .metrics
                    .as_ref()
                    .and_then(|m| m.source.as_ref());
                assert_eq!(source.is_some(), service);
                if let Some(source) = source {
                    assert!(source.service.is_some());
                    assert!(source.url.is_none());
                    assert!(source.token.is_none());
                }
            }
        }
    }
}

#[gpui_kit::test]
async fn monitoring_forget_then_a_new_token_keeps_the_new_token(cx: &mut TestAppContext) {
    let directory = tempfile::tempdir().unwrap();
    let file = directory.path().join("freshkube.yaml");
    std::fs::write(&file, url_setup()).unwrap();
    let (runtime, handle, _pilot, page) = live(cx, directory.path(), "cluster-a").await;
    let secrets = Arc::new(freshkube_core::secrets::MemoryStore::default());
    let account = "Prometheus https://metrics.example.test/";
    secrets
        .keys
        .lock()
        .unwrap()
        .insert(account.into(), "old-example-token".into());
    set_secrets(cx, &runtime, &page, directory.path(), secrets.clone());
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("metrics-token-forget", cx);
        window.render_frame(cx);
        replace(window, cx, "metrics-token-input", "new-example-token");
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("metrics-save", cx);
    })
    .unwrap();
    saved(cx, handle, &page).await;
    assert_eq!(
        secrets
            .keys
            .lock()
            .unwrap()
            .get(account)
            .map(String::as_str),
        Some("new-example-token")
    );
    let text = std::fs::read_to_string(file).unwrap();
    assert!(
        setup::parse(text.as_bytes())
            .unwrap()
            .metrics
            .store
            .unwrap()
            .token
            .is_some()
    );
    assert!(!text.contains("new-example-token"));
}

#[gpui_kit::test]
async fn monitoring_a_changed_url_asks_for_its_own_token(cx: &mut TestAppContext) {
    let directory = tempfile::tempdir().unwrap();
    let file = directory.path().join("freshkube.yaml");
    std::fs::write(&file, url_setup()).unwrap();
    let (runtime, handle, _pilot, page) = live(cx, directory.path(), "cluster-a").await;
    let secrets = Arc::new(freshkube_core::secrets::MemoryStore::default());
    set_secrets(cx, &runtime, &page, directory.path(), secrets.clone());
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(window.find("metrics-token-kept").visible());
        replace(
            window,
            cx,
            "metrics-url",
            "https://other-metrics.example.test",
        );
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(window.try_find("metrics-token-kept").is_none());
        assert!(window.find("metrics-token-input").visible());
        assert_eq!(
            page.read(cx)
                .monitoring
                .as_ref()
                .unwrap()
                .token
                .read(cx)
                .value(),
            ""
        );
        assert!(window.find("metrics-token-row-hint").visible());
        replace(window, cx, "metrics-token-input", "other-example-token");
        replace(window, cx, "metrics-url", "https://metrics.example.test/");
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(window.try_find("metrics-token-kept").is_none());
        assert!(window.find("metrics-token-input").visible());
        replace(
            window,
            cx,
            "metrics-url",
            "https://other-metrics.example.test/",
        );
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("metrics-save", cx);
    })
    .unwrap();
    saved(cx, handle, &page).await;
    let store = setup::parse(&std::fs::read(file).unwrap())
        .unwrap()
        .metrics
        .store
        .unwrap();
    assert_eq!(
        store.url.as_deref(),
        Some("https://other-metrics.example.test/")
    );
    assert!(store.token.is_some());
    assert_eq!(
        secrets
            .keys
            .lock()
            .unwrap()
            .get("Prometheus https://other-metrics.example.test/")
            .map(String::as_str),
        Some("other-example-token")
    );
}

#[gpui_kit::test]
async fn monitoring_columns_folder_action_opens_the_monitoring_section(cx: &mut TestAppContext) {
    for (width, height, text_size) in [(1280., 880., 13.), (760., 560., 20.)] {
        let directory = tempfile::tempdir().unwrap();
        std::fs::write(directory.path().join("freshkube.yaml"), SETUP).unwrap();
        let (_runtime, handle, pilot, page) = live(cx, directory.path(), "cluster-a").await;
        cx.simulate_window_resize(handle, gpui_kit::size(px(width), px(height)));
        cx.update_window(handle, |_, window, cx| {
            crate::text_size::set(text_size, cx);
            window.render_frame(cx);
            if height == 560. {
                assert!(
                    window.find("monitoring-dashboards").bounds().bottom()
                        > window.find("settings-page").bounds().bottom()
                );
            }
        })
        .unwrap();
        cx.update_window(handle, |_, window, cx| {
            pilot.update(cx, |pilot, cx| {
                pilot.navigate(crate::desktop::Page::Monitoring, window, cx)
            });
            // Exercise a cold form as well as returning to an existing one.
            if text_size == 20. {
                page.update(cx, |page, _| page.monitoring = None);
            }
            window.render_frame(cx);
            if window.try_find("monitoring-add-folder").is_none() {
                window.click("nav-dashboard-list", cx);
                window.render_frame(cx);
            }
            window.click("monitoring-add-folder", cx);
        })
        .unwrap();
        cx.run_until_parked();
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            assert_eq!(
                pilot.read(cx).navigation.page,
                crate::desktop::Page::Settings
            );
            assert_eq!(
                pilot.read(cx).settings_page.read(cx).section(),
                SettingsSection::Monitoring
            );
            assert!(!pilot.read(cx).navigation.settings_open);
            assert!(
                window.find("monitoring-folder-choose").visible(),
                "{width} x {height}, text {text_size}: folder {:?}, viewport {:?}, scroll {:?}",
                window.find("monitoring-dashboards").bounds(),
                window.find("settings-page").bounds(),
                page.read(cx).monitoring_scroll.offset()
            );
            let group = window.find("monitoring-dashboards").bounds();
            let viewport = window.find("settings-page").bounds();
            assert!(group.top() >= viewport.top());
            assert!(group.bottom() <= viewport.bottom());
            window.click("monitoring-folder-choose", cx);
        })
        .unwrap();
        assert!(cx.did_prompt_for_paths());
        cx.simulate_path_prompt_response(|_| None);
        cx.run_until_parked();
    }
}

#[gpui_kit::test]
async fn monitoring_keeps_its_draft_after_a_clusters_save(cx: &mut TestAppContext) {
    let directory = tempfile::tempdir().unwrap();
    let file = directory.path().join("freshkube.yaml");
    std::fs::write(&file, SETUP).unwrap();
    let (_runtime, handle, pilot, page) = live(cx, directory.path(), "cluster-a").await;
    cx.update_window(handle, |_, window, cx| {
        replace(window, cx, "metrics-port", "9091")
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        pilot.update(cx, |pilot, cx| {
            pilot.show_settings(SettingsSection::Clusters, window, cx)
        });
        page.update(cx, |page, cx| {
            let mut next = page.workspace.clone();
            let mut entry = next.clusters[0].clone();
            entry.id = "cluster-b".into();
            entry.context = "cluster-b".into();
            next.clusters.push(entry);
            page.commit(next, "Added cluster-b.".into(), cx);
        });
    })
    .unwrap();
    cx.wait_for_io(handle, |_, cx| page.read(cx).saving.is_none())
        .await;
    cx.run_until_parked();
    open(cx, handle, &pilot);
    cx.update_window(handle, |_, window, cx| {
        let draft = page.read(cx).monitoring.as_ref().unwrap();
        assert!(draft.error.is_none());
        assert!(draft.dirty);
        assert_eq!(draft.port.read(cx).value(), "9091");
        assert_eq!(draft.seen, draft.file.read(cx).seen);
        window.click("metrics-save", cx);
    })
    .unwrap();
    saved(cx, handle, &page).await;
    let setup = setup::parse(&std::fs::read(file).unwrap()).unwrap();
    assert_eq!(setup.workspace.clusters.len(), 2);
    assert_eq!(setup.metrics.store.unwrap().service.unwrap().port, 9091);
}

#[gpui_kit::test]
async fn monitoring_escape_closes_the_picker_then_reverts_only_the_focused_group(
    cx: &mut TestAppContext,
) {
    let directory = tempfile::tempdir().unwrap();
    std::fs::write(directory.path().join("freshkube.yaml"), SETUP).unwrap();
    let (_runtime, handle, _pilot, page) = live(cx, directory.path(), "cluster-outside").await;
    cx.update_window(handle, |_, window, cx| {
        replace(window, cx, "metrics-port", "9091")
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("metrics-entry", cx);
        window.render_frame(cx);
        assert_eq!(window.find("metrics-entry").expanded(), Some(true));
        window.press("escape", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(window.find("metrics-entry").expanded(), Some(false));
        assert_eq!(
            page.read(cx)
                .monitoring
                .as_ref()
                .unwrap()
                .port
                .read(cx)
                .value(),
            "9091"
        );
        window.click("monitoring-source-url", cx);
        window.render_frame(cx);
        replace(
            window,
            cx,
            "monitoring-source-address",
            "https://context-metrics.example.test",
        );
        window.press("escape", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(window.try_find("monitoring-source-address").is_none());
        assert_eq!(
            page.read(cx)
                .monitoring
                .as_ref()
                .unwrap()
                .port
                .read(cx)
                .value(),
            "9091"
        );
        window.click("monitoring-folder-choose", cx);
    })
    .unwrap();
    assert!(cx.did_prompt_for_paths());
    cx.simulate_path_prompt_response(|_| None);
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.press("escape", cx);
        assert_eq!(
            page.read(cx)
                .monitoring
                .as_ref()
                .unwrap()
                .port
                .read(cx)
                .value(),
            "9091"
        );
        window.click("metrics-port", cx);
        window.press("escape", cx);
        assert_eq!(
            page.read(cx)
                .monitoring
                .as_ref()
                .unwrap()
                .port
                .read(cx)
                .value(),
            "9090"
        );
    })
    .unwrap();
}

#[gpui_kit::test]
async fn monitoring_a_refusal_after_staging_restores_the_previous_token(cx: &mut TestAppContext) {
    use freshkube_core::secrets::{MemoryStore, SecretStore};
    use std::sync::atomic::{AtomicUsize, Ordering};
    struct BreakAfterStaging {
        memory: MemoryStore,
        file: std::path::PathBuf,
        writes: AtomicUsize,
    }
    impl SecretStore for BreakAfterStaging {
        fn read(&self, account: &str) -> Result<Option<String>, String> {
            self.memory.read(account)
        }
        fn write(&self, account: &str, value: &str) -> Result<(), String> {
            self.memory.write(account, value)?;
            if self.writes.fetch_add(1, Ordering::SeqCst) == 0 {
                std::fs::write(&self.file, "version: [").unwrap();
            }
            Ok(())
        }
        fn forget(&self, account: &str) -> Result<(), String> {
            self.memory.forget(account)
        }
    }
    let directory = tempfile::tempdir().unwrap();
    let file = directory.path().join("freshkube.yaml");
    std::fs::write(&file, url_setup()).unwrap();
    let (runtime, handle, _pilot, page) = live(cx, directory.path(), "cluster-a").await;
    let secrets = Arc::new(BreakAfterStaging {
        memory: MemoryStore::default(),
        file: file.clone(),
        writes: AtomicUsize::new(0),
    });
    let account = "Prometheus https://metrics.example.test/";
    secrets
        .memory
        .keys
        .lock()
        .unwrap()
        .insert(account.into(), "previous-example-token".into());
    set_secrets(cx, &runtime, &page, directory.path(), secrets.clone());
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("metrics-token-replace", cx);
        window.render_frame(cx);
        replace(
            window,
            cx,
            "metrics-token-input",
            "replacement-example-token",
        );
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("metrics-save", cx);
    })
    .unwrap();
    saved(cx, handle, &page).await;
    assert_eq!(
        secrets.writes.load(Ordering::SeqCst),
        2,
        "the token was staged and then restored"
    );
    assert_eq!(
        secrets.read(account).unwrap().as_deref(),
        Some("previous-example-token")
    );
    assert_eq!(std::fs::read_to_string(file).unwrap(), "version: [");
    cx.read(|cx| assert!(page.read(cx).monitoring.as_ref().unwrap().error.is_some()));
}

#[gpui_kit::test]
async fn monitoring_a_first_token_save_can_skip_a_refused_fallback_read(cx: &mut TestAppContext) {
    use freshkube_core::secrets::{FileStore, MemoryStore, SecretStore};
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    struct RefuseAfterWrite {
        store: FileStore,
        file: std::path::PathBuf,
        refuse: AtomicBool,
        writes: AtomicUsize,
    }
    impl SecretStore for RefuseAfterWrite {
        fn read(&self, account: &str) -> Result<Option<String>, String> {
            self.store.read(account)
        }
        fn write(&self, account: &str, value: &str) -> Result<(), String> {
            self.store.write(account, value)?;
            self.writes.fetch_add(1, Ordering::SeqCst);
            if self.refuse.swap(false, Ordering::SeqCst) {
                std::fs::write(&self.file, "version: [").unwrap();
            }
            Ok(())
        }
        fn forget(&self, account: &str) -> Result<(), String> {
            self.store.forget(account)
        }
    }
    let directory = tempfile::tempdir().unwrap();
    let file = directory.path().join("freshkube.yaml");
    std::fs::write(&file, SETUP).unwrap();
    let (runtime, handle, _pilot, page) = live(cx, directory.path(), "cluster-a").await;
    assert!(!directory.path().join("keys.json").exists());
    let fallback = Arc::new(MemoryStore::default());
    fallback.unavailable.store(true, Ordering::SeqCst);
    let secrets = Arc::new(RefuseAfterWrite {
        store: FileStore::new(&directory.path().join("preferences.json"), Some(fallback)),
        file: file.clone(),
        refuse: AtomicBool::new(false),
        writes: AtomicUsize::new(0),
    });
    set_secrets(cx, &runtime, &page, directory.path(), secrets.clone());
    cx.update_window(handle, |_, window, cx| {
        window.click("metrics-store-url", cx);
        window.render_frame(cx);
        replace(window, cx, "metrics-url", "https://metrics.example.test");
        replace(window, cx, "metrics-token-input", "first-example-token");
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("metrics-save", cx);
    })
    .unwrap();
    saved(cx, handle, &page).await;
    let first = "Prometheus https://metrics.example.test/";
    assert_eq!(
        secrets.read(first).unwrap().as_deref(),
        Some("first-example-token")
    );
    assert!(
        setup::parse(&std::fs::read(&file).unwrap())
            .unwrap()
            .metrics
            .store
            .unwrap()
            .token
            .is_some()
    );
    cx.read(|cx| assert!(page.read(cx).monitoring.as_ref().unwrap().error.is_none()));
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        replace(
            window,
            cx,
            "metrics-url",
            "https://other-metrics.example.test",
        );
        window.render_frame(cx);
        replace(window, cx, "metrics-token-input", "refused-example-token");
    })
    .unwrap();
    cx.run_until_parked();
    secrets.refuse.store(true, Ordering::SeqCst);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("metrics-save", cx);
    })
    .unwrap();
    saved(cx, handle, &page).await;
    assert_eq!(secrets.writes.load(Ordering::SeqCst), 2);
    assert_eq!(
        secrets
            .read("Prometheus https://other-metrics.example.test/")
            .unwrap(),
        None
    );
    assert_eq!(
        secrets.read(first).unwrap().as_deref(),
        Some("first-example-token")
    );
    assert_eq!(std::fs::read_to_string(file).unwrap(), "version: [");
    cx.read(|cx| {
        let error = page
            .read(cx)
            .monitoring
            .as_ref()
            .unwrap()
            .error
            .as_ref()
            .unwrap();
        assert!(error.contains("The new token was removed; its previous value couldn’t be read."));
    });
}
