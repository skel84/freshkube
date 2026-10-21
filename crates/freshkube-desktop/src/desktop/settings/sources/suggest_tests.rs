//! Find suggestions on Delivery sources in the real app: the build
//! namespace, found from where PipelineRuns are.
use super::super::super::SettingsSection;
use super::super::super::edit_tests::{open_section, open_settings, saved, write};
use super::super::super::tests::launch;
use super::Fake;
use crate::desktop::tests::fixture;
use freshkube_delivery::discover::{BuildFindings, BuildRead, NamespaceCounts, example};
use freshkube_ui::real_io::RealIo;
use gpui_kit::{AnyWindowHandle, AppContext, Entity, TestAppContext, test::TestWindowExt};
use std::path::Path;
use std::sync::Arc;

type View = Entity<crate::desktop::Pilot>;

/// Two entries, a delivery block with a Stage naming and a key this version
/// doesn't know, which an accepted suggestion must keep.
const FILE: &str = r#"{"version":1,"clusters":[
    {"id":"core-fra","role":"core","context":"core-fra"},
    {"id":"cicd-fra","role":"cicd","context":"cicd-fra"}],
  "delivery":{"stage_naming":{"name_template":"{stage}-{project}"},"future":{"keep":true}}}"#;

fn findings(counts: &[(&str, u64)]) -> BuildFindings {
    BuildFindings::runs_only(
        "cicd-fra",
        BuildRead::Done(NamespaceCounts {
            counts: counts.iter().map(|(n, c)| ((*n).to_owned(), *c)).collect(),
            capped: false,
        }),
    )
}

fn list(view: &View, cx: &gpui_kit::App) -> Entity<super::super::SourcesList> {
    view.read(cx).settings_page.read(cx).sources.clone()
}

fn answer_with(view: &View, answer: BuildFindings, cx: &mut gpui_kit::App) {
    let fake: Fake = Arc::new(move || {
        let answer = answer.clone();
        Box::pin(async move { answer })
    });
    list(view, cx).update(cx, |list, _| list.finder.fake = Some(fake));
}

async fn found(cx: &mut TestAppContext, handle: AnyWindowHandle, view: &View) {
    let view = view.clone();
    cx.wait_for_io(handle, move |_, cx| {
        let list = list(&view, cx);
        let finder = &list.read(cx).finder;
        !finder.read.is_loading() && finder.read.data().is_some()
    })
    .await;
    cx.run_until_parked();
}

async fn start(
    cx: &mut TestAppContext,
    directory: &Path,
    answer: BuildFindings,
) -> (tokio::runtime::Runtime, AnyWindowHandle, View) {
    write(directory, FILE);
    let (runtime, handle, view) = launch(cx, directory);
    open_settings(cx, handle, &view).await;
    cx.update_window(handle, |_, window, cx| {
        answer_with(&view, answer, cx);
        view.update(cx, |pilot, cx| {
            pilot.show_settings(SettingsSection::Sources, window, cx)
        });
        window.render_frame(cx);
    })
    .unwrap();
    (runtime, handle, view)
}

fn suggestion(view: &View, cx: &TestAppContext) -> Option<(String, String)> {
    cx.read(|cx| {
        list(view, cx)
            .read(cx)
            .suggestion()
            .map(|(entry, state)| (entry.to_owned(), state.to_owned()))
    })
}

#[gpui_kit::test]
async fn find_lists_the_suggestion_in_its_group_with_its_evidence(cx: &mut TestAppContext) {
    cx.executor().allow_parking();
    let guard = tempfile::tempdir().unwrap();
    let (_runtime, handle, view) = start(cx, guard.path(), findings(&[("shop-ci", 6)])).await;
    cx.update_window(handle, |_, window, cx| {
        // Nothing is read until Find is pressed.
        assert!(
            window
                .try_find("settings-source-build-suggestion")
                .is_none()
        );
        window.click("settings-sources-find", cx);
    })
    .unwrap();
    found(cx, handle, &view).await;
    assert_eq!(
        suggestion(&view, cx),
        Some(("shop-ci".to_owned(), "Suggested".to_owned()))
    );
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("settings-source-build-suggestion", cx);
        window.render_frame(cx);
        assert!(window.find("settings-source-evidence-shop-ci").visible());
        let status = view.read(cx).settings_page.read(cx).status().unwrap();
        assert!(
            status.text(None).contains("1 suggested"),
            "{}",
            status.text(None)
        );
    })
    .unwrap();
    assert_eq!(
        std::fs::read_to_string(guard.path().join("freshkube.yaml")).unwrap(),
        FILE
    );
}

