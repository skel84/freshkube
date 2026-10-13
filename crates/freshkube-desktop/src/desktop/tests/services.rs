use super::*;

#[gpui_kit::test]
fn context_switch_invalidates_service_target(cx: &mut TestAppContext) {
    let (_runtime, handle, view) = fixture(cx, 1280., 820.);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        open_node_tab(window, cx, super::super::nodes::NodeTab::Services);
        window.render_frame(cx);
        window.within("services-region").click("kubelet", cx);
    })
    .unwrap();
    cx.run_until_parked();
    let old_epoch = cx.update(|cx| {
        assert_eq!(
            view.read(cx).selected_service(cx).as_deref(),
            Some("kubelet")
        );
        view.read(cx).epoch
    });
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        contexts(window, cx);
        window.click(("context", 1usize), cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let pilot = view.read(cx);
        assert!(pilot.epoch > old_epoch);
        assert_eq!(pilot.applied.context.as_deref(), Some("staging-eu"));
        assert_eq!(pilot.selected_node.as_deref(), Some("stg-cp-01"));
        assert!(pilot.selected_service(cx).is_none());
        assert!(
            window
                .find("applied-config")
                .label()
                .unwrap()
                .contains("staging-eu")
        );
        contexts(window, cx);
        assert_eq!(window.find(("context", 1usize)).selected(), Some(true));
        // Nothing is selected, so there is no logs action to trigger.
        assert!(window.try_find("service-logs").is_none());
        assert_eq!(view.read(cx).page, Page::Nodes);
    })
    .unwrap();
}

#[gpui_kit::test]
fn service_keyboard_selection_filter_retains_domain_id(cx: &mut TestAppContext) {
    let (_runtime, handle, view) = fixture(cx, 1280., 820.);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        open_node_tab(window, cx, super::super::nodes::NodeTab::Services);
        window.render_frame(cx);
        window.within("services-region").click("apid", cx);
        window.press("down", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(
            view.read(cx).selected_service(cx).as_deref(),
            Some("auditd")
        );
        assert!(
            window
                .find("selected-service")
                .label()
                .unwrap()
                .contains("Started task auditd")
        );
        // The selection scrolled the stacked details into view; the
        // filter is back at the page's top.
        let scroll = view.read(cx).service_scroll(cx);
        scroll.set_offset(gpui_kit::point(px(0.), px(0.)));
        window.render_frame(cx);
        window.click("service-filter", cx);
        window.input("apid", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(
            view.read(cx).selected_service(cx).as_deref(),
            Some("auditd")
        );
        assert!(
            window
                .within("services-region")
                .try_find("auditd")
                .is_none()
        );
        assert!(window.within("services-region").find("apid").visible());
        window.press("secondary-a", cx);
        window.press("backspace", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(
            window.within("services-region").find("auditd").selected(),
            Some(true)
        );
        window.click("refresh-services", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(
            view.read(cx).selected_service(cx).as_deref(),
            Some("auditd")
        );
        assert_eq!(
            window.within("services-region").find("auditd").selected(),
            Some(true)
        );
    })
    .unwrap();
}

/// The arrows step through the services the filter shows, not the node's
/// whole list: with the selection filtered out, both land on the one row
/// left.
#[gpui_kit::test]
fn service_arrows_step_through_the_filtered_rows(cx: &mut TestAppContext) {
    let (_runtime, handle, view) = fixture(cx, 1280., 820.);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        open_node_tab(window, cx, super::super::nodes::NodeTab::Services);
        window.render_frame(cx);
        window.click("service-filter", cx);
        window.input("apid", cx);
    })
    .unwrap();
    cx.run_until_parked();
    for key in ["down", "up"] {
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            // A click puts the keyboard on the list; the selection is then
            // a service the filter hides.
            window.within("services-region").click("apid", cx);
            view.update(cx, |pilot, cx| {
                pilot.select_service(Some("auditd".into()), cx)
            });
            window.press(key, cx);
            assert_eq!(
                view.read(cx).selected_service(cx).as_deref(),
                Some("apid"),
                "{key}"
            );
        })
        .unwrap();
    }
}

