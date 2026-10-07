use std::sync::Arc;
use std::time::Duration;

use freshkube_core::operations::{NodeTarget, OperationKind, OperationStatus};
use gpui_kit::component::Root;
use gpui_kit::test::TestWindowExt;
use gpui_kit::{AppContext, ElementId, Entity, TestAppContext, WindowHandle, px, size};
use tokio::runtime::{Builder, Runtime};

// Not `super::*`: gpui_kit's glob would shadow the built-in `#[test]`.
use super::{
    Operations, OperationsScreen, PreviewKey, PreviewState, ScreenPanel, ScreenSource,
    blocked_reason, example_preview, verdict,
};
use crate::backend::Target;
use crate::desktop::layout_check;
use crate::desktop::tests::fixture as app;
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

/// Without a target the page keeps its header, says why it is empty and
/// gives the status bar no segment.
#[gpui_kit::test]
fn no_target_sits_under_the_header_without_a_segment(cx: &mut TestAppContext) {
    let (_runtime, screen, handle) = mount(cx, "prod-fra", 50);
    cx.update_window(handle.into(), |_, window, cx| {
        window.render_frame(cx);
        assert!(screen.update(cx, |screen, _| screen.status().is_some()));
        screen.update(cx, |screen, cx| screen.set_source(None, window, cx));
        window.render_frame(cx);
        let toolbar = window.find("ops-toolbar").bounds();
        let state = window.find("ops-state").bounds();
        assert!(state.top() >= toolbar.bottom(), "{state:?} {toolbar:?}");
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
