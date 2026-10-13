//! Each kind of control in Nodes asks the shell through a `NodesEvent`,
//! which the shell hears once the click or key's update ends: these drive
//! the real click or key, let the event land, then look.

use super::super::NodesEvent;
use super::*;

/// A card's Open opens its node.
#[gpui_kit::test]
fn a_cards_open_opens_its_node(cx: &mut TestAppContext) {
    let (_runtime, handle, pilot) = fixture(cx, 1500., 880.);
    let row = step(cx, handle, |window, cx| {
        window.render_frame(cx);
        window.click("nav-nodes", cx);
        window.render_frame(cx);
        window.click("nodes-view-cards", cx);
        window.render_frame(cx);
        let row = pilot.read(cx).node_workspace.read(cx).rows[1].clone();
        window.click(row.open_id.clone(), cx);
        row
    });
    pilot.read_with(cx, |pilot, cx| {
        let nodes = pilot.node_workspace.read(cx);
        assert!(nodes.open);
        assert_eq!(nodes.selected.as_ref(), Some(&row.key));
    });
}

/// More's Events and YAML show those tabs. Expanded, the pane is wide
/// enough for More to show at the strip's end without a scroll.
#[gpui_kit::test]
fn mores_items_show_their_tabs(cx: &mut TestAppContext) {
    let (_runtime, handle, pilot) = fixture(cx, 1500., 880.);
    step(cx, handle, |window, cx| {
        open_first_node(window, cx);
        window.click("node-expand", cx);
    });
    for (item, tab) in [(0usize, NodeTab::Events), (1, NodeTab::Yaml)] {
        step(cx, handle, |window, cx| {
            window.click("node-more", cx);
            window.render_frame(cx);
            window.within("popup-menu").click(item, cx);
        });
        pilot.read_with(cx, |pilot, cx| {
            assert_eq!(pilot.node_workspace.read(cx).tab, tab);
        });
    }
}

/// Retry, when Nodes failed, asks the shell to read again.
#[gpui_kit::test]
fn retry_asks_the_shell_to_refresh(cx: &mut TestAppContext) {
    let (_runtime, handle, pilot) = fixture(cx, 1500., 880.);
    let heard = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
    let nodes = pilot.read_with(cx, |pilot, _| pilot.node_workspace.clone());
    let _subscription = cx.update({
        let heard = heard.clone();
        |cx| {
            cx.subscribe(&nodes, move |_, event: &NodesEvent, _| {
                heard.borrow_mut().push(event.clone())
            })
        }
    });
    step(cx, handle, |window, cx| {
        window.render_frame(cx);
        window.click("nav-nodes", cx);
        nodes.update(cx, |nodes, cx| {
            nodes.empty = Some(super::super::Empty::Failed("Read result".into()));
            cx.notify();
        });
        window.render_frame(cx);
        window.click("nodes-retry", cx);
    });
    let heard = heard.borrow();
    assert!(
        matches!(heard.as_slice(), [NodesEvent::Refresh]),
        "{heard:?}"
    );
}

/// An attention row in the node's pane opens where it points.
#[gpui_kit::test]
fn a_pane_attention_row_opens_its_destination(cx: &mut TestAppContext) {
    use crate::presentation::attention::Destination;
    let (_runtime, handle, pilot) = fixture(cx, 1500., 880.);
    let row = step(cx, handle, |window, cx| {
        window.render_frame(cx);
        window.click("nav-nodes", cx);
        window.render_frame(cx);
        crate::desktop::tests::click_node(window, cx, "node-talos-wk-fra1-02");
        window.render_frame(cx);
        let row = (pilot.read(cx).node_workspace.read(cx).attention)
            .by_node
            .get("talos-wk-fra1-02")
            .and_then(|rows| rows.first().cloned())
            .expect("an attention row for the failing node");
        window.within(row.id.clone()).click("attention-open", cx);
        row
    });
    pilot.read_with(cx, |pilot, cx| match &row.open {
        Destination::Node(key, tab) => {
            let nodes = pilot.node_workspace.read(cx);
            assert_eq!((nodes.selected.as_ref(), nodes.tab), (Some(key), *tab));
        }
        other => assert_ne!(pilot.page, Page::Nodes, "{other:?}"),
    });
}

/// Command-Shift-] and Command-Shift-[ step through the pane's tabs.
#[gpui_kit::test]
fn command_shift_brackets_step_the_pane_tabs(cx: &mut TestAppContext) {
    let (_runtime, handle, pilot) = fixture(cx, 1500., 880.);
    step(cx, handle, |window, cx| {
        open_first_node(window, cx);
        pilot.update(cx, |pilot, cx| {
            window.focus(&pilot.node_workspace.read(cx).focus.clone(), cx)
        });
        window.press("secondary-}", cx);
    });
    let next = pilot.read_with(cx, |pilot, cx| pilot.node_workspace.read(cx).tab);
    assert_ne!(next, NodeTab::Overview);
    step(cx, handle, |window, cx| window.press("secondary-{", cx));
    pilot.read_with(cx, |pilot, cx| {
        assert_eq!(pilot.node_workspace.read(cx).tab, NodeTab::Overview);
    });
}
