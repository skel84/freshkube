//! Deep links on example data: Copy link, opening a link and each outcome.
use crate::debug_options::LinkShape;
use crate::desktop::testing::record_notices;
use crate::desktop::tests::{fixture, mount};
use crate::desktop::{Page, Pilot};
use crate::resources::{self, example, live};
use crate::{DebugOptions, GpuiOptions};
use freshkube_nav::{
    KindKey, Link, LinkOutcome, LinkPage, Name, Namespace, ObjectUid, ViewState, ViewTab,
};
use gpui_kit::component::WindowExt as _;
use gpui_kit::test::TestWindowExt;
use gpui_kit::{AnyWindowHandle, AppContext as _, Entity, TestAppContext};

const LINK_UID: &str = "0f8fad5b-d9cb-469f-a165-70867728950e";

fn shaped(cx: &mut TestAppContext, shape: LinkShape) -> (AnyWindowHandle, Entity<Pilot>) {
    let options = GpuiOptions::fixture().with_debug(DebugOptions {
        links: Some(shape),
        ..Default::default()
    });
    let (runtime, handle, view) = mount(cx, options, 1280., 820.);
    // The runtime must outlive the test.
    std::mem::forget(runtime);
    (handle, view)
}

fn open_text(cx: &mut TestAppContext, handle: AnyWindowHandle, view: &Entity<Pilot>, text: &str) {
    let text = text.to_owned();
    cx.update_window(handle, |_, window, cx| {
        view.update(cx, |pilot, cx| pilot.open_link_text(&text, window, cx))
    })
    .unwrap();
    cx.run_until_parked();
}

fn copy(cx: &mut TestAppContext, handle: AnyWindowHandle, view: &Entity<Pilot>) {
    cx.update_window(handle, |_, window, cx| {
        view.update(cx, |pilot, cx| pilot.copy_link(window, cx))
    })
    .unwrap();
    cx.run_until_parked();
}

fn clipboard(cx: &mut TestAppContext) -> String {
    cx.read_from_clipboard()
        .and_then(|item| item.text())
        .expect("a link on the clipboard")
}

fn current(cx: &mut TestAppContext, view: &Entity<Pilot>) -> Option<ViewState> {
    cx.read(|cx| view.read(cx).current_view(cx))
}

/// A deployment of the example cluster in the window.
fn deployment(cx: &mut TestAppContext, view: &Entity<Pilot>) -> resources::model::ResourceIdentity {
    let context = cx.read(|cx| view.read(cx).applied.context.clone().unwrap());
    example::read(&context, "deployments.apps", None, live::now())
        .unwrap()
        .1
        .remove(0)
        .identity
}

fn open_deployment(
    cx: &mut TestAppContext,
    handle: AnyWindowHandle,
    view: &Entity<Pilot>,
    tab: resources::Tab,
) -> resources::model::ResourceIdentity {
    let identity = deployment(cx, view);
    cx.update_window(handle, |_, window, cx| {
        view.update(cx, |pilot, cx| {
            pilot.open_object(
                freshkube_core::resources::builtin("deployments.apps").unwrap(),
                resources::model::ObjectRef::from(identity.clone()),
                tab,
                window,
                cx,
            )
        })
    })
    .unwrap();
    cx.run_until_parked();
    identity
}

fn link_for(identity: &resources::model::ResourceIdentity, extra: &str) -> String {
    format!(
        "freshkube://open?page=resources&kind=deployments.apps&ns={}&name={}{extra}",
        identity.namespace, identity.name
    )
}

/// The fingerprint the fixture window opens on (`prod-fra`) puts in a link.
fn tied(link: &str) -> String {
    use freshkube_core::fingerprint::strong_from_uid;
    format!(
        "{}cluster=u-{}&ca={}&{}",
        "freshkube://open?",
        strong_from_uid("example/kube-system/prod-fra"),
        strong_from_uid("example/ca/prod-fra"),
        link.strip_prefix("freshkube://open?").expect("a link")
    )
}

fn card(cx: &mut TestAppContext, view: &Entity<Pilot>) -> Option<resources::LinkOutcomeCard> {
    cx.read(|cx| {
        view.read(cx)
            .pages
            .resources
            .read(cx)
            .link_outcome()
            .cloned()
    })
}

