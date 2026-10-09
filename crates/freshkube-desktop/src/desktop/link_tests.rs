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
            pilot.registry.adopt(key("dev-fra"));
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
    // A switch while the startup read is out is refused ("Still reading the
    // configuration"), which left the first entry's name on a slow runner.
    cx.wait_for(handle, std::time::Duration::from_secs(30), |_, cx| {
        !view.read(cx).config_loading
    })
    .await;
    cx.update_window(handle, |_, window, cx| {
        view.update(cx, |pilot, cx| {
            pilot.switch_cluster("lab".into(), window, cx)
        })
    })
    .unwrap();
    cx.run_until_parked();
    // Named at once, whatever the read does; then it reads as failed.
    cx.read(|cx| assert_eq!(view.read(cx).context_display.name.as_ref(), "acme-lab"));
    cx.wait_for(handle, std::time::Duration::from_secs(30), |_, cx| {
        matches!(
            view.read(cx).context_display.state,
            super::shell::Connection::Failed
        )
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
    // And the notice that was actually pushed says why.
    cx.read(|cx| {
        assert_eq!(
            view.read(cx).told.last().map(String::as_str),
            Some("dev-fra reconnected since this link was made; open it again")
        )
    });
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

#[gpui_kit::test]
fn a_window_that_is_no_entrys_retires_nothing(cx: &mut TestAppContext) {
    let (_runtime, _handle, view) = fixture(cx, 1280., 820.);
    cx.update(|cx| {
        view.update(cx, |pilot, _| {
            assert_eq!(pilot.registry.active_key(), &SessionKey::Implicit);
            pilot
                .registry
                .retire_connection(Some("c".into()), &Definition::default());
            assert!(pilot.registry.retired("c").is_none());
        })
    });
}

/// checkout's Deployment in `dev-fra`, a workspace entry that isn't open.
const DEV_API: &str = "application-part-dev-fra/3/checkout/checkout-api";
/// checkout's Deployment in `prod-lon`, which no workspace entry is.
const LON_WORKER: &str = "application-part-prod-lon/3/checkout/checkout-worker";

/// The fixture on checkout's application page, with `part` selected.
fn on_checkout_part(
    cx: &mut TestAppContext,
    part: &'static str,
) -> (tokio::runtime::Runtime, AnyWindowHandle, Entity<Pilot>) {
    let (runtime, handle, view) = fixture(cx, 1280., 880.);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("nav-applications", cx);
        window.render_frame(cx);
        window.double_click("application-kargo:checkout", cx);
        window.render_frame(cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click(part, cx);
        window.render_frame(cx);
    })
    .unwrap();
    (runtime, handle, view)
}

/// Presses the Inspector's button for the selected part.
fn press_open(cx: &mut TestAppContext, handle: AnyWindowHandle) {
    cx.update_window(handle, |_, window, cx| {
        window.click("application-detail-open", cx);
        window.render_frame(cx);
    })
    .unwrap();
    cx.run_until_parked();
}

fn on_application_page(cx: &mut TestAppContext, view: &Entity<Pilot>) -> bool {
    cx.read(|cx| {
        let pilot = view.read(cx);
        pilot.applications().1 == Page::Applications
            && pilot.applications().0.read(cx).open_page().is_some()
    })
}

#[gpui_kit::test]
fn a_part_in_another_workspace_entry_switches_there_and_opens(cx: &mut TestAppContext) {
    let (_runtime, handle, view) = on_checkout_part(cx, DEV_API);
    let before = active(cx, &view);
    cx.update_window(handle, |_, window, _| {
        // Short on the button; the whole sentence is its tooltip and label.
        let button = window.find("application-detail-open");
        assert_eq!(
            button.label(),
            Some("Switch to dev-fra and open checkout-api there, in Resources")
        );
    })
    .unwrap();
    press_open(cx, handle);
    // Asked first; nothing moves until the answer.
    let (question, detail) = cx.pending_prompt().expect("asked before switching");
    assert_eq!(
        question,
        "Switch to dev-fra to open Deployment checkout-api?"
    );
    assert!(detail.contains("Port forwards keep running"), "{detail}");
    assert_eq!(active(cx, &view), before);
    let told = notices(cx, handle);
    cx.simulate_prompt_answer("Switch");
    cx.run_until_parked();
    assert_eq!(active(cx, &view), key("dev-fra"));
    assert_eq!(
        opened(cx, &view),
        Some((example::connection("dev-fra"), "checkout-api".into()))
    );
    assert_eq!(notices(cx, handle), told, "nothing to say");
    cx.read(|cx| {
        let pilot = view.read(cx);
        assert_eq!(pilot.applications().1, Page::Resources);
        assert!(pilot.pending_link.is_none(), "spent once opened");
    });
    // Another context closed the page. Back on it, dev-fra is the open
    // cluster: its part opens at once, and core-fra's is the switch away.
    cx.update_window(handle, |_, window, cx| {
        window.click("nav-applications", cx);
        window.render_frame(cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.double_click("application-kargo:checkout", cx);
        window.render_frame(cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("application-part-core-fra/0/checkout/dev", cx);
        window.render_frame(cx);
        assert_eq!(
            window.find("application-detail-open").label(),
            Some("Switch to core-fra and open dev there, in Resources")
        );
        window.click(DEV_API, cx);
        window.render_frame(cx);
        assert_eq!(
            window.find("application-detail-open").label(),
            Some("Open in Resources")
        );
        window.click("application-detail-open", cx);
    })
    .unwrap();
    cx.run_until_parked();
    assert!(cx.pending_prompt().is_none(), "opens without asking");
    assert_eq!(
        opened(cx, &view),
        Some((example::connection("dev-fra"), "checkout-api".into()))
    );
}

#[gpui_kit::test]
fn the_row_menu_and_o_ask_the_same_question(cx: &mut TestAppContext) {
    let (_runtime, handle, _view) = on_checkout_part(cx, DEV_API);
    cx.update_window(handle, |_, window, cx| {
        window.right_click(DEV_API, cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let menu = window.within("popup-menu");
        assert_eq!(
            menu.find(0usize).label(),
            Some("Switch to dev-fra and open")
        );
        window.within("popup-menu").click(0usize, cx);
    })
    .unwrap();
    cx.run_until_parked();
    let (question, _) = cx.pending_prompt().expect("the menu asks");
    assert_eq!(
        question,
        "Switch to dev-fra to open Deployment checkout-api?"
    );
    cx.simulate_prompt_answer("Cancel");
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.press("o", cx);
    })
    .unwrap();
    cx.run_until_parked();
    let (question, _) = cx.pending_prompt().expect("O asks");
    assert_eq!(
        question,
        "Switch to dev-fra to open Deployment checkout-api?"
    );
    cx.simulate_prompt_answer("Cancel");
    cx.run_until_parked();
}

#[gpui_kit::test]
fn cancelling_the_switch_changes_nothing(cx: &mut TestAppContext) {
    let (_runtime, handle, view) = on_checkout_part(cx, DEV_API);
    let before = active(cx, &view);
    let context = cx.read(|cx| view.read(cx).applied.context.clone());
    // A link an earlier switch holds outlives the question Cancel answers.
    view.update(cx, |pilot, _| {
        pilot.pending_link = Some(super::switch::PendingLink::for_test(
            "stage-fra",
            pilot.entry_generation,
            pod_link("stage-fra"),
            LinkWork::Open {
                kind: builtin("pods").unwrap(),
                tab: resources::Tab::Overview,
            },
        ));
    });
    press_open(cx, handle);
    assert!(cx.pending_prompt().is_some());
    cx.simulate_prompt_answer("Cancel");
    cx.run_until_parked();
    assert_eq!(active(cx, &view), before);
    assert!(on_application_page(cx, &view), "the page stays");
    assert_eq!(opened(cx, &view), None);
    cx.read(|cx| {
        let pilot = view.read(cx);
        assert_eq!(pilot.applied.context, context);
        let held = pilot.pending_link.as_ref().expect("still held");
        assert_eq!(held.entry, "stage-fra");
    });
}

/// A part of the entry that is already active, as after a switch the page
/// hasn't heard of yet, waits for that entry's own connection: it never
/// opens on the one the window had before it answered.
#[gpui_kit::test]
fn a_part_of_the_active_entry_waits_for_its_connection(cx: &mut TestAppContext) {
    let (_runtime, handle, view) = fixture(cx, 1280., 820.);
    let deployments = builtin("deployments.apps").unwrap();
    let (_, rows) = example::read("dev-fra", "deployments.apps", None, live::now()).unwrap();
    let mut object = ObjectRef::from(
        rows.iter()
            .find(|row| row.identity.name == "checkout-api")
            .expect("dev-fra runs checkout-api")
            .identity
            .clone(),
    );
    object.connection = None;
    switch(cx, handle, &view, "dev-fra");
    cx.update_window(handle, |_, window, cx| {
        view.update(cx, |pilot, cx| {
            // The entry's source hasn't answered: no context is applied.
            pilot.applied.context = None;
            let link = resources::ResourceLink::Object(
                deployments.clone(),
                object.clone(),
                resources::Tab::Overview,
            );
            pilot.open_on_entry("dev-fra".into(), link, window, cx);
            let held = pilot.pending_link.as_ref().expect("held for dev-fra");
            assert_eq!(held.entry, "dev-fra");
        })
    })
    .unwrap();
    assert!(cx.pending_prompt().is_none(), "nothing to ask");
    cx.run_until_parked();
    assert_eq!(opened(cx, &view), None, "nothing opens before it answers");
    cx.update_window(handle, |_, window, cx| {
        view.update(cx, |pilot, cx| {
            pilot.applied.context = Some("dev-fra".into());
            pilot.open_pending_link(window, cx);
        })
    })
    .unwrap();
    cx.run_until_parked();
    assert_eq!(
        opened(cx, &view),
        Some((example::connection("dev-fra"), "checkout-api".into()))
    );
    cx.read(|cx| assert!(view.read(cx).pending_link.is_none()));
}

/// A running shell is folded into the one question, never asked about
/// twice.
#[gpui_kit::test]
fn a_running_shell_is_asked_about_in_the_same_question(cx: &mut TestAppContext) {
    let (_runtime, handle, view) = on_checkout_part(cx, DEV_API);
    let context = cx.read(|cx| view.read(cx).applied.context.clone().unwrap());
    let (_, rows) = example::read(&context, "pods", None, live::now()).unwrap();
    let pod = rows
        .iter()
        .find(|row| row.cells[2] == "Running")
        .unwrap()
        .identity
        .clone();
    start_shell(handle, &view, &pod, cx);
    // The shell's tab took the dock; the page keeps its part.
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click(DEV_API, cx);
        window.render_frame(cx);
    })
    .unwrap();
    press_open(cx, handle);
    let (question, detail) = cx.pending_prompt().expect("asked once");
    assert_eq!(
        question,
        format!(
            "Switch to dev-fra to open Deployment checkout-api and end the shell in {}?",
            pod.name
        )
    );
    assert!(detail.contains("Port forwards keep running"), "{detail}");
    cx.simulate_prompt_answer("Switch and end the shell");
    cx.run_until_parked();
    assert!(cx.pending_prompt().is_none(), "no second question");
    assert_eq!(active(cx, &view), key("dev-fra"));
    assert_eq!(
        opened(cx, &view),
        Some((example::connection("dev-fra"), "checkout-api".into()))
    );
}

