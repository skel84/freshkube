use std::sync::Arc;

use freshkube_core::diagnostic_runner::{
    DiagnosticCheck, DiagnosticFix, DiagnosticFixAction, DiagnosticSnapshot,
};
use freshkube_core::diagnostics::{CheckCategory, CheckStatus, CniType};
use gpui_kit::component::Root;
use gpui_kit::test::TestWindowExt;
use gpui_kit::{AppContext, Entity, TestAppContext, Window, WindowHandle, px, size};
use tokio::runtime::{Builder, Runtime};

// Not `super::*`: gpui_kit's glob would shadow the built-in `#[test]`.
use super::source::{Shown, Tally};
use super::{DiagnosticsScreen, ScreenPanel, ScreenSource, example, log_service};
use crate::backend::Target;
use crate::desktop::layout_check;
use crate::desktop::nodes::NodeTab;
use crate::desktop::tests::{fixture as app, open_node_tab};
use crate::{fixture, presentation};

const DIAGNOSTICS_FRAME: layout_check::PageFrame = layout_check::PageFrame {
    page: "diagnostics-page",
    title: "diagnostics-title",
    title_text: "Diagnostics",
    content: "diagnostics-split",
};

fn source(context: &str, node: &str) -> ScreenSource {
    let nodes = presentation::node_summaries(&fixture::cluster(context, 1));
    let summary = nodes.iter().find(|summary| summary.name == node).unwrap();
    ScreenSource {
        target: Target {
            epoch: 1,
            context: context.into(),
            node: summary.name.clone(),
            address: summary.address.clone(),
        },
        nodes: Arc::new(nodes),
        live: None,
    }
}

fn mount(
    cx: &mut TestAppContext,
    context: &str,
    node: &str,
) -> (Runtime, Entity<DiagnosticsScreen>, WindowHandle<Root>) {
    let runtime = Builder::new_multi_thread()
        .worker_threads(1)
        .enable_all()
        .build()
        .unwrap();
    cx.update(|cx| {
        gpui_kit::init(cx);
        crate::theme::install(cx);
    });
    let source = source(context, node);
    let mut screen = None;
    let handle = cx.open_window(size(px(1100.), px(1500.)), |window, cx| {
        let view = cx.new(|cx| {
            let mut view = DiagnosticsScreen::new(runtime.handle().clone(), window, cx);
            view.set_source(Some(source), window, cx);
            view.activate(window, cx);
            view
        });
        screen = Some(view.clone());
        Root::new(view, window, cx)
    });
    cx.run_until_parked();
    (runtime, screen.unwrap(), handle)
}

fn names(screen: &DiagnosticsScreen) -> Vec<String> {
    screen
        .visible()
        .iter()
        .map(|check| check.name.clone())
        .collect()
}

#[gpui_kit::test]
fn keyboard_selection_updates_the_details(cx: &mut TestAppContext) {
    let (_runtime, screen, handle) = mount(cx, "prod-fra", "talos-cp-fra1-01");
    cx.update_window(handle.into(), |_, window, cx| {
        window.render_frame(cx);
        let names = names(screen.read(cx));
        assert!(names.len() > 5);
        assert_eq!(
            window.find(("diagnostic-check", 0usize)).selected(),
            Some(true)
        );
        assert_eq!(
            window.find("diagnostic-detail-title").label(),
            Some(names[0].as_str())
        );
        window.click(("diagnostic-check", 0usize), cx);
        window.press("down", cx);
        window.render_frame(cx);
        assert_eq!(
            window.find(("diagnostic-check", 1usize)).selected(),
            Some(true)
        );
        assert_eq!(
            window.find("diagnostic-detail-title").label(),
            Some(names[1].as_str())
        );
        window.press("end", cx);
        window.render_frame(cx);
        let last = names.len() - 1;
        assert_eq!(
            window.find(("diagnostic-check", last)).selected(),
            Some(true)
        );
        assert_eq!(
            window.find("diagnostic-detail-title").label(),
            Some(names[last].as_str())
        );
        window.press("home", cx);
        window.render_frame(cx);
        assert_eq!(
            window.find(("diagnostic-check", 0usize)).selected(),
            Some(true)
        );
        window.find("diagnostics-tally");
    })
    .unwrap();
}