fn opened(
    cx: &mut TestAppContext,
    view: &Entity<Pilot>,
) -> Option<resources::model::ResourceIdentity> {
    cx.read(|cx| {
        view.read(cx)
            .pages
            .resources
            .read(cx)
            .detail_identity(cx)
            .cloned()
    })
}

fn drawn(cx: &mut TestAppContext, handle: AnyWindowHandle, id: &str) -> bool {
    let id = id.to_owned();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.try_find(id).is_some()
    })
    .unwrap()
}

#[gpui_kit::test]
fn a_page_without_a_place_to_link_has_no_view(cx: &mut TestAppContext) {
    let (_runtime, handle, view) = fixture(cx, 1280., 820.);
    for page in [Page::Settings, Page::Operations] {
        cx.update_window(handle, |_, window, cx| {
            view.update(cx, |pilot, cx| pilot.navigate(page, window, cx))
        })
        .unwrap();
        assert_eq!(current(cx, &view), None, "{page:?}");
    }
    let notices = record_notices(cx, &view);
    copy(cx, handle, &view);
    assert_eq!(notices.borrow().as_slice(), ["This page can’t be linked."]);
}

#[gpui_kit::test]
fn the_view_names_the_page_the_kind_the_object_and_its_tab(cx: &mut TestAppContext) {
    let (_runtime, handle, view) = fixture(cx, 1280., 820.);
    cx.update_window(handle, |_, window, cx| {
        view.update(cx, |pilot, cx| pilot.navigate(Page::Etcd, window, cx))
    })
    .unwrap();
    assert_eq!(current(cx, &view), Some(ViewState::page(LinkPage::Etcd)));

    let identity = open_deployment(cx, handle, &view, resources::Tab::Yaml);
    let seen = current(cx, &view).unwrap();
    assert_eq!(seen.page, LinkPage::Resources);
    assert_eq!(
        seen.kind.as_ref().map(KindKey::as_str),
        Some("deployments.apps")
    );
    assert_eq!(
        seen.ns.as_ref().map(Namespace::as_str),
        Some(identity.namespace.as_str())
    );
    let object = seen.object.as_ref().expect("the open object");
    assert_eq!(object.name.as_str(), identity.name);
    assert_eq!(
        object.uid.as_ref().map(ObjectUid::as_str),
        Some(identity.uid.as_str())
    );
    assert_eq!(seen.tab, ViewTab::Yaml);
}

#[gpui_kit::test]
fn copy_link_puts_the_canonical_link_on_the_clipboard_and_says_what_it_opens(
    cx: &mut TestAppContext,
) {
    let (_runtime, handle, view) = fixture(cx, 1280., 820.);
    let notices = record_notices(cx, &view);
    let identity = open_deployment(cx, handle, &view, resources::Tab::Yaml);
    copy(cx, handle, &view);
    let text = clipboard(cx);
    assert_eq!(
        text,
        tied(&link_for(
            &identity,
            &format!("&uid={}&tab=yaml", identity.uid)
        ))
    );
    let said = notices.borrow().last().cloned().unwrap();
    assert!(
        said.starts_with("Link copied. It opens deployments.apps"),
        "{said}"
    );
    assert!(said.contains("(YAML)"), "{said}");
    assert!(said.contains("reader’s own access"), "{said}");
    // Nothing a person could paste carries more than identities.
    assert!(Link::parse(&text).is_ok());
}

#[gpui_kit::test]
fn a_copied_link_opens_the_same_view_again(cx: &mut TestAppContext) {
    let (_runtime, handle, view) = fixture(cx, 1280., 820.);
    let identity = open_deployment(cx, handle, &view, resources::Tab::Yaml);
    let before = current(cx, &view).unwrap();
    copy(cx, handle, &view);
    let text = clipboard(cx);
    // Somewhere else.
    cx.update_window(handle, |_, window, cx| {
        view.update(cx, |pilot, cx| pilot.navigate(Page::Overview, window, cx))
    })
    .unwrap();
    open_text(cx, handle, &view, &text);
    assert_eq!(current(cx, &view), Some(before));
    assert_eq!(opened(cx, &view), Some(identity));
    assert_eq!(card(cx, &view), None);
}

