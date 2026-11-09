//! What the cluster says before an action is offered (#854, slice 2): a
//! refused action is shown disabled with its reason, never hidden; a review
//! that did not answer leaves it on; the level's reason wins; and nothing is
//! asked for a row that is not selected. The answers only guide, so nothing
//! here sends a write.
use super::*;
use freshkube_core::access::{Offer, example::Shape};
use freshkube_core::write::everyday::Everyday;
use freshkube_reads::OfferDesk;

use super::acts::row_menu;
use crate::screen::WorkloadAct;

type Heard = Rc<RefCell<Vec<(Everyday, String)>>>;

fn desk(cx: &gpui_kit::App) -> Entity<OfferDesk> {
    freshkube_reads::existing_offer_desk(cx).expect("the screen made the desk")
}

/// Opens the deployments list, the example answering `shape`, with the first
/// row selected the way a click does, and records what the list asks of the
/// shell.
fn opened(
    cx: &mut TestAppContext,
    shape: Shape,
) -> (Runtime, Entity<ResourcesScreen>, AnyWindowHandle, Heard) {
    let (runtime, screen, handle) = mount(cx, Some("homelab"));
    let heard: Heard = Rc::default();
    cx.update(|cx| {
        desk(cx).update(cx, |desk, _| desk.set_example(shape));
        let heard = heard.clone();
        cx.subscribe(&screen, move |_, act: &WorkloadAct, _| {
            heard
                .borrow_mut()
                .push((act.act, act.identity.name.clone()));
        })
        .detach();
    });
    cx.update_window(handle, |_, window, cx| {
        screen.update(cx, |screen, cx| {
            screen.set_kind(kind("deployments.apps"), window, cx);
            screen.focus(window, cx);
        });
        window.render_frame(cx);
        let first = identity_at(&screen, 0, cx);
        screen.update(cx, |screen, cx| screen.select_identity(&first, window, cx));
        window.render_frame(cx);
    })
    .unwrap();
    cx.run_until_parked();
    (runtime, screen, handle, heard)
}

fn block(
    screen: &Entity<ResourcesScreen>,
    act: Everyday,
    cx: &mut TestAppContext,
) -> Option<String> {
    cx.read(|cx| {
        screen
            .read(cx)
            .act_block(act, cx)
            .map(|reason| reason.to_string())
    })
}

fn namespace(screen: &Entity<ResourcesScreen>, cx: &mut TestAppContext) -> String {
    cx.read(|cx| identity_at(screen, 0, cx).namespace)
}

fn click(cx: &mut TestAppContext, handle: AnyWindowHandle, id: &'static str) {
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click(id, cx);
    })
    .unwrap();
    cx.run_until_parked();
}

#[gpui_kit::test]
fn a_refused_action_is_disabled_with_its_reason_and_does_nothing(cx: &mut TestAppContext) {
    let (_runtime, screen, handle, heard) = opened(cx, Shape::ActionsRefused);
    let space = namespace(&screen, cx);
    assert_eq!(
        block(&screen, Everyday::Delete, cx),
        Some(format!("Needs delete on deployments in {space}"))
    );
    // A patch is allowed in two namespaces and refused elsewhere.
    let writes_here = matches!(space.as_str(), "shop" | "payments");
    assert_eq!(
        block(&screen, Everyday::Restart, cx),
        (!writes_here).then(|| format!("Needs patch on deployments in {space}"))
    );
    // Shown, not hidden: the menu lists it, and neither the toolbar's button
    // nor its key asks the shell.
    let items = row_menu(cx, handle, &screen);
    assert!(items.contains(&Some("Delete…".into())), "{items:?}");
    click(cx, handle, "resource-delete");
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.press("backspace", cx);
        window.press("delete", cx);
    })
    .unwrap();
    cx.run_until_parked();
    assert!(
        heard
            .borrow()
            .iter()
            .all(|(act, _)| *act != Everyday::Delete),
        "{:?}",
        heard.borrow()
    );
}

