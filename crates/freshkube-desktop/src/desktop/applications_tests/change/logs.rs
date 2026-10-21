use super::*;

/// The dock's log tabs, by title.
fn dock_tabs(view: &Entity<Pilot>, cx: &mut TestAppContext) -> Vec<String> {
    cx.update(|cx| {
        view.read(cx)
            .pages
            .dock
            .read(cx)
            .tabs
            .iter()
            .map(|tab| tab.title.to_string())
            .collect()
    })
}

/// The release run's policy check ran on the builds' cluster, `cicd-fra`;
/// with it open, its Logs opens the step pod's log in the dock, every step
/// at once, falling back to the container core named.
#[gpui_kit::test]
fn a_steps_logs_open_in_the_dock_on_the_open_cluster(cx: &mut TestAppContext) {
    use crate::applications::testing::change::ChangeEvent;
    use crate::resources::{LogsAt, ResourceLink};
    use freshkube_delivery::testing::{RELEASE_NAMESPACE, VERIFY_TASK};
    use std::{cell::RefCell, rc::Rc};

    let (_runtime, handle, view) = open_on(cx, 1280., 880., "cicd-fra");
    let asked: Rc<RefCell<Vec<ResourceLink>>> = Rc::default();
    let page = cx.update(|cx| change(&view, cx).unwrap());
    let _subscription = cx.update(|cx| {
        let asked = asked.clone();
        cx.subscribe(&page, move |_, event: &ChangeEvent, _| {
            if let ChangeEvent::Open(link) = event {
                asked.borrow_mut().push((**link).clone());
            }
        })
    });
    cx.update_window(handle, |_, window, cx| {
        window.click(hop("policy"), cx);
        window.render_frame(cx);
        let logs = action(&view, "Logs of verify · cicd-fra", cx);
        window.click(logs, cx);
        window.render_frame(cx);
    })
    .unwrap();
    cx.run_until_parked();

    let asked = asked.borrow();
    let [ResourceLink::Logs(request)] = asked.as_slice() else {
        panic!("{asked:?}");
    };
    let identity = &request.target.identity;
    assert_eq!(
        (
            identity.resource.as_str(),
            identity.namespace.as_str(),
            identity.name.clone()
        ),
        ("pods", RELEASE_NAMESPACE, format!("{VERIFY_TASK}-pod"))
    );
    assert_eq!(
        request.at,
        Some(LogsAt {
            container: "step-validate".into(),
            previous: false,
            all: true,
        })
    );
    assert_eq!(dock_tabs(&view, cx), [format!("Pod {VERIFY_TASK}-pod")]);
}

/// The push run's steps ran on the CI cluster, which the example doesn't
/// open: its Logs is greyed out and opens nothing.
#[gpui_kit::test]
fn a_steps_logs_on_a_cluster_that_is_not_open_do_not_open(cx: &mut TestAppContext) {
    let (_runtime, handle, view) = open(cx, 1280., 880.);
    cx.update_window(handle, |_, window, cx| {
        window.click(hop("push-run"), cx);
        window.render_frame(cx);
        let logs = action(&view, "Logs of build · cicd-fra", cx);
        window.click(logs, cx);
        window.render_frame(cx);
    })
    .unwrap();
    cx.run_until_parked();
    assert!(dock_tabs(&view, cx).is_empty());
}