#[gpui_kit::test]
fn a_link_to_a_list_scopes_it_to_the_namespace(cx: &mut TestAppContext) {
    let (_runtime, handle, view) = fixture(cx, 1280., 820.);
    open_text(
        cx,
        handle,
        &view,
        "freshkube://open?page=resources&kind=pods&ns=payments",
    );
    let seen = current(cx, &view).unwrap();
    assert_eq!(seen.kind.as_ref().map(KindKey::as_str), Some("pods"));
    assert_eq!(seen.ns.as_ref().map(Namespace::as_str), Some("payments"));
    assert_eq!(seen.object, None);
}

#[gpui_kit::test]
fn a_link_opens_a_node_on_its_tab(cx: &mut TestAppContext) {
    let (_runtime, handle, view) = fixture(cx, 1280., 820.);
    let node = cx.read(|cx| {
        view.read(cx)
            .pages
            .node_workspace
            .read(cx)
            .rows
            .first()
            .and_then(|row| row.key.kubernetes.clone().or(row.key.talos.clone()))
            .expect("an example node")
    });
    open_text(
        cx,
        handle,
        &view,
        &format!("freshkube://open?page=nodes&node={node}&ntab=pods"),
    );
    let seen = current(cx, &view).unwrap();
    assert_eq!(seen.page, LinkPage::Nodes);
    assert_eq!(seen.node.as_ref().map(Name::as_str), Some(node.as_str()));
    assert_eq!(seen.node_tab, freshkube_nav::NodeTab::Pods);
}

#[gpui_kit::test]
fn a_node_the_cluster_does_not_list_says_so_and_opens_the_table(cx: &mut TestAppContext) {
    let (_runtime, handle, view) = fixture(cx, 1280., 820.);
    let notices = record_notices(cx, &view);
    open_text(
        cx,
        handle,
        &view,
        "freshkube://open?page=nodes&node=no-such-node",
    );
    cx.read(|cx| assert_eq!(view.read(cx).navigation.page, Page::Nodes));
    let said = notices.borrow().last().cloned().unwrap();
    assert!(said.starts_with("Can’t open node no-such-node"), "{said}");
}

#[gpui_kit::test]
fn a_missing_object_shows_not_found_with_why_and_next(cx: &mut TestAppContext) {
    let (_runtime, handle, view) = fixture(cx, 1280., 820.);
    open_text(
        cx,
        handle,
        &view,
        "freshkube://open?page=resources&kind=deployments.apps&ns=payments&name=gone-for-good",
    );
    let shown = card(cx, &view).expect("a card");
    assert_eq!(shown.outcome, LinkOutcome::NotFound);
    assert_eq!(shown.subject, "deployments.apps payments/gone-for-good");
    assert_eq!(opened(cx, &view), None);
    assert!(drawn(cx, handle, "link-outcome-not-found"));
    assert!(drawn(cx, handle, "link-outcome-close"));
    assert!(!drawn(cx, handle, "link-outcome-open-current"));
}

#[gpui_kit::test]
fn a_kind_the_cluster_does_not_serve_shows_not_served(cx: &mut TestAppContext) {
    let (_runtime, handle, view) = fixture(cx, 1280., 820.);
    open_text(
        cx,
        handle,
        &view,
        "freshkube://open?page=resources&kind=widgets.example.dev&ns=payments&name=w1",
    );
    let shown = card(cx, &view).expect("a card");
    assert_eq!(shown.outcome, LinkOutcome::NotServed);
    assert_eq!(shown.subject, "widgets.example.dev");
    assert!(drawn(cx, handle, "link-outcome-not-served"));
    cx.read(|cx| assert_eq!(view.read(cx).navigation.page, Page::Resources));
}

#[gpui_kit::test]
fn a_custom_kind_the_example_serves_opens(cx: &mut TestAppContext) {
    let (_runtime, handle, view) = fixture(cx, 1280., 820.);
    open_text(
        cx,
        handle,
        &view,
        "freshkube://open?page=resources&kind=certificates.cert-manager.io",
    );
    assert_eq!(card(cx, &view), None);
    assert_eq!(
        current(cx, &view)
            .and_then(|seen| seen.kind)
            .as_ref()
            .map(KindKey::as_str),
        Some("certificates.cert-manager.io")
    );
}

