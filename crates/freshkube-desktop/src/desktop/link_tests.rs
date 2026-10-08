//! Links that name another workspace entry's session.
use super::session::{Definition, SessionKey};
use super::switch::{LinkRoute, LinkWork};
use super::tests::{fixture, mount, start_shell};
use super::{Page, Pilot};
use crate::GpuiOptions;
use crate::resources::model::ObjectRef;
use crate::resources::{self, example, live};
use freshkube_core::resources::builtin;

use gpui_kit::AppContext;
use gpui_kit::test::{TestAppContextExt, TestWindowExt};
use gpui_kit::{AnyWindowHandle, Entity, TestAppContext};

fn key(id: &str) -> SessionKey {
    SessionKey::Entry(id.into())
}

fn switch(cx: &mut TestAppContext, handle: AnyWindowHandle, view: &Entity<Pilot>, id: &str) {
    cx.update_window(handle, |_, window, cx| {
        view.update(cx, |pilot, cx| pilot.switch_cluster(id.into(), window, cx))
    })
    .unwrap();
    cx.run_until_parked();
}

fn active(cx: &mut TestAppContext, view: &Entity<Pilot>) -> SessionKey {
    cx.read(|cx| view.read(cx).registry.active_key().clone())
}

/// A pod of the example cluster `context`, as a link made in it names it.
fn pod_link(context: &str) -> ObjectRef {
    let (_, rows) = example::read(context, "pods", None, live::now()).unwrap();
    let mut link = ObjectRef::from(rows[0].identity.clone());
    link.connection = Some(example::connection(context));
    link
}

fn open(cx: &mut TestAppContext, handle: AnyWindowHandle, view: &Entity<Pilot>, link: ObjectRef) {
    cx.update_window(handle, |_, window, cx| {
        view.update(cx, |pilot, cx| {
            pilot.open_object(
                builtin("pods").unwrap(),
                link,
                resources::Tab::Overview,
                window,
                cx,
            )
        });
    })
    .unwrap();
    cx.run_until_parked();
}

fn opened(cx: &mut TestAppContext, view: &Entity<Pilot>) -> Option<(String, String)> {
    cx.read(|cx| {
        let pilot = view.read(cx);
        pilot
            .resources
            .read(cx)
            .detail_identity(cx)
            .map(|identity| (identity.connection.clone(), identity.name.clone()))
    })
}

#[gpui_kit::test]
fn a_link_into_another_entry_opens_that_entry_then_the_object(cx: &mut TestAppContext) {
    let (_runtime, handle, view) = fixture(cx, 1280., 820.);
    switch(cx, handle, &view, "dev-fra");
    switch(cx, handle, &view, "core-fra");
    assert_eq!(active(cx, &view), key("core-fra"));
    let link = pod_link("dev-fra");
    open(cx, handle, &view, link.clone());
    // The link's own cluster is open, with its own object.
    assert_eq!(active(cx, &view), key("dev-fra"));
    assert_eq!(
        opened(cx, &view),
        Some((example::connection("dev-fra"), link.name))
    );
    cx.read(|cx| assert!(view.read(cx).pending_link.is_none()));
}

#[gpui_kit::test]
fn a_link_naming_no_cluster_or_the_open_one_stays_here(cx: &mut TestAppContext) {
    let (_runtime, handle, view) = fixture(cx, 1280., 820.);
    switch(cx, handle, &view, "dev-fra");
    let mut link = pod_link("dev-fra");
    open(cx, handle, &view, link.clone());
    assert_eq!(active(cx, &view), key("dev-fra"));
    assert!(opened(cx, &view).is_some());
    link.connection = None;
    open(cx, handle, &view, link);
    assert_eq!(active(cx, &view), key("dev-fra"));
}

#[gpui_kit::test]
fn a_link_from_a_cluster_the_window_never_held_is_refused(cx: &mut TestAppContext) {
    let (_runtime, handle, view) = fixture(cx, 1280., 820.);
    switch(cx, handle, &view, "dev-fra");
    let mut link = pod_link("core-fra");
    link.connection = Some("not-a-session".into());
    open(cx, handle, &view, link);
    assert_eq!(active(cx, &view), key("dev-fra"));
    assert_eq!(opened(cx, &view), None);
}