/// `prod-lon` is no workspace entry: its parts keep the refusal, greyed
/// out, and O says why.
#[gpui_kit::test]
fn a_cluster_the_workspace_does_not_list_is_still_refused(cx: &mut TestAppContext) {
    let (_runtime, handle, view) = on_checkout_part(cx, LON_WORKER);
    let before = active(cx, &view);
    cx.update_window(handle, |_, window, cx| {
        assert_eq!(
            window.find("application-detail-open").label(),
            Some("Open in Resources")
        );
        window.press("o", cx);
    })
    .unwrap();
    cx.run_until_parked();
    assert!(cx.pending_prompt().is_none(), "nothing to switch to");
    assert_eq!(active(cx, &view), before);
    cx.read(|cx| {
        assert_eq!(
            view.read(cx).told.last().map(String::as_str),
            Some("Can’t open checkout-worker: it belongs to a cluster that isn’t open")
        );
    });
}

/// An entry the page was told of but the workspace file no longer lists
/// is refused as any other cluster that isn't open.
#[gpui_kit::test]
fn an_entry_no_longer_listed_is_refused(cx: &mut TestAppContext) {
    let (_runtime, handle, view) = fixture(cx, 1280., 820.);
    let before = active(cx, &view);
    let link = resources::ResourceLink::Object(
        builtin("deployments.apps").unwrap(),
        ObjectRef {
            namespace: "checkout".into(),
            name: "checkout-api".into(),
            uid: String::new(),
            connection: Some("removed-fra".into()),
        },
        resources::Tab::Overview,
    );
    cx.update_window(handle, |_, window, cx| {
        view.update(cx, |pilot, cx| {
            pilot.open_on_entry("removed-fra".into(), link, window, cx)
        })
    })
    .unwrap();
    cx.run_until_parked();
    assert!(cx.pending_prompt().is_none());
    assert_eq!(active(cx, &view), before);
    cx.read(|cx| {
        assert_eq!(
            view.read(cx).told.last().map(String::as_str),
            Some("Can’t open checkout-api: it belongs to a cluster that isn’t open")
        );
    });
}