#[gpui_kit::test]
fn the_same_name_with_another_uid_shows_replaced_and_offers_the_current_one(
    cx: &mut TestAppContext,
) {
    let (_runtime, handle, view) = fixture(cx, 1280., 820.);
    let identity = deployment(cx, &view);
    assert_ne!(identity.uid, LINK_UID);
    open_text(
        cx,
        handle,
        &view,
        &link_for(&identity, &format!("&uid={LINK_UID}")),
    );
    let shown = card(cx, &view).expect("a card");
    assert_eq!(shown.outcome, LinkOutcome::Replaced);
    assert_eq!(
        shown.current.as_ref().map(|current| current.uid.clone()),
        Some(identity.uid.clone())
    );
    assert_eq!(opened(cx, &view), None);
    assert!(drawn(cx, handle, "link-outcome-replaced"));
    // Nothing opens the current one until the reader says so.
    cx.update_window(handle, |_, window, cx| {
        window.click("link-outcome-open-current", cx)
    })
    .unwrap();
    cx.run_until_parked();
    assert_eq!(opened(cx, &view), Some(identity));
    assert_eq!(card(cx, &view), None);
}

#[gpui_kit::test]
fn a_link_with_the_right_uid_opens_the_object(cx: &mut TestAppContext) {
    let (_runtime, handle, view) = fixture(cx, 1280., 820.);
    let identity = deployment(cx, &view);
    open_text(
        cx,
        handle,
        &view,
        &link_for(&identity, &format!("&uid={}", identity.uid)),
    );
    assert_eq!(card(cx, &view), None);
    assert_eq!(opened(cx, &view), Some(identity));
}

#[gpui_kit::test]
fn a_refused_read_shows_cant_read(cx: &mut TestAppContext) {
    let (handle, view) = shaped(cx, LinkShape::Refused);
    let identity = deployment(cx, &view);
    open_text(cx, handle, &view, &link_for(&identity, ""));
    assert_eq!(
        card(cx, &view).map(|card| card.outcome),
        Some(LinkOutcome::CantRead)
    );
    assert_eq!(opened(cx, &view), None);
    assert!(drawn(cx, handle, "link-outcome-cant-read"));
}

#[gpui_kit::test]
fn the_debug_shapes_force_their_outcome(cx: &mut TestAppContext) {
    for (shape, outcome) in [
        (LinkShape::NotFound, LinkOutcome::NotFound),
        (LinkShape::NotServed, LinkOutcome::NotServed),
        (LinkShape::Replaced, LinkOutcome::Replaced),
    ] {
        let (handle, view) = shaped(cx, shape);
        let identity = deployment(cx, &view);
        open_text(cx, handle, &view, &link_for(&identity, ""));
        assert_eq!(
            card(cx, &view).map(|card| card.outcome),
            Some(outcome),
            "{shape:?}"
        );
    }
}

#[gpui_kit::test]
fn closing_the_card_or_opening_something_else_clears_it(cx: &mut TestAppContext) {
    let (_runtime, handle, view) = fixture(cx, 1280., 820.);
    let missing =
        "freshkube://open?page=resources&kind=deployments.apps&ns=payments&name=gone-for-good";
    open_text(cx, handle, &view, missing);
    assert!(card(cx, &view).is_some());
    cx.update_window(handle, |_, window, cx| {
        window.click("link-outcome-close", cx)
    })
    .unwrap();
    cx.run_until_parked();
    assert_eq!(card(cx, &view), None);
    // Another kind drops it as well.
    open_text(cx, handle, &view, missing);
    assert!(card(cx, &view).is_some());
    open_text(
        cx,
        handle,
        &view,
        "freshkube://open?page=resources&kind=services",
    );
    assert_eq!(card(cx, &view), None);
}

#[gpui_kit::test]
fn a_link_that_opens_nothing_leaves_the_keyboard_on_the_list(cx: &mut TestAppContext) {
    let (_runtime, handle, view) = fixture(cx, 1280., 820.);
    open_text(
        cx,
        handle,
        &view,
        "freshkube://open?page=resources&kind=deployments.apps&ns=payments&name=gone-for-good",
    );
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let resources = view.read(cx).pages.resources.clone();
        assert!(resources.read(cx).list_focused(window));
    })
    .unwrap();
}