/// Stacked under the list, a newly selected service's details scroll into
/// view, by pointer and by keyboard; beside the list, nothing scrolls, and
/// a refresh never reveals again (#238).
#[gpui_kit::test]
fn a_selected_service_brings_its_stacked_details_into_view(cx: &mut TestAppContext) {
    // The node's inspector stacks them beside the table and in a narrow
    // window; expanded in a wide one, it sets them side by side, and a
    // short window lets that page scroll too.
    for (case, width, height, expanded, stacked) in [
        ("beside the table", 1280., 820., false, true),
        ("narrow window", 760., 820., false, true),
        ("expanded", 1280., 560., true, false),
    ] {
        let (_runtime, handle, view) = fixture(cx, width, height);
        let scroll = cx
            .update_window(handle, |_, window, cx| {
                window.render_frame(cx);
                open_node_tab(window, cx, super::super::nodes::NodeTab::Services);
                if expanded {
                    view.update(cx, |view, cx| view.toggle_node_expanded(cx));
                }
                let scroll = view.read(cx).service_scroll(cx);
                // Draws until no view asks for another frame, so the count
                // after an action is the reveal's alone.
                let settle = |window: &mut gpui_kit::Window, cx: &mut gpui_kit::App| {
                    for _ in 0..10 {
                        window.render_frame(cx);
                        if window.simulate_next_frame(cx) == 0 {
                            return;
                        }
                    }
                    panic!("{case}: frames never settled");
                };
                let check = |window: &mut gpui_kit::Window,
                             cx: &mut gpui_kit::App,
                             step: &str,
                             before: gpui_kit::Pixels| {
                    // The frame that drew the selection laid the details out and
                    // moved the page; it asks for the next frame, which draws
                    // them there.
                    let asked = window.simulate_next_frame(cx);
                    assert_eq!(
                        asked > 0,
                        stacked,
                        "{case}, {step}: {asked} frames asked for"
                    );
                    window.render_frame(cx);
                    let page = window.find("services-page").bounds();
                    let details = window.find("selected-service").bounds();
                    let list = window.find("services-region").bounds();
                    assert_eq!(
                        details.top() >= list.bottom(),
                        stacked,
                        "{case}: the details {details:?} and the list {list:?}"
                    );
                    if !stacked {
                        assert_eq!(scroll.offset().y, before, "{case}, {step}: scrolled");
                        return;
                    }
                    assert!(
                        details.top() >= page.top() - px(0.5),
                        "{case}, {step}: the details' top {details:?} is above {page:?}"
                    );
                    for id in ["service-logs", "service-restart"] {
                        let action = window.find(id).bounds();
                        assert!(
                            action.bottom() <= page.bottom() + px(0.5),
                            "{case}, {step}: {id} at {action:?} is below {page:?}"
                        );
                    }
                };
                settle(window, cx);
                if !stacked {
                    // Side by side, a page already scrolled stays where it is.
                    let max = scroll.max_offset().y;
                    assert!(max > px(0.), "{case}: the page doesn't scroll");
                    scroll.set_offset(gpui_kit::point(px(0.), -max.min(px(40.))));
                    settle(window, cx);
                }
                let before = scroll.offset().y;
                window.within("services-region").click("apid", cx);
                check(window, cx, "pointer", before);
                // Stacked, back to the top first; then a key moves the selection.
                if stacked {
                    scroll.set_offset(gpui_kit::point(px(0.), px(0.)));
                }
                settle(window, cx);
                let before = scroll.offset().y;
                window.press("down", cx);
                check(window, cx, "keyboard", before);
                assert_eq!(
                    view.read(cx).selected_service(cx).as_deref(),
                    Some("auditd")
                );
                assert!(view.read(cx).service_focused(window, cx));
                // A refresh keeps the selection and reveals nothing.
                scroll.set_offset(gpui_kit::point(px(0.), px(0.)));
                settle(window, cx);
                window.click("refresh-services", cx);
                assert_eq!(window.simulate_next_frame(cx), 0, "{case}: refresh");
                scroll
            })
            .unwrap();
        cx.run_until_parked();
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            window.simulate_next_frame(cx);
            window.render_frame(cx);
            assert_eq!(
                view.read(cx).selected_service(cx).as_deref(),
                Some("auditd")
            );
            assert_eq!(scroll.offset().y, px(0.), "{case}: the refresh revealed");
        })
        .unwrap();
    }
}

