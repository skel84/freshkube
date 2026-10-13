//! Nodes is a cached view of its own: a notify of the shell no longer draws
//! it again, so every change it shows must notify it. These count its
//! draws (`probe::count`) for a bare shell notify, for its own changes and
//! for the changes the shell makes on its behalf.

use super::*;
use crate::desktop::Pilot;
use gpui_kit::{Context, Entity, Window};

fn draws() -> usize {
    crate::desktop::probe::count(Page::Nodes.slug())
}

fn draw(window: &mut Window, cx: &mut gpui_kit::App) {
    window.draw(cx).clear(cx);
}

/// Nodes' page drawn and settled, with talos-wk-fra1-02 open on `tab`.
fn settled(
    tab: NodeTab,
    cx: &mut TestAppContext,
) -> (
    tokio::runtime::Runtime,
    gpui_kit::AnyWindowHandle,
    Entity<Pilot>,
) {
    let (runtime, handle, pilot) = fixture(cx, 1500., 880.);
    cx.update_window(handle, |_, window, cx| {
        pilot.update(cx, |pilot, cx| {
            pilot.open_node_by_name("talos-wk-fra1-02", tab, window, cx)
        });
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        draw(window, cx);
        draw(window, cx);
    })
    .unwrap();
    cx.run_until_parked();
    (runtime, handle, pilot)
}

