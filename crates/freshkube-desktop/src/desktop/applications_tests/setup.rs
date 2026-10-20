//! The real setup owner and shell hand-off, with invented files and example
//! delivery reads. No configuration or credentials outside the temporary folder.
use super::*;
use crate::resources::{ClusterAccess, ClusterSource};
use gpui_kit::base::test_support;
use gpui_kit::{App, ElementId, Window};

const GOOD: &str = "version: 1\nclusters: []\n";
const BROKEN: &str = "version: [\n# invented-private-marker\n";

fn applications(view: &Entity<Pilot>, cx: &App) -> Entity<ApplicationsPage> {
    view.read(cx).applications().0
}

fn open_changes(view: &Entity<Pilot>, window: &mut Window, cx: &mut App) {
    view.update(cx, |pilot, cx| {
        pilot.navigate_from_keyboard(Page::Applications, window, cx)
    });
    applications(view, cx).update(cx, |page, cx| page.show_changes(window, cx));
    window.render_frame(cx);
}

fn follow(view: &Entity<Pilot>, from_list: bool, window: &mut Window, cx: &mut App) {
    if from_list {
        let changes = applications(view, cx).read(cx).changes_page().clone();
        changes.update(cx, |page, cx| {
            page.select("checkout/wonky-otter", window, cx)
        });
        changes.update(cx, |page, cx| page.focus(window, cx));
        window.press("enter", cx);
    } else {
        applications(view, cx).update(cx, |page, cx| {
            page.show_list(window, cx);
            page.open_named("checkout", window, cx);
            page.follow_named("prod-ams", window, cx);
        });
    }
}

fn assert_banner(window: &mut Window, cx: &mut App, id: &'static str, words: Option<&str>) {
    window.render_frame(cx);
    if let Some(words) = words {
        let banner = window.find(id);
        assert!(banner.visible());
        let label = banner.label().unwrap();
        if id == "change-missing" {
            assert!(label.contains(words), "{label}");
            assert_eq!(label.matches("freshkube.yaml").count(), 1);
        } else {
            assert_eq!(label, words);
        }
        assert!(!words.contains("invented-private-marker"));
        let id = ElementId::from(id);
        assert_eq!(
            test_support::snapshots(window)
                .iter()
                .filter(|element| element.path().last() == Some(&id))
                .count(),
            1,
            "one persistent banner, including after Settings notifies"
        );
    } else if id == "change-missing" {
        assert!(window.try_find(id).is_none_or(|banner| {
            !banner
                .label()
                .unwrap_or_default()
                .contains("freshkube.yaml")
        }));
    } else {
        assert!(window.try_find(id).is_none());
    }
}

fn notify_settings(
    view: &Entity<Pilot>,
    handle: AnyWindowHandle,
    id: &'static str,
    problem: &str,
    cx: &mut TestAppContext,
) {
    let handoffs = freshkube_probe::probe::count("applications.setup_handoff");
    cx.update(|cx| {
        view.read(cx)
            .settings_page
            .clone()
            .update(cx, |_, cx| cx.notify());
    });
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.simulate_next_frame(cx);
        assert_banner(window, cx, id, Some(problem));
    })
    .unwrap();
    assert_eq!(
        freshkube_probe::probe::count("applications.setup_handoff"),
        handoffs
    );
}

async fn reload(view: &Entity<Pilot>, cx: &mut TestAppContext) {
    let file = cx.read(|cx| view.read(cx).setup_file.clone());
    file.update(cx, |file, cx| file.reload(cx)).await;
    cx.run_until_parked();
}

/// Both entry routes inherit a warning; both already-open pages update on a
/// new problem and recover when the exact last-good setup is reloaded.
#[gpui_kit::test]
async fn unused_setup_warns_on_both_pages_and_clears_on_reload(cx: &mut TestAppContext) {
    cx.executor().allow_parking();
    for last_good in [false, true] {
        for from_list in [false, true] {
            let directory = tempfile::tempdir().unwrap();
            let path = directory.path().join("freshkube.yaml");
            std::fs::write(&path, if last_good { GOOD } else { BROKEN }).unwrap();
            let options =
                crate::GpuiOptions::new(Some(directory.path().join("missing-talos")), None, 100)
                    .with_kubeconfig(directory.path().join("missing-kubeconfig"))
                    .with_preferences(Some(directory.path().join("preferences.json")));
            let (_runtime, handle, view) = crate::desktop::testing::mount(cx, options, 1280., 880.);
            // Only the setup is read from disk. All delivery data is invented.
            cx.update_window(handle, |_, _, cx| {
                applications(&view, cx).update(cx, |page, cx| {
                    page.set_source(
                        Some(ClusterSource {
                            id: "example-session".into(),
                            context: "example-cluster".into(),
                            access: ClusterAccess::EXAMPLE,
                        }),
                        None,
                        cx,
                    );
                });
            })
            .unwrap();
            if last_good {
                std::fs::write(&path, BROKEN).unwrap();
                reload(&view, cx).await;
            }
            let problem = cx.read(|cx| view.read(cx).setup_file.read(cx).problem.clone().unwrap());
            assert!(problem.starts_with("freshkube.yaml isn't used: invalid YAML at line"));
            assert!(problem.ends_with(if last_good {
                "The last good setup stays in use."
            } else {
                "Freshkube starts as with no file."
            }));
            cx.update_window(handle, |_, window, cx| {
                open_changes(&view, window, cx);
                assert_banner(window, cx, "changes-setup-problem", Some(&problem));
            })
            .unwrap();
            notify_settings(&view, handle, "changes-setup-problem", &problem, cx);
            cx.update_window(handle, |_, window, cx| follow(&view, from_list, window, cx))
                .unwrap();
            cx.run_until_parked();
            notify_settings(&view, handle, "change-missing", &problem, cx);

            // The setup remains unchanged (or absent), but its reason changes.
            std::fs::write(&path, "version: 999\n").unwrap();
            reload(&view, cx).await;
            let different =
                cx.read(|cx| view.read(cx).setup_file.read(cx).problem.clone().unwrap());
            assert_ne!(different, problem);
            cx.update_window(handle, |_, window, cx| {
                assert_banner(window, cx, "change-missing", Some(&different));
            })
            .unwrap();
            std::fs::write(&path, GOOD).unwrap();
            reload(&view, cx).await;
            cx.update_window(handle, |_, window, cx| {
                assert_banner(window, cx, "change-missing", None);
                open_changes(&view, window, cx);
                assert_banner(window, cx, "changes-setup-problem", None);
                follow(&view, from_list, window, cx);
            })
            .unwrap();
            cx.run_until_parked();

            // A save can recreate a deleted file with the same setup. It
            // must clear the already-open page as well as the hidden list.
            std::fs::remove_file(&path).unwrap();
            reload(&view, cx).await;
            let gone = cx.read(|cx| view.read(cx).setup_file.read(cx).problem.clone().unwrap());
            cx.update_window(handle, |_, window, cx| {
                assert_banner(window, cx, "change-missing", Some(&gone));
            })
            .unwrap();
            let file = cx.read(|cx| view.read(cx).setup_file.clone());
            file.update(cx, |file, cx| {
                file.save_with(freshkube_core::workspace::Seen::Missing, |_| {}, cx)
            })
            .await
            .unwrap();
            cx.run_until_parked();
            cx.update_window(handle, |_, window, cx| {
                assert_banner(window, cx, "change-missing", None);
                open_changes(&view, window, cx);
                assert_banner(window, cx, "changes-setup-problem", None);
            })
            .unwrap();
        }
    }
}