#[gpui_kit::test]
fn failing_check_shows_its_fix_and_logs(cx: &mut TestAppContext) {
    let (_runtime, screen, handle) = mount(cx, "prod-fra", "talos-wk-fra1-02");
    cx.update_window(handle.into(), |_, window, cx| {
        window.render_frame(cx);
        let ix = screen
            .read(cx)
            .visible()
            .iter()
            .position(|check| check.id == "service_kubelet")
            .unwrap();
        assert_eq!(
            screen.read(cx).visible()[ix].status,
            CheckStatus::Fail,
            "an unhealthy kubelet is a failure"
        );
        // Far down the list, so select by key like the keyboard would.
        screen.update(cx, |screen, _| {
            screen.selected = Some("services:service_kubelet".into());
        });
        window.render_frame(cx);
        let fix = window
            .find("diagnostic-detail-fix")
            .label()
            .map(str::to_owned)
            .unwrap();
        assert!(fix.contains("Restart kubelet"), "{fix}");
        assert!(fix.contains("service kubelet restart"), "{fix}");
        assert!(window.find("diagnostic-detail-evidence").label().is_some());
        // A failing check with an executable fix can apply it.
        assert!(window.find("diagnostic-apply-fix").visible());
        window.find("diagnostic-open-logs");
        let check = screen.read(cx).visible()[ix].clone();
        assert_eq!(log_service(&check).as_deref(), Some("kubelet"));
        assert!(matches!(
            check.fix.unwrap().action,
            DiagnosticFixAction::RestartService { .. }
        ));
    })
    .unwrap();
}

#[gpui_kit::test]
fn unavailable_kubernetes_keeps_talos_checks(cx: &mut TestAppContext) {
    let (_runtime, screen, handle) = mount(cx, "homelab", "talos-home");
    cx.update_window(handle.into(), |_, window, cx| {
        window.render_frame(cx);
        let checks: Vec<_> = screen.read(cx).visible().into_iter().cloned().collect();
        let status = |id: &str| {
            checks
                .iter()
                .find(|check| check.id == id)
                .unwrap_or_else(|| panic!("no check {id}"))
                .status
                .clone()
        };
        // Talos-side checks keep their answers.
        assert_eq!(status("memory"), CheckStatus::Pass);
        assert_eq!(status("cpu_load"), CheckStatus::Pass);
        assert_eq!(status("etcd"), CheckStatus::Pass);
        assert_eq!(status("br_netfilter"), CheckStatus::Pass);
        // Kubernetes-side checks are unknown, never failed.
        assert_eq!(status("kubernetes_api"), CheckStatus::Unknown);
        assert_eq!(status("pod_health"), CheckStatus::Unknown);
        assert_eq!(status("flannel_pods"), CheckStatus::Unknown);
        assert_eq!(status("addons"), CheckStatus::Unknown);
        assert!(
            checks
                .iter()
                .filter(|check| check.category == CheckCategory::Kubernetes)
                .all(|check| check.status != CheckStatus::Fail)
        );
        let notice = window
            .find("partial-notice")
            .label()
            .map(str::to_owned)
            .unwrap();
        assert!(notice.contains("Kubernetes"), "{notice}");
        let meta = screen.read(cx).derived.as_ref().unwrap().meta.clone();
        assert!(meta.iter().any(|part| part == "addons unknown"), "{meta:?}");
    })
    .unwrap();
}

fn problems(screen: &DiagnosticsScreen) -> usize {
    screen
        .loader
        .data()
        .unwrap()
        .checks
        .iter()
        .filter(|check| matches!(check.status, CheckStatus::Warn | CheckStatus::Fail))
        .count()
}