#[gpui_kit::test]
async fn accept_writes_only_delivery_build_namespace(cx: &mut TestAppContext) {
    cx.executor().allow_parking();
    let guard = tempfile::tempdir().unwrap();
    let (_runtime, handle, view) = start(cx, guard.path(), findings(&[("shop-ci", 6)])).await;
    cx.update_window(handle, |_, window, cx| {
        window.click("settings-sources-find", cx)
    })
    .unwrap();
    found(cx, handle, &view).await;
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("settings-source-build-suggestion", cx);
        window.press("enter", cx);
    })
    .unwrap();
    saved(cx, handle, &view).await;
    let text = std::fs::read_to_string(guard.path().join("freshkube.yaml")).unwrap();
    let setup = freshkube_core::setup::parse(text.as_bytes()).unwrap();
    let delivery = setup.delivery.unwrap();
    assert_eq!(delivery.build_namespace.as_deref(), Some("shop-ci"));
    assert_eq!(
        delivery.stage_naming.unwrap().name_template,
        "{stage}-{project}"
    );
    assert!(text.contains("future"), "unknown keys are kept: {text}");
    assert_eq!(setup.workspace.clusters.len(), 2);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(
            window
                .find("settings-saved")
                .label()
                .unwrap()
                .starts_with("Set delivery.build_namespace to shop-ci.")
        );
    })
    .unwrap();
    // The value is now set, so it is a row and the suggestion is gone.
    assert_eq!(suggestion(&view, cx), None);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(window.find("settings-source-build-namespace").visible());
    })
    .unwrap();
}

#[gpui_kit::test]
async fn accept_over_a_changed_file_writes_nothing(cx: &mut TestAppContext) {
    cx.executor().allow_parking();
    let guard = tempfile::tempdir().unwrap();
    let (_runtime, handle, view) = start(cx, guard.path(), findings(&[("shop-ci", 6)])).await;
    cx.update_window(handle, |_, window, cx| {
        window.click("settings-sources-find", cx)
    })
    .unwrap();
    found(cx, handle, &view).await;
    let edited = FILE.replace("keep", "kept");
    write(guard.path(), &edited);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("settings-source-build-suggestion", cx);
        window.press("a", cx);
    })
    .unwrap();
    saved(cx, handle, &view).await;
    assert_eq!(
        std::fs::read_to_string(guard.path().join("freshkube.yaml")).unwrap(),
        edited
    );
    assert!(suggestion(&view, cx).is_some(), "the item stays listed");
}

#[gpui_kit::test]
async fn ignore_writes_nothing_and_stays_hidden(cx: &mut TestAppContext) {
    cx.executor().allow_parking();
    let guard = tempfile::tempdir().unwrap();
    let (_runtime, handle, view) = start(cx, guard.path(), findings(&[("shop-ci", 6)])).await;
    cx.update_window(handle, |_, window, cx| {
        window.click("settings-sources-find", cx)
    })
    .unwrap();
    found(cx, handle, &view).await;
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("settings-source-build-suggestion", cx);
        window.press("backspace", cx);
    })
    .unwrap();
    cx.run_until_parked();
    assert_eq!(suggestion(&view, cx), None);
    cx.update_window(handle, |_, window, cx| {
        window.click("settings-sources-find", cx)
    })
    .unwrap();
    found(cx, handle, &view).await;
    assert_eq!(suggestion(&view, cx), None);
    cx.update_window(handle, |_, _, cx| {
        let status = view.read(cx).settings_page.read(cx).status().unwrap();
        assert!(
            status.text(None).contains("1 ignored"),
            "{}",
            status.text(None)
        );
    })
    .unwrap();
    assert_eq!(
        std::fs::read_to_string(guard.path().join("freshkube.yaml")).unwrap(),
        FILE
    );
}

