//! The delivery sources list in the real app: where each part is read,
//! and what Check sources finds, in each state.
use super::super::SettingsSection;
use super::super::edit_tests::{open_section, write};
use super::super::tests::launch;
#[cfg(unix)]
use super::Check;
use super::Read;
#[cfg(unix)]
use crate::desktop::Page;
use crate::desktop::{layout_check, tests::fixture};
use freshkube_core::workspace::{self, Entry, Loaded, Part, Role, Workspace};
#[cfg(unix)]
use freshkube_delivery::entry::Checked;
use freshkube_delivery::entry::Reach;
use freshkube_ui::real_io::RealIo;
use gpui_kit::{AppContext, TestAppContext, test::TestWindowExt};
#[cfg(unix)]
use std::time::Duration;

const PAGE: layout_check::TablePage = layout_check::TablePage {
    page: "settings-page",
    title: "settings-sources-title",
    title_text: "Delivery sources",
    table: "settings-sources-table-scroll",
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
        pilot.show_settings(SettingsSection::Sources, window, cx)
    });
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
fn the_example_lists_each_part_on_a_table_page_and_reads_nothing(cx: &mut TestAppContext) {
    let (_runtime, handle, view) = fixture(cx, 1280., 880.);
    for size in [None, Some(20.)] {
        cx.update_window(handle, |_, window, cx| {
            if let Some(size) = size {
                crate::text_size::set(size, cx);
            }
            open(window, cx, &view);
            layout_check::assert_table_page(window, cx, &PAGE);
            for read in ["kargo", "builds", "argocd", "workloads"] {
                assert!(window.find(format!("settings-source-{read}")).visible());
            }
            assert!(window.try_find("settings-destinations-list").is_none());
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
fn the_glyph_column_stays_at_the_table_edge_and_contains_every_mark(cx: &mut TestAppContext) {
    for (width, height) in [(1280., 880.), (760., 560.)] {
        let (_runtime, handle, view) = fixture(cx, width, height);
        cx.update_window(handle, |_, window, cx| {
            open(window, cx, &view);
            layout_check::assert_table_page(window, cx, &PAGE);
            let table = window.find(PAGE.table).bounds();
            let glyph_width = crate::ui::dp_px(freshkube_ui::table::GLYPH_WIDTH, window);
            let header = gpui_kit::base::test_support::snapshots(window)
                .into_iter()
                .find(|element| element.label() == Some("Part"))
                .expect("Part header")
                .bounds();
            assert!((header.left() - table.left() - glyph_width).abs() <= gpui_kit::px(1.));
            // The reserved column is present even before there are marks.
            for checked in [false, true] {
                if checked {
                    window.click("settings-sources-check", cx);
                    window.render_frame(cx);
                }
                for read in ["kargo", "builds", "argocd", "workloads"] {
                    let glyph = window
                        .find(format!("settings-source-{read}-glyph"))
                        .bounds();
                    assert!(
                        (glyph.left() - table.left() - gpui_kit::px(1.)).abs() <= gpui_kit::px(1.),
                        "glyph column moved at {width}×{height}: {glyph:?} in {table:?}"
                    );
                    assert!((glyph.size.width - glyph_width).abs() <= gpui_kit::px(1.));
                    let part = window.find(format!("settings-source-{read}-part")).bounds();
                    assert!((part.left() - glyph.right()).abs() <= gpui_kit::px(1.));
                    let mark = window.try_find(format!("settings-source-{read}-mark"));
                    if checked {
                        let mark = mark.expect("checked source has a mark");
                        assert!(mark.visible());
                        let mark = mark.bounds();
                        assert!(mark.left() >= glyph.left() && mark.right() <= glyph.right());
                        assert!((mark.center().x - glyph.center().x).abs() <= gpui_kit::px(1.));
                    } else {
                        assert!(mark.is_none());
                    }
                }
            }
        })
        .unwrap();
    }
}

#[gpui_kit::test]
fn every_part_label_fits_whole_after_check_sources_at_both_sizes(cx: &mut TestAppContext) {
    for (width, height) in [(1280., 880.), (760., 560.)] {
        let (_runtime, handle, view) = fixture(cx, width, height);
        cx.update_window(handle, |_, window, cx| {
            open(window, cx, &view);
            layout_check::settle_header(window, cx);
            let before: Vec<_> = [KARGO, BUILDS, ARGOCD, WORKLOADS]
                .map(|read| {
                    window
                        .find(format!("settings-source-{}-label", read.key()))
                        .bounds()
                })
                .into();
            window.click("settings-sources-check", cx);
            layout_check::settle_header(window, cx);
            for (read, before) in [KARGO, BUILDS, ARGOCD, WORKLOADS].into_iter().zip(before) {
                let label = window.find(format!("settings-source-{}-label", read.key()));
                assert!(label.visible());
                let bounds = label.bounds();
                let run = gpui_kit::TextRun {
                    len: read.label().len(),
                    font: gpui_kit::font(crate::ui::MONO_FONT),
                    color: Default::default(),
                    background_color: None,
                    underline: None,
                    strikethrough: None,
                };
                // Shape the whole label in the shared table row's actual font and size.
                let whole = window
                    .text_system()
                    .shape_line(
                        read.label().into(),
                        crate::ui::dp_px(12.5, window),
                        &[run],
                        None,
                    )
                    .width();
                assert!(
                    whole <= bounds.size.width + gpui_kit::px(1.),
                    "{} is cut after Check sources at {width}×{height}: {whole:?} in {bounds:?}",
                    read.label()
                );
                assert_eq!(bounds, before, "{} moved after Check sources", read.label());
            }
        })
        .unwrap();
    }
}

#[gpui_kit::test]
fn reads_truncates_inside_the_table_at_both_window_sizes(cx: &mut TestAppContext) {
    const READS: &str = "settings-source-builds-reads";
    for (width, height) in [(1280., 880.), (760., 560.)] {
        let (_runtime, handle, view) = fixture(cx, width, height);
        cx.update_window(handle, |_, window, cx| {
            open(window, cx, &view);
            if width == 1280. {
                let entry = freshkube_ui::table::word_cut_style("settings-source-workloads-entry").unwrap();
                assert_eq!(entry.text.as_ref(), MAPPED);
                use freshkube_ui::table::TableColumn;
                let sources = list(&view, cx).read(cx);
                for column in &sources.columns {
                    let values = sources.rows.iter().map(|row| if column.label().as_ref() == "Entry" { &row.entry } else { &row.why });
                    if matches!(column.label().as_ref(), "Entry" | "Chosen by") {
                        assert_eq!(column.width(), freshkube_ui::table::fit(column.label(), values, freshkube_ui::table::WIDEST));
                    }
                }
            }
            // Exercise a value too long even for the wide table.
            list(&view, cx).update(cx, |list, cx| {
                let row = list.rows.iter_mut().find(|row| row.read == BUILDS).unwrap();
                row.reads = "PipelineRuns and TaskRuns from Pipelines as Code, including retained runs and their delivery metadata".into();
                cx.notify();
            });
            layout_check::settle_header(window, cx);
            let table = window.find(PAGE.table).bounds();
            let cell = window.find(format!("{READS}-cell")).bounds();
            assert!(
                cell.left() >= table.left() && cell.right() <= table.right() + gpui_kit::px(1.),
                "Reads leaves the viewport at {width}×{height}: {cell:?} in {table:?}"
            );
            let shown = freshkube_ui::table::word_cut_style(READS).expect("Reads painted");
            assert!(shown.text.ends_with('…'), "Reads is not cut: {}", shown.text);
            let run = gpui_kit::TextRun {
                len: shown.text.len(),
                font: gpui_kit::font(shown.font),
                color: shown.color,
                background_color: None,
                underline: None,
                strikethrough: None,
            };
            let painted = window
                .text_system()
                .shape_line(shown.text, shown.size, &[run], None)
                .width();
            let padding = crate::ui::dp_px(2. * freshkube_ui::table::CELL_PAD, window);
            assert!(
                painted <= cell.size.width - padding + gpui_kit::px(1.),
                "the cut text exceeds the cell: {painted:?} in {cell:?}"
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
        list(&view, cx).read(cx).focus_handle().focus(window, cx);
        window.press("c", cx);
        window.render_frame(cx);
        assert_eq!(state(&view, KARGO, cx), "Readable");
    })
    .unwrap();
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

/// A check in flight is dropped by a new workspace, by another section and
/// by the page hiding, a press while it runs starts nothing, and a late
/// answer never lands.
#[cfg(unix)]
#[gpui_kit::test]
async fn a_check_in_flight_is_dropped_and_its_answer_never_lands(cx: &mut TestAppContext) {
    cx.executor().allow_parking();
    let guard = tempfile::tempdir().unwrap();
    waiting_workspace(guard.path());
    let (_runtime, handle, view) = launch(cx, guard.path());
    open_section(cx, handle, &view, SettingsSection::Sources).await;
    cx.update_window(handle, |_, window, cx| {
        window.click("settings-sources-check", cx);
        window.render_frame(cx);
        assert_eq!(state(&view, KARGO, cx), "Reading");
        let sources = list(&view, cx);
        let first = sources.read(cx).read.revision();
        let first_request = request(&sources, cx);
        // C while it runs starts nothing.
        list(&view, cx).read(cx).focus_handle().focus(window, cx);
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
        assert_eq!(state(&view, KARGO, cx), "Reading");

        // (c) Another section of Settings drops it, and coming back starts
        // nothing.
        view.update(cx, |pilot, cx| {
            pilot.show_settings(SettingsSection::Clusters, window, cx)
        });
        assert!(!sources.read(cx).is_checking());
        assert_eq!(sources.read(cx).check, Check::Never);
        sources.update(cx, |list, cx| {
            list.answer(&second_request, Ok(Checked::default()), cx)
        });
        open(window, cx, &view);
        assert!(!sources.read(cx).is_checking());
        assert_eq!(state(&view, KARGO, cx), "Not checked");

        // (d) Leaving Settings drops it.
        window.click("settings-sources-check", cx);
        let third_request = request(&sources, cx);
        view.update(cx, |pilot, cx| {
            pilot.navigate_from_keyboard(Page::Overview, window, cx)
        });
        assert!(!sources.read(cx).is_checking());
        assert_eq!(sources.read(cx).check, Check::Never);
        sources.update(cx, |list, cx| {
            list.answer(&third_request, Ok(Checked::default()), cx)
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
    open_section(cx, handle, &view, SettingsSection::Sources).await;
    cx.update_window(handle, |_, window, cx| {
        list(&view, cx).update(cx, |list, _| list.deadline = Duration::from_secs(1));
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
    open_section(cx, handle, &view, SettingsSection::Sources).await;
    cx.update_window(handle, |_, window, cx| {
        window.click("settings-sources-check", cx);
        window.render_frame(cx);
        assert!(
            states(&view, cx)
                .iter()
                .all(|(_, _, state)| state == "Reading")
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

#[gpui_kit::test]
fn selection_explains_roles_and_pins_in_the_shared_inspector(cx: &mut TestAppContext) {
    let (_runtime, handle, view) = fixture(cx, 1280., 880.);
    cx.update_window(handle, |_, window, cx| {
        open(window, cx, &view);
        window.click("settings-source-kargo", cx);
        window.render_frame(cx);
        assert_eq!(list(&view, cx).read(cx).selected, Some(KARGO));
        assert!(window.find("settings-source-choice").visible());
        assert!(
            list(&view, cx).read(cx).rows[0]
                .explanation
                .contains("first entry with the CI/CD role")
        );
        assert!(window.find("settings-source-reads").visible());
        let mut workspace = view.read(cx).settings_page.read(cx).workspace().clone();
        workspace.sources.set(Part::Kargo, Some("core-fra".into()));
        set_workspace(&view, workspace, true, cx);
        window.render_frame(cx);
        assert!(
            list(&view, cx).read(cx).rows[0]
                .explanation
                .contains("core-fra, pinned in freshkube.yaml")
        );
        assert_eq!(
            window.find("settings-source-inspector-title").label(),
            Some("Kargo")
        );
        // Example mode refuses both visible controls and dispatched actions.
        window.click("settings-source-follow-roles", cx);
        list(&view, cx).update(cx, |list, cx| list.pin(Some("cicd-fra".into()), cx));
        window.press("escape", cx);
        window.render_frame(cx);
        assert!(window.try_find("settings-source-inspector").is_none());
        window.press("down", cx);
        window.render_frame(cx);
        assert!(window.find("settings-source-inspector").visible());
    })
    .unwrap();
    cx.run_until_parked();
    cx.update(|cx| {
        assert_eq!(
            view.read(cx)
                .settings_page
                .read(cx)
                .workspace()
                .sources
                .pin(Part::Kargo),
            Some("core-fra")
        );
    });
}

#[gpui_kit::test]
fn sources_table_and_inspector_follow_the_shared_geometry_at_every_size(cx: &mut TestAppContext) {
    for (width, height) in [(1280., 880.), (760., 560.)] {
        for text in [13., 20.] {
            let (_runtime, handle, view) = fixture(cx, width, height);
            cx.update_window(handle, |_, window, cx| {
                crate::text_size::set(text, cx);
                open(window, cx, &view);
                layout_check::assert_table_page(window, cx, &PAGE);
                window.click("settings-source-kargo", cx);
                layout_check::assert_inspector(
                    window,
                    cx,
                    "settings-sources-split",
                    "settings-sources-table",
                    "settings-source-inspector",
                    "settings-source-inspector-title",
                );
                let table = window.find("settings-sources-table").bounds();
                let inspector = window.find("settings-source-inspector").bounds();
                let beside =
                    crate::screens::page_width(window) >= freshkube_ui::inspector::SPLIT_WIDTH;
                assert_eq!(beside, width == 1280. && text == 13.);
                if beside {
                    assert!(inspector.left() >= table.right());
                } else {
                    assert!(inspector.top() >= table.bottom());
                }
            })
            .unwrap();
        }
    }
}

#[gpui_kit::test]
fn checking_uses_loading_rows_and_completion_publishes_the_status_time(cx: &mut TestAppContext) {
    let (_runtime, handle, view) = fixture(cx, 1280., 880.);
    cx.update_window(handle, |_, window, cx| {
        open(window, cx, &view);
        let sources = list(&view, cx);
        let request = sources.update(cx, |list, cx| {
            let request = list.read.start(list.identity());
            list.check = super::Check::Running;
            list.derive_rows();
            list.publish_status(cx);
            cx.notify();
            request
        });
        window.render_frame(cx);
        assert!(window.find("settings-sources-loading").visible());
        assert!(window.try_find("settings-source-kargo").is_none());
        assert!(
            !gpui_kit::base::test_support::snapshots(window)
                .iter()
                .any(|s| s.label() == Some("Checking…"))
        );
        sources.update(cx, |list, cx| {
            list.answer(&request, Ok(list.example_check()), cx)
        });
        window.render_frame(cx);
        assert!(window.try_find("settings-sources-loading").is_none());
        assert!(window.find("settings-source-kargo").visible());
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let checked = list(&view, cx).read(cx).checked_at.clone().unwrap();
        let page = view.read(cx).settings_page.read(cx);
        let status = page.status().unwrap();
        assert!(
            status
                .text(None)
                .contains(&format!("Last checked {checked}"))
        );
    })
    .unwrap();
}

const PIN_SETUP: &str = r#"version: 1
clusters:
- {id: cluster-a, role: core, context: acme-a, future_entry: {keep: true}}
- {id: cluster-b, role: cicd, context: acme-b}
sources: {builds: cluster-b, argocd: cluster-a, future_pin: {keep: true}}
destinations: [{name: acme, entry: cluster-a}]
metrics: {mode: automatic, future_metrics: keep}
coroot: {server: 'https://observations.example.test', sign_in: anonymous}
future_setup: {keep: [1, 2]}
"#;

fn dispatch_pin_from_menu(
    sources: &List,
    window: &mut gpui_kit::Window,
    cx: &mut gpui_kit::App,
    entry: Option<&str>,
) {
    use freshkube_ui::{menu::MenuAction, table::TableSource};
    let actions = sources.update(cx, |list, cx| list.row_menu(&KARGO, window, cx));
    let action = if let Some(entry) = entry {
        let items = actions
            .into_iter()
            .find_map(|action| match action {
                MenuAction::Submenu { items, .. } => Some(items),
                _ => None,
            })
            .unwrap();
        items
            .into_iter()
            .find_map(|item| match item {
                MenuAction::Item {
                    label,
                    action,
                    enabled: true,
                    ..
                } if label.as_ref() == entry => Some(action),
                _ => None,
            })
            .unwrap()
    } else {
        actions
            .into_iter()
            .find_map(|item| match item {
                MenuAction::Item {
                    label,
                    action,
                    enabled: true,
                    ..
                } if label.as_ref() == "Follow roles" => Some(action),
                _ => None,
            })
            .unwrap()
    };
    sources
        .read(cx)
        .focus_handle()
        .dispatch_action(action.as_ref(), window, cx);
}

#[gpui_kit::test]
async fn pin_and_follow_roles_keep_other_parts_and_unknown_keys(cx: &mut TestAppContext) {
    use super::super::edit_tests::{read_workspace, saved};
    cx.executor().allow_parking();
    let guard = tempfile::tempdir().unwrap();
    write(guard.path(), PIN_SETUP);
    let before = freshkube_core::setup::parse(PIN_SETUP.as_bytes()).unwrap();
    let (_runtime, handle, view) = launch(cx, guard.path());
    open_section(cx, handle, &view, SettingsSection::Sources).await;
    for (entry, inspector) in [
        (Some("cluster-a"), false),
        (None, true),
        (Some("cluster-a"), true),
        (None, false),
    ] {
        cx.update_window(handle, |_, window, cx| {
            let sources = list(&view, cx);
            if inspector {
                window.click("settings-source-kargo", cx);
                window.render_frame(cx);
                if entry.is_some() {
                    window.click("settings-source-pin", cx);
                    window.render_frame(cx);
                } else {
                    window.click("settings-source-follow-roles", cx);
                }
            } else if entry.is_none() {
                window.right_click("settings-source-kargo", cx);
            } else {
                dispatch_pin_from_menu(&sources, window, cx, entry);
            }
        })
        .unwrap();
        cx.run_until_parked();
        if inspector && entry.is_some() || !inspector && entry.is_none() {
            cx.update_window(handle, |_, window, cx| {
                window.render_frame(cx);
                window
                    .within("popup-menu")
                    .click(if inspector { 0usize } else { 3usize }, cx);
            })
            .unwrap();
            cx.run_until_parked();
        }
        saved(cx, handle, &view).await;
        assert_eq!(read_workspace(guard.path()).sources.pin(Part::Kargo), entry);
        let mut expected = before.clone();
        expected
            .workspace
            .sources
            .set(Part::Kargo, entry.map(str::to_owned));
        let after = freshkube_core::setup::parse(
            &std::fs::read(guard.path().join("freshkube.yaml")).unwrap(),
        )
        .unwrap();
        assert_eq!(after, expected);
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            assert!(window.try_find("settings-saved").is_none());
            let status = view.read(cx).settings_page.read(cx).status().unwrap();
            assert!(status.text(None).contains("Saved"), "{}", status.text(None));
        })
        .unwrap();
    }
    write(guard.path(), "version: [broken");
    cx.update_window(handle, |_, window, cx| {
        dispatch_pin_from_menu(&list(&view, cx), window, cx, Some("cluster-a"));
    })
    .unwrap();
    cx.run_until_parked();
    saved(cx, handle, &view).await;
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(window.find("settings-save-failed").visible());
        let status = view.read(cx).settings_page.read(cx).status().unwrap();
        assert!(
            !status.text(None).contains("Saved"),
            "{}",
            status.text(None)
        );
    })
    .unwrap();
}

#[gpui_kit::test]
async fn a_broken_setup_refuses_pin_edits_without_replacing_the_file(cx: &mut TestAppContext) {
    cx.executor().allow_parking();
    let guard = tempfile::tempdir().unwrap();
    write(guard.path(), PIN_SETUP);
    let (_runtime, handle, view) = launch(cx, guard.path());
    open_section(cx, handle, &view, SettingsSection::Sources).await;
    write(guard.path(), "version: [broken");
    cx.update_window(handle, |_, window, cx| {
        dispatch_pin_from_menu(&list(&view, cx), window, cx, Some("cluster-a"));
    })
    .unwrap();
    cx.run_until_parked();
    super::super::edit_tests::saved(cx, handle, &view).await;
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(window.find("settings-save-failed").visible());
        assert_eq!(
            view.read(cx)
                .settings_page
                .read(cx)
                .workspace()
                .sources
                .pin(Part::Kargo),
            None
        );
    })
    .unwrap();
    assert_eq!(
        std::fs::read_to_string(guard.path().join("freshkube.yaml")).unwrap(),
        "version: [broken"
    );
}

#[gpui_kit::test]
async fn an_example_pin_edit_leaves_the_setup_file_untouched(cx: &mut TestAppContext) {
    cx.executor().allow_parking();
    let guard = tempfile::tempdir().unwrap();
    write(guard.path(), PIN_SETUP);
    let (_runtime, handle, view) = launch(cx, guard.path());
    open_section(cx, handle, &view, SettingsSection::Sources).await;
    cx.update_window(handle, |_, window, cx| {
        let workspace = view.read(cx).settings_page.read(cx).workspace().clone();
        set_workspace(&view, workspace, true, cx);
        window.click("settings-source-kargo", cx);
        window.render_frame(cx);
        window.click("settings-source-pin", cx);
        assert!(window.try_find("popup-menu").is_none());
        list(&view, cx).update(cx, |list, cx| list.pin(Some("cluster-a".into()), cx));
    })
    .unwrap();
    cx.run_until_parked();
    assert_eq!(
        std::fs::read_to_string(guard.path().join("freshkube.yaml")).unwrap(),
        PIN_SETUP
    );
    assert!(!guard.path().join("freshkube.yaml.bak").exists());
}

#[gpui_kit::test]
async fn a_hand_edit_refuses_a_pin_without_replacing_the_file(cx: &mut TestAppContext) {
    cx.executor().allow_parking();
    let guard = tempfile::tempdir().unwrap();
    write(guard.path(), PIN_SETUP);
    let (_runtime, handle, view) = launch(cx, guard.path());
    open_section(cx, handle, &view, SettingsSection::Sources).await;
    let edited = PIN_SETUP.replace("acme-a", "acme-changed");
    write(guard.path(), &edited);
    cx.update_window(handle, |_, window, cx| {
        dispatch_pin_from_menu(&list(&view, cx), window, cx, Some("cluster-a"));
    })
    .unwrap();
    cx.run_until_parked();
    super::super::edit_tests::saved(cx, handle, &view).await;
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(window.find("settings-save-failed").visible());
        assert!(view.read(cx).settings_page.read(cx).setup_is_pending());
    })
    .unwrap();
    assert_eq!(
        std::fs::read_to_string(guard.path().join("freshkube.yaml")).unwrap(),
        edited
    );
}
