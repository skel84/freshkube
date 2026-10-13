//! The delivery sources list in the real app: where each part is read,
//! and what Check sources finds, in each state.
use super::super::edit_tests::{open_settings, write};
use super::super::tests::launch;
#[cfg(unix)]
use super::Check;
use super::Read;
use crate::desktop::{Page, layout_check, tests::fixture};
use freshkube_core::workspace::{self, Destination, Entry, Key, Loaded, Part, Role, Workspace};
#[cfg(unix)]
use freshkube_delivery::entry::Checked;
use freshkube_delivery::entry::Reach;
use freshkube_ui::real_io::RealIo;
use gpui_kit::{AppContext, TestAppContext, test::TestWindowExt};
#[cfg(unix)]
use std::time::Duration;

const FRAME: layout_check::PageFrame = layout_check::PageFrame {
    page: "settings-page",
    title: "settings-sources-title",
    title_text: "Delivery sources",
    content: "settings-sources-table-scroll",
};

const TABLE: layout_check::Table = layout_check::Table {
    table: Some("settings-sources-table-scroll"),
    list: "settings-sources-list",
};

const KARGO: Read = Read::Part(Part::Kargo);
const BUILDS: Read = Read::Part(Part::Builds);
const ARGOCD: Read = Read::Part(Part::ArgoCd);
const WORKLOADS: Read = Read::Workloads;
/// The example's destinations, in its order.
const MAPPED: &str = "dev-fra, stage-fra, prod-ams";

type View = gpui_kit::Entity<crate::desktop::Pilot>;
type List = gpui_kit::Entity<super::SourcesList>;

fn list(view: &View, cx: &gpui_kit::App) -> List {
    view.read(cx).settings_page.read(cx).sources.clone()
}

/// A completion token for answering the pending check in the test, without
/// replacing its work. Loader's test helper begins the synthetic answer.
#[cfg(unix)]
fn request(
    list: &List,
    cx: &mut gpui_kit::App,
) -> freshkube_core::snapshot::Request<super::CheckIdentity> {
    list.update(cx, |list, _| list.read.begin(list.identity()))
}

fn states(view: &View, cx: &gpui_kit::App) -> Vec<(Read, String, String)> {
    list(view, cx)
        .read(cx)
        .states()
        .into_iter()
        .map(|(read, entry, state)| (read, entry.to_owned(), state.to_owned()))
        .collect()
}

fn state(view: &View, read: Read, cx: &gpui_kit::App) -> String {
    let states = states(view, cx);
    let found = states.iter().find(|(row, _, _)| *row == read);
    found.map(|(_, _, state)| state.clone()).unwrap_or_default()
}

fn tooltip(view: &View, read: Read, cx: &gpui_kit::App) -> String {
    let list = list(view, cx);
    let list = list.read(cx);
    let row = list.rows.iter().find(|row| row.read == read).unwrap();
    row.tooltip.to_string()
}

fn row(read: Read, entry: &str, state: &str) -> (Read, String, String) {
    (read, entry.to_owned(), state.to_owned())
}

fn open(window: &mut gpui_kit::Window, cx: &mut gpui_kit::App, view: &View) {
    view.update(cx, |pilot, cx| {
        pilot.navigate_from_keyboard(Page::Settings, window, cx)
    });
    window.render_frame(cx);
}

/// Scrolls the page to its last list.
fn to_the_bottom(window: &mut gpui_kit::Window, cx: &mut gpui_kit::App) {
    window.scroll(
        "settings-page",
        gpui_kit::ScrollDelta::Pixels(gpui_kit::point(gpui_kit::px(0.), gpui_kit::px(-5000.))),
        cx,
    );
    window.render_frame(cx);
}

fn set_workspace(view: &View, workspace: Workspace, example: bool, cx: &mut gpui_kit::App) {
    view.update(cx, |pilot, cx| {
        pilot.settings_page.update(cx, |page, cx| {
            page.set_workspace(&Loaded::Workspace(workspace), example, cx)
        })
    });
}