#[gpui_kit::test]
fn p_shows_only_the_problems(cx: &mut TestAppContext) {
    let (_runtime, screen, handle) = mount(cx, "prod-fra", "talos-wk-fra1-02");
    cx.update_window(handle.into(), |_, window, cx| {
        window.render_frame(cx);
        let all = screen.read(cx).visible().len();
        let problems = problems(screen.read(cx));
        assert!(problems >= 2 && problems < all);
        assert!(window.try_find(("diagnostic-check", all - 1)).is_some());
        window.click(("diagnostic-check", 0usize), cx);
        window.press("p", cx);
        window.render_frame(cx);
        let shown = screen.read(cx).visible();
        assert_eq!(shown.len(), problems);
        assert!(
            shown
                .iter()
                .all(|check| matches!(check.status, CheckStatus::Warn | CheckStatus::Fail))
        );
        assert!(window.try_find(("diagnostic-check", problems)).is_none());
        assert!(
            window
                .try_find(("diagnostic-check", problems - 1))
                .is_some()
        );
        // Both problem chips show as chosen.
        let shown = screen.read(cx).shown;
        assert_eq!(shown, Shown::Problems);
        for (tally, chosen) in [
            (Tally::Failing, true),
            (Tally::Warnings, true),
            (Tally::Unknown, false),
            (Tally::Passing, false),
        ] {
            assert_eq!(shown.chosen(tally), chosen, "{tally:?}");
        }
        window.press("p", cx);
        window.render_frame(cx);
        assert_eq!(screen.read(cx).visible().len(), all);
    })
    .unwrap();
}

#[gpui_kit::test]
fn the_chips_count_every_status_and_filter_to_one(cx: &mut TestAppContext) {
    let (_runtime, screen, handle) = mount(cx, "prod-fra", "talos-wk-fra1-02");
    cx.update_window(handle.into(), |_, window, cx| {
        window.render_frame(cx);
        let checks = screen.read(cx).loader.data().unwrap().checks.clone();
        let all = checks.len();
        for (tally, chip, what) in [
            (Tally::Failing, "diagnostics-tally-failing", "failing"),
            (Tally::Warnings, "diagnostics-tally-warnings", "warnings"),
            (Tally::Unknown, "diagnostics-tally-unknown", "unknown"),
            (Tally::Passing, "diagnostics-tally-passing", "passing"),
        ] {
            let count = checks
                .iter()
                .filter(|check| Tally::of(&check.status) == tally)
                .count();
            assert!(count > 0, "the example has no {what} check");
            assert_eq!(
                window.find(chip).label(),
                Some(format!("{count} {what}").as_str())
            );
            window.click(chip, cx);
            window.render_frame(cx);
            assert_eq!(screen.read(cx).shown, Shown::Only(tally));
            let shown = screen.read(cx).visible();
            assert_eq!(shown.len(), count, "{what}");
            assert!(shown.iter().all(|check| Tally::of(&check.status) == tally));
            // The details follow the first listed check.
            assert_eq!(
                window.find("diagnostic-detail-title").label(),
                Some(shown[0].name.as_str())
            );
            // Pressing it again clears the filter.
            window.click(chip, cx);
            window.render_frame(cx);
            assert_eq!(screen.read(cx).visible().len(), all);
        }
    })
    .unwrap();
}

#[gpui_kit::test]
fn categories_head_their_checks_with_the_worst_status(cx: &mut TestAppContext) {
    let (_runtime, _screen, handle) = mount(cx, "prod-fra", "talos-wk-fra1-02");
    cx.update_window(handle.into(), |_, window, cx| {
        window.render_frame(cx);
        let services = window
            .find("diagnostic-section-services")
            .label()
            .map(str::to_owned)
            .unwrap();
        assert!(services.starts_with("Services · "), "{services}");
        assert!(services.contains("1 failing"), "{services}");
        let cni = window
            .find("diagnostic-section-cni")
            .label()
            .map(str::to_owned)
            .unwrap();
        assert!(cni.starts_with("CNI (Flannel) · "), "{cni}");
        // Groups come in the TUI's order, above their rows.
        let system = window.find("diagnostic-section-system").bounds();
        let first = window.find(("diagnostic-check", 0usize)).bounds();
        assert!(system.bottom() <= first.top());
        assert!(system.top() < window.find("diagnostic-section-kubernetes").bounds().top());
    })
    .unwrap();
}