#[gpui_kit::test]
fn health_filter_narrows_the_list_and_shows_the_unhealthy_message(cx: &mut TestAppContext) {
    let (_runtime, handle, view) = fixture(cx, 1280., 820.);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        pick_target(window, cx, 4);
        assert_eq!(view.read(cx).selected_node.as_deref(), Some(DEGRADED_NODE));
        open_node_tab(window, cx, super::super::nodes::NodeTab::Services);
        window.render_frame(cx);
        window.click("health-unhealthy", cx);
        window.render_frame(cx);
        let region = window.within("services-region");
        assert!(region.find("kubelet").visible());
        assert!(region.try_find("apid").is_none());
        window.within("services-region").click("kubelet", cx);
        window.render_frame(cx);
        assert!(
            window
                .find("selected-service")
                .label()
                .unwrap()
                .contains("connection refused")
        );
        window.click("health-all", cx);
        window.render_frame(cx);
        assert!(window.within("services-region").find("apid").visible());
        assert_eq!(
            view.read(cx).selected_service(cx).as_deref(),
            Some("kubelet")
        );
    })
    .unwrap();
}

#[gpui_kit::test]
fn selected_service_opens_matching_log_collection(cx: &mut TestAppContext) {
    // Opening logs starts the synthetic producer, which wakes GPUI from a
    // Tokio thread; let the scheduler park for those external wakes.
    cx.executor().allow_parking();
    let (_runtime, handle, view) = fixture(cx, 1280., 820.);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        open_services_with_room(window, cx);
        window.render_frame(cx);
        window.within("services-region").click("containerd", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("service-logs", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(view.read(cx).page, Page::Nodes);
        assert!(window.find("logs-viewport").visible());
        assert_eq!(
            window
                .find(SharedString::from("collect-containerd"))
                .checked(),
            Some(true)
        );
        assert_eq!(
            window.find(SharedString::from("collect-apid")).checked(),
            Some(false)
        );
    })
    .unwrap();
}

#[gpui_kit::test]
fn embedded_services_start_with_their_toolbar(cx: &mut TestAppContext) {
    let (_runtime, handle, _view) = fixture(cx, 1280., 820.);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        // Expanded, so the toolbar keeps one row.
        open_services_with_room(window, cx);
        window.render_frame(cx);
        // The tab names the screen and the heading the node, so no title
        // or node line sits above the toolbar: its controls are the first
        // row, one inset under the tab strip.
        let content = window.find("node-inspector-content").bounds();
        let inset = crate::ui::dp_px(freshkube_ui::page::PANE_PADDING, window);
        for id in ["service-filter", "health-all", "refresh-services"] {
            let control = window.find(id).bounds();
            let row = (control.top() - content.top()) - inset;
            assert!(
                row >= px(0.) && row <= crate::ui::dp_px(4., window),
                "{id} sits {row:?} below the inset"
            );
        }
    })
    .unwrap();
}

#[gpui_kit::test]
fn log_collection_keeps_running_on_other_screens(cx: &mut TestAppContext) {
    let (_runtime, handle, view) = fixture(cx, 1280., 820.);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        open_node_tab(window, cx, super::super::nodes::NodeTab::Logs);
        window.render_frame(cx);
        window.click("logs-collection", cx);
        window.render_frame(cx);
        assert!(view.read(cx).logs.read(cx).is_collecting());
        window.click("nav-overview", cx);
        window.render_frame(cx);
        assert_eq!(view.read(cx).page, Page::Overview);
        assert!(view.read(cx).logs.read(cx).is_collecting());
        assert!(view.read(cx).logs.read(cx).collecting_count() > 0);
        open_node_tab(window, cx, super::super::nodes::NodeTab::Logs);
        window.render_frame(cx);
        assert_eq!(
            window.find("logs-collection").label(),
            Some("Stop collecting")
        );
        assert!(
            window
                .find("logs-status")
                .label()
                .unwrap()
                .starts_with("Collecting")
        );
    })
    .unwrap();
}