#[gpui_kit::test]
fn the_example_lists_each_part_under_the_destinations_and_reads_nothing(cx: &mut TestAppContext) {
    let (_runtime, handle, view) = fixture(cx, 1280., 880.);
    for size in [None, Some(20.)] {
        cx.update_window(handle, |_, window, cx| {
            if let Some(size) = size {
                crate::text_size::set(size, cx);
            }
            open(window, cx, &view);
            to_the_bottom(window, cx);
            layout_check::assert_edge_frame(window, cx, &FRAME);
            let rows = layout_check::assert_table(window, cx, &TABLE);
            assert!(rows.header.is_some(), "{rows:#?}");
            for read in ["kargo", "builds", "argocd", "workloads"] {
                assert!(window.find(format!("settings-source-{read}")).visible());
            }
            let below = window.find("settings-sources-title").bounds().top();
            let above = window.find("settings-destinations-list").bounds();
            assert!(above.bottom() <= below, "{above:?} {below:?}");
            assert_eq!(
                states(&view, cx),
                vec![
                    row(KARGO, "cicd-fra", "Not checked"),
                    row(BUILDS, "cicd-fra", "Not checked"),
                    row(ARGOCD, "core-fra", "Not checked"),
                    row(WORKLOADS, MAPPED, "Not checked"),
                ]
            );
            assert!(!list(&view, cx).read(cx).is_checking());
            // Nothing is checked, so no glyph says anything yet.
            assert!(window.try_find("settings-source-kargo-mark").is_none());
            // Reads is cut at a word in a narrow window; the row's tooltip
            // holds the whole of it, as the cell's own does.
            let builds = tooltip(&view, BUILDS, cx);
            assert!(
                builds.contains("Reads PipelineRuns and TaskRuns from Pipelines as Code"),
                "{builds}"
            );
        })
        .unwrap();
    }
}

#[gpui_kit::test]
fn check_sources_finds_each_state(cx: &mut TestAppContext) {
    let (_runtime, handle, view) = fixture(cx, 1280., 880.);
    cx.update_window(handle, |_, window, cx| {
        open(window, cx, &view);
        to_the_bottom(window, cx);
        list(&view, cx).update(cx, |list, _| {
            list.set_example_answer("cicd-fra", KARGO, Reach::refused("forbidden"));
            list.set_example_answer(
                "cicd-fra",
                BUILDS,
                Reach::unreachable("no answer within 5 s"),
            );
            list.set_example_answer("core-fra", ARGOCD, Reach::not_served("not served"));
            list.set_example_answer(
                "prod-ams",
                WORKLOADS,
                Reach::unreachable("no answer within 5 s"),
            );
        });
        window.click("settings-sources-check", cx);
        window.render_frame(cx);
        assert_eq!(
            states(&view, cx),
            vec![
                row(KARGO, "cicd-fra", "Refused"),
                row(BUILDS, "cicd-fra", "Unreachable"),
                row(ARGOCD, "core-fra", "Not served"),
                row(WORKLOADS, MAPPED, "2 readable, 1 unreachable"),
            ]
        );
        for read in ["kargo", "builds", "argocd", "workloads"] {
            let mark = format!("settings-source-{read}-mark");
            assert!(window.find(mark).visible(), "{read}");
        }
        assert_eq!(
            window.find("settings-source-kargo-mark").label(),
            Some("Refused")
        );
        // Each workloads entry says its own state.
        let workloads = tooltip(&view, WORKLOADS, cx);
        assert!(
            workloads.contains("Workloads answered on dev-fra")
                && workloads.contains("Couldn't reach prod-ams: no answer within 5 s"),
            "{workloads}"
        );
        // Argo CD says where it was read.
        let argocd = tooltip(&view, ARGOCD, cx);
        assert!(argocd.contains("argocd-cm in argocd"), "{argocd}");

        // Checked again, everything answers.
        list(&view, cx).update(cx, |list, _| list.example_answers.clear());
        window.click("settings-sources-check", cx);
        window.render_frame(cx);
        assert_eq!(state(&view, WORKLOADS, cx), "3 readable");
        assert!(
            states(&view, cx)[..3]
                .iter()
                .all(|(_, _, state)| state == "Readable")
        );
    })
    .unwrap();
}