/// A link made by the cluster's name for an object the entry doesn't hold
/// says so as any link to a missing object does, and holds nothing after.
#[gpui_kit::test]
fn a_missing_object_after_the_switch_says_so(cx: &mut TestAppContext) {
    let (_runtime, handle, view) = fixture(cx, 1280., 820.);
    let link = resources::ResourceLink::Object(
        builtin("deployments.apps").unwrap(),
        ObjectRef {
            namespace: "checkout".into(),
            name: "gone-api".into(),
            uid: String::new(),
            connection: Some("dev-fra".into()),
        },
        resources::Tab::Overview,
    );
    cx.update_window(handle, |_, window, cx| {
        view.update(cx, |pilot, cx| {
            pilot.open_on_entry("dev-fra".into(), link, window, cx)
        })
    })
    .unwrap();
    cx.run_until_parked();
    let before = notices(cx, handle);
    let told = cx.read(|cx| view.read(cx).told.len());
    cx.simulate_prompt_answer("Switch");
    cx.run_until_parked();
    assert_eq!(active(cx, &view), key("dev-fra"));
    assert_eq!(opened(cx, &view), None);
    // The read for its identity says it can't open it; the switch itself
    // says nothing, not that the entry reconnected.
    assert_eq!(notices(cx, handle), before + 1, "says it can't open it");
    cx.read(|cx| {
        let pilot = view.read(cx);
        assert_eq!(pilot.told.len(), told, "{:?}", &pilot.told[told..]);
        assert!(pilot.pending_link.is_none());
    });
}

