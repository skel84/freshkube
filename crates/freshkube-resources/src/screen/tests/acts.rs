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
    opened(cx, "deployments.apps")
}

/// Opens a kind's list on its first row and records what it asks.
fn opened(
    cx: &mut TestAppContext,
    key: &str,
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
            screen.set_kind(kind(key), window, cx);
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
    assert!(items.contains(&Some("Roll back…".into())), "{items:?}");
    for other in ["Suspend or resume", "Run now"] {
        assert!(!items.contains(&Some(other.into())), "{items:?}");
    }
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

#[gpui_kit::test]
fn a_replicaset_the_pane_shows_owned_by_a_deployment_does_not_offer_scale(cx: &mut TestAppContext) {
    let (_runtime, screen, handle) = mount(cx, Some("homelab"));
    cx.update_window(handle, |_, window, cx| {
        screen.update(cx, |screen, cx| {
            screen.set_kind(kind("replicasets.apps"), window, cx);
            screen.focus(window, cx);
        });
        window.render_frame(cx);
    })
    .unwrap();
    cx.run_until_parked();
    let owned = cx.read(|cx| {
        let rows = screen.read(cx).projection.len();
        (0..rows)
            .map(|ix| identity_at(&screen, ix, cx))
            .find(|identity| identity.name.starts_with("api-"))
            .expect("an example ReplicaSet of api")
    });
    // Before the pane has read it the list cannot know the owner: Scale is on
    // offer.
    cx.update(|cx| {
        screen.update(cx, |screen, cx| {
            let ix = (0..screen.projection.len())
                .find(|ix| {
                    screen
                        .projection
                        .row(&screen.store, *ix)
                        .is_some_and(|row| row.identity == owned)
                })
                .unwrap();
            screen.projection.select(&screen.store, Some(ix));
            cx.notify();
        })
    });
    let before = cx.read(|cx| screen.read(cx).acts(cx));
    assert_eq!(before, [Everyday::Scale]);
    // Open it: the pane reads the object and says who owns it.
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.within(row_id(&owned)).click("name", cx);
        window.render_frame(cx);
    })
    .unwrap();
    cx.executor().advance_clock(KEYBOARD_PAUSE);
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| window.render_frame(cx))
        .unwrap();
    let owner = cx.read(|cx| {
        screen
            .read(cx)
            .detail
            .read(cx)
            .controller_of(&owned)
            .map(str::to_owned)
    });
    assert_eq!(owner.as_deref(), Some("Deployment"));
    // The row's menu leaves Scale off, and the toolbar's offer agrees.
    let labels = cx.read(|cx| {
        screen
            .read(cx)
            .act_menu(cx)
            .into_iter()
            .filter_map(|line| match line {
                freshkube_ui::menu::MenuAction::Item { label, .. } => Some(label.to_string()),
                _ => None,
            })
            .collect::<Vec<_>>()
    });
    assert!(labels.is_empty(), "a ReplicaSet has only Scale: {labels:?}");
    let offered = cx.read(|cx| screen.read(cx).acts(cx));
    assert!(offered.is_empty(), "{offered:?}");
    // Its key still asks, so the shell can say why Scale is not done.
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
        screen.update(cx, |screen, cx| screen.focus(window, cx));
        window.render_frame(cx);
        window.press("s", cx);
    })
    .unwrap();
    cx.run_until_parked();
    assert_eq!(*heard.borrow(), [(Everyday::Scale, owned.name.clone())]);
}

#[gpui_kit::test]
fn the_example_cronjob_offers_suspend_or_resume(cx: &mut TestAppContext) {
    let (_runtime, screen, handle) = mount(cx, Some("homelab"));
    cx.update_window(handle, |_, window, cx| {
        screen.update(cx, |screen, cx| {
            screen.set_kind(kind("cronjobs.batch"), window, cx);
            screen.focus(window, cx);
        });
        window.render_frame(cx);
    })
    .unwrap();
    cx.run_until_parked();
    let items = row_menu(cx, handle, &screen);
    assert!(
        items.contains(&Some("Suspend or resume".into())),
        "{items:?}"
    );
    assert!(!items.contains(&Some("Scale…".into())), "{items:?}");
}

/// (kind, menu labels, keys, toolbar ids, acts) of the GitOps and certificate
/// nudges: what each kind offers, and how.
#[allow(clippy::type_complexity)]
fn nudges() -> [(
    &'static str,
    &'static [&'static str],
    &'static [&'static str],
    &'static [&'static str],
    &'static [Everyday],
); 3] {
    [
        (
            "applications.argoproj.io",
            &["Refresh", "Hard refresh"],
            &["g", "shift-g"],
            &["resource-argo-refresh", "resource-argo-hard-refresh"],
            &[Everyday::ArgoRefresh, Everyday::ArgoHardRefresh],
        ),
        (
            "kustomizations.kustomize.toolkit.fluxcd.io",
            &["Reconcile"],
            &["c"],
            &["resource-flux-reconcile"],
            &[Everyday::FluxReconcile],
        ),
        (
            "certificates.cert-manager.io",
            &["Renew"],
            &["w"],
            &["resource-renew"],
            &[Everyday::Renew],
        ),
    ]
}