#[gpui_kit::test]
fn a_part_with_no_entry_says_so_before_any_check(cx: &mut TestAppContext) {
    let (_runtime, handle, view) = fixture(cx, 1280., 880.);
    cx.update_window(handle, |_, window, cx| {
        // No core entry, Builds pinned to an entry that is gone, and no
        // destination mapped.
        let mut lone = Workspace::new(vec![
            Entry::new("cluster-b", Role::Cicd, "cluster-b"),
            Entry::new("cluster-c", Role::Environment, "cluster-c"),
        ]);
        lone.sources.set(Part::Builds, Some("cluster-x".into()));
        set_workspace(&view, lone, true, cx);
        open(window, cx, &view);
        to_the_bottom(window, cx);
        assert_eq!(
            states(&view, cx),
            vec![
                row(KARGO, "cluster-b", "Not checked"),
                row(BUILDS, "cluster-x", "Not in the workspace"),
                row(ARGOCD, "None", "Not in the workspace"),
                row(WORKLOADS, "None", "Not in the workspace"),
            ]
        );
        assert!(window.find("settings-source-argocd-mark").visible());
        window.click("settings-sources-check", cx);
        window.render_frame(cx);
        assert_eq!(
            states(&view, cx),
            vec![
                row(KARGO, "cluster-b", "Readable"),
                row(BUILDS, "cluster-x", "Not in the workspace"),
                row(ARGOCD, "None", "Not in the workspace"),
                row(WORKLOADS, "None", "Not in the workspace"),
            ]
        );
        let argocd = tooltip(&view, ARGOCD, cx);
        assert!(
            argocd.contains("No Core entry in the workspace"),
            "{argocd}"
        );
        let workloads = tooltip(&view, WORKLOADS, cx);
        assert!(
            workloads.contains("map one under Destinations"),
            "{workloads}"
        );
    })
    .unwrap();
}

#[gpui_kit::test]
fn a_workspace_of_one_reads_the_open_cluster(cx: &mut TestAppContext) {
    let (_runtime, handle, view) = fixture(cx, 1280., 880.);
    cx.update_window(handle, |_, window, cx| {
        view.update(cx, |pilot, cx| {
            pilot.settings_page.update(cx, |page, cx| {
                page.set_workspace(&Loaded::Missing, false, cx)
            })
        });
        open(window, cx, &view);
        assert!(
            states(&view, cx)
                .iter()
                .all(|(_, entry, state)| entry == "The open cluster" && state == "Not checked")
        );
        // Example data is open, so its example answers.
        window.click("settings-sources-check", cx);
        window.render_frame(cx);
        assert!(
            states(&view, cx)
                .iter()
                .all(|(_, _, state)| state == "Readable")
        );
        // In a sentence, the open cluster isn't a name.
        let kargo = tooltip(&view, KARGO, cx);
        assert!(
            kargo.contains("Kargo answered on the open cluster"),
            "{kargo}"
        );

        // Another open connection forgets what the check found.
        let sources = list(&view, cx);
        sources.update(cx, |list, cx| list.set_source(None, cx));
        assert!(
            states(&view, cx)
                .iter()
                .all(|(_, _, state)| state == "Not checked")
        );
    })
    .unwrap();
}

#[gpui_kit::test]
fn a_new_workspace_forgets_the_check(cx: &mut TestAppContext) {
    let (_runtime, handle, view) = fixture(cx, 1280., 880.);
    cx.update_window(handle, |_, window, cx| {
        open(window, cx, &view);
        window.click("settings-sources-check", cx);
        window.render_frame(cx);
        assert_eq!(state(&view, KARGO, cx), "Readable");
        set_workspace(&view, workspace::example(), true, cx);
        window.render_frame(cx);
        assert_eq!(state(&view, KARGO, cx), "Not checked");
    })
    .unwrap();
}

#[gpui_kit::test]
fn c_checks_from_the_list(cx: &mut TestAppContext) {
    let (_runtime, handle, view) = fixture(cx, 1280., 880.);
    cx.update_window(handle, |_, window, cx| {
        open(window, cx, &view);
        to_the_bottom(window, cx);
        window.click("settings-source-kargo", cx);
        window.press("c", cx);
        window.render_frame(cx);
        assert_eq!(state(&view, KARGO, cx), "Readable");
    })
    .unwrap();
}