#[gpui_kit::test]
async fn several_namespaces_and_a_refusal_have_no_accept(cx: &mut TestAppContext) {
    cx.executor().allow_parking();
    let guard = tempfile::tempdir().unwrap();
    let (_runtime, handle, view) = start(cx, guard.path(), example::several_build_findings()).await;
    cx.update_window(handle, |_, window, cx| {
        window.click("settings-sources-find", cx)
    })
    .unwrap();
    found(cx, handle, &view).await;
    assert_eq!(
        suggestion(&view, cx),
        Some(("3 namespaces".to_owned(), "Several namespaces".to_owned()))
    );
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("settings-source-build-suggestion", cx);
        window.press("enter", cx);
    })
    .unwrap();
    cx.run_until_parked();
    assert_eq!(
        std::fs::read_to_string(guard.path().join("freshkube.yaml")).unwrap(),
        FILE
    );
    // A refused list suggests nothing, and says so.
    cx.update_window(handle, |_, _, cx| {
        answer_with(
            &view,
            BuildFindings::runs_only(
                "cicd-fra",
                BuildRead::Refused("Builds refused on cicd-fra".into()),
            ),
            cx,
        );
    })
    .unwrap();
    cx.update_window(handle, |_, window, cx| {
        window.click("settings-sources-find", cx)
    })
    .unwrap();
    found(cx, handle, &view).await;
    assert_eq!(suggestion(&view, cx), None);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(window.find("settings-sources-note-0").visible());
    })
    .unwrap();
}

#[gpui_kit::test]
async fn hiding_drops_find(cx: &mut TestAppContext) {
    cx.executor().allow_parking();
    let guard = tempfile::tempdir().unwrap();
    let (_runtime, handle, view) = start(cx, guard.path(), findings(&[("shop-ci", 6)])).await;
    let (send, receive) = tokio::sync::oneshot::channel::<()>();
    let gate = Arc::new(std::sync::Mutex::new(Some(receive)));
    cx.update_window(handle, |_, window, cx| {
        let late = findings(&[("shop-ci", 6)]);
        list(&view, cx).update(cx, |list, _| {
            list.finder.fake = Some(Arc::new(move || {
                let (gate, late) = (gate.clone(), late.clone());
                Box::pin(async move {
                    let receive = gate.lock().unwrap().take();
                    if let Some(receive) = receive {
                        _ = receive.await;
                    }
                    late
                })
            }));
        });
        window.click("settings-sources-find", cx);
        assert!(list(&view, cx).read(cx).finder.read.is_loading());
    })
    .unwrap();
    open_section(cx, handle, &view, SettingsSection::Clusters).await;
    cx.update_window(handle, |_, _, cx| {
        assert!(!list(&view, cx).read(cx).finder.read.is_loading());
    })
    .unwrap();
    _ = send.send(());
    cx.run_until_parked();
    open_section(cx, handle, &view, SettingsSection::Sources).await;
    assert_eq!(suggestion(&view, cx), None);
    cx.read(|cx| assert!(list(&view, cx).read(cx).finder.read.data().is_none()));
}

#[gpui_kit::test]
fn example_data_answers_at_once_and_cannot_accept(cx: &mut TestAppContext) {
    let (_runtime, handle, view) = fixture(cx, 1280., 880.);
    cx.update_window(handle, |_, window, cx| {
        view.update(cx, |pilot, cx| {
            pilot.show_settings(SettingsSection::Sources, window, cx)
        });
        window.render_frame(cx);
        window.click("settings-sources-find", cx);
        window.render_frame(cx);
        window.click("settings-source-build-suggestion", cx);
        window.press("enter", cx);
        window.render_frame(cx);
        assert!(window.try_find("settings-saved").is_none());
    })
    .unwrap();
    assert_eq!(
        suggestion(&view, cx),
        Some(("{project}".to_owned(), "Partial fit".to_owned()))
    );
    cx.read(|cx| {
        assert!(view.read(cx).settings_page.read(cx).saving.is_none());
    });
}

