use super::CollectedOverview;
use crate::{desktop::fixture, resources};
use freshkube_core::{AccessIdentity, AccessSessionId, ConfigurationRevision};
use gpui_kit::{App, AppContext, SharedString, Window};
use gpui_kit::{TestAppContext, test::TestWindowExt};

fn revisions() -> (ConfigurationRevision, ConfigurationRevision) {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("config");
    std::fs::write(&path, "before").unwrap();
    let before = ConfigurationRevision::from_kubeconfig_sources(std::slice::from_ref(&path));
    std::fs::write(&path, "after!").unwrap();
    (
        before,
        ConfigurationRevision::from_kubeconfig_sources(&[path]),
    )
}

#[gpui_kit::test]
fn access_replacement_resets_observations_and_rejects_late_overview_results(
    cx: &mut TestAppContext,
) {
    let (_runtime, handle, view) = crate::desktop::tests::fixture(cx, 1280., 820.);
    let (before, after) = revisions();
    let old_access = AccessIdentity::new(AccessSessionId::new(), before);
    let new_access = AccessIdentity::new(AccessSessionId::new(), after);
    cx.update_window(handle, |_, window, cx| {
        view.update(cx, |view, cx| {
            view.switch.registry.active_mut().access = Some(old_access);
            view.switch.registry.active_mut().access_configuration = Some(before);
            let observation = view
                .switch
                .registry
                .active()
                .summary_session
                .as_ref()
                .unwrap()
                .core
                .identity()
                .clone();
            let context = view.applied.context.clone().unwrap();
            let pod = resources::example::read(&context, "pods", None, resources::live::now())
                .unwrap()
                .1
                .into_iter()
                .find(|row| row.cells[2] == "Running")
                .unwrap()
                .identity;
            let forwards = crate::forwards::list(cx);
            let forward = forwards.update(cx, |list, cx| {
                list.start(
                    crate::forwards::ForwardSpec {
                        runtime: view.runtime.clone(),
                        access: resources::ClusterAccess::EXAMPLE,
                        target: freshkube_core::resources::ForwardTarget::of(
                            &resources::example::kind("pods").unwrap(),
                            &pod.name,
                            &pod.uid,
                        )
                        .unwrap(),
                        identity: pod.clone(),
                        context: context.clone(),
                        port: 8080,
                        local_port: None,
                    },
                    cx,
                )
            });
            assert!(forward.read(cx).running());
            let port = forward.read(cx).local_port;
            let request = view.overview.begin(view.applied.clone());
            let epoch = view.epoch;
            view.overview_received(
                epoch,
                request.clone(),
                Ok(CollectedOverview {
                    configuration: after,
                    access: Some(new_access),
                    cluster: Ok(fixture::cluster(&context, 1)),
                }),
                window,
                cx,
            );
            assert!(view.switch.registry.active().access == Some(new_access));
            assert_ne!(
                view.switch
                    .registry
                    .active()
                    .summary_session
                    .as_ref()
                    .unwrap()
                    .core
                    .identity(),
                &observation
            );
            assert_eq!(
                view.switch.registry.active().access_configuration,
                Some(after)
            );
            assert!(forward.read(cx).running());
            assert_eq!(forward.read(cx).local_port, port);
            assert_eq!(forward.read(cx).port_of(&pod), Some(8080));
            let mut other_session = pod.clone();
            other_session.connection = new_access.key();
            assert_eq!(forward.read(cx).port_of(&other_session), None);
            view.overview_received(
                epoch,
                request,
                Ok(CollectedOverview {
                    configuration: before,
                    access: Some(old_access),
                    cluster: Err("obsolete".into()),
                }),
                window,
                cx,
            );
            assert!(view.switch.registry.active().access == Some(new_access));
            assert!(view.overview.error().is_none());

            let observation = view
                .switch
                .registry
                .active()
                .summary_session
                .as_ref()
                .unwrap()
                .core
                .identity()
                .clone();
            view.select_node(None, window, cx);
            assert_eq!(
                view.switch
                    .registry
                    .active()
                    .summary_session
                    .as_ref()
                    .unwrap()
                    .core
                    .identity(),
                &observation
            );
            let id = forward.read(cx).id;
            forwards.update(cx, |list, cx| list.stop(id, cx));
            assert!(!forward.read(cx).running());
            let request = view.overview.begin(view.applied.clone());
            view.overview_received(
                view.epoch,
                request,
                Ok(CollectedOverview {
                    configuration: after,
                    access: Some(new_access),
                    cluster: Err("transport unavailable".into()),
                }),
                window,
                cx,
            );
            assert!(view.overview.is_stale());
            assert!(view.overview.data().is_some());
            assert!(view.switch.registry.active().access == Some(new_access));
            assert_eq!(
                view.switch
                    .registry
                    .active()
                    .summary_session
                    .as_ref()
                    .unwrap()
                    .core
                    .identity(),
                &observation
            );
        });
    })
    .unwrap();
}