#[test]
fn example_covers_every_status() {
    let snapshot = example(&source("prod-fra", "talos-wk-fra1-02")).unwrap();
    let has = |status: CheckStatus| snapshot.checks.iter().any(|check| check.status == status);
    assert!(has(CheckStatus::Pass));
    assert!(has(CheckStatus::Warn));
    assert!(has(CheckStatus::Unknown));
    assert!(
        snapshot
            .checks
            .iter()
            .any(|check| check.status == CheckStatus::Fail && check.fix.is_some())
    );
    assert_eq!(snapshot.cni.cni_type.as_ref(), Some(&CniType::Flannel));
    assert_eq!(snapshot.addons.detected_names(), vec!["cert-manager"]);
    assert!(example(&source("prod-fra", "talos-wk-fra1-03")).is_err());
}

#[gpui_kit::test]
fn silent_target_offers_retry_without_data(cx: &mut TestAppContext) {
    let (_runtime, screen, handle) = mount(cx, "prod-fra", "talos-wk-fra1-03");
    cx.update_window(handle.into(), |_, window, cx| {
        window.render_frame(cx);
        assert!(screen.read(cx).loader.data().is_none());
        window.find("screen-retry");
        assert!(window.try_find("diagnostic-list").is_none());
    })
    .unwrap();
}

#[gpui_kit::test]
fn changing_target_drops_old_data(cx: &mut TestAppContext) {
    let (_runtime, screen, handle) = mount(cx, "prod-fra", "talos-cp-fra1-01");
    cx.update_window(handle.into(), |_, window, cx| {
        screen.update(cx, |screen, cx| {
            screen.selected = Some("system:memory".into());
            screen.shown = Shown::Problems;
            assert!(screen.loader.data().is_some());
            screen.set_source(Some(source("prod-fra", "talos-wk-fra1-02")), window, cx);
            assert!(screen.loader.data().is_none());
            assert!(screen.selected.is_none());
            assert_eq!(screen.shown, Shown::All);
        });
    })
    .unwrap();
}

fn open_fix(
    cx: &mut TestAppContext,
    handle: WindowHandle<Root>,
    screen: &Entity<DiagnosticsScreen>,
    check_key: &str,
) {
    cx.update_window(handle.into(), |_, window, cx| {
        screen.update(cx, |screen, _| screen.selected = Some(check_key.into()));
        window.render_frame(cx);
        window.click("diagnostic-apply-fix", cx);
    })
    .unwrap();
    crate::mutation::settle_confirmation(cx, handle.into());
}

/// Replaces the loaded snapshot with an edited copy, as a refresh would.
fn edit_snapshot(
    cx: &mut TestAppContext,
    screen: &Entity<DiagnosticsScreen>,
    edit: impl FnOnce(&mut DiagnosticSnapshot),
) {
    screen.update(cx, |screen, _| {
        let mut snapshot = screen.loader.data().unwrap().clone();
        edit(&mut snapshot);
        let target = screen.source.as_ref().unwrap().target.clone();
        screen.loader.resolve(target, Ok(snapshot));
    });
}

fn check_mut<'a>(snapshot: &'a mut DiagnosticSnapshot, id: &str) -> &'a mut DiagnosticCheck {
    snapshot
        .checks
        .iter_mut()
        .find(|check| check.id == id)
        .unwrap()
}

const KUBELET: &str = "services:service_kubelet";