#[gpui_kit::test]
fn the_page_with_the_suggestion_keeps_the_table_layout(cx: &mut TestAppContext) {
    use crate::desktop::layout_check;
    const PAGE: layout_check::TablePage = layout_check::TablePage {
        page: "settings-page",
        title: "settings-sources-title",
        title_text: "Delivery sources",
        table: "settings-sources-table-scroll",
        list: "settings-sources-list",
    };
    for (width, height) in [(1280., 880.), (760., 560.)] {
        let (_runtime, handle, view) = fixture(cx, width, height);
        cx.update_window(handle, |_, window, cx| {
            view.update(cx, |pilot, cx| {
                pilot.show_settings(SettingsSection::Sources, window, cx)
            });
            window.render_frame(cx);
            window.click("settings-sources-find", cx);
            window.render_frame(cx);
            layout_check::assert_table_page(window, cx, &PAGE);
        })
        .unwrap();
    }
}

#[gpui_kit::test]
fn partial_fit_shows_warn_and_the_unexplained(cx: &mut TestAppContext) {
    let (_runtime, handle, view) = fixture(cx, 1280., 1200.);
    cx.update_window(handle, |_, window, cx| {
        view.update(cx, |pilot, cx| {
            pilot.show_settings(SettingsSection::Sources, window, cx)
        });
        window.render_frame(cx);
        window.click("settings-sources-find", cx);
        window.render_frame(cx);
        // The partial fit is marked, and its row says which template.
        assert!(
            window
                .find("settings-source-build-suggestion-mark")
                .visible()
        );
        window.click("settings-source-build-suggestion", cx);
        window.render_frame(cx);
        assert!(window.find("settings-source-evidence-fits").visible());
        assert!(window.find("settings-source-evidence-no-runs").visible());
        assert!(
            window
                .find("settings-source-evidence-unexplained")
                .visible()
        );
        assert!(window.find("settings-source-evidence-ci-shared").visible());
    })
    .unwrap();
}

#[gpui_kit::test]
async fn accept_writes_a_template_as_the_build_namespace(cx: &mut TestAppContext) {
    cx.executor().allow_parking();
    let guard = tempfile::tempdir().unwrap();
    let (_runtime, handle, view) = start(cx, guard.path(), example::build_findings()).await;
    cx.update_window(handle, |_, window, cx| {
        window.click("settings-sources-find", cx)
    })
    .unwrap();
    found(cx, handle, &view).await;
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("settings-source-build-suggestion", cx);
        window.press("enter", cx);
    })
    .unwrap();
    saved(cx, handle, &view).await;
    let text = std::fs::read_to_string(guard.path().join("freshkube.yaml")).unwrap();
    let setup = freshkube_core::setup::parse(text.as_bytes()).unwrap();
    assert_eq!(
        setup.delivery.unwrap().build_namespace.as_deref(),
        Some("{project}")
    );
}

fn notes(view: &View, cx: &TestAppContext) -> Vec<String> {
    cx.read(|cx| {
        list(view, cx)
            .read(cx)
            .finder
            .notes
            .iter()
            .map(|(_, text)| text.to_string())
            .collect()
    })
}

#[gpui_kit::test]
async fn a_failed_find_says_so_and_suggests_nothing(cx: &mut TestAppContext) {
    cx.executor().allow_parking();
    let guard = tempfile::tempdir().unwrap();
    let (_runtime, handle, view) = start(cx, guard.path(), findings(&[("shop-ci", 6)])).await;
    cx.update_window(handle, |_, window, cx| {
        list(&view, cx).update(cx, |list, _| {
            list.finder.deadline = std::time::Duration::from_millis(20);
            list.finder.fake = Some(Arc::new(|| {
                Box::pin(std::future::pending::<BuildFindings>())
            }));
        });
        window.click("settings-sources-find", cx);
    })
    .unwrap();
    let waited = view.clone();
    cx.wait_for_io(handle, move |_, cx| {
        let list = list(&waited, cx);
        let finder = &list.read(cx).finder;
        !finder.read.is_loading() && finder.read.error().is_some()
    })
    .await;
    cx.run_until_parked();
    assert_eq!(suggestion(&view, cx), None);
    assert!(
        notes(&view, cx)
            .iter()
            .any(|text| text.starts_with("Finding suggestions failed")),
        "{:?}",
        notes(&view, cx)
    );
    assert_eq!(
        std::fs::read_to_string(guard.path().join("freshkube.yaml")).unwrap(),
        FILE
    );
}