/// Many destinations make the page long: the clusters keep their room and
/// the frame scrolls down to the sources, in a tall window and a short one.
#[gpui_kit::test]
fn twenty_destinations_leave_the_clusters_their_rows(cx: &mut TestAppContext) {
    for (width, height) in [(1280., 880.), (760., 560.)] {
        let (_runtime, handle, view) = fixture(cx, width, height);
        cx.update_window(handle, |_, window, cx| {
            let mut example = workspace::example();
            example.destinations = (0..20)
                .map(|n| Destination::new(Key::Name(format!("shop-api-{n}")), "dev-fra"))
                .collect();
            set_workspace(&view, example, true, cx);
            open(window, cx, &view);
            let clusters = window.find("settings-table-scroll").bounds();
            let least = crate::ui::dp_px(freshkube_ui::page::SHORT_LIST_HEIGHT, window);
            // The floor holds the table's frame; its hairlines are outside.
            assert!(
                clusters.size.height + gpui_kit::px(2.) >= least,
                "{width}x{height}: {clusters:?}"
            );
            assert!(window.find("settings-cluster-core-fra").visible());
            to_the_bottom(window, cx);
            assert!(
                window.find("settings-source-workloads").visible(),
                "{width}x{height}"
            );
            let page = window.find("settings-page").bounds();
            let sources = window.find("settings-sources-list").bounds();
            assert!(sources.bottom() <= page.bottom(), "{sources:?} {page:?}");
        })
        .unwrap();
    }
}

/// A live workspace whose entries' credential plugin sleeps, so a check
/// stays in flight for as long as a test needs.
#[cfg(unix)]
fn waiting_workspace(directory: &std::path::Path) {
    let kubeconfig = directory.join("kubeconfig");
    let user = serde_json::json!({"exec": {
        "apiVersion": "client.authentication.k8s.io/v1beta1",
        "command": "sleep",
        "args": ["2"],
        "interactiveMode": "Never",
    }});
    let config = serde_json::json!({
        "apiVersion": "v1",
        "kind": "Config",
        "clusters": [{"name": "lab", "cluster": {"server": "https://127.0.0.1:1"}}],
        "users": [{"name": "plugin", "user": user}],
        "contexts": [
            {"name": "acme-a", "context": {"cluster": "lab", "user": "plugin"}},
            {"name": "acme-b", "context": {"cluster": "lab", "user": "plugin"}},
        ],
    });
    std::fs::write(&kubeconfig, config.to_string()).unwrap();
    write(
        directory,
        &serde_json::json!({
            "version": 1,
            "kubeconfig": kubeconfig,
            "clusters": [
                {"id": "cluster-a", "role": "core", "context": "acme-a"},
                {"id": "cluster-b", "role": "cicd", "context": "acme-b"},
            ],
        })
        .to_string(),
    );
}

/// A check in flight is dropped by a new workspace and by the page hiding,
/// a press while it runs starts nothing, and a late answer never lands.
#[cfg(unix)]
#[gpui_kit::test]
async fn a_check_in_flight_is_dropped_and_its_answer_never_lands(cx: &mut TestAppContext) {
    cx.executor().allow_parking();
    let guard = tempfile::tempdir().unwrap();
    waiting_workspace(guard.path());
    let (_runtime, handle, view) = launch(cx, guard.path());
    open_settings(cx, handle, &view).await;
    cx.update_window(handle, |_, window, cx| {
        to_the_bottom(window, cx);
        window.click("settings-sources-check", cx);
        window.render_frame(cx);
        assert_eq!(state(&view, KARGO, cx), "Checking…");
        let sources = list(&view, cx);
        let first = sources.read(cx).read.revision();
        let first_request = request(&sources, cx);
        // C while it runs starts nothing.
        window.click("settings-source-kargo", cx);
        window.press("c", cx);
        assert_eq!(sources.read(cx).read.revision(), first);

        // (a) A new workspace drops it, and its answer is ignored.
        let reloaded = view.read(cx).settings_page.read(cx).workspace().clone();
        set_workspace(&view, reloaded, false, cx);
        assert_eq!(state(&view, KARGO, cx), "Not checked");
        assert!(!sources.read(cx).is_checking());
        sources.update(cx, |list, cx| {
            list.answer(&first_request, Ok(Checked::default()), cx)
        });
        assert_eq!(state(&view, KARGO, cx), "Not checked");

        // (b) Checked again, the first answer still never lands.
        window.click("settings-sources-check", cx);
        let second = sources.read(cx).read.revision();
        let second_request = request(&sources, cx);
        assert_ne!(second, first);
        sources.update(cx, |list, cx| {
            list.answer(&first_request, Ok(Checked::default()), cx)
        });
        assert_eq!(state(&view, KARGO, cx), "Checking…");

        // (c) Leaving Settings drops it.
        view.update(cx, |pilot, cx| {
            pilot.navigate_from_keyboard(Page::Overview, window, cx)
        });
        assert!(!sources.read(cx).is_checking());
        assert_eq!(sources.read(cx).check, Check::Never);
        sources.update(cx, |list, cx| {
            list.answer(&second_request, Ok(Checked::default()), cx)
        });
        assert_eq!(state(&view, KARGO, cx), "Not checked");
    })
    .unwrap();
}