#[gpui_kit::test]
fn apply_fix_is_offered_only_for_problems_with_an_executable_fix(cx: &mut TestAppContext) {
    let (_runtime, screen, handle) = mount(cx, "prod-fra", "talos-wk-fra1-02");
    edit_snapshot(cx, &screen, |snapshot| {
        snapshot.checks.push(
            DiagnosticCheck::fail("guidance", CheckCategory::System, "Needs a human", "Broken")
                .with_fix(DiagnosticFix {
                    description: "Run this on the host".into(),
                    action: DiagnosticFixAction::CopyGuidance {
                        title: "Host".into(),
                        text: "sysctl -w x=1".into(),
                    },
                }),
        );
    });
    cx.update_window(handle.into(), |_, window, cx| {
        // Failing check, executable fix.
        screen.update(cx, |screen, _| screen.selected = Some(KUBELET.into()));
        window.render_frame(cx);
        assert!(window.find("diagnostic-apply-fix").visible());
        // Copy-only guidance never becomes a button.
        screen.update(cx, |screen, _| {
            screen.selected = Some("system:guidance".into())
        });
        window.render_frame(cx);
        assert!(window.try_find("diagnostic-apply-fix").is_none());
        assert!(
            window
                .find("diagnostic-detail-fix")
                .label()
                .unwrap()
                .contains("sysctl")
        );
    })
    .unwrap();
    // The same check once its status is unknown or healthy: nothing to fix.
    for status in [CheckStatus::Unknown, CheckStatus::Pass] {
        edit_snapshot(cx, &screen, |snapshot| {
            check_mut(snapshot, "service_kubelet").status = status.clone();
        });
        cx.update_window(handle.into(), |_, window, cx| {
            screen.update(cx, |screen, _| screen.selected = Some(KUBELET.into()));
            window.render_frame(cx);
            assert!(
                window.try_find("diagnostic-apply-fix").is_none(),
                "no fix is offered for {status:?}"
            );
        })
        .unwrap();
    }
    // A warning with an executable fix is offered one.
    edit_snapshot(cx, &screen, |snapshot| {
        check_mut(snapshot, "service_kubelet").status = CheckStatus::Warn;
    });
    cx.update_window(handle.into(), |_, window, cx| {
        window.render_frame(cx);
        assert!(window.find("diagnostic-apply-fix").visible());
    })
    .unwrap();
    // And not while another operation holds the slot.
    let operations = cx.update(crate::mutation::Operations::global);
    let ticket = operations
        .update(cx, |operations, cx| operations.begin("Drain", cx))
        .ok()
        .unwrap();
    cx.update_window(handle.into(), |_, window, cx| {
        window.render_frame(cx);
        window.click("diagnostic-apply-fix", cx);
        window.render_frame(cx);
        assert!(window.try_find("confirm-dialog").is_none());
    })
    .unwrap();
    operations.update(cx, |operations, cx| operations.finish(ticket, cx));
}

#[gpui_kit::test]
fn confirming_a_fix_in_example_mode_simulates_it_and_frees_the_slot(cx: &mut TestAppContext) {
    let (_runtime, screen, handle) = mount(cx, "prod-fra", "talos-wk-fra1-02");
    open_fix(cx, handle, &screen, KUBELET);
    cx.update_window(handle.into(), |_, window, cx| {
        assert_eq!(
            window.find("confirm-dialog").label(),
            Some("Apply fix: Restart kubelet")
        );
        assert!(window.try_find("diagnostic-fix-result").is_none());
        window.click("confirm-ok", cx);
        window.render_frame(cx);
        assert!(window.try_find("confirm-dialog").is_none());
        assert!(crate::mutation::Operations::current(cx).is_none());
        let result = window
            .find("diagnostic-fix-result")
            .label()
            .map(str::to_owned)
            .unwrap();
        assert!(result.contains("Example data"), "{result}");
        // The diagnostics were run again.
        assert!(screen.read(cx).loader.data().is_some());
    })
    .unwrap();
}