#[gpui_kit::test]
async fn while_finding_nothing_is_listed_and_find_does_not_restart(cx: &mut TestAppContext) {
    cx.executor().allow_parking();
    let guard = tempfile::tempdir().unwrap();
    let (_runtime, handle, view) = start(cx, guard.path(), findings(&[("shop-ci", 6)])).await;
    let (send, receive) = tokio::sync::oneshot::channel::<()>();
    let gate = Arc::new(std::sync::Mutex::new(Some(receive)));
    let started = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    cx.update_window(handle, |_, window, cx| {
        let (counted, late) = (started.clone(), findings(&[("shop-ci", 6)]));
        list(&view, cx).update(cx, |list, _| {
            list.finder.fake = Some(Arc::new(move || {
                counted.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                let (gate, late) = (gate.clone(), late.clone());
                Box::pin(async move {
                    let receive = gate.lock().unwrap().take();
                    if let Some(receive) = receive {
                        _ = receive.await;
                    }
                    late
                })
            }));
        });
        window.click("settings-sources-find", cx);
        window.press("f", cx);
        window.press("f", cx);
        window.click("settings-sources-find", cx);
        window.render_frame(cx);
        assert!(list(&view, cx).read(cx).finder.read.is_loading());
        assert!(
            window
                .try_find("settings-source-build-suggestion")
                .is_none()
        );
    })
    .unwrap();
    assert_eq!(suggestion(&view, cx), None);
    _ = send.send(());
    found(cx, handle, &view).await;
    assert_eq!(started.load(std::sync::atomic::Ordering::SeqCst), 1);
    assert!(suggestion(&view, cx).is_some());
    // One group and one row, however many times Find was pressed.
    cx.read(|cx| {
        let list = list(&view, cx);
        let list = list.read(cx);
        assert_eq!(
            freshkube_ui::table::TableSource::line_count(list),
            list.rows.len() + 2
        );
    });
}

#[gpui_kit::test]
async fn no_runs_is_said_and_suggests_nothing(cx: &mut TestAppContext) {
    cx.executor().allow_parking();
    let guard = tempfile::tempdir().unwrap();
    let (_runtime, handle, view) = start(cx, guard.path(), findings(&[])).await;
    cx.update_window(handle, |_, window, cx| {
        window.click("settings-sources-find", cx)
    })
    .unwrap();
    found(cx, handle, &view).await;
    assert_eq!(suggestion(&view, cx), None);
    assert_eq!(notes(&view, cx), ["No PipelineRuns found"]);
}

#[gpui_kit::test]
async fn keys_with_modifiers_and_without_a_suggestion_do_nothing(cx: &mut TestAppContext) {
    cx.executor().allow_parking();
    let guard = tempfile::tempdir().unwrap();
    let (_runtime, handle, view) = start(cx, guard.path(), findings(&[("shop-ci", 6)])).await;
    cx.update_window(handle, |_, window, cx| {
        window.click("settings-sources-find", cx)
    })
    .unwrap();
    found(cx, handle, &view).await;
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("settings-source-build-suggestion", cx);
        window.press("secondary-a", cx);
        window.press("ctrl-a", cx);
    })
    .unwrap();
    cx.run_until_parked();
    assert_eq!(
        std::fs::read_to_string(guard.path().join("freshkube.yaml")).unwrap(),
        FILE
    );
    assert!(suggestion(&view, cx).is_some());
    // With nothing selected, Enter and Backspace have no suggestion to act on.
    cx.update_window(handle, |_, window, cx| {
        list(&view, cx).update(cx, |list, cx| {
            list.selected = None;
            cx.notify();
        });
        window.press("enter", cx);
        window.press("backspace", cx);
    })
    .unwrap();
    cx.run_until_parked();
    assert_eq!(
        std::fs::read_to_string(guard.path().join("freshkube.yaml")).unwrap(),
        FILE
    );
    assert!(suggestion(&view, cx).is_some(), "nothing was ignored");
}