#[gpui_kit::test]
fn cancelling_the_shell_question_holds_no_link_and_changes_nothing(cx: &mut TestAppContext) {
    let (_runtime, handle, view) = fixture(cx, 1280., 820.);
    switch(cx, handle, &view, "dev-fra");
    switch(cx, handle, &view, "core-fra");
    let context = cx.read(|cx| view.read(cx).applied.context.clone().unwrap());
    let (_, rows) = example::read(&context, "pods", None, live::now()).unwrap();
    let pod = rows
        .iter()
        .find(|row| row.cells[2] == "Running")
        .unwrap()
        .identity
        .clone();
    start_shell(handle, &view, &pod, cx);
    open(cx, handle, &view, pod_link("dev-fra"));
    assert!(cx.pending_prompt().is_some());
    cx.simulate_prompt_answer("Cancel");
    cx.run_until_parked();
    assert_eq!(active(cx, &view), key("core-fra"));
    cx.read(|cx| assert!(view.read(cx).pending_link.is_none()));
    // Agreeing ends the shell, switches, and opens the object.
    open(cx, handle, &view, pod_link("dev-fra"));
    cx.simulate_prompt_answer("End the shell");
    cx.run_until_parked();
    assert_eq!(active(cx, &view), key("dev-fra"));
    assert!(opened(cx, &view).is_some());
}

fn route(cx: &mut TestAppContext, view: &Entity<Pilot>, link: &ObjectRef) -> LinkRoute {
    cx.read(|cx| view.read(cx).route_link(link, cx))
}

fn refusal(route: LinkRoute) -> String {
    match route {
        LinkRoute::Refuse(message) => message,
        LinkRoute::Here => "here".into(),
        LinkRoute::Activate(id) => format!("activate {id}"),
    }
}

#[gpui_kit::test]
fn a_link_from_an_older_session_of_the_open_entry_is_refused_and_says_so(cx: &mut TestAppContext) {
    let (_runtime, handle, view) = fixture(cx, 1280., 820.);
    switch(cx, handle, &view, "dev-fra");
    let mut link = pod_link("dev-fra");
    link.connection = Some("an-earlier-session".into());
    cx.update(|cx| {
        view.update(cx, |pilot, _| {
            let definition = pilot.active_definition.clone();
            pilot
                .registry
                .retire_connection(Some("an-earlier-session".into()), &definition);
        })
    });
    assert_eq!(
        refusal(route(cx, &view, &link)),
        "dev-fra reconnected since this link was made; open it again"
    );
    // It is never re-resolved against the new session.
    open(cx, handle, &view, link);
    assert_eq!(opened(cx, &view), None);
}

#[gpui_kit::test]
fn a_link_whose_entry_was_edited_or_removed_is_not_activated(cx: &mut TestAppContext) {
    let (_runtime, handle, view) = fixture(cx, 1280., 820.);
    switch(cx, handle, &view, "dev-fra");
    switch(cx, handle, &view, "core-fra");
    let mut link = pod_link("dev-fra");
    link.connection = Some("dev-session".into());
    cx.update(|cx| {
        view.update(cx, |pilot, _| {
            // Retired under the active key, so file it under dev-fra's.
            pilot.registry.adopt(key("dev-fra"));
            pilot
                .registry
                .retire_connection(Some("dev-session".into()), &Definition::default());
            pilot.registry.adopt(key("core-fra"));
        })
    });
    // Its definition at the time is not the one the file has now.
    assert!(
        refusal(route(cx, &view, &link)).contains("reconnected since this link was made"),
        "{}",
        refusal(route(cx, &view, &link))
    );
    // Removed from the workspace: forgotten, and refused as any unknown one.
    cx.update(|cx| {
        view.update(cx, |pilot, _| {
            pilot
                .registry
                .retain_parked(|key| key != &self::key("dev-fra"));
        })
    });
    assert!(refusal(route(cx, &view, &link)).contains("isn’t open"));
}

#[gpui_kit::test]
fn the_ended_connections_are_bounded_and_the_oldest_go_first(cx: &mut TestAppContext) {
    let (_runtime, _handle, view) = fixture(cx, 1280., 820.);
    cx.update(|cx| {
        view.update(cx, |pilot, _| {
            let definition = Definition::default();
            for n in 0..100 {
                pilot
                    .registry
                    .retire_connection(Some(format!("session-{n}")), &definition);
            }
            assert!(pilot.registry.retired("session-0").is_none());
            assert!(pilot.registry.retired("session-99").is_some());
            assert!(pilot.registry.retired("session-36").is_some());
            assert!(pilot.registry.retired("session-35").is_none());
        })
    });
}