#[gpui_kit::test]
fn a_config_patch_fix_is_confirmed_with_its_reboot_warning(cx: &mut TestAppContext) {
    let (_runtime, screen, handle) = mount(cx, "prod-fra", "talos-wk-fra1-02");
    edit_snapshot(cx, &screen, |snapshot| {
        snapshot.checks.push(
            DiagnosticCheck::fail(
                "kernel_module",
                CheckCategory::Cni,
                "br_netfilter",
                "Missing",
            )
            .with_fix(DiagnosticFix {
                description: "Add br_netfilter kernel module".into(),
                action: DiagnosticFixAction::ApplyConfigPatch {
                    yaml: "machine:\n  kernel:\n    modules:\n      - name: br_netfilter".into(),
                    requires_reboot: true,
                },
            }),
        );
    });
    open_fix(cx, handle, &screen, "cni:kernel_module");
    cx.update_window(handle.into(), |_, window, cx| {
        assert_eq!(
            window.find("confirm-dialog").label(),
            Some("Apply fix: Add br_netfilter kernel module")
        );
        window.click("confirm-ok", cx);
        window.render_frame(cx);
        assert!(crate::mutation::Operations::current(cx).is_none());
        // The refresh that follows replaces the injected check, so the
        // outcome is read from the screen rather than from that check.
        let notice = screen.read(cx).notice.as_ref().unwrap();
        assert!(notice.ok, "{}", notice.text);
        assert!(notice.text.contains("Example data"), "{}", notice.text);
    })
    .unwrap();
}

#[gpui_kit::test]
fn a_fix_for_a_check_that_changed_is_refused(cx: &mut TestAppContext) {
    let (_runtime, screen, handle) = mount(cx, "prod-fra", "talos-wk-fra1-02");
    open_fix(cx, handle, &screen, KUBELET);
    // A refresh brought a different result for the same check.
    edit_snapshot(cx, &screen, |snapshot| {
        check_mut(snapshot, "service_kubelet").message = "Running (healthy)".into();
        check_mut(snapshot, "service_kubelet").status = CheckStatus::Pass;
    });
    cx.update_window(handle.into(), |_, window, cx| {
        window.click("confirm-ok", cx);
        window.render_frame(cx);
        assert!(window.find("confirm-blocked").visible());
        assert!(crate::mutation::Operations::current(cx).is_none());
        assert!(window.try_find("diagnostic-fix-result").is_none());
        window.click("confirm-cancel", cx);
    })
    .unwrap();
    // So is one for another target.
    cx.run_until_parked();
    edit_snapshot(cx, &screen, |snapshot| {
        check_mut(snapshot, "service_kubelet").status = CheckStatus::Fail;
    });
    open_fix(cx, handle, &screen, KUBELET);
    let other = source("prod-fra", "talos-cp-fra1-01");
    cx.update_window(handle.into(), |_, window, cx| {
        screen.update(cx, |screen, cx| screen.set_source(Some(other), window, cx));
        window.click("confirm-ok", cx);
        window.render_frame(cx);
        assert!(window.find("confirm-blocked").visible());
        assert!(window.try_find("diagnostic-fix-result").is_none());
    })
    .unwrap();
}

#[gpui_kit::test]
fn a_fix_is_refused_while_another_operation_runs(cx: &mut TestAppContext) {
    let (_runtime, screen, handle) = mount(cx, "prod-fra", "talos-wk-fra1-02");
    open_fix(cx, handle, &screen, KUBELET);
    let operations = cx.update(crate::mutation::Operations::global);
    let ticket = operations
        .update(cx, |operations, cx| {
            operations.begin("Reboot talos-wk-fra1-02", cx)
        })
        .ok()
        .unwrap();
    cx.update_window(handle.into(), |_, window, cx| {
        window.click("confirm-ok", cx);
        window.render_frame(cx);
        assert!(window.find("confirm-blocked").visible());
        assert!(window.try_find("diagnostic-fix-result").is_none());
        assert!(
            crate::mutation::Operations::current(cx)
                .is_some_and(|running| running.label.starts_with("Reboot"))
        );
    })
    .unwrap();
    operations.update(cx, |operations, cx| operations.finish(ticket, cx));
}

