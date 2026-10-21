//! The fixture uses an in-memory Results transport, without any API or socket.
use super::*;
use freshkube_core::resources::ForwardEnd;
use freshkube_resources::forwards::{self, State};
use gpui_kit::InputEvent as _;

fn open_changes(window: &mut Window, cx: &mut App) {
    window.render_frame(cx);
    window.click("nav-applications", cx);
    window.render_frame(cx);
    window.click("nav-changes", cx);
    window.render_frame(cx);
}

fn owner(view: &Entity<Pilot>, cx: &mut App) -> Entity<forwards::ForwardView> {
    let source = view.read(cx).source().kube.unwrap().id;
    forwards::list(cx)
        .read(cx)
        .find(&source, "acme", "results", 8443, cx)
        .unwrap()
}
fn assert_example_starting(view: &Entity<Pilot>, cx: &mut App) {
    assert_eq!(owner(view, cx).read(cx).snapshot().state, State::Starting);
    assert_eq!(forwards::running(cx), 1);
}
fn assert_example_listening(view: &Entity<Pilot>, cx: &mut App) {
    let owner = owner(view, cx);
    assert!(matches!(
        owner.read(cx).snapshot().state,
        State::Listening(_)
    ));
    assert_eq!(forwards::running(cx), 1);
}
fn listen(cx: &mut TestAppContext) {
    cx.executor().advance_clock(Duration::from_secs(1));
    cx.run_until_parked();
}