#[gpui_kit::test]
fn a_link_never_opens_a_dialog_a_shell_or_a_forward(cx: &mut TestAppContext) {
    let (_runtime, handle, view) = fixture(cx, 1280., 820.);
    let identity = deployment(cx, &view);
    let hostile = link_for(
        &identity,
        "&action=delete&dialog=confirm&shell=bash&forward=8080&route=operations&tab=logs",
    );
    open_text(cx, handle, &view, &hostile);
    cx.update_window(handle, |_, window, cx| {
        assert!(!window.has_active_dialog(cx));
        let pilot = view.read(cx);
        assert_ne!(pilot.navigation.page, Page::Operations);
        assert!(crate::resources::shell::running_anywhere(cx).is_empty());
    })
    .unwrap();
    assert!(cx.pending_prompt().is_none());
}

#[gpui_kit::test]
fn a_link_that_is_not_valid_opens_nothing_and_says_why(cx: &mut TestAppContext) {
    let (_runtime, handle, view) = fixture(cx, 1280., 820.);
    let notices = record_notices(cx, &view);
    let page = cx.read(|cx| view.read(cx).navigation.page);
    open_text(cx, handle, &view, "freshkube://open?page=settings");
    open_text(cx, handle, &view, "https://example.com/?page=overview");
    open_text(cx, handle, &view, "freshkube://open?v=9&page=overview");
    let said = notices.borrow().clone();
    assert_eq!(said.len(), 3, "{said:?}");
    assert!(said[0].starts_with("Can’t open this link:"), "{said:?}");
    assert!(said[1].starts_with("Can’t open this link:"), "{said:?}");
    assert_eq!(
        said[2],
        "This link was made by a newer Freshkube. Update to open it."
    );
    cx.read(|cx| assert_eq!(view.read(cx).navigation.page, page));
}

#[gpui_kit::test]
fn command_k_offers_one_open_link_result_for_a_pasted_link(cx: &mut TestAppContext) {
    let (_runtime, handle, view) = fixture(cx, 1500., 1050.);
    let identity = deployment(cx, &view);
    let text = link_for(&identity, "");
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("search-everything", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.input(&text, cx);
    })
    .unwrap();
    cx.run_until_parked();
    assert!(drawn(cx, handle, "search-open-link"));
    cx.update_window(handle, |_, window, cx| window.press("enter", cx))
        .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(!window.has_active_dialog(cx));
    })
    .unwrap();
    assert_eq!(opened(cx, &view), Some(identity));
}

#[gpui_kit::test]
fn command_k_offers_no_open_link_result_for_other_text(cx: &mut TestAppContext) {
    let (_runtime, handle, _view) = fixture(cx, 1500., 1050.);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("search-everything", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.input("deployments", cx);
    })
    .unwrap();
    cx.run_until_parked();
    assert!(!drawn(cx, handle, "search-open-link"));
}

#[gpui_kit::test]
fn tab_logs_opens_the_docks_logs_and_the_pane_on_details(cx: &mut TestAppContext) {
    let (_runtime, handle, view) = fixture(cx, 1280., 820.);
    let identity = deployment(cx, &view);
    cx.read(|cx| assert!(!view.read(cx).pages.dock.read(cx).has_tabs()));
    open_text(cx, handle, &view, &link_for(&identity, "&tab=logs"));
    cx.read(|cx| {
        let pilot = view.read(cx);
        assert!(
            pilot.pages.dock.read(cx).has_tabs(),
            "the logs open in the dock"
        );
        assert_ne!(
            pilot.pages.resources.read(cx).detail_tab(cx),
            resources::Tab::Logs
        );
    });
    assert_eq!(
        opened(cx, &view).map(|opened| opened.name),
        Some(identity.name)
    );
}

#[gpui_kit::test]
fn the_panes_copy_link_button_copies_the_open_objects_link(cx: &mut TestAppContext) {
    let (_runtime, handle, view) = fixture(cx, 1280., 820.);
    let identity = open_deployment(cx, handle, &view, resources::Tab::Overview);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("detail-copy-link", cx);
    })
    .unwrap();
    cx.run_until_parked();
    assert_eq!(
        clipboard(cx),
        tied(&link_for(&identity, &format!("&uid={}", identity.uid)))
    );
}