#[gpui_kit::test]
fn a_held_link_opens_nothing_while_its_entry_has_no_source_and_is_dropped_when_it_moves_on(
    cx: &mut TestAppContext,
) {
    let (_runtime, handle, view) = fixture(cx, 1280., 820.);
    switch(cx, handle, &view, "dev-fra");
    cx.update_window(handle, |_, window, cx| {
        view.update(cx, |pilot, cx| {
            // The entry's source never arrives: no context was applied.
            pilot.applied.context = None;
            pilot.pending_link = Some(super::switch::PendingLink::for_test(
                "dev-fra",
                pilot.entry_generation,
                pod_link("dev-fra"),
                LinkWork::Open {
                    kind: builtin("pods").unwrap(),
                    tab: resources::Tab::Overview,
                },
            ));
            pilot.navigate_from_keyboard(Page::Nodes, window, cx);
            pilot.navigate_from_keyboard(Page::Overview, window, cx);
            pilot.open_pending_link(window, cx);
            assert!(pilot.pending_link.is_some());
        })
    })
    .unwrap();
    assert_eq!(opened(cx, &view), None);
    // The entry moving on (a new open, an error) drops it with the entry kept.
    cx.update_window(handle, |_, window, cx| {
        view.update(cx, |pilot, cx| {
            pilot.entry_generation = pilot.entry_generation.wrapping_add(1);
            pilot.applied.context = Some("dev-fra".into());
            pilot.open_pending_link(window, cx);
            assert!(pilot.pending_link.is_none());
            pilot.pending_link = Some(super::switch::PendingLink::for_test(
                "dev-fra",
                pilot.entry_generation,
                pod_link("dev-fra"),
                LinkWork::Open {
                    kind: builtin("pods").unwrap(),
                    tab: resources::Tab::Overview,
                },
            ));
            pilot.applied.context = None;
            // And leaving the entry by hand drops it too.
            pilot.leave_entry(cx);
            pilot.open_pending_link(window, cx);
            assert!(pilot.pending_link.is_none());
        })
    })
    .unwrap();
    assert_eq!(opened(cx, &view), None);
}

fn folder_with_two_entries() -> tempfile::TempDir {
    let guard = tempfile::tempdir().unwrap();
    let missing = guard.path().join("missing-kubeconfig");
    std::fs::write(
        guard.path().join("workspace.json"),
        format!(
            r#"{{"version":1,"kubeconfig":{},"clusters":[{{"id":"edge","role":"environment","context":"acme-edge"}},{{"id":"lab","role":"cicd","context":"acme-lab"}}]}}"#,
            serde_json::to_string(&missing).unwrap()
        ),
    )
    .unwrap();
    guard
}

#[gpui_kit::test]
async fn the_header_names_a_picked_entry_whose_kubeconfig_cannot_be_read(cx: &mut TestAppContext) {
    cx.executor().allow_parking();
    let guard = folder_with_two_entries();
    let options = GpuiOptions::new(None, None, 100)
        .with_preferences(Some(guard.path().join("preferences.json")));
    let (_runtime, handle, view) = mount(cx, options, 1280., 820.);
    cx.update_window(handle, |_, window, cx| {
        view.update(cx, |pilot, cx| {
            pilot.switch_cluster("lab".into(), window, cx)
        })
    })
    .unwrap();
    cx.run_until_parked();
    // Named at once; once the read has failed it reads as a failed connection.
    // Waits rather than asserting at once: a loaded runner finishes the
    // switch's reads later (the first Windows CI run saw the old name).
    cx.wait_for(handle, std::time::Duration::from_secs(30), |_, cx| {
        let display = &view.read(cx).context_display;
        display.name.as_ref() == "acme-lab"
            && matches!(display.state, super::shell::Connection::Failed)
    })
    .await;
    cx.read(|cx| {
        let pilot = view.read(cx);
        assert_eq!(pilot.registry.active_key(), &key("lab"));
        assert_eq!(pilot.display_context(cx), "acme-lab");
        assert_eq!(pilot.context_display.name.as_ref(), "acme-lab");
        assert!(!pilot.context_display.status.contains("No context"));
        assert!(matches!(
            pilot.context_display.state,
            super::shell::Connection::Failed
        ));
    });
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let label = window
            .find("context-switcher")
            .label()
            .unwrap_or_default()
            .to_owned();
        assert_eq!(label, "acme-lab");
    })
    .unwrap();
}