#[gpui_kit::test]
fn detected_configuration_replacement_asks_nothing_of_a_running_shell(cx: &mut TestAppContext) {
    let (_runtime, handle, view) = crate::desktop::tests::fixture(cx, 1280., 820.);
    let (before, after) = revisions();
    let access = AccessIdentity::new(AccessSessionId::new(), before);
    let context = cx.read(|cx| view.read(cx).applied.context.clone().unwrap());
    let pod = resources::example::read(&context, "pods", None, resources::live::now())
        .unwrap()
        .1
        .into_iter()
        .find(|row| row.cells[2] == "Running")
        .unwrap()
        .identity;
    let row: SharedString =
        format!("resource-row:{}/{}/{}", pod.namespace, pod.name, pod.uid).into();
    let step = |cx: &mut TestAppContext, act: &dyn Fn(&mut Window, &mut App)| {
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            act(window, cx);
        })
        .unwrap();
        cx.run_until_parked();
    };
    step(cx, &|window, cx| {
        view.update(cx, |view, cx| {
            view.open_kind(resources::example::kind("pods").unwrap(), window, cx)
        });
    });
    step(cx, &|window, cx| window.click("resource-view-all", cx));
    step(cx, &|window, cx| window.click(row.clone(), cx));
    crate::desktop::tests::start_shell(handle, &view, &pod, cx);
    assert!(!cx.read(resources::shell::running_anywhere).is_empty());
    view.update(cx, |view, _| {
        // The replacement fails before connecting: no real cluster is read.
        view.mode.set_example(false);
        view.switch.registry.active_mut().access = Some(access);
        view.switch.registry.active_mut().access_configuration = Some(before);
    });
    let deliver = |window: &mut Window, cx: &mut App| {
        view.update(cx, |view, cx| {
            let request = view.overview.begin(view.applied.clone());
            view.overview_received(
                view.epoch,
                request,
                Ok(CollectedOverview {
                    configuration: after,
                    access: None,
                    cluster: Err("replacement cannot connect".into()),
                }),
                window,
                cx,
            );
        });
    };
    step(cx, &deliver);
    // A running shell keeps the connection it started on (#878): the
    // replacement asks nothing of it and ends nothing.
    assert!(!cx.has_pending_prompt());
    assert!(!cx.read(resources::shell::running_anywhere).is_empty());
    assert_eq!(
        cx.read(|cx| view.read(cx).switch.registry.active().access_configuration),
        Some(after)
    );
    assert!(cx.read(|cx| view.read(cx).overview.data().is_none()));
}

/// A new context replaces the whole session: nothing that the old one held,
/// from its access to its tasks, is there for the next.
#[gpui_kit::test]
fn changing_context_through_the_shell_replaces_the_whole_session(cx: &mut TestAppContext) {
    let (_runtime, handle, view) = crate::desktop::tests::fixture(cx, 1280., 820.);
    let (before, _) = revisions();
    let access = AccessIdentity::new(AccessSessionId::new(), before);
    let sender = cx
        .update_window(handle, |_, window, cx| {
            view.update(cx, |view, cx| {
                // Nothing is published for the new context, so what is left in
                // the session after the change is only what was carried over.
                view.mode.hold_mut().talos = true;
                let session = view.switch.registry.active_mut();
                session.access = Some(access);
                session.access_configuration = Some(before);
                session.summary_health = Some(Err("old".into()));
                let identity = view
                    .switch
                    .registry
                    .summary_watch
                    .identity()
                    .unwrap()
                    .clone();
                let sender = view.switch.registry.summary_watch.open(
                    identity,
                    freshkube_reads::Pace {
                        capacity: 1,
                        most: 1,
                        every: std::time::Duration::ZERO,
                    },
                    |view: &mut super::Pilot| &mut view.switch.registry.summary_watch,
                    |_, _, _| {},
                    cx,
                );
                assert!(view.switch.registry.summary_watch.is_running());
                let epoch = view.switch.registry.summary_watch.generation();
                let other = crate::fixture::CONTEXTS[1].to_string();
                assert_ne!(view.applied.context.as_deref(), Some(other.as_str()));
                view.select_context(other, window, cx);
                let session = view.switch.registry.active();
                assert!(session.access.is_none());
                assert!(session.access_configuration.is_none());
                assert!(session.summary_health.is_none());
                assert!(!view.switch.registry.summary_watch.is_running());
                assert!(view.switch.registry.summary_watch.generation() != epoch);
                sender
            })
        })
        .unwrap();
    // GPUI drops the cancelled delivery future on its next executor turn.
    cx.run_until_parked();
    assert!(sender.is_closed());
}