fn notices(cx: &mut TestAppContext, handle: AnyWindowHandle) -> usize {
    use gpui_kit::component::WindowExt as _;
    cx.update_window(handle, |_, window, cx| window.notifications(cx).len())
        .unwrap()
}

/// Another link, or a switch elsewhere, before the entry opens drops the
/// held one: it never lands on a page the user has left.
#[gpui_kit::test]
fn another_link_or_switch_drops_a_link_held_for_an_entry(cx: &mut TestAppContext) {
    let (_runtime, handle, view) = fixture(cx, 1280., 820.);
    let hold = |cx: &mut TestAppContext, view: &Entity<Pilot>| {
        let view = view.clone();
        cx.update(move |cx| {
            view.update(cx, |pilot, _| {
                pilot.pending_link = Some(super::switch::PendingLink::for_test(
                    "dev-fra",
                    pilot.entry_generation,
                    pod_link("dev-fra"),
                    LinkWork::Open {
                        kind: builtin("pods").unwrap(),
                        tab: resources::Tab::Overview,
                    },
                ));
            })
        })
    };
    hold(cx, &view);
    // A link here replaces it.
    let here = cx.read(|cx| view.read(cx).applied.context.clone().unwrap());
    open(cx, handle, &view, pod_link(&here));
    cx.read(|cx| assert!(view.read(cx).pending_link.is_none()));
    hold(cx, &view);
    switch(cx, handle, &view, "stage-fra");
    assert_eq!(active(cx, &view), key("stage-fra"));
    cx.read(|cx| assert!(view.read(cx).pending_link.is_none()));
}