/// A check that outlasts its deadline fails, and says so on each row it
/// would have read. The deadline is Tokio's, so the test shortens it rather
/// than advancing GPUI's clock.
#[cfg(unix)]
#[gpui_kit::test]
async fn a_check_past_its_deadline_fails(cx: &mut TestAppContext) {
    cx.executor().allow_parking();
    let guard = tempfile::tempdir().unwrap();
    waiting_workspace(guard.path());
    let (_runtime, handle, view) = launch(cx, guard.path());
    open_settings(cx, handle, &view).await;
    cx.update_window(handle, |_, window, cx| {
        list(&view, cx).update(cx, |list, _| list.deadline = Duration::from_secs(1));
        to_the_bottom(window, cx);
        window.click("settings-sources-check", cx);
    })
    .unwrap();
    let waiting = view.clone();
    cx.wait_for_io(handle, move |_, cx| {
        !list(&waiting, cx).read(cx).is_checking()
    })
    .await;
    cx.update_window(handle, |_, _, cx| {
        assert!(matches!(list(&view, cx).read(cx).check, Check::Failed(_)));
        assert_eq!(state(&view, KARGO, cx), "Unreachable");
        assert_eq!(state(&view, ARGOCD, cx), "Unreachable");
        // No destination is mapped, which needed no read.
        assert_eq!(state(&view, WORKLOADS, cx), "Not in the workspace");
        let kargo = tooltip(&view, KARGO, cx);
        assert!(kargo.contains("took longer than 1 s"), "{kargo}");
    })
    .unwrap();
}

/// An entry the kubeconfig doesn't define is unreachable, and says why; the
/// check reads only the workspace's own kubeconfig, which here is missing.
#[gpui_kit::test]
async fn a_live_entry_missing_from_the_kubeconfig_is_unreachable(cx: &mut TestAppContext) {
    cx.executor().allow_parking();
    let guard = tempfile::tempdir().unwrap();
    let kubeconfig = guard.path().join("kubeconfig");
    write(
        guard.path(),
        &serde_json::json!({
            "version": 1,
            "kubeconfig": kubeconfig,
            "clusters": [
                {"id": "cluster-a", "role": "core", "context": "acme-a"},
                {"id": "cluster-b", "role": "cicd", "context": "acme-b"},
                {"id": "cluster-c", "role": "environment", "context": "acme-c"},
            ],
            "destinations": [{"name": "c", "entry": "cluster-c"}],
        })
        .to_string(),
    );
    let (_runtime, handle, view) = launch(cx, guard.path());
    open_settings(cx, handle, &view).await;
    cx.update_window(handle, |_, window, cx| {
        to_the_bottom(window, cx);
        window.click("settings-sources-check", cx);
        window.render_frame(cx);
        assert!(
            states(&view, cx)
                .iter()
                .all(|(_, _, state)| state == "Checking…")
        );
    })
    .unwrap();
    let waiting = view.clone();
    cx.wait_for_io(handle, move |_, cx| {
        !list(&waiting, cx).read(cx).is_checking()
    })
    .await;
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(
            states(&view, cx),
            vec![
                row(KARGO, "cluster-b", "Unreachable"),
                row(BUILDS, "cluster-b", "Unreachable"),
                row(ARGOCD, "cluster-a", "Unreachable"),
                row(WORKLOADS, "cluster-c", "Unreachable"),
            ]
        );
        let kargo = tooltip(&view, KARGO, cx);
        assert!(
            kargo.contains("Couldn't reach cluster-b: the kubeconfig has no context acme-b"),
            "{kargo}"
        );
    })
    .unwrap();
}