#[gpui_kit::test]
fn the_levels_reason_wins_over_the_clusters(cx: &mut TestAppContext) {
    let (_runtime, screen, _handle, _) = opened(cx, Shape::ActionsRefused);
    assert!(
        block(&screen, Everyday::Delete, cx)
            .is_some_and(|reason| reason.starts_with("Needs delete"))
    );
    cx.update(|cx| {
        screen.update(cx, |screen, cx| {
            screen.set_write_block(Some("This cluster is read-only."), cx)
        })
    });
    for act in [Everyday::Delete, Everyday::Restart] {
        assert_eq!(
            block(&screen, act, cx).as_deref(),
            Some("This cluster is read-only.")
        );
    }
}

#[gpui_kit::test]
fn a_review_that_failed_leaves_the_action_on_and_says_so(cx: &mut TestAppContext) {
    let (_runtime, screen, handle, heard) = opened(cx, Shape::Failed);
    assert_eq!(block(&screen, Everyday::Restart, cx), None);
    let offer = cx.read(|cx| screen.read(cx).act_offer(Everyday::Restart, cx));
    assert_eq!(offer, Offer::Unchecked);
    assert_eq!(offer.note(), Some("Couldn't check your access"));
    // It still asks the shell: the gate decides.
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.press("r", cx);
    })
    .unwrap();
    cx.run_until_parked();
    let name = cx.read(|cx| identity_at(&screen, 0, cx).name);
    assert_eq!(*heard.borrow(), [(Everyday::Restart, name)]);
}

#[gpui_kit::test]
fn an_allowed_action_is_on_and_says_nothing(cx: &mut TestAppContext) {
    let (_runtime, screen, _handle, _) = opened(cx, Shape::Full);
    assert_eq!(block(&screen, Everyday::Delete, cx), None);
    let offer = cx.read(|cx| screen.read(cx).act_offer(Everyday::Delete, cx));
    assert_eq!(offer, Offer::Allowed);
    assert_eq!(offer.note(), None);
}

#[gpui_kit::test]
fn safety_g20_only_the_selected_rows_actions_are_asked_about(cx: &mut TestAppContext) {
    let (_runtime, screen, handle, _) = opened(cx, Shape::ActionsRefused);
    let rows = cx.read(|cx| {
        let screen = screen.read(cx);
        screen.store.len()
    });
    assert!(rows > 3, "the example lists several deployments: {rows}");
    let first = cx.read(|cx| identity_at(&screen, 0, cx).name);
    let asked = |cx: &mut TestAppContext| -> Vec<String> {
        cx.read(|cx| {
            desk(cx)
                .read(cx)
                .asked()
                .iter()
                .filter_map(|question| question.name.clone())
                .collect()
        })
    };
    let names = asked(cx);
    assert!(!names.is_empty());
    assert!(names.iter().all(|name| *name == first), "{names:?}");
    // Moving the selection asks about the next row, and still no other.
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.press("down", cx);
        window.render_frame(cx);
    })
    .unwrap();
    cx.run_until_parked();
    let second = cx.read(|cx| {
        screen
            .read(cx)
            .projection
            .selected()
            .map(|id| id.name.clone())
    });
    let names = asked(cx);
    let mut distinct: Vec<_> = names.iter().collect();
    distinct.sort();
    distinct.dedup();
    assert_eq!(distinct.len(), 2, "{names:?}");
    assert!(names.contains(&second.unwrap()), "{names:?}");
}

#[gpui_kit::test]
fn safety_g20_a_hidden_page_asks_no_review(cx: &mut TestAppContext) {
    let (_runtime, screen, handle, _) = opened(cx, Shape::ActionsRefused);
    cx.update_window(handle, |_, window, cx| {
        screen.update(cx, |screen, cx| screen.set_visible(false, window, cx));
        desk(cx).update(cx, |desk, _| desk.forget_answers());
        let second = identity_at(&screen, 1, cx);
        screen.update(cx, |screen, cx| screen.select_identity(&second, window, cx));
    })
    .unwrap();
    cx.run_until_parked();
    let asked = cx.read(|cx| desk(cx).read(cx).asked().len());
    assert_eq!(asked, 0);
}
