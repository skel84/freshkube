//! The shipped fixture workspace exposes Forward without any API or socket.
use super::*;
use freshkube_core::resources::{ForwardEnd, ForwardFailureKind};
use freshkube_resources::forwards::{self, State};

fn open_changes(window: &mut Window, cx: &mut App) {
    window.render_frame(cx);
    window.click("nav-applications", cx);
    window.render_frame(cx);
    window.click("nav-changes", cx);
    window.render_frame(cx);
}

fn assert_example_failure(view: &Entity<Pilot>, cx: &mut App) {
    let source = view.read(cx).source().kube.unwrap().id;
    let list = forwards::list(cx);
    let owner = list
        .read(cx)
        .find(&source, "acme", "results", 8443, cx)
        .unwrap_or_else(|| {
            panic!(
                "no owner for {source}; status {}",
                page(view, cx).read(cx).status(cx).text(None)
            )
        });
    let snapshot = owner.read(cx).snapshot();
    let State::Ended(ForwardEnd::Failed(failure)) = snapshot.state else {
        panic!("fixture must finish through the example resolver: {snapshot:?}");
    };
    assert_eq!(failure.kind, ForwardFailureKind::NoPods);
    assert_eq!(failure.message, "The Service selects no pods");
    assert_eq!(
        owner.read(cx).local_port,
        None,
        "failure precedes socket binding"
    );
    assert_eq!(forwards::running(cx), 0);
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
    cx.update_window(window, |_, window, cx| {
        window.render_frame(cx);
        assert_example_failure(&view, cx);
        let changes = page(&view, cx).read(cx).changes_page().clone();
        assert!(
            page(&view, cx)
                .read(cx)
                .status(cx)
                .text(None)
                .contains("The Service selects no pods")
        );
        assert!(!changes.read(cx).lines_text().is_empty());
    })
    .unwrap();
}

fn inspector_forward(cx: &mut TestAppContext, from_changes: bool) {
    let (_runtime, window, view) = fixture(cx, 1280., 880.);
    cx.update_window(window, |_, window, cx| {
        view.update(cx, |view, cx| {
            view.switch_cluster("cicd-fra".into(), window, cx)
        });
        window.render_frame(cx);
        window.click("nav-applications", cx);
        window.render_frame(cx);
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
        window.click(gpui_kit::SharedString::from(format!("change-hop-{key}")), cx);
        window.render_frame(cx);
        assert_eq!(window.find("change-forward-results").label(), Some("Forward"));
        selected = Some(key);
        let source = view.read(cx).source().kube.unwrap().id;
        assert!(forwards::list(cx).read(cx).find(&source, "acme", "results", 8443, cx).is_none());
        window.click("change-forward-results", cx);
    }).unwrap();
    cx.run_until_parked();
    cx.update_window(window, |_, window, cx| {
        window.render_frame(cx);
        assert_example_failure(&view, cx);
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
    inspector_forward(cx, true);
}

#[gpui_kit::test]
fn fixture_change_from_application_has_an_example_only_forward(cx: &mut TestAppContext) {
    inspector_forward(cx, false);
}