#[gpui_kit::test]
fn an_attention_rows_copy_link_names_where_open_goes(cx: &mut TestAppContext) {
    use crate::presentation::attention::Destination;
    let (_runtime, handle, view) = fixture(cx, 1280., 820.);
    let identity = deployment(cx, &view);
    let object = resources::model::ObjectRef::from(identity.clone());
    let notices = record_notices(cx, &view);
    for (destination, expected) in [
        (
            Destination::Object("deployments.apps", object, resources::Tab::Events),
            tied(&format!(
                "freshkube://open?page=resources&kind=deployments.apps&ns={}&name={}&uid={}&tab=events",
                identity.namespace, identity.name, identity.uid
            )),
        ),
        (
            Destination::Page(Page::Etcd),
            tied("freshkube://open?page=etcd"),
        ),
        (
            Destination::Service {
                node: "cp-1".into(),
                service: "etcd".into(),
                logs: true,
            },
            tied("freshkube://open?page=nodes&node=cp-1&ntab=logs"),
        ),
    ] {
        cx.update_window(handle, |_, window, cx| {
            view.update(cx, |pilot, cx| {
                pilot.open_destination(Destination::CopyLink(Box::new(destination)), window, cx)
            })
        })
        .unwrap();
        cx.run_until_parked();
        assert_eq!(clipboard(cx), expected);
    }
    // Copying opens nothing: the window is where it was.
    cx.read(|cx| assert_ne!(view.read(cx).navigation.page, Page::Etcd));
    assert_eq!(notices.borrow().len(), 3);
}

#[gpui_kit::test]
fn a_link_given_at_launch_opens_once_the_window_is_up(cx: &mut TestAppContext) {
    let options = GpuiOptions::fixture().with_open_link(Some("freshkube://open?page=etcd".into()));
    let (_runtime, _handle, view) = mount(cx, options, 1280., 820.);
    cx.run_until_parked();
    cx.read(|cx| assert_eq!(view.read(cx).navigation.page, Page::Etcd));
}

#[gpui_kit::test]
fn links_the_system_opens_us_with_open_as_they_arrive(cx: &mut TestAppContext) {
    let (sender, receiver) = futures::channel::mpsc::unbounded::<String>();
    // One that came before the window did waits in the channel.
    sender
        .unbounded_send("freshkube://open?page=health".into())
        .unwrap();
    let (_runtime, _handle, view) =
        mount(cx, GpuiOptions::fixture().with_urls(receiver), 1280., 820.);
    cx.run_until_parked();
    cx.read(|cx| assert_eq!(view.read(cx).navigation.page, Page::Health));
    sender
        .unbounded_send("freshkube://open?page=etcd".into())
        .unwrap();
    cx.run_until_parked();
    cx.read(|cx| assert_eq!(view.read(cx).navigation.page, Page::Etcd));
    // A bad one says so and moves nothing.
    let notices = record_notices(cx, &view);
    sender
        .unbounded_send("freshkube://open?page=settings".into())
        .unwrap();
    cx.run_until_parked();
    cx.read(|cx| assert_eq!(view.read(cx).navigation.page, Page::Etcd));
    assert_eq!(notices.borrow().len(), 1);
}

#[gpui_kit::test]
fn a_link_waits_for_a_cluster_to_read_and_only_so_many_wait(cx: &mut TestAppContext) {
    cx.executor().allow_parking();
    let directory = tempfile::tempdir().unwrap();
    let options = GpuiOptions::new(Some(directory.path().join("missing")), None, 100)
        .with_preferences(Some(directory.path().join("preferences.json")));
    let (_runtime, handle, view) = mount(cx, options, 1280., 820.);
    cx.read(|cx| assert!(view.read(cx).source().kube.is_none()));
    for _ in 0..6 {
        cx.update_window(handle, |_, window, cx| {
            view.update(cx, |pilot, cx| {
                pilot.open_external_link("freshkube://open?page=etcd".into(), window, cx)
            })
        })
        .unwrap();
    }
    cx.read(|cx| {
        let pilot = view.read(cx);
        assert_eq!(pilot.deep_link.waiting.len(), 4);
        assert_ne!(pilot.navigation.page, Page::Etcd);
    });
}
