use super::*;

/// An entry the workspace gains while the change page shows reaches its
/// links at once: prod-ams's Deployment goes from greyed out to Switch and
/// open without the page being opened again.
#[gpui_kit::test]
fn a_workspace_change_reaches_the_change_page_at_once(cx: &mut TestAppContext) {
    let (_runtime, handle, view) = open_with(cx, &["dev-fra"], false);
    let labels = |view: &Entity<Pilot>, cx: &mut TestAppContext| {
        cx.update_window(handle, |_, window, cx| {
            window.click(hop("prod-ams-pods"), cx);
            window.render_frame(cx);
            change(view, cx).unwrap().read(cx).action_labels()
        })
        .unwrap()
    };
    let before = labels(&view, cx);
    assert!(
        before.iter().all(|label| !label.starts_with("Switch")),
        "{before:?}"
    );
    cx.update(|cx| {
        list(&view, cx).update(cx, |page, cx| {
            page.set_clusters(
                crate::applications::Clusters {
                    entries: vec!["dev-fra".into(), "prod-ams".into()],
                    active: Some("dev-fra".into()),
                    mapping: Default::default(),
                    ..Default::default()
                },
                cx,
            )
        })
    });
    let after = labels(&view, cx);
    assert!(
        after.contains(&"Switch and open the Deployment · prod-ams".to_owned()),
        "{after:?}"
    );
}

/// In example data the workspace lists prod-ams, whose example cluster
/// holds the checkout-api Deployment its Stage reports: Switch and open
/// asks, switches there, and ends on the Deployment's Overview.
#[gpui_kit::test]
fn switch_and_open_lands_on_prod_amss_deployment_in_example_data(cx: &mut TestAppContext) {
    let (_runtime, handle, view) = open(cx, 1280., 880.);
    let heard = crate::desktop::testing::record_notices(cx, &view);
    cx.update_window(handle, |_, window, cx| {
        window.click(hop("prod-ams-pods"), cx);
        window.render_frame(cx);
        let label = "Switch and open the Deployment · prod-ams";
        assert_eq!(change(&view, cx).unwrap().read(cx).action_why(label), None);
        window.click(action(&view, label, cx), cx);
        window.render_frame(cx);
    })
    .unwrap();
    cx.run_until_parked();
    let (question, _) = cx.pending_prompt().expect("asked before switching");
    assert_eq!(
        question,
        "Switch to prod-ams to open Deployment checkout-api?"
    );
    cx.simulate_prompt_answer("Switch");
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let pilot = view.read(cx);
        assert_eq!(pilot.applications().1, Page::Resources);
        let opened = pilot.opened_object(cx).expect("the Deployment opened");
        assert_eq!(
            (
                opened.connection.as_str(),
                opened.resource.as_str(),
                opened.namespace.as_str(),
                opened.name.as_str()
            ),
            (
                crate::resources::example::connection("prod-ams").as_str(),
                "deployments.apps",
                "checkout",
                "checkout-api"
            )
        );
        assert_eq!(pilot.opened_tab(cx), crate::resources::Tab::Overview);
        assert!(
            heard.borrow().is_empty(),
            "nothing to say: {:?}",
            heard.borrow()
        );
    })
    .unwrap();
}

/// An object on another workspace entry's cluster offers Switch and open,
/// in the Inspector and the row menu, and asks the shell to switch.
#[gpui_kit::test]
fn an_object_on_another_workspace_entry_switches_and_opens(cx: &mut TestAppContext) {
    use crate::applications::testing::change::ChangeEvent;
    use crate::resources::ResourceLink;
    use std::{cell::RefCell, rc::Rc};

    let (_runtime, handle, view) = open_with(cx, &["dev-fra", "prod-ams"], false);
    let asked: Rc<RefCell<Vec<(String, ResourceLink)>>> = Rc::default();
    let page = cx.update(|cx| change(&view, cx).unwrap());
    let _subscription = cx.update(|cx| {
        let asked = asked.clone();
        cx.subscribe(&page, move |_, event: &ChangeEvent, _| {
            if let ChangeEvent::Switch(entry, link) = event {
                asked
                    .borrow_mut()
                    .push((entry.to_string(), (**link).clone()));
            }
        })
    });
    cx.update_window(handle, |_, window, cx| {
        window.click(hop("prod-ams-pods"), cx);
        window.render_frame(cx);
        let labels = page.read(cx).action_labels();
        let label = labels
            .iter()
            .find(|label| label.starts_with("Switch and open"))
            .unwrap_or_else(|| panic!("{labels:?}"))
            .clone();
        assert!(label.ends_with("· prod-ams"), "{label}");
        assert_eq!(page.read(cx).action_why(&label), None);
        window.click(action(&view, &label, cx), cx);
        window.render_frame(cx);
        window.right_click(hop("prod-ams-pods"), cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let menu = window.within("popup-menu");
        assert_eq!(
            menu.find(0usize).label(),
            Some("Switch to prod-ams and open")
        );
    })
    .unwrap();
    let asked = asked.borrow();
    let [(entry, ResourceLink::Object(_, object, _))] = asked.as_slice() else {
        panic!("{asked:?}");
    };
    assert_eq!(entry, "prod-ams");
    assert!(!object.name.is_empty());
}

/// Kargo is read on `cicd-fra`: with it open, O opens the Freight there.
#[gpui_kit::test]
fn o_opens_the_freight_in_resources(cx: &mut TestAppContext) {
    let (_runtime, handle, view) = open_on(cx, 1280., 880., "cicd-fra");
    cx.update_window(handle, |_, window, cx| {
        window.click(hop("freight"), cx);
        window.render_frame(cx);
        window.press("o", cx);
        window.render_frame(cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(view.read(cx).applications().1, Page::Resources);
        let identity = view.read(cx).opened_object(cx).expect("opened");
        assert_eq!(identity.resource, "freights.kargo.akuity.io");
        assert_eq!(identity.address(), "checkout/wonky-otter");
        // The rail's Applications comes back to the change.
        window.click("nav-applications", cx);
        window.render_frame(cx);
        assert!(window.find("change-page").visible());
    })
    .unwrap();
}

/// The pods of a Stage live on its own cluster, which no example context
/// opens: the button is greyed out and the menu item too.
#[gpui_kit::test]
fn a_hop_on_a_cluster_that_is_not_open_does_not_open(cx: &mut TestAppContext) {
    let (_runtime, handle, view) = open(cx, 1280., 880.);
    cx.update_window(handle, |_, window, cx| {
        window.click(hop("prod-lon-pods"), cx);
        window.render_frame(cx);
        window.click("change-action-0", cx);
        window.render_frame(cx);
        window.right_click(hop("prod-lon-pods"), cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let menu = window.within("popup-menu");
        assert_eq!(menu.find(0usize).label(), Some("Open in Resources"));
        window.within("popup-menu").click(0usize, cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(view.read(cx).applications().1, Page::Applications);
        assert!(view.read(cx).opened_object(cx).is_none());
        assert!(change(&view, cx).is_some());
    })
    .unwrap();
}