#[gpui_kit::test]
fn diagnostics_has_the_edge_frame_at_both_text_sizes(cx: &mut TestAppContext) {
    for text in [None, Some(20.)] {
        let (_runtime, handle, _view) = app(cx, 1280., 880.);
        cx.update_window(handle, |_, window, cx| {
            if let Some(text) = text {
                crate::text_size::set(text, cx);
            }
            open_node_tab(window, cx, NodeTab::Diagnostics);
            window.render_frame(cx);
        })
        .unwrap();
        cx.run_until_parked();
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            layout_check::assert_edge_frame(window, cx, &DIAGNOSTICS_FRAME);
            layout_check::assert_table(
                window,
                cx,
                &layout_check::Table {
                    table: Some("diagnostic-table-scroll"),
                    list: "diagnostic-list",
                },
            );
        })
        .unwrap();
    }
}

/// In the node's expanded inspector at 1280 one text size up, the details
/// beside the table leave room for every column, so Result truncates
/// rather than scrolling out of view.
#[gpui_kit::test]
fn the_columns_fit_beside_the_details(cx: &mut TestAppContext) {
    let (_runtime, handle, _view) = app(cx, 1280., 880.);
    cx.update_window(handle, |_, window, cx| {
        crate::text_size::set(14., cx);
        open_node_tab(window, cx, NodeTab::Diagnostics);
        window.render_frame(cx);
        window.click("node-expand", cx);
        window.render_frame(cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let list = window.find("diagnostic-list").bounds();
        let details = window.find("diagnostic-details").bounds();
        assert!(
            details.left() >= list.right(),
            "the details aren't beside the table"
        );
        let result = window.find(("diagnostic-sort", 2usize)).bounds();
        assert!(
            result.right() <= list.right() + px(1.),
            "Result {result:?} runs past the list {list:?}"
        );
        layout_check::assert_table(
            window,
            cx,
            &layout_check::Table {
                table: Some("diagnostic-table-scroll"),
                list: "diagnostic-list",
            },
        );
    })
    .unwrap();
}

/// The state in the list's place sits under the toolbar, which keeps the
/// title and Refresh.
fn under_the_toolbar(window: &Window, id: &'static str) {
    let toolbar = window.find("diagnostics-toolbar").bounds();
    window.find("diagnostics-title");
    window.find("diagnostics-refresh");
    let state = window.find(id).bounds();
    assert!(
        state.top() >= toolbar.bottom(),
        "{id} {state:?} isn't under the toolbar {toolbar:?}"
    );
    assert!(window.try_find("diagnostic-list").is_none());
}

#[gpui_kit::test]
fn every_state_sits_under_the_toolbar(cx: &mut TestAppContext) {
    let (_runtime, screen, handle) = mount(cx, "prod-fra", "talos-wk-fra1-03");
    cx.update_window(handle.into(), |_, window, cx| {
        window.render_frame(cx);
        // A node that isn't responding.
        under_the_toolbar(window, "screen-retry");
        // A responding node whose read failed.
        screen.update(cx, |screen, cx| {
            let source = source("prod-fra", "talos-wk-fra1-02");
            let target = source.target.clone();
            screen.set_source(Some(source), window, cx);
            screen.loader.resolve(target, Err("the API refused".into()));
        });
        window.render_frame(cx);
        under_the_toolbar(window, "screen-retry");
        // No node.
        screen.update(cx, |screen, cx| screen.set_source(None, window, cx));
        window.render_frame(cx);
        under_the_toolbar(window, "diagnostics-state");
        assert!(window.try_find("screen-retry").is_none());
    })
    .unwrap();
}
