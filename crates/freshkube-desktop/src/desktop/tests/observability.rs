use super::*;

#[gpui_kit::test]
fn coroot_links_resolve_real_subjects_and_reject_old_providers(cx: &mut TestAppContext) {
    let (_runtime, handle, view) = fixture(cx, 1280., 880.);
    let (pod, access) = cx.read(|cx| {
        let view = view.read(cx);
        let source = view.source().kube.unwrap();
        let pod = crate::resources::example::read(
            &source.context,
            "pods",
            None,
            crate::resources::live::now(),
        )
        .unwrap()
        .1
        .into_iter()
        .find(|row| row.cells[2] == "Running")
        .unwrap()
        .identity;
        (pod, source.id)
    });
    let mut old = None;
    cx.update_window(handle, |_, window, cx| {
        view.update(cx, |view, cx| {
            view.navigate(Page::Observability, window, cx);
            old = Some(view.pages.observability.update(cx, |page, cx| {
                page.fixture_link(access.clone(), &pod.namespace, &pod.name, cx)
            }));
        });
        window.render_frame(cx);
        window.click("obs-open-object", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        assert_eq!(view.read(cx).navigation.page, Page::Resources);
        assert_eq!(
            view.read(cx).pages.resources.read(cx).detail_identity(cx),
            Some(&pod)
        );
        view.update(cx, |view, cx| {
            view.navigate(Page::Observability, window, cx);
            view.pages.observability.update(cx, |page, cx| {
                page.fixture_link(access.clone(), &pod.namespace, &pod.name, cx);
                cx.emit(old.take().unwrap());
            });
        });
    })
    .unwrap();
    cx.run_until_parked();
    cx.update(|cx| assert_eq!(view.read(cx).navigation.page, Page::Observability));
}

#[gpui_kit::test]
fn coroot_links_open_without_asking_while_a_shell_runs(cx: &mut TestAppContext) {
    use crate::resources::{example, live, shell};
    let (_runtime, handle, view) = fixture(cx, 1280., 880.);
    let (pods, access) = cx.read(|cx| {
        let source = view.read(cx).source().kube.unwrap();
        let pods: Vec<_> = example::read(&source.context, "pods", None, live::now())
            .unwrap()
            .1
            .into_iter()
            .filter(|row| row.cells[2] == "Running")
            .take(2)
            .map(|row| row.identity)
            .collect();
        (pods, source.id)
    });
    let step = |cx: &mut TestAppContext,
                act: &dyn Fn(&mut gpui_kit::Window, &mut gpui_kit::App)| {
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            act(window, cx);
        })
        .unwrap();
        cx.run_until_parked();
    };
    start_shell(handle, &view, &pods[0], cx);
    let running = vec![SharedString::from(pods[0].name.clone())];
    assert_eq!(cx.read(shell::running_anywhere), running);
    step(cx, &|window, cx| {
        view.update(cx, |view, cx| {
            view.navigate(Page::Observability, window, cx);
            view.pages.observability.update(cx, |page, cx| {
                page.fixture_link(access.clone(), &pods[1].namespace, &pods[1].name, cx);
            });
        })
    });
    // The shell lives in the dock, so following a link leaves it be.
    step(cx, &|window, cx| window.click("obs-open-object", cx));
    assert!(!cx.has_pending_prompt());
    cx.read(|cx| {
        assert_eq!(shell::running_anywhere(cx), running);
        assert_eq!(view.read(cx).navigation.page, Page::Resources);
        assert_eq!(
            view.read(cx).pages.resources.read(cx).detail_identity(cx),
            Some(&pods[1])
        );
    });
}

/// Through the shell, at 760 by 560 and text size 20, the stacked incident
/// inspector's body is short: the first line under "Service level
/// objectives" starts at its fold, and the wheel over the body brings it up
/// whole while the page stays.
#[gpui_kit::test]
fn the_wheel_brings_an_incident_inspectors_first_objective_up_whole(cx: &mut TestAppContext) {
    use crate::observability::Destination;
    use gpui_kit::{ScrollDelta, point};
    let (_runtime, handle, view) = fixture(cx, 760., 560.);
    cx.update_window(handle, |_, window, cx| {
        crate::text_size::set(20., cx);
        view.update(cx, |view, cx| {
            view.navigate(Page::Observability, window, cx);
            view.pages
                .observability
                .update(cx, |page, cx| page.open(Destination::Incidents, cx));
        });
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        for _ in 0..8 {
            window.render_frame(cx);
            if window.simulate_next_frame(cx) == 0 {
                break;
            }
        }
        let body = window.find("obs-incident-detail-body").bounds();
        let first = window.find("obs-incident-slo-0-name").bounds();
        let table = window.find("obs-incidents-table").bounds().top();
        assert!(
            first.bottom() > body.bottom(),
            "the line starts below the fold: {first:?} in {body:?}"
        );
        window.scroll(
            "obs-incident-detail-body",
            ScrollDelta::Pixels(point(px(0.), -(first.bottom() - body.bottom()) - px(8.))),
            cx,
        );
        window.render_frame(cx);
        let first = window.find("obs-incident-slo-0-name").bounds();
        assert!(
            first.top() >= body.top() && first.bottom() <= body.bottom(),
            "the first line is whole in the body: {first:?} in {body:?}"
        );
        assert_eq!(
            window.find("obs-incidents-table").bounds().top(),
            table,
            "the page stays while the body scrolls"
        );
    })
    .unwrap();
}