#[gpui_kit::test]
fn cluster_services_route_actions_to_the_retained_node_pane(cx: &mut TestAppContext) {
    // Logs starts the synthetic producer, which wakes GPUI from a Tokio
    // thread; let the scheduler park for those external wakes.
    cx.executor().allow_parking();
    let (_runtime, handle, view) = fixture(cx, 1280., 820.);
    let row = "system-service-talos-wk-fra1-02-kubelet";
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.press("secondary-7", cx);
        window.render_frame(cx);
        assert_eq!(view.read(cx).page, Page::SystemServices);
        assert!(window.find(row).visible());
        assert!(window.try_find("restart-service").is_none());
        window.click(row, cx);
        window.click("system-service-open", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(view.read(cx).page, Page::Nodes);
        assert_eq!(view.read(cx).selected_node.as_deref(), Some(DEGRADED_NODE));
        assert_eq!(
            view.read(cx).selected_service(cx).as_deref(),
            Some("kubelet")
        );
        assert_eq!(
            view.read(cx).node_workspace.read(cx).tab,
            super::super::nodes::NodeTab::Services
        );
        // Back on the page, the row is still selected and the list has
        // the keyboard, so L opens its logs.
        window.press("secondary-7", cx);
        window.render_frame(cx);
        window.press("l", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(
            view.read(cx).node_workspace.read(cx).tab,
            super::super::nodes::NodeTab::Logs
        );
        assert_eq!(
            window.find("logs-collection").label(),
            Some("Stop collecting")
        );
        assert!(window.find("logs-status").visible());
    })
    .unwrap();
}

#[gpui_kit::test]
async fn a_talosconfig_that_fails_to_load_ends_the_services_wait(cx: &mut TestAppContext) {
    cx.executor().allow_parking();
    // Never the user's default talosconfig: a missing file.
    let options = GpuiOptions::new(Some(scratch("no-such-talosconfig-services")), None, 100);
    let (_runtime, handle, view) = mount(cx, options, 1280., 820.);
    cx.wait_for(handle, std::time::Duration::from_secs(2), |_, cx| {
        view.read(cx).config_error.is_some()
    })
    .await;
    cx.update(|cx| {
        let pilot = view.read(cx);
        // Neither page waits on an overview that will never be asked.
        let reading = pilot.system_services.read(cx).reading().clone();
        assert!(
            matches!(reading, super::super::system_services::Reading::Failed(_)),
            "{reading:?}"
        );
        assert!(!pilot.nodes_wait(cx));
    });
}

/// A service selected from elsewhere after a failed read that kept nothing
/// waits through the next read, and stays when the services answer with
/// it, as the shell kept it before the tab owned its selection.
#[gpui_kit::test]
fn a_selection_waits_for_the_read_after_a_failure(cx: &mut TestAppContext) {
    let (_runtime, handle, view) = fixture(cx, 1280., 820.);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        open_node_tab(window, cx, super::super::nodes::NodeTab::Services);
    })
    .unwrap();
    cx.run_until_parked();
    let (target, services) = cx.update(|cx| {
        let pilot = view.read(cx);
        let target = crate::backend::Target {
            epoch: pilot.epoch,
            context: pilot.applied.context.clone().unwrap_or_default(),
            node: pilot.selected_node.clone().unwrap_or_default(),
            address: "fixture".into(),
        };
        (target, pilot.services.data().cloned().unwrap())
    });
    assert!(services.iter().any(|service| service.id == "kubelet"));
    let selected = |cx: &mut TestAppContext| cx.update(|cx| view.read(cx).selected_service(cx));
    // The read fails with nothing kept, and a link selects kubelet.
    cx.update(|cx| {
        view.update(cx, |pilot, cx| {
            pilot.services = crate::state::Snapshot::default();
            let request = pilot.services.begin(target.clone());
            pilot.services.apply(&request, Err("offline".into()));
            pilot.push_services(cx);
            pilot.select_service(Some("kubelet".into()), cx);
        })
    });
    assert_eq!(selected(cx).as_deref(), Some("kubelet"));
    // The next read starts empty: the selection waits.
    let request = cx.update(|cx| {
        view.update(cx, |pilot, cx| {
            let request = pilot.services.begin(target.clone());
            pilot.push_services(cx);
            request
        })
    });
    assert_eq!(selected(cx).as_deref(), Some("kubelet"));
    // It answers with kubelet: the selection stays.
    cx.update(|cx| {
        view.update(cx, |pilot, cx| {
            pilot.services.apply(&request, Ok(services.clone()));
            pilot.push_services(cx);
        })
    });
    assert_eq!(selected(cx).as_deref(), Some("kubelet"));
    // An answer without it drops it.
    cx.update(|cx| {
        view.update(cx, |pilot, cx| {
            let request = pilot.services.begin(target.clone());
            pilot.services.apply(&request, Ok(Vec::new()));
            pilot.push_services(cx);
        })
    });
    assert_eq!(selected(cx), None);
}