#[gpui_kit::test]
fn each_nudge_is_in_the_row_menu_of_its_kind_and_not_of_a_deployment(cx: &mut TestAppContext) {
    for (key, labels, ..) in nudges() {
        let (_runtime, screen, handle, _) = opened(cx, key);
        let items = row_menu(cx, handle, &screen);
        for label in labels {
            assert!(items.contains(&Some((*label).into())), "{key}: {items:?}");
        }
    }
    let (_runtime, screen, handle, _) = deployments(cx);
    let items = row_menu(cx, handle, &screen);
    for (_, labels, ..) in nudges() {
        for label in labels {
            assert!(!items.contains(&Some((*label).into())), "{items:?}");
        }
    }
}

#[gpui_kit::test]
fn the_nudges_keys_the_menu_and_the_toolbar_ask_for_the_selected_object(cx: &mut TestAppContext) {
    for (key, labels, keys, buttons, acts) in nudges() {
        // The keys.
        let (_runtime, screen, handle, heard) = opened(cx, key);
        for pressed in keys {
            cx.update_window(handle, |_, window, cx| {
                window.render_frame(cx);
                window.press(pressed, cx);
            })
            .unwrap();
            cx.run_until_parked();
        }
        let name = cx.read(|cx| identity_at(&screen, 0, cx).name);
        let expected: Vec<_> = acts.iter().map(|act| (*act, name.clone())).collect();
        assert_eq!(*heard.borrow(), expected, "{key} keys");
        // The menu.
        let (_runtime, screen, handle, heard) = opened(cx, key);
        for label in labels {
            let items = row_menu(cx, handle, &screen);
            cx.update_window(handle, |_, window, cx| pick(window, &items, label, cx))
                .unwrap();
            cx.run_until_parked();
        }
        assert_eq!(heard.borrow().len(), labels.len(), "{key} menu");
        // The toolbar.
        let (_runtime, _screen, handle, heard) = opened(cx, key);
        for button in buttons {
            cx.update_window(handle, |_, window, cx| {
                window.render_frame(cx);
                window.click(*button, cx);
            })
            .unwrap();
            cx.run_until_parked();
        }
        assert_eq!(heard.borrow().len(), buttons.len(), "{key} toolbar");
    }
}

#[gpui_kit::test]
fn a_blocked_cluster_disables_every_nudge_and_its_key_does_nothing(cx: &mut TestAppContext) {
    for (key, labels, keys, buttons, _) in nudges() {
        let (_runtime, screen, handle, heard) = opened(cx, key);
        cx.update(|cx| {
            screen.update(cx, |screen, cx| {
                screen.set_write_block(Some("This cluster is read-only."), cx)
            })
        });
        for label in labels {
            let items = row_menu(cx, handle, &screen);
            // Shown, not hidden.
            assert!(items.contains(&Some((*label).into())), "{key}: {items:?}");
            cx.update_window(handle, |_, window, cx| pick(window, &items, label, cx))
                .unwrap();
        }
        for pressed in keys {
            cx.update_window(handle, |_, window, cx| {
                window.render_frame(cx);
                window.press(pressed, cx);
            })
            .unwrap();
        }
        for button in buttons {
            cx.update_window(handle, |_, window, cx| {
                window.render_frame(cx);
                window.click(*button, cx);
            })
            .unwrap();
        }
        cx.run_until_parked();
        assert!(heard.borrow().is_empty(), "{key}");
    }
}

#[gpui_kit::test]
fn the_example_cronjob_offers_run_now_and_a_deployment_does_not(cx: &mut TestAppContext) {
    let (_runtime, screen, handle) = mount(cx, Some("homelab"));
    cx.update_window(handle, |_, window, cx| {
        screen.update(cx, |screen, cx| {
            screen.set_kind(kind("cronjobs.batch"), window, cx);
            screen.focus(window, cx);
        });
        window.render_frame(cx);
    })
    .unwrap();
    cx.run_until_parked();
    let items = row_menu(cx, handle, &screen);
    assert!(items.contains(&Some("Run now".into())), "{items:?}");
    assert!(!items.contains(&Some("Roll back…".into())), "{items:?}");
}

#[gpui_kit::test]
fn b_and_j_ask_only_for_the_kinds_that_have_the_action(cx: &mut TestAppContext) {
    let (_runtime, _screen, handle, heard) = deployments(cx);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.press("j", cx);
        window.press("b", cx);
    })
    .unwrap();
    cx.run_until_parked();
    let name = cx.read(|cx| identity_at(&_screen, 0, cx).name);
    // A Deployment has no Run now; its `j` does nothing.
    assert_eq!(*heard.borrow(), [(Everyday::Rollback, name)]);

    let (_runtime, screen, handle) = mount(cx, Some("homelab"));
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
            screen.set_kind(kind("cronjobs.batch"), window, cx);
            screen.focus(window, cx);
        });
        window.render_frame(cx);
        screen.update(cx, |screen, cx| {
            screen.projection.select(&screen.store, Some(0));
            cx.notify();
        });
        window.render_frame(cx);
        window.press("b", cx);
        window.press("j", cx);
    })
    .unwrap();
    cx.run_until_parked();
    let name = cx.read(|cx| identity_at(&screen, 0, cx).name);
    assert_eq!(*heard.borrow(), [(Everyday::RunNow, name)]);
}

#[gpui_kit::test]
fn a_blocked_cluster_asks_for_neither_roll_back_nor_run_now(cx: &mut TestAppContext) {
    let (_runtime, screen, handle, heard) = deployments(cx);
    cx.update(|cx| {
        screen.update(cx, |screen, cx| {
            screen.set_write_block(Some("This cluster is read-only."), cx)
        })
    });
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.press("b", cx);
        window.press("j", cx);
    })
    .unwrap();
    cx.run_until_parked();
    assert!(heard.borrow().is_empty());
}
