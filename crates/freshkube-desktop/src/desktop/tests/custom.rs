use super::*;

#[gpui_kit::test]
fn custom_resources_are_discovered_on_expand_and_open_their_kinds(cx: &mut TestAppContext) {
    // The short window: opening the section or a group scrolls its rows in.
    let (_runtime, handle, view) = fixture(cx, 760., 560.);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(window.find("nav-k8s-group-custom").selected(), Some(false));
        assert!(view.read(cx).custom.read(cx).groups().is_none());

        // With no custom kind shown yet, the page stays while the column
        // discovers the groups.
        area(window, cx, "nav-k8s-group-custom");
        window.click("nav-collapse", cx);
        window.render_frame(cx);
        assert_eq!(window.find("nav-k8s-group-custom").selected(), Some(true));
        assert_eq!(view.read(cx).page, Page::Overview);
        assert!(shown_in_column(window, "nav-k8s-api-velero.io"));
        let group = "nav-k8s-api-cert-manager.io";
        assert_eq!(window.find(group).expanded(), Some(false));
        assert!(
            window
                .try_find("nav-k8s-api-cert-manager.io-status")
                .is_none()
        );

        reveal(window, cx, group);
        window.click(group, cx);
        window.render_frame(cx);
        window.render_frame(cx);
        assert_eq!(window.find(group).expanded(), Some(true));
        assert!(shown_in_column(window, "nav-k8s-issuers.cert-manager.io"));

        let certificates = "nav-k8s-certificates.cert-manager.io";
        reveal(window, cx, certificates);
        window.click(certificates, cx);
        window.render_frame(cx);
        assert_eq!(view.read(cx).page, Page::Resources);
        assert_eq!(
            view.read(cx).resource_kind.key(),
            "certificates.cert-manager.io"
        );
        assert_eq!(window.find(certificates).selected(), Some(true));
        assert_eq!(window.find("page-title").label(), Some("Certificate"));
        assert!(window.find("resource-list").visible());

        // Another area hides the column; Custom Resources then opens the
        // custom kind shown last. Opening a kind, as the keyboard or
        // FRESHKUBE_KIND would, shows its group.
        area(window, cx, "nav-overview");
        assert!(window.try_find(certificates).is_none());
        area(window, cx, "nav-k8s-group-custom");
        assert_eq!(view.read(cx).page, Page::Resources);
        assert_eq!(window.find(certificates).selected(), Some(true));
        let kind = crate::resources::example::kind("certificaterequests.cert-manager.io").unwrap();
        view.update(cx, |view, cx| view.open_kind(kind, window, cx));
        window.render_frame(cx);
        window.render_frame(cx);
        let requests = "nav-k8s-certificaterequests.cert-manager.io";
        assert!(shown_in_column(window, requests));
        assert_eq!(window.find(requests).selected(), Some(true));
        assert_ne!(window.find(certificates).selected(), Some(true));
        assert_eq!(
            window.find("page-title").label(),
            Some("CertificateRequest")
        );
    })
    .unwrap();
}

#[gpui_kit::test]
fn custom_groups_say_why_they_show_no_kinds(cx: &mut TestAppContext) {
    let (_runtime, handle, view) = fixture(cx, 1280., 1000.);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        view.update(cx, |view, cx| {
            view.show_custom(cx);
            for group in [
                "external.metrics.k8s.io",
                "metrics.k8s.io",
                "monitoring.coreos.com",
                "traefik.containo.us",
                "velero.io",
            ] {
                view.toggle_api_group(group, cx);
            }
        });
        window.render_frame(cx);
        for (id, label, retry) in [
            ("nav-k8s-api-velero.io-status", "Not permitted", true),
            // A group whose definition was removed after /apis listed it.
            (
                "nav-k8s-api-traefik.containo.us-status",
                "No longer served",
                true,
            ),
            (
                "nav-k8s-api-external.metrics.k8s.io-status",
                "Couldn't discover",
                true,
            ),
            (
                "nav-k8s-api-metrics.k8s.io-status",
                "Nothing to list",
                false,
            ),
            // Versions that failed while another was read.
            (
                "nav-k8s-api-monitoring.coreos.com-partial",
                "v1beta1 not served · v1alpha1 unreadable",
                true,
            ),
        ] {
            reveal(window, cx, id);
            let row = window.find(SharedString::from(id));
            assert_eq!(row.role(), Some(gpui_kit::Role::Status), "{id}");
            assert_eq!(row.label(), Some(label), "{id}");
            assert_eq!(
                window
                    .try_find(SharedString::from(format!("{id}-retry")))
                    .is_some(),
                retry,
                "{id}"
            );
        }
        assert!(
            window
                .try_find("nav-k8s-servicemonitors.monitoring.coreos.com")
                .is_some()
        );
        assert!(
            window
                .try_find("nav-k8s-api-cert-manager.io-status")
                .is_none()
        );
    })
    .unwrap();
}

#[gpui_kit::test]
fn custom_resources_follow_the_connection_and_wait_for_discovery(cx: &mut TestAppContext) {
    let (_runtime, handle, view) = fixture(cx, 760., 560.);
    let certificates = "nav-k8s-certificates.cert-manager.io";
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let kind = crate::resources::example::kind("certificates.cert-manager.io").unwrap();
        view.update(cx, |view, cx| view.open_kind(kind, window, cx));
        window.render_frame(cx);
        window.click("nav-collapse", cx);
        window.render_frame(cx);
        assert!(shown_in_column(window, certificates));
        // Another connection discovers again.
        window.press("alt-down", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(view.read(cx).applied.context.as_deref(), Some("staging-eu"));
        // The section and the group stay open, with the kinds found again.
        assert_eq!(window.find("nav-k8s-group-custom").selected(), Some(true));
        assert_eq!(
            window.find("nav-k8s-api-cert-manager.io").expanded(),
            Some(true)
        );
        assert!(window.try_find(certificates).is_some());

        // While the groups are read, the column says so and the reveal
        // waits for them, unless the column is scrolled by hand.
        let reopen = |view: &mut Pilot, cx: &mut gpui_kit::Context<Pilot>| {
            view.set_area(Area::Overview, cx);
            view.show_custom(cx);
        };
        view.update(cx, |view, cx| {
            view.custom
                .update(cx, |custom, _| custom.hold_reading(None));
            reopen(view, cx);
        });
        window.render_frame(cx);
        window.render_frame(cx);
        assert_eq!(
            window.find("nav-k8s-custom-status").label(),
            Some("Discovering…")
        );
        assert_eq!(view.read(cx).column_reveal, Some(ColumnReveal::Custom));
        window.scroll(
            "nav-column",
            gpui_kit::ScrollDelta::Pixels(gpui_kit::point(px(0.), px(40.))),
            cx,
        );
        assert_eq!(view.read(cx).column_reveal, None);

        view.update(cx, reopen);
        window.render_frame(cx);
        assert_eq!(view.read(cx).column_reveal, Some(ColumnReveal::Custom));
        view.update(cx, |view, cx| {
            view.custom.update(cx, |custom, cx| custom.retry(cx))
        });
    })
    .unwrap();
    // The column hears of the groups found when the shell does.
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.render_frame(cx);
        assert_eq!(view.read(cx).column_reveal, None);
        assert!(shown_in_column(window, "nav-k8s-api-velero.io"));
        assert!(window.try_find(certificates).is_some());
    })
    .unwrap();
}
