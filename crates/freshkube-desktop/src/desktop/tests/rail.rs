use super::*;

#[gpui_kit::test]
fn opening_a_kind_scrolls_its_group_into_the_short_sidebar(cx: &mut TestAppContext) {
    let (_runtime, handle, view) = fixture(cx, 760., 560.);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let key = "validatingadmissionpolicybindings.admissionregistration.k8s.io";
        let nav = format!("nav-k8s-{key}");
        assert!(window.try_find(SharedString::from(nav.clone())).is_none());
        // As the keyboard or FRESHKUBE_KIND would open it.
        view.update(cx, |view, cx| view.open_builtin(key, window, cx));
        window.render_frame(cx);
        window.render_frame(cx);
        assert!(shown_in_column(window, &nav));
        assert_eq!(window.find(SharedString::from(nav)).checked(), Some(true));
        assert_eq!(
            window.find("page-title").label(),
            Some("Validating Admission Policy Bindings")
        );

        // A group's rail button opens its first kind, with the group's kinds.
        view.update(cx, |view, cx| view.open_builtin("pods", window, cx));
        window.render_frame(cx);
        window.render_frame(cx);
        assert!(shown_in_column(window, "nav-k8s-pods"));
        area(window, cx, "nav-k8s-group-storage");
        window.render_frame(cx);
        assert_eq!(
            view.read(cx).navigation.resource_kind.key(),
            "persistentvolumeclaims"
        );
        reveal(window, cx, "nav-k8s-csinodes.storage.k8s.io");
    })
    .unwrap();
}

#[gpui_kit::test]
fn rail_areas_list_their_kinds_and_open_the_last_one(cx: &mut TestAppContext) {
    let (_runtime, handle, view) = fixture(cx, 1280., 1000.);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        // Overview has no column; a group's rail button shows its kinds.
        assert_eq!(window.find("nav-overview").selected(), Some(true));
        assert!(window.try_find("nav-column").is_none());
        let workloads = window.find("nav-k8s-group-workloads");
        assert_eq!(workloads.role(), Some(gpui_kit::Role::Tab));
        assert_eq!(workloads.selected(), Some(false));
        area(window, cx, "nav-k8s-group-workloads");
        assert_eq!(view.read(cx).navigation.resource_kind.key(), "pods");
        assert_eq!(
            window.find("nav-k8s-group-workloads").selected(),
            Some(true)
        );
        assert!(window.try_find("nav-k8s-deployments.apps").is_some());
        assert!(window.try_find("nav-k8s-services").is_none());

        area(window, cx, "nav-k8s-group-networking");
        assert_eq!(view.read(cx).navigation.page, Page::Resources);
        assert_eq!(view.read(cx).navigation.resource_kind.key(), "services");
        assert_eq!(window.find("nav-k8s-services").selected(), Some(true));
        assert_eq!(
            window.find("nav-k8s-group-workloads").selected(),
            Some(false)
        );
        assert!(window.find("resource-list").visible());
        assert_eq!(window.find("page-title").label(), Some("Services"));

        // Each group opens the kind it showed last.
        let ingresses = "nav-k8s-ingresses.networking.k8s.io";
        window.click(ingresses, cx);
        window.render_frame(cx);
        area(window, cx, "nav-k8s-group-workloads");
        assert_eq!(view.read(cx).navigation.resource_kind.key(), "pods");
        area(window, cx, "nav-k8s-group-networking");
        assert_eq!(
            view.read(cx).navigation.resource_kind.key(),
            "ingresses.networking.k8s.io"
        );
        assert_eq!(window.find(ingresses).selected(), Some(true));

        // Namespaces and Events are kinds of their own, with no column.
        window.click("nav-k8s-events", cx);
        window.render_frame(cx);
        assert_eq!(view.read(cx).navigation.resource_kind.key(), "events");
        assert_eq!(window.find("nav-k8s-events").selected(), Some(true));
        assert!(window.try_find("nav-column").is_none());

        // Other pages hide the table, and coming back shows the last kind.
        window.press("secondary-1", cx);
        window.render_frame(cx);
        assert!(window.try_find("resources-page").is_none());
        for _ in 0..5 {
            window.press("ctrl-tab", cx);
            window.render_frame(cx);
        }
        assert_eq!(view.read(cx).navigation.page, Page::Resources);
        assert_eq!(
            view.read(cx).navigation.resource_kind.key(),
            "ingresses.networking.k8s.io"
        );
        assert!(window.find("resources-page").visible());
        assert_eq!(
            window.find("nav-k8s-group-networking").selected(),
            Some(true)
        );
    })
    .unwrap();
}

#[gpui_kit::test]
fn the_rail_marks_areas_the_overview_finds_problems_in(cx: &mut TestAppContext) {
    let (_runtime, handle, view) = fixture(cx, 1280., 1000.);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let marks = &view.read(cx).navigation.rail_marks;
        // The example cluster has NotReady nodes and a node at 92% memory,
        // High: both warn.
        assert_eq!(marks.tone(Area::Nodes), Some(crate::ui::Tone::Warn));
        assert_eq!(marks.tone(Area::ControlPlane), Some(crate::ui::Tone::Warn));
        assert_eq!(
            marks.tone(Area::Group("workloads")),
            Some(crate::ui::Tone::Warn)
        );
        assert_eq!(marks.tone(Area::Overview), None);
        assert_eq!(marks.tone(Area::Group("networking")), None);
    })
    .unwrap();
}
