use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use freshkube_core::cluster_overview::KubeconfigSelection;
use freshkube_core::operations::{NodeTarget, OperationKind, OperationStatus};
use freshkube_core::{AccessIdentity, AccessSessionId, ConfigurationRevision};
use gpui_kit::component::Root;
use gpui_kit::test::TestWindowExt;
use gpui_kit::{AppContext, ElementId, Entity, TestAppContext, WindowHandle, px, size};
use tokio::runtime::{Builder, Runtime};

// Not `super::*`: gpui_kit's glob would shadow the built-in `#[test]`.
use super::{
    Operations, OperationsScreen, PreviewKey, PreviewState, ScreenPanel, ScreenSource,
    blocked_reason, example_preview, run_client, verdict,
};
use crate::backend::Target;
use crate::desktop::layout_check;
use crate::desktop::tests::fixture as app;
use crate::resources::talos::AppliedAccess;
use crate::{fixture, presentation};

fn source(context: &str, node_ix: usize) -> ScreenSource {
    let nodes = presentation::node_summaries(&fixture::cluster(context, 1));
    let summary = nodes[node_ix].clone();
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
    step_ms: u64,
) -> (Runtime, Entity<OperationsScreen>, WindowHandle<Root>) {
    // The example run pauses on Tokio's clock between its steps. A paused
    // current-thread runtime makes that clock virtual and runs the worker
    // only when the test drives it (`step`), on the test's own thread, so no
    // test waits on the wall clock. `start_paused` needs Tokio's `test-util`,
    // which only the dev-dependencies turn on.
    let runtime = Builder::new_current_thread()
        .enable_all()
        .start_paused(true)
        .build()
        .unwrap();
    cx.update(|cx| {
        gpui_kit::init(cx);
        crate::theme::install(cx);
    });
    let source = source(context, 0);
    let mut screen = None;
    let handle = cx.open_window(size(px(1500.), px(1000.)), |window, cx| {
        let view = cx.new(|cx| {
            let mut view = OperationsScreen::new(runtime.handle().clone(), window, cx);
            view.step = Duration::from_millis(step_ms);
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

/// Moves the run's virtual clock on by one of its steps, then lets GPUI take
/// what the worker sent.
fn step(cx: &mut TestAppContext, runtime: &Runtime, screen: &Entity<OperationsScreen>) {
    let step = cx.read(|cx| screen.read(cx).step);
    runtime.block_on(async { tokio::time::sleep(step).await });
    cx.run_until_parked();
}

/// Steps the run until it has finished and released the slot, failing after
/// `steps` steps.
fn wait_until_finished(
    cx: &mut TestAppContext,
    runtime: &Runtime,
    screen: &Entity<OperationsScreen>,
    steps: usize,
    what: &str,
) {
    cx.run_until_parked();
    for _ in 0..steps {
        if cx.read(|cx| finished(screen, cx)) {
            return;
        }
        step(cx, runtime, screen);
    }
    assert!(
        cx.read(|cx| finished(screen, cx)),
        "{what} hadn't finished after {steps} steps"
    );
}

/// The id of prod-fra's node row `ix`, from the node's name.
fn row(ix: usize) -> gpui_kit::SharedString {
    let nodes = presentation::node_summaries(&fixture::cluster("prod-fra", 1));
    format!("ops-node-{}", nodes[ix].name).into()
}

fn names(screen: &Entity<OperationsScreen>, cx: &TestAppContext) -> Vec<String> {
    cx.read(|cx| {
        screen
            .read(cx)
            .selected
            .iter()
            .map(|t| t.name.clone())
            .collect()
    })
}

fn render(cx: &mut TestAppContext, handle: WindowHandle<Root>) {
    cx.update_window(handle.into(), |_, window, cx| window.render_frame(cx))
        .unwrap();
}

/// Opens the confirmation for the current selection and lets it settle.
fn review(cx: &mut TestAppContext, handle: WindowHandle<Root>) {
    cx.update_window(handle.into(), |_, window, cx| {
        window.render_frame(cx);
        window.click("ops-review", cx);
    })
    .unwrap();
    crate::mutation::settle_confirmation(cx, handle.into());
}

fn confirm(cx: &mut TestAppContext, handle: WindowHandle<Root>) {
    cx.update_window(handle.into(), |_, window, _| {
        assert!(window.find("confirm-dialog").visible());
    })
    .unwrap();
    press_ok(cx, handle);
}

fn press_ok(cx: &mut TestAppContext, handle: WindowHandle<Root>) {
    cx.update_window(handle.into(), |_, window, cx| {
        // The first pointer press after a dialog opens only settles it.
        window.click("confirm-dialog", cx);
        window.click("confirm-ok", cx);
    })
    .unwrap();
    // The dialog layer only redraws when its root is notified.
    handle.update(cx, |_, _, cx| cx.notify()).unwrap();
    render(cx, handle);
}

fn finished(screen: &Entity<OperationsScreen>, cx: &gpui_kit::App) -> bool {
    screen.read(cx).run.as_ref().is_some_and(|run| run.finished)
        && Operations::current(cx).is_none()
}

fn results(
    screen: &Entity<OperationsScreen>,
    cx: &TestAppContext,
) -> Vec<(String, OperationStatus)> {
    cx.read(|cx| {
        screen
            .read(cx)
            .run
            .as_ref()
            .unwrap()
            .results
            .iter()
            .map(|r| (r.target.name.clone(), r.status))
            .collect()
    })
}

fn set_operation(
    cx: &mut TestAppContext,
    handle: WindowHandle<Root>,
    screen: &Entity<OperationsScreen>,
    kind: OperationKind,
) {
    cx.update_window(handle.into(), |_, window, cx| {
        screen.update(cx, |screen, cx| screen.set_operation(kind, window, cx));
        window.render_frame(cx);
    })
    .unwrap();
}

#[gpui_kit::test]
fn selection_keeps_its_order_and_the_order_can_change(cx: &mut TestAppContext) {
    let (_runtime, screen, handle) = mount(cx, "prod-fra", 1);
    // The target node starts selected.
    assert_eq!(names(&screen, cx), ["talos-cp-fra1-01"]);
    cx.update_window(handle.into(), |_, window, cx| {
        window.render_frame(cx);
        window.click(row(3), cx);
        window.click(row(1), cx);
        window.render_frame(cx);
        // Both clicked nodes join the run; the table's selection is the
        // cursor, on the last one clicked.
        let label = |ix| window.find(row(ix)).label().unwrap_or_default().to_owned();
        assert!(label(3).contains("selected, run order 2"), "{}", label(3));
        assert!(label(2).contains("not selected"), "{}", label(2));
        assert_eq!(window.find(row(1)).selected(), Some(true));
        assert_eq!(window.find(row(3)).selected(), Some(false));
        window.click(("ops-move-earlier", 2usize), cx);
        window.render_frame(cx);
    })
    .unwrap();
    let after = names(&screen, cx);
    assert_eq!(
        after,
        ["talos-cp-fra1-01", "talos-cp-fra1-02", "talos-wk-fra1-01"]
    );
    cx.update_window(handle.into(), |_, window, cx| {
        let label = window
            .find(("ops-plan-target", 1usize))
            .label()
            .unwrap()
            .to_owned();
        assert!(label.contains("talos-cp-fra1-02"), "{label}");
        // Deselecting keeps the order of the rest.
        window.click(row(1), cx);
        window.render_frame(cx);
    })
    .unwrap();
    assert_eq!(names(&screen, cx), ["talos-cp-fra1-01", "talos-wk-fra1-01"]);
}

#[gpui_kit::test]
fn keyboard_selects_reorders_and_clears(cx: &mut TestAppContext) {
    let (_runtime, screen, handle) = mount(cx, "prod-fra", 1);
    cx.update_window(handle.into(), |_, window, cx| {
        window.render_frame(cx);
        // Clicking the target row deselects it and puts focus in the roster.
        window.click(row(0), cx);
        window.press("down", cx);
        window.press("space", cx);
        window.press("down", cx);
        window.press("enter", cx);
        window.render_frame(cx);
    })
    .unwrap();
    assert_eq!(
        names(&screen, cx),
        ["talos-cp-fra1-02", "talos-cp-fra1-03-baremetal-rack-b7"]
    );
    cx.update_window(handle.into(), |_, window, cx| {
        window.press("alt-up", cx);
        window.render_frame(cx);
    })
    .unwrap();
    assert_eq!(
        names(&screen, cx),
        ["talos-cp-fra1-03-baremetal-rack-b7", "talos-cp-fra1-02"]
    );
    cx.update_window(handle.into(), |_, window, cx| {
        window.press("escape", cx);
        window.render_frame(cx);
        let row = window.find(row(1)).label().unwrap().to_owned();
        assert!(row.contains("not selected"), "{row}");
    })
    .unwrap();
    assert!(names(&screen, cx).is_empty());
}

#[test]
fn unknown_etcd_blocks_reboot_and_shutdown_but_not_drain_or_cordon() {
    // staging-eu's etcd sample is incomplete.
    let source = source("staging-eu", 0);
    let key = |operation| PreviewKey {
        context: "staging-eu".into(),
        epoch: 1,
        operation,
        targets: vec![NodeTarget::new(
            source.target.node.clone(),
            source.target.address.clone(),
        )],
    };
    for kind in [OperationKind::Reboot, OperationKind::Shutdown] {
        let preview = example_preview(&source, key(kind));
        let v = verdict(kind, &preview.nodes[0]);
        assert_eq!(v.label, "Unknown");
        assert!(v.blocks);
        let reason = blocked_reason(&PreviewState::Ready(preview), kind).unwrap();
        assert!(reason.contains("unknown"), "{reason}");
    }
    for kind in [
        OperationKind::Drain,
        OperationKind::Cordon,
        OperationKind::Uncordon,
    ] {
        let preview = example_preview(&source, key(kind));
        assert_eq!(verdict(kind, &preview.nodes[0]).label, "No etcd impact");
        assert_eq!(blocked_reason(&PreviewState::Ready(preview), kind), None);
    }
}

#[test]
fn a_sole_control_plane_is_unsafe_to_reboot() {
    let source = source("homelab", 0);
    let key = PreviewKey {
        context: "homelab".into(),
        epoch: 1,
        operation: OperationKind::Reboot,
        targets: vec![NodeTarget::new(
            source.target.node.clone(),
            source.target.address.clone(),
        )],
    };
    let preview = example_preview(&source, key);
    let reason = blocked_reason(&PreviewState::Ready(preview), OperationKind::Reboot).unwrap();
    assert!(reason.contains("refused"), "{reason}");
    let healthy = source_healthy_reboot_is_safe();
    assert_eq!(healthy, "Safe");
}

fn source_healthy_reboot_is_safe() -> &'static str {
    let source = source("prod-fra", 3);
    let key = PreviewKey {
        context: "prod-fra".into(),
        epoch: 1,
        operation: OperationKind::Reboot,
        targets: vec![NodeTarget::new(
            source.target.node.clone(),
            source.target.address.clone(),
        )],
    };
    let preview = example_preview(&source, key);
    verdict(OperationKind::Reboot, &preview.nodes[0]).label
}

#[gpui_kit::test]
fn the_screen_refuses_reboot_on_unknown_etcd_and_says_so(cx: &mut TestAppContext) {
    let (_runtime, screen, handle) = mount(cx, "staging-eu", 1);
    set_operation(cx, handle, &screen, OperationKind::Reboot);
    cx.update_window(handle.into(), |_, window, cx| {
        let blocked = window.find("ops-blocked").label().unwrap().to_owned();
        assert!(blocked.contains("unknown"), "{blocked}");
        let verdict = window
            .find(("ops-verdict", 0usize))
            .label()
            .unwrap()
            .to_owned();
        assert!(verdict.contains("Unknown"), "{verdict}");
        assert!(!verdict.contains("Safe"), "{verdict}");
        // Reviewing is not possible: no confirmation opens.
        window.click("ops-review", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle.into(), |_, window, _| {
        assert!(window.try_find("confirm-dialog").is_none());
    })
    .unwrap();
    for kind in [OperationKind::Drain, OperationKind::Cordon] {
        set_operation(cx, handle, &screen, kind);
        cx.update_window(handle.into(), |_, window, _| {
            assert!(window.try_find("ops-blocked").is_none(), "{kind:?}");
        })
        .unwrap();
    }
}

#[gpui_kit::test]
fn confirming_in_example_mode_runs_the_simulation_and_shows_results(cx: &mut TestAppContext) {
    let (runtime, screen, handle) = mount(cx, "prod-fra", 25);
    review(cx, handle);
    cx.update_window(handle.into(), |_, window, _| {
        assert!(window.find("confirm-dialog").visible());
    })
    .unwrap();
    // Nothing runs until the user confirms.
    assert!(cx.read(|cx| Operations::current(cx).is_none()));
    confirm(cx, handle);
    assert!(cx.read(Operations::current).is_some());
    // A drain of one node takes 7 steps.
    wait_until_finished(cx, &runtime, &screen, 10, "the simulated run");
    assert!(cx.read(Operations::current).is_none());
    assert_eq!(
        results(&screen, cx),
        [("talos-cp-fra1-01".to_owned(), OperationStatus::Succeeded)]
    );
    render(cx, handle);
    cx.update_window(handle.into(), |_, window, _| {
        let result = window
            .find(("ops-result", 0usize))
            .label()
            .unwrap()
            .to_owned();
        assert!(result.contains("Completed"), "{result}");
        assert!(result.contains("evicted"), "{result}");
        let context = window.find("ops-run-context").label().unwrap().to_owned();
        assert!(context.contains("prod-fra"), "{context}");
        let status = window.find("ops-run-status").label().unwrap().to_owned();
        assert!(status.contains("1 of 1 completed"), "{status}");
        let audit = window.find("ops-audit-note").label().unwrap().to_owned();
        assert!(audit.contains("Example data"), "{audit}");
        assert!(window.try_find(("ops-audit-entry", 0usize)).is_some());
    })
    .unwrap();
    // The audit shown is simulated and was never given a file.
    cx.read(|cx| {
        let view = screen.read(cx).audit.data().unwrap();
        assert!(view.path.is_none());
        assert!(!view.entries.is_empty());
    });
}

#[gpui_kit::test]
fn cancelling_stops_the_simulation_between_steps(cx: &mut TestAppContext) {
    let (runtime, screen, handle) = mount(cx, "prod-fra", 200);
    cx.update_window(handle.into(), |_, window, cx| {
        window.render_frame(cx);
        window.click(row(1), cx);
    })
    .unwrap();
    review(cx, handle);
    confirm(cx, handle);
    // One step in, so the run is between steps when Cancel is pressed.
    step(cx, &runtime, &screen);
    render(cx, handle);
    cx.update_window(handle.into(), |_, window, cx| {
        window.click("ops-run-cancel", cx);
    })
    .unwrap();
    // The worker sees the cancel at its next step.
    wait_until_finished(cx, &runtime, &screen, 4, "the run to stop");
    let outcome = results(&screen, cx);
    assert_eq!(outcome.len(), 2, "{outcome:?}");
    assert!(
        outcome
            .iter()
            .any(|(_, status)| *status == OperationStatus::Cancelled),
        "{outcome:?}"
    );
    assert!(
        outcome
            .iter()
            .all(|(_, status)| *status != OperationStatus::Succeeded),
        "{outcome:?}"
    );
    render(cx, handle);
    cx.update_window(handle.into(), |_, window, _| {
        let status = window.find("ops-run-status").label().unwrap().to_owned();
        assert!(status.contains("cancelled"), "{status}");
        let first = window
            .find(("ops-result", 0usize))
            .label()
            .unwrap()
            .to_owned();
        assert!(first.contains("Cancelled"), "{first}");
    })
    .unwrap();
}

#[gpui_kit::test]
fn changing_the_selection_after_opening_the_confirmation_blocks_it(cx: &mut TestAppContext) {
    let (_runtime, screen, handle) = mount(cx, "prod-fra", 1);
    review(cx, handle);
    cx.update_window(handle.into(), |_, window, cx| {
        screen.update(cx, |screen, cx| {
            screen.toggle(
                NodeTarget::new("talos-cp-fra1-02", "10.20.0.12"),
                window,
                cx,
            )
        });
        window.render_frame(cx);
    })
    .unwrap();
    press_ok(cx, handle);
    cx.update_window(handle.into(), |_, window, _| {
        assert!(window.find("confirm-blocked").visible());
    })
    .unwrap();
    assert!(cx.read(Operations::current).is_none());
    assert!(cx.read(|cx| screen.read(cx).run.is_none()));
}

#[gpui_kit::test]
fn a_busy_slot_is_refused_before_and_while_confirming(cx: &mut TestAppContext) {
    let (_runtime, screen, handle) = mount(cx, "prod-fra", 1);
    // Held by something else: reviewing explains instead of opening.
    let ticket = cx.update(|cx| {
        let operations = Operations::global(cx);
        operations
            .update(cx, |operations, cx| {
                operations.begin("prod-fra: Reboot talos-cp-fra1-02", cx)
            })
            .ok()
            .unwrap()
    });
    render(cx, handle);
    cx.update_window(handle.into(), |_, window, cx| {
        let busy = window.find("ops-busy").label().unwrap().to_owned();
        assert!(busy.contains("Reboot talos-cp-fra1-02"), "{busy}");
        window.click("ops-review", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle.into(), |_, window, _| {
        assert!(window.try_find("confirm-dialog").is_none());
    })
    .unwrap();
    // Released, the dialog opens; taken again before confirming, it refuses.
    cx.update(|cx| {
        Operations::global(cx).update(cx, |operations, cx| operations.finish(ticket, cx))
    });
    review(cx, handle);
    let ticket = cx.update(|cx| {
        Operations::global(cx)
            .update(cx, |operations, cx| operations.begin("other", cx))
            .ok()
            .unwrap()
    });
    press_ok(cx, handle);
    cx.update_window(handle.into(), |_, window, _| {
        assert!(window.find("confirm-blocked").visible());
    })
    .unwrap();
    assert!(cx.read(|cx| screen.read(cx).run.is_none()));
    drop(ticket);
}

#[gpui_kit::test]
fn a_partial_failure_stays_visible(cx: &mut TestAppContext) {
    let (runtime, screen, handle) = mount(cx, "prod-fra", 2);
    // talos-wk-fra1-02 answers; talos-wk-fra1-03 does not.
    cx.update_window(handle.into(), |_, window, cx| {
        window.render_frame(cx);
        window.click(row(0), cx);
        window.click(row(4), cx);
        window.click(row(5), cx);
    })
    .unwrap();
    review(cx, handle);
    confirm(cx, handle);
    // One drain, the pause between nodes, and a drain failing halfway: 12 steps.
    wait_until_finished(cx, &runtime, &screen, 15, "the run");
    assert_eq!(
        results(&screen, cx),
        [
            ("talos-wk-fra1-02".to_owned(), OperationStatus::Succeeded),
            ("talos-wk-fra1-03".to_owned(), OperationStatus::Failed),
        ]
    );
    render(cx, handle);
    cx.update_window(handle.into(), |_, window, _| {
        let status = window.find("ops-run-status").label().unwrap().to_owned();
        assert!(status.contains("1 of 2 completed"), "{status}");
        assert!(status.contains("1 failed"), "{status}");
        let failed = window
            .find(("ops-result", 1usize))
            .label()
            .unwrap()
            .to_owned();
        assert!(failed.contains("Failed"), "{failed}");
        assert!(failed.contains("Scheduling unknown"), "{failed}");
        assert!(failed.contains("could not be evicted"), "{failed}");
    })
    .unwrap();
}

#[gpui_kit::test]
fn a_run_keeps_its_context_when_the_target_changes(cx: &mut TestAppContext) {
    let (runtime, screen, handle) = mount(cx, "prod-fra", 40);
    review(cx, handle);
    confirm(cx, handle);
    cx.update_window(handle.into(), |_, window, cx| {
        let other = source("staging-eu", 0);
        screen.update(cx, |screen, cx| screen.set_source(Some(other), window, cx));
        window.render_frame(cx);
        let context = window.find("ops-run-context").label().unwrap().to_owned();
        assert!(context.contains("prod-fra"), "{context}");
        assert!(context.contains("not the context"), "{context}");
    })
    .unwrap();
    // The selection and preview belong to the new target.
    assert_eq!(names(&screen, cx), ["stg-cp-01"]);
    // A drain of one node takes 7 steps.
    wait_until_finished(cx, &runtime, &screen, 10, "the run");
    assert_eq!(
        results(&screen, cx),
        [("talos-cp-fra1-01".to_owned(), OperationStatus::Succeeded)]
    );
}

/// The split of roster and plan runs edge to edge under the toolbar.
const OPERATIONS_SPLIT: layout_check::PageFrame = layout_check::PageFrame {
    page: "ops-page",
    title: "ops-title",
    title_text: "Operations",
    content: "ops-split",
};

const OPERATIONS_TABLE: layout_check::Table = layout_check::Table {
    table: Some("ops-table-scroll"),
    list: "ops-list",
};

/// Operations draws its roster as Pods draws its list: the shared table,
/// bare and edge to edge under the toolbar, with sentence-case column
/// labels, at both text sizes; the operation and its options sit in an
/// inset above it. Where it reads and when is the status bar's segment.
#[gpui_kit::test]
fn operations_draws_its_roster_as_a_bare_table_with_its_status_in_the_bar(cx: &mut TestAppContext) {
    for text in [None, Some(20.)] {
        let (_runtime, handle, _view) = app(cx, 1280., 880.);
        cx.update_window(handle, |_, window, cx| {
            if let Some(text) = text {
                crate::text_size::set(text, cx);
            }
            // Lifecycle, then the next page in the column.
            window.press("secondary-9", cx);
            window.render_frame(cx);
            window.press("ctrl-tab", cx);
            window.render_frame(cx);
        })
        .unwrap();
        cx.run_until_parked();
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            layout_check::assert_edge_frame(window, cx, &OPERATIONS_SPLIT);
            let rows = layout_check::assert_table(window, cx, &OPERATIONS_TABLE);
            assert!(rows.header.is_some(), "{rows:#?}");
            layout_check::assert_bare(window, "ops-table-scroll");
            for (ix, label) in ["Order", "Node", "Address", "Role"].into_iter().enumerate() {
                let header = window
                    .find(("ops-sort", ix))
                    .label()
                    .unwrap_or_default()
                    .to_owned();
                assert_eq!(header, label);
            }
            // The operation's choice sits inset above the table.
            let page = window.find("ops-page").bounds();
            let kinds = window.find("ops-kinds").bounds();
            let table = window.find("ops-table-scroll").bounds();
            assert!(
                kinds.left() > page.left() && kinds.right() < page.right(),
                "{kinds:?}"
            );
            assert!(kinds.bottom() <= table.top(), "{kinds:?} {table:?}");
            // The plan sits beside the roster, or under it at 20 px.
            let plan = window.find("ops-plan").bounds();
            if text.is_none() {
                assert!(plan.left() >= table.right(), "{plan:?} {table:?}");
            } else {
                assert!(plan.top() >= table.bottom(), "{plan:?} {table:?}");
            }
            window.find("ops-refresh");
            assert!(window.try_find("screen-refresh").is_none());
            let scope = window.find("operations-scope");
            assert!(
                scope.path().contains(&ElementId::from("status-bar")),
                "the segment is in the status bar"
            );
            let line = scope.label().unwrap_or_default().to_owned();
            assert!(line.contains("example data"), "{line}");
        })
        .unwrap();
    }
}

/// A target whose roster names no node the operations can reach says so in
/// the table, under its header.
#[gpui_kit::test]
fn an_empty_roster_says_so_in_the_table(cx: &mut TestAppContext) {
    let (_runtime, screen, handle) = mount(cx, "prod-fra", 50);
    cx.update_window(handle.into(), |_, window, cx| {
        let mut source = source("prod-fra", 0);
        source.nodes = Arc::new(Vec::new());
        screen.update(cx, |screen, cx| screen.set_source(Some(source), window, cx));
        window.render_frame(cx);
        let empty = window.find("ops-empty");
        assert!(empty.visible());
        assert!(empty.bounds().top() >= window.find(("ops-sort", 0usize)).bounds().bottom());
        assert!(window.try_find(row(0)).is_none());
    })
    .unwrap();
    assert!(names(&screen, cx).is_empty());
}

/// Choosing nodes in the table, by pointer and keys, runs nothing: clicks,
/// a double-click, Space and Enter only add nodes to the run or take them
/// out. Review still opens the confirmation, and no step runs while it
/// waits.
#[gpui_kit::test]
fn the_table_chooses_nodes_and_review_still_asks_before_any_step(cx: &mut TestAppContext) {
    let (_runtime, screen, handle) = mount(cx, "prod-fra", 1);
    cx.update_window(handle.into(), |_, window, cx| {
        window.render_frame(cx);
        window.click(row(4), cx);
        window.press("down", cx);
        window.press("space", cx);
        window.render_frame(cx);
    })
    .unwrap();
    assert_eq!(
        names(&screen, cx),
        ["talos-cp-fra1-01", "talos-wk-fra1-02", "talos-wk-fra1-03"]
    );
    // Enter on the cursor takes the node out again, as Space does.
    cx.update_window(handle.into(), |_, window, cx| {
        window.press("enter", cx);
        window.render_frame(cx);
    })
    .unwrap();
    assert_eq!(names(&screen, cx), ["talos-cp-fra1-01", "talos-wk-fra1-02"]);
    // A double-click is two clicks: the node goes in and out again.
    cx.update_window(handle.into(), |_, window, cx| {
        window.double_click(row(1), cx);
        window.render_frame(cx);
        assert!(window.try_find("confirm-dialog").is_none());
    })
    .unwrap();
    assert_eq!(names(&screen, cx), ["talos-cp-fra1-01", "talos-wk-fra1-02"]);
    assert!(cx.read(Operations::current).is_none());
    assert!(cx.read(|cx| screen.read(cx).run.is_none()));
    review(cx, handle);
    cx.update_window(handle.into(), |_, window, _| {
        assert!(window.find("confirm-dialog").visible());
    })
    .unwrap();
    assert!(cx.read(Operations::current).is_none());
    assert!(cx.read(|cx| screen.read(cx).run.is_none()));
}

/// Without a target the page keeps its header and gives the status bar no
/// segment: while the overview is read the roster shows its loading rows,
/// and once it answered the page says why it is empty.
#[gpui_kit::test]
fn no_target_sits_under_the_header_without_a_segment(cx: &mut TestAppContext) {
    let (_runtime, screen, handle) = mount(cx, "prod-fra", 50);
    cx.update_window(handle.into(), |_, window, cx| {
        window.render_frame(cx);
        assert!(screen.update(cx, |screen, _| screen.status().is_some()));
        screen.update(cx, |screen, cx| screen.set_source(None, window, cx));
        window.render_frame(cx);
        let toolbar = window.find("ops-toolbar").bounds();
        let loading = window.find("ops-loading").bounds();
        assert!(loading.top() >= toolbar.bottom(), "{loading:?} {toolbar:?}");
        assert!(window.try_find("ops-state").is_none());
        assert!(screen.read(cx).loading_motion(cx).is_some());
        crate::screens::set_reading(crate::screens::Reading::Answered, cx);
        window.render_frame(cx);
        let state = window.find("ops-state").bounds();
        assert!(state.top() >= toolbar.bottom(), "{state:?} {toolbar:?}");
        assert!(window.try_find("ops-loading").is_none());
        assert!(screen.read(cx).loading_motion(cx).is_none());
        assert!(window.try_find("ops-body").is_none());
        assert!(screen.update(cx, |screen, _| screen.status().is_none()));
    })
    .unwrap();
}

/// At 760 px and the largest text, each node's etcd verdict, a chip and its
/// sentence, wraps inside the plan's card rather than running past it.
#[gpui_kit::test]
fn the_etcd_verdict_wraps_inside_the_plan(cx: &mut TestAppContext) {
    // Tall, so the plan is drawn without scrolling; the width is what counts.
    let (_runtime, handle, _view) = app(cx, 760., 2400.);
    cx.update_window(handle, |_, window, cx| {
        crate::text_size::set(20., cx);
        window.press("secondary-9", cx);
        window.render_frame(cx);
        window.press("ctrl-tab", cx);
        window.render_frame(cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let row = window.find(("ops-plan-target", 0usize)).bounds();
        let detail = window.find(("ops-verdict-detail", 0usize)).bounds();
        assert!(
            detail.right() <= row.right() && detail.left() >= row.left(),
            "{detail:?} inside {row:?}"
        );
    })
    .unwrap();
}

/// What the screen has read: the nodes its preview is for, if it has one,
/// and its audit log's revision and whether the log is in.
#[derive(Debug, PartialEq)]
struct Reads {
    preview: Option<Vec<String>>,
    audit: (u64, bool),
}

fn reads(screen: &Entity<OperationsScreen>, cx: &gpui_kit::App) -> Reads {
    let screen = screen.read(cx);
    Reads {
        preview: screen
            .preview
            .key()
            .map(|key| key.targets.iter().map(|t| t.name.clone()).collect()),
        audit: (screen.audit.revision(), screen.audit.data().is_some()),
    }
}

/// Lifecycle, then the next page in Control plane's column.
fn open_operations(window: &mut gpui_kit::Window, cx: &mut gpui_kit::App) {
    window.press("secondary-9", cx);
    window.render_frame(cx);
    window.press("ctrl-tab", cx);
    window.render_frame(cx);
}

/// The shell hands every retained screen a new target, shown or not. Once
/// left, Operations reads nothing for it until it is shown again.
#[gpui_kit::test]
fn hidden_operations_reads_nothing_for_a_new_target_until_shown(cx: &mut TestAppContext) {
    let (_runtime, handle, pilot) = app(cx, 1280., 880.);
    cx.update_window(handle, |_, window, cx| open_operations(window, cx))
        .unwrap();
    cx.run_until_parked();
    let screen = cx.read(|cx| crate::desktop::tests::screen::<OperationsScreen>(&pilot, cx));
    let shown = cx.read(|cx| reads(&screen, cx));
    assert!(shown.preview.is_some() && shown.audit.1, "{shown:?}");

    cx.update_window(handle, |_, window, cx| {
        window.press("secondary-2", cx);
        window.render_frame(cx);
        crate::desktop::tests::pick_target(window, cx, 4);
    })
    .unwrap();
    cx.run_until_parked();
    let node = cx.read(|cx| screen.read(cx).source.as_ref().unwrap().target.node.clone());
    assert_ne!(Some(&vec![node.clone()]), shown.preview.as_ref());
    // The new target cleared the old preview and audit and asked for neither.
    let hidden = cx.read(|cx| reads(&screen, cx));
    assert_eq!(hidden.preview, None);
    assert!(!hidden.audit.1, "{hidden:?}");
    assert_eq!(names(&screen, cx), std::slice::from_ref(&node));

    cx.update_window(handle, |_, window, cx| open_operations(window, cx))
        .unwrap();
    cx.run_until_parked();
    let again = cx.read(|cx| reads(&screen, cx));
    assert_eq!(again.preview, Some(vec![node]));
    assert!(again.audit.1, "{again:?}");
}

/// The overview publishes the same target again while Operations is hidden:
/// its preview and audit log stay as they are, read once.
#[gpui_kit::test]
fn a_same_target_update_while_hidden_reads_nothing(cx: &mut TestAppContext) {
    let (_runtime, screen, handle) = mount(cx, "prod-fra", 1);
    let before = cx.read(|cx| reads(&screen, cx));
    assert!(before.preview.is_some() && before.audit.1, "{before:?}");
    cx.update_window(handle.into(), |_, window, cx| {
        for _ in 0..3 {
            let same = source("prod-fra", 0);
            screen.update(cx, |screen, cx| screen.set_source(Some(same), window, cx));
        }
    })
    .unwrap();
    cx.run_until_parked();
    assert_eq!(cx.read(|cx| reads(&screen, cx)), before);
}

/// A submitted run outlives the screen being left and its target changing:
/// it finishes on the cluster it started on, and the hidden screen starts no
/// preview for the new target meanwhile.
#[gpui_kit::test]
fn a_submitted_run_carries_on_through_hidden_source_updates(cx: &mut TestAppContext) {
    let (runtime, screen, handle) = mount(cx, "prod-fra", 40);
    review(cx, handle);
    confirm(cx, handle);
    step(cx, &runtime, &screen);
    cx.update_window(handle.into(), |_, window, cx| {
        for other in [source("staging-eu", 0), source("staging-eu", 0)] {
            screen.update(cx, |screen, cx| screen.set_source(Some(other), window, cx));
        }
    })
    .unwrap();
    cx.run_until_parked();
    assert_eq!(cx.read(|cx| reads(&screen, cx)).preview, None);
    // A drain of one node takes 7 steps.
    wait_until_finished(cx, &runtime, &screen, 10, "the run");
    assert_eq!(
        results(&screen, cx),
        [("talos-cp-fra1-01".to_owned(), OperationStatus::Succeeded)]
    );
    assert_eq!(cx.read(|cx| reads(&screen, cx)).preview, None);
}

/// A talosconfig, a kubeconfig and the token file it names, in a fresh
/// temporary directory, and the access a preview took on them.
struct AccessFiles {
    _directory: tempfile::TempDir,
    talosconfig: std::path::PathBuf,
    kubeconfig: std::path::PathBuf,
    token: std::path::PathBuf,
    access: AppliedAccess,
}

fn access_files() -> AccessFiles {
    let directory = tempfile::tempdir().unwrap();
    let talosconfig = directory.path().join("talosconfig");
    let kubeconfig = directory.path().join("kubeconfig");
    let token = directory.path().join("token");
    std::fs::write(&talosconfig, "context: example\n").unwrap();
    std::fs::write(&token, "first-token\n").unwrap();
    std::fs::write(
        &kubeconfig,
        format!(
            "apiVersion: v1\nkind: Config\ncurrent-context: example\n\
             clusters:\n- name: example\n  cluster:\n    server: https://192.0.2.1:6443\n\
             users:\n- name: example\n  user:\n    tokenFile: {}\n\
             contexts:\n- name: example\n  context:\n    cluster: example\n    user: example\n",
            token.display()
        ),
    )
    .unwrap();
    let selection = KubeconfigSelection::File {
        path: kubeconfig.clone(),
        context: None,
    };
    let configuration = ConfigurationRevision::for_talos_sources(Some(&talosconfig), &selection);
    let access = AppliedAccess {
        config_path: Some(talosconfig.clone()),
        selection,
        configuration,
        identity: AccessIdentity::new(AccessSessionId::new(), configuration),
    };
    AccessFiles {
        _directory: directory,
        talosconfig,
        kubeconfig,
        token,
        access,
    }
}

/// Stands in for the Kubernetes API: counts the clients built for it and
/// the mutations that reach it.
#[derive(Clone, Default)]
struct FakeApi {
    built: Arc<AtomicUsize>,
    mutations: Arc<AtomicUsize>,
}

impl FakeApi {
    /// A fake connector: builds a client for this API, doing `meanwhile`
    /// while it does.
    async fn connect(self, meanwhile: impl FnOnce()) -> Result<FakeApi, String> {
        self.built.fetch_add(1, Ordering::SeqCst);
        meanwhile();
        Ok(self)
    }

    fn counts(&self) -> (usize, usize) {
        (
            self.built.load(Ordering::SeqCst),
            self.mutations.load(Ordering::SeqCst),
        )
    }
}

/// As core's runner: nothing is submitted until the connection answers, then
/// the confirmed mutation once.
async fn submit(
    connect: impl std::future::Future<Output = Result<FakeApi, String>>,
) -> Result<(), String> {
    let api = connect.await?;
    api.mutations.fetch_add(1, Ordering::SeqCst);
    Ok(())
}

fn block_on<T>(future: impl std::future::Future<Output = T>) -> T {
    Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(future)
}

/// Unchanged access still submits, exactly once.
#[test]
fn a_run_on_unchanged_access_submits_once() {
    let files = access_files();
    let api = FakeApi::default();
    let access = files.access.clone();
    let connect = api.clone().connect(|| {});
    block_on(submit(run_client(
        Some(access.clone()),
        Some(access),
        connect,
    )))
    .unwrap();
    assert_eq!(api.counts(), (1, 1));
}

/// Replacing the kubeconfig, the credential it names or the talosconfig
/// between the preview and the confirmation refuses the run before a client
/// is built, so nothing is submitted.
#[test]
fn access_replaced_after_the_preview_submits_nothing() {
    for replace in ["kubeconfig", "token", "talosconfig"] {
        let files = access_files();
        let path = match replace {
            "kubeconfig" => &files.kubeconfig,
            "token" => &files.token,
            _ => &files.talosconfig,
        };
        let mut contents = std::fs::read(path).unwrap();
        contents.extend_from_slice(b"# replaced\n");
        std::fs::write(path, contents).unwrap();
        let api = FakeApi::default();
        let access = files.access.clone();
        let connect = api.clone().connect(|| {});
        let error = block_on(submit(run_client(
            Some(access.clone()),
            Some(access),
            connect,
        )))
        .expect_err(replace);
        assert!(error.contains("changed"), "{replace}: {error}");
        assert_eq!(api.counts(), (0, 0), "{replace}");
    }
}

/// A credential replaced while the client is being built is caught by the
/// check after it: the client is dropped and nothing is submitted.
#[test]
fn access_replaced_while_the_client_is_built_submits_nothing() {
    let files = access_files();
    let api = FakeApi::default();
    let token = files.token.clone();
    let connect = api.clone().connect(move || {
        std::fs::write(&token, "second-token\n").unwrap();
    });
    let access = files.access.clone();
    let error = block_on(submit(run_client(
        Some(access.clone()),
        Some(access),
        connect,
    )))
    .unwrap_err();
    assert!(error.contains("changed"), "{error}");
    assert_eq!(api.counts(), (1, 0));
}

/// A confirmed source whose access isn't the preview's, or a preview that
/// recorded none, refuses the run before a client is built.
#[test]
fn a_run_without_the_previews_access_submits_nothing() {
    let files = access_files();
    let other = AppliedAccess {
        identity: AccessIdentity::new(AccessSessionId::new(), files.access.configuration),
        ..files.access.clone()
    };
    for (previewed, confirmed) in [
        (Some(files.access.clone()), Some(other)),
        (Some(files.access.clone()), None),
        (None, Some(files.access.clone())),
    ] {
        let api = FakeApi::default();
        let connect = api.clone().connect(|| {});
        assert!(block_on(submit(run_client(previewed, confirmed, connect))).is_err());
        assert_eq!(api.counts(), (0, 0));
    }
}