#[gpui_kit::test]
fn with_no_entry_and_no_context_the_header_still_says_so(cx: &mut TestAppContext) {
    let (_runtime, _handle, view) = fixture(cx, 1280., 820.);
    cx.update(|cx| {
        view.update(cx, |pilot, cx| {
            pilot.applied.context = None;
            assert_eq!(pilot.display_context(cx), "No context");
        })
    });
}

fn hold(cx: &mut TestAppContext, view: &Entity<Pilot>, entry: &str, link: ObjectRef) {
    cx.update(|cx| {
        view.update(cx, |pilot, _| {
            pilot.pending_link = Some(super::switch::PendingLink::for_test(
                entry,
                pilot.entry_generation,
                link,
                LinkWork::Open {
                    kind: builtin("pods").unwrap(),
                    tab: resources::Tab::Overview,
                },
            ));
        })
    });
}

fn settle(cx: &mut TestAppContext, handle: AnyWindowHandle, view: &Entity<Pilot>) {
    cx.update_window(handle, |_, window, cx| {
        view.update(cx, |pilot, cx| pilot.open_pending_link(window, cx))
    })
    .unwrap();
    cx.run_until_parked();
}

fn held(cx: &mut TestAppContext, view: &Entity<Pilot>) -> bool {
    cx.read(|cx| view.read(cx).pending_link.is_some())
}

#[gpui_kit::test]
fn another_switch_drops_a_held_link(cx: &mut TestAppContext) {
    let (_runtime, handle, view) = fixture(cx, 1280., 820.);
    switch(cx, handle, &view, "dev-fra");
    hold(cx, &view, "dev-fra", pod_link("dev-fra"));
    switch(cx, handle, &view, "core-fra");
    assert!(!held(cx, &view));
    settle(cx, handle, &view);
    assert_eq!(opened(cx, &view), None);
    assert_eq!(active(cx, &view), key("core-fra"));
}

#[gpui_kit::test]
fn a_link_whose_connection_is_not_the_entrys_now_says_it_reconnected(cx: &mut TestAppContext) {
    let (_runtime, handle, view) = fixture(cx, 1280., 820.);
    switch(cx, handle, &view, "dev-fra");
    let mut link = pod_link("dev-fra");
    link.connection = Some("another-access".into());
    hold(cx, &view, "dev-fra", link);
    settle(cx, handle, &view);
    assert!(!held(cx, &view));
    assert_eq!(opened(cx, &view), None, "never opened by name alone");
}

#[gpui_kit::test]
fn a_manual_pick_retires_the_entry_it_left_and_a_fresh_entry_retires_nothing(
    cx: &mut TestAppContext,
) {
    let (_runtime, handle, view) = fixture(cx, 1280., 820.);
    switch(cx, handle, &view, "dev-fra");
    cx.read(|cx| {
        let registry = &view.read(cx).registry;
        assert!(registry.retired(&example::connection("dev-fra")).is_none());
    });
    switch(cx, handle, &view, "core-fra");
    cx.read(|cx| {
        let registry = &view.read(cx).registry;
        assert!(registry.retired(&example::connection("dev-fra")).is_some());
        assert!(registry.retired(&example::connection("core-fra")).is_none());
    });
    // A hand-made change leaves core-fra, which keeps its id for links.
    cx.update(|cx| {
        view.update(cx, |pilot, cx| {
            pilot.leave_entry(cx);
            pilot.applied.context = Some("dev-fra".into());
        })
    });
    assert_eq!(active(cx, &view), SessionKey::Implicit);
    cx.read(|cx| {
        let pilot = view.read(cx);
        let retired = pilot.registry.retired(&example::connection("core-fra"));
        assert_eq!(retired.map(|retired| &retired.key), Some(&key("core-fra")));
        assert!(matches!(
            pilot.route_link(&pod_link("core-fra"), cx),
            LinkRoute::Activate(id) if id == "core-fra"
        ));
    });
}