#[gpui_kit::test]
fn fixture_changes_switches_without_starting_then_forwards_only_on_press(cx: &mut TestAppContext) {
    let (_runtime, window, view) = fixture(cx, 1280., 880.);
    cx.update_window(window, |_, window, cx| {
        open_changes(window, cx);
        assert_eq!(
            window.find("changes-forward-results").label(),
            Some("Switch to cicd-fra")
        );
        assert!(
            forwards::list(cx)
                .read(cx)
                .find(
                    &view.read(cx).source().kube.unwrap().id,
                    "acme",
                    "results",
                    8443,
                    cx
                )
                .is_none()
        );
        window.click("changes-forward-results", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.simulate_prompt_answer("Switch");
    cx.run_until_parked();
    cx.update_window(window, |_, window, cx| {
        open_changes(window, cx);
        assert_eq!(
            window.find("changes-forward-results").label(),
            Some("Forward")
        );
        let source = view.read(cx).source().kube.unwrap().id;
        assert!(
            forwards::list(cx)
                .read(cx)
                .find(&source, "acme", "results", 8443, cx)
                .is_none()
        );
        window.click("changes-refresh", cx);
        window.render_frame(cx);
        assert!(
            forwards::list(cx)
                .read(cx)
                .find(&source, "acme", "results", 8443, cx)
                .is_none()
        );
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(window, |_, window, cx| {
        window.render_frame(cx);
        window.click("changes-forward-results", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update(|cx| assert_example_starting(&view, cx));
    listen(cx);
    cx.update_window(window, |_, window, cx| {
        window.render_frame(cx);
        assert_example_listening(&view, cx);
        let changes = page(&view, cx).read(cx).changes_page().clone();
        assert!(
            page(&view, cx)
                .read(cx)
                .status(cx)
                .text(None)
                .contains("forward is listening")
        );
        assert!(!changes.read(cx).lines_text().is_empty());
    })
    .unwrap();
}

fn inspector_forward(
    cx: &mut TestAppContext,
    from_changes: bool,
    width: f32,
    height: f32,
    text: f32,
) {
    let (_runtime, window, view) = fixture(cx, width, height);
    cx.update(|cx| crate::text_size::set(text, cx));
    cx.update_window(window, |_, window, cx| {
        view.update(cx, |view, cx| {
            view.switch_cluster("cicd-fra".into(), window, cx)
        });
        window.render_frame(cx);
        window.click("nav-applications", cx);
        window.render_frame(cx);
        if width < 900. {
            window.press("secondary-b", cx);
            window.render_frame(cx);
        }
        if from_changes {
            window.click("nav-changes", cx);
            window.render_frame(cx);
            let changes = page(&view, cx).read(cx).changes_page().clone();
            changes.update(cx, |page, cx| {
                page.select("checkout/wonky-otter", window, cx)
            });
            window.render_frame(cx);
            window.press("enter", cx);
        } else {
            window.click("application-kargo:checkout", cx);
            window.press("enter", cx);
            window.render_frame(cx);
            window.click("application-part-cicd-fra/0/checkout/prod-ams", cx);
            window.press("f", cx);
        }
    })
    .unwrap();
    cx.run_until_parked();
    let mut selected = None;
    cx.update_window(window, |_, window, cx| {
        let app = page(&view, cx);
        let change = if from_changes {
            app.read(cx).changes_page().read(cx).change_page().unwrap().clone()
        } else {
            app.read(cx).change_page().unwrap().clone()
        };
        let key = change.read(cx).change.hops.iter().find(|hop| matches!(&hop.shows, freshkube_delivery::change::Shows::Hop(detail) if detail.kind == "PipelineRun" || detail.kind == "Builds")).unwrap().key.clone();
        window.render_frame(cx);
        if height < 600. {
            change.update(cx, |page, cx| page.select(&key, window, cx));
        } else {
            window.click(gpui_kit::SharedString::from(format!("change-hop-{key}")), cx);
        }
        selected = Some(key);
    }).unwrap();
    cx.run_until_parked();
    cx.update_window(window, |_, window, cx| {
        window.render_frame(cx);
        for _ in 0..4 {
            if window.simulate_next_frame(cx) == 0 {
                break;
            }
        }
        window.render_frame(cx);
        assert_eq!(
            window.find("change-forward-results").label(),
            Some("Forward")
        );
        if height < 600. {
            // The page inset is outside the graph and notice's own scrollers.
            let bounds = window.find("change-page").bounds();
            let position = gpui_kit::point(bounds.left() + px(1.), bounds.top() + px(10.));
            window.dispatch_event(
                gpui_kit::MouseMoveEvent {
                    position,
                    pressed_button: None,
                    modifiers: Default::default(),
                }
                .to_platform_input(),
                cx,
            );
            window.dispatch_event(
                gpui_kit::ScrollWheelEvent {
                    position,
                    delta: gpui_kit::ScrollDelta::Pixels(gpui_kit::point(px(0.), px(-2000.))),
                    ..Default::default()
                }
                .to_platform_input(),
                cx,
            );
            window.render_frame(cx);
            let action = window.find("change-forward-results").bounds();
            let page_bounds = window.find("change-page").bounds();
            assert!(
                action.top() >= page_bounds.top() && action.bottom() <= page_bounds.bottom(),
                "action {action:?}, page {page_bounds:?}"
            );
        }
        let notice = window.find("change-detail-notice");
        assert!(notice.label().unwrap().contains("Forward"));
        assert!(!notice.label().unwrap().contains("isn't installed"));
        let source = view.read(cx).source().kube.unwrap().id;
        assert!(
            forwards::list(cx)
                .read(cx)
                .find(&source, "acme", "results", 8443, cx)
                .is_none()
        );
        window.click("change-forward-results", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update(|cx| assert_example_starting(&view, cx));
    listen(cx);
    cx.update_window(window, |_, window, cx| {
        window.render_frame(cx);
        assert_example_listening(&view, cx);
        let app = page(&view, cx);
        let change = if from_changes {
            app.read(cx).changes_page().read(cx).change_page().unwrap()
        } else {
            app.read(cx).change_page().unwrap()
        };
        assert_eq!(
            change.read(cx).selected().map(|key| key.as_ref()),
            selected.as_deref()
        );
        assert!(window.try_find("change-detail").is_some());
        assert_eq!(
            window.find("change-forward-results").label(),
            Some("Forward")
        );
    })
    .unwrap();
}

#[gpui_kit::test]
fn fixture_change_from_changes_has_an_example_only_forward(cx: &mut TestAppContext) {
    inspector_forward(cx, true, 1280., 880., 13.);
}

#[gpui_kit::test]
fn fixture_change_from_application_has_an_example_only_forward(cx: &mut TestAppContext) {
    inspector_forward(cx, false, 1280., 880., 13.);
}

#[gpui_kit::test]
fn results_status_uses_available_room_at_default_text_size(cx: &mut TestAppContext) {
    let (_runtime, window, view) = fixture(cx, 1280., 880.);
    cx.update_window(window, |_, window, cx| {
        view.update(cx, |view, cx| {
            view.switch_cluster("cicd-fra".into(), window, cx)
        });
        open_changes(window, cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(window, |_, window, cx| {
        window.render_frame(cx);
        let shown = freshkube_ui::status::shown("applications-scope").unwrap();
        assert!(
            shown.contains("Tekton Results isn't forwarded yet"),
            "{shown}"
        );
    })
    .unwrap();
}

#[gpui_kit::test]
fn fixture_stacked_change_keeps_forward_reachable_at_text_twenty(cx: &mut TestAppContext) {
    inspector_forward(cx, true, 760., 560., 20.);
}

#[gpui_kit::test]
fn fixture_forward_listens_disables_duplicate_start_and_stops_from_ports(cx: &mut TestAppContext) {
    let (_runtime, window, view) = fixture(cx, 1280., 880.);
    cx.update_window(window, |_, window, cx| {
        view.update(cx, |view, cx| {
            view.switch_cluster("cicd-fra".into(), window, cx)
        });
        open_changes(window, cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(window, |_, window, cx| {
        window.render_frame(cx);
        window.click("changes-forward-results", cx);
        assert_example_starting(&view, cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(window, |_, window, cx| {
        window.render_frame(cx);
        let shown = freshkube_ui::status::shown("applications-scope").unwrap();
        assert!(shown.contains("starting"), "{shown}");
    })
    .unwrap();
    listen(cx);
    cx.update_window(window, |_, window, cx| {
        window.render_frame(cx);
        assert_example_listening(&view, cx);
        let epoch = owner(&view, cx).read(cx).snapshot().epoch;
        window.click("changes-forward-results", cx);
        assert_eq!(
            owner(&view, cx).read(cx).snapshot().epoch,
            epoch,
            "disabled Forward starts nothing"
        );
        window.click("forwards", cx);
        window.render_frame(cx);
        window.click(
            gpui_kit::SharedString::from(format!("forward-{}-stop", epoch.id)),
            cx,
        );
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(window, |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(
            owner(&view, cx).read(cx).snapshot().state,
            State::Ended(ForwardEnd::Stopped)
        );
        assert_eq!(forwards::running(cx), 0);
        assert!(
            page(&view, cx)
                .read(cx)
                .status(cx)
                .text(None)
                .contains("ended: stopped")
        );
    })
    .unwrap();
}
