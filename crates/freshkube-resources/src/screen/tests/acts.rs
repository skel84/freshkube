//! The everyday actions on the list (#857): offered from the row menu, the
//! keys and the toolbar for the kinds that have them, disabled with the
//! reason while writes are blocked, and asking the shell, never sending.
use super::*;
use freshkube_core::write::everyday::Everyday;

use crate::screen::WorkloadAct;

type Heard = Rc<RefCell<Vec<(Everyday, String)>>>;

/// Opens the deployments list on its first row and records what it asks.
fn deployments(
    cx: &mut TestAppContext,
) -> (Runtime, Entity<ResourcesScreen>, AnyWindowHandle, Heard) {
    let (runtime, screen, handle) = mount(cx, Some("homelab"));
    let heard: Heard = Rc::default();
    cx.update(|cx| {
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
        screen.update(cx, |screen, cx| {
            screen.projection.select(&screen.store, Some(0));
            cx.notify();
        });
        window.render_frame(cx);
    })
    .unwrap();
    cx.run_until_parked();
    (runtime, screen, handle, heard)
}

fn row_menu(
    cx: &mut TestAppContext,
    handle: AnyWindowHandle,
    screen: &Entity<ResourcesScreen>,
) -> Vec<Option<String>> {
    let row = cx
        .update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            row_id(&identity_at(screen, 0, cx))
        })
        .unwrap();
    open_menu(cx, handle, |window, cx| {
        window.within(row).right_click("name", cx)
    })
}

#[gpui_kit::test]
fn a_deployment_offers_scale_and_restart_and_a_pod_offers_neither(cx: &mut TestAppContext) {
    let (_runtime, screen, handle) = mount(cx, Some("homelab"));
    let pods = row_menu(cx, handle, &screen);
    assert!(!pods.contains(&Some("Scale…".into())), "{pods:?}");
    assert!(!pods.contains(&Some("Restart".into())), "{pods:?}");
    let (_runtime, screen, handle, _) = deployments(cx);
    let items = row_menu(cx, handle, &screen);
    assert!(items.contains(&Some("Scale…".into())), "{items:?}");
    assert!(items.contains(&Some("Restart".into())), "{items:?}");
    assert!(
        !items.contains(&Some("Suspend or resume".into())),
        "{items:?}"
    );
}

#[gpui_kit::test]
fn the_menu_and_the_keys_ask_for_the_selected_object(cx: &mut TestAppContext) {
    let (_runtime, screen, handle, heard) = deployments(cx);
    let items = row_menu(cx, handle, &screen);
    cx.update_window(handle, |_, window, cx| pick(window, &items, "Restart", cx))
        .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.press("s", cx);
    })
    .unwrap();
    cx.run_until_parked();
    // A kind with no suspend does nothing for its key.
    cx.update_window(handle, |_, window, cx| window.press("p", cx))
        .unwrap();
    cx.run_until_parked();
    let heard = heard.borrow();
    let name = cx.read(|cx| identity_at(&screen, 0, cx).name);
    assert_eq!(
        *heard,
        [(Everyday::Restart, name.clone()), (Everyday::Scale, name)]
    );
}

#[gpui_kit::test]
fn a_blocked_cluster_shows_the_actions_disabled_and_a_key_does_nothing(cx: &mut TestAppContext) {
    let (_runtime, screen, handle, heard) = deployments(cx);
    cx.update(|cx| {
        screen.update(cx, |screen, cx| {
            screen.set_write_block(Some("This cluster is read-only."), cx)
        })
    });
    let items = row_menu(cx, handle, &screen);
    // Shown, not hidden.
    assert!(items.contains(&Some("Restart".into())), "{items:?}");
    cx.update_window(handle, |_, window, cx| {
        pick(window, &items, "Restart", cx);
        window.render_frame(cx);
        window.press("r", cx);
    })
    .unwrap();
    cx.run_until_parked();
    assert!(heard.borrow().is_empty());
    // The toolbar's button is there and does nothing either.
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("resource-restart", cx);
    })
    .unwrap();
    cx.run_until_parked();
    assert!(heard.borrow().is_empty());
}