/// How many times Nodes draws for `change`, made on the shell.
fn redrawn(
    handle: gpui_kit::AnyWindowHandle,
    pilot: &Entity<Pilot>,
    cx: &mut TestAppContext,
    change: impl FnOnce(&mut Pilot, &mut Window, &mut Context<Pilot>),
) -> usize {
    let before = draws();
    cx.update_window(handle, |_, window, cx| {
        pilot.update(cx, |pilot, cx| change(pilot, window, cx))
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| draw(window, cx))
        .unwrap();
    draws() - before
}

/// A notify of the shell that changes none of what Nodes shows leaves it as
/// drawn; its rows, a selection, a tab, the pane's Pods list, the theme and
/// the text size draw it again. A change may take a settling frame or two
/// more (the table's reveal, the log's measured height), so these count
/// whether Nodes draws, not how often.
#[gpui_kit::test]
fn nodes_redraws_for_its_own_changes(cx: &mut TestAppContext) {
    use gpui_kit::{
        component::{Theme, ThemeMode},
        px,
    };
    let (_runtime, handle, pilot) = settled(NodeTab::Overview, cx);
    let mut redrawn = |change: &dyn Fn(&mut Pilot, &mut Window, &mut Context<Pilot>)| {
        redrawn(handle, &pilot, cx, |pilot, window, cx| {
            change(pilot, window, cx)
        })
    };
    let shell = redrawn(&|_, _, cx| cx.notify());
    let rows = redrawn(&|pilot, _, cx| pilot.rebuild_joined_nodes(cx));
    let select = redrawn(&|pilot, window, cx| {
        pilot.open_node_by_name("talos-wk-fra1-03", NodeTab::Overview, window, cx)
    });
    let tab = redrawn(&|pilot, window, cx| pilot.show_node_tab(NodeTab::Pods, window, cx));
    let unchanged = redrawn(&|_, _, cx| cx.notify());
    let pods = redrawn(&|pilot, _, cx| {
        let pods = pilot.node_workspace.read(cx).pods.clone();
        pods.update(cx, |_, cx| cx.notify());
    });
    let cached = redrawn(&|pilot, _, cx| pilot.notify_cached(cx));
    let text_size = redrawn(&|_, _, cx| Theme::update(cx, |theme| theme.font_size = px(20.)));
    let theme = redrawn(&|_, window, cx| Theme::change(ThemeMode::Dark, Some(window), cx));
    let counts = [
        shell, rows, select, tab, unchanged, pods, cached, text_size, theme,
    ];
    let drawn = counts.map(|count| count > 0);
    assert_eq!(
        drawn,
        [false, true, true, true, false, true, true, true, true],
        "{counts:?}"
    );
}

/// The pointer moving onto a row draws Nodes again for the row's hover
/// tint.
#[gpui_kit::test]
fn a_row_hover_redraws_nodes(cx: &mut TestAppContext) {
    use gpui_kit::{InputEvent, MouseMoveEvent};
    let (_runtime, handle, _pilot) = settled(NodeTab::Overview, cx);
    let before = draws();
    cx.update_window(handle, |_, window, cx| {
        let row = window.find("node-talos-wk-fra1-03").bounds().center();
        window.dispatch_event(
            MouseMoveEvent {
                position: row,
                ..Default::default()
            }
            .to_platform_input(),
            cx,
        );
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| draw(window, cx))
        .unwrap();
    assert!(draws() > before, "the hover draws Nodes");
}

/// The Logs tab's dot shows while logs are collected. Seen from another
/// tab, the log panel isn't drawn, so its notify reaches Nodes only
/// because Nodes observes it.
#[gpui_kit::test]
fn the_logs_dot_follows_collection_from_another_tab(cx: &mut TestAppContext) {
    use crate::logs::TalosPanel;
    let (_runtime, handle, pilot) = settled(NodeTab::Logs, cx);
    let collecting = |cx: &mut TestAppContext| {
        pilot.read_with(cx, |pilot, cx| {
            pilot
                .node_workspace
                .read(cx)
                .logs
                .read(cx)
                .collecting_count()
        })
    };
    step(cx, handle, |window, cx| window.click("logs-collection", cx));
    assert!(collecting(cx) > 0, "collecting from the Logs tab");
    redrawn(handle, &pilot, cx, |pilot, window, cx| {
        pilot.show_node_tab(NodeTab::Overview, window, cx)
    });
    let stopped = redrawn(handle, &pilot, cx, |pilot, window, cx| {
        let logs = pilot.node_workspace.read(cx).logs.clone();
        logs.update(cx, |logs, cx| logs.set_target(None, Vec::new(), window, cx));
    });
    assert_eq!(collecting(cx), 0);
    assert!(stopped > 0, "the dot goes when collection stops");
}

/// A switch between Kubernetes-only and Talos rebuilds the table's columns
/// and draws them.
#[gpui_kit::test]
fn a_source_mode_switch_redraws_the_columns(cx: &mut TestAppContext) {
    let (_runtime, handle, pilot) = settled(NodeTab::Overview, cx);
    let redraws = redrawn(handle, &pilot, cx, |pilot, _, cx| {
        pilot.sync_nodes_source_mode(cx)
    });
    assert!(redraws > 0);
}

/// The first history source makes the Overview tab's chart show; Nodes
/// draws it, though the chart, not yet in the tree, can't ask.
#[gpui_kit::test]
fn the_first_history_source_draws_the_chart(cx: &mut TestAppContext) {
    let (_runtime, handle, pilot) = settled(NodeTab::Overview, cx);
    let history = pilot.read_with(cx, |pilot, cx| {
        pilot.node_workspace.read(cx).history.clone()
    });
    let source = pilot.read_with(cx, |pilot, cx| pilot.monitoring.read(cx).history());
    assert!(source.is_some(), "the example has a history source");
    let gone = redrawn(handle, &pilot, cx, |_, _, cx| {
        history.update(cx, |history, cx| history.set_source(None, cx))
    });
    assert!(!history.read_with(cx, |history, _| history.shows()));
    assert!(gone > 0, "the chart goes");
    let first = redrawn(handle, &pilot, cx, |pilot, _, cx| pilot.push_history(cx));
    assert!(history.read_with(cx, |history, _| history.shows()));
    assert!(first > 0, "the chart shows");
}

/// A new source for node usage, as the shell hands every page, rebuilds
/// the table's usage cells and draws them.
#[gpui_kit::test]
fn a_metrics_source_change_redraws_the_usage_cells(cx: &mut TestAppContext) {
    let (_runtime, handle, pilot) = settled(NodeTab::Overview, cx);
    let redraws = redrawn(handle, &pilot, cx, |pilot, window, cx| {
        pilot
            .node_workspace
            .update(cx, |nodes, _| nodes.metrics.source = Some("another".into()));
        pilot.push_source(window, cx);
    });
    assert!(redraws > 0);
}
