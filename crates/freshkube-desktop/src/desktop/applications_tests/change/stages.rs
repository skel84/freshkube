use super::*;

#[gpui_kit::test]
fn hops_come_in_travel_order_with_fine_stages_folded(cx: &mut TestAppContext) {
    let (_runtime, handle, view) = open(cx, 1280., 880.);
    cx.update_window(handle, |_, window, cx| {
        let page = change(&view, cx).unwrap();
        assert_eq!(
            page.read(cx).lines_text(),
            [
                "# Build",
                "pr",
                "pr-run",
                "push-run",
                "signature",
                "policy",
                "image",
                "# Freight",
                "freight",
                // dev and stage are fine, so they fold while prod needs a
                // look; travel order stays.
                "# Stage dev",
                "# Stage stage",
                "# Stage prod-ams",
                "prod-ams-eligible",
                "prod-ams-promotion",
                "prod-ams-verification",
                "prod-ams-argocd",
                "prod-ams-pods",
                "# Stage prod-lon",
                "prod-lon-eligible",
                "prod-lon-promotion",
                "prod-lon-verification",
                "prod-lon-argocd",
                "prod-lon-deployment",
                "prod-lon-pods",
            ]
        );
        assert!(window.find("change-showing").visible());
        assert_eq!(
            window.find("change-hop-pr-run-link").label(),
            Some("Claimed")
        );
        assert_eq!(
            window.find("change-hop-prod-ams-pods-link").label(),
            Some("Unknown")
        );
        // A gate has no link: it isn't joined to anything.
        assert!(
            window
                .try_find("change-hop-prod-ams-promotion-link")
                .is_none()
        );
        window.click("change-show-all", cx);
        window.render_frame(cx);
        let lines = page.read(cx).lines_text();
        assert_eq!(
            lines.iter().filter(|line| !line.starts_with('#')).count(),
            30
        );
        assert!(window.try_find("change-showing").is_none());
    })
    .unwrap();
}

/// A Stage known only from what Argo CD reports on another workspace
/// cluster stays open, however fine Argo CD says it is.
#[gpui_kit::test]
fn a_stage_mapped_to_another_cluster_never_folds(cx: &mut TestAppContext) {
    let (_runtime, handle, view) = open(cx, 1280., 880.);
    cx.update_window(handle, |_, _, cx| {
        let page = change(&view, cx).unwrap();
        page.update(cx, |page, _| {
            let mut change = page.change.clone();
            let stage = change
                .stages
                .iter_mut()
                .find(|stage| stage.name == "stage")
                .expect("the Stage stage");
            stage.cluster = freshkube_delivery::change::Destination::Entry {
                entry: "stage-fra".into(),
                via: "server https://stage-fra.example.test:6443".into(),
            };
            page.rows.clear();
            page.show(change);
        });
        let lines = page.read(cx).lines_text();
        let at = lines
            .iter()
            .position(|line| line == "# Stage stage")
            .expect("its group");
        assert!(
            lines.get(at + 1).is_some_and(|line| !line.starts_with('#')),
            "its rows show: {lines:#?}"
        );
        assert!(lines.contains(&"# Stage dev".to_owned()));
        assert!(
            !lines.iter().any(|line| line.starts_with("dev-")),
            "dev still folds"
        );
    })
    .unwrap();
    // Its chevron still folds it by hand. Group 3 is the Stage stage.
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("change-fold-3", cx);
        window.render_frame(cx);
        let lines = change(&view, cx).unwrap().read(cx).lines_text();
        let at = lines
            .iter()
            .position(|line| line == "# Stage stage")
            .unwrap();
        assert!(
            lines.get(at + 1).is_none_or(|line| line.starts_with('#')),
            "folded by hand: {lines:#?}"
        );
    })
    .unwrap();
}

#[gpui_kit::test]
fn a_fold_opens_and_closes_its_stage(cx: &mut TestAppContext) {
    let (_runtime, handle, view) = open(cx, 1280., 880.);
    cx.update_window(handle, |_, window, cx| {
        let page = change(&view, cx).unwrap();
        // Group 2 is the Stage dev.
        window.click("change-fold-2", cx);
        window.render_frame(cx);
        assert!(page.read(cx).lines_text().contains(&"dev-pods".to_owned()));
        window.click("change-fold-2", cx);
        window.render_frame(cx);
        assert!(!page.read(cx).lines_text().contains(&"dev-pods".to_owned()));
    })
    .unwrap();
}

#[gpui_kit::test]
fn a_chip_shows_only_its_hops_in_every_stage(cx: &mut TestAppContext) {
    let (_runtime, handle, view) = open(cx, 1280., 880.);
    cx.update_window(handle, |_, window, cx| {
        let page = change(&view, cx).unwrap();
        window.click("change-tally-failing", cx);
        window.render_frame(cx);
        assert_eq!(
            page.read(cx).lines_text(),
            ["# Stage prod-lon", "prod-lon-verification"]
        );
        window.click("change-tally-ok", cx);
        window.render_frame(cx);
        let lines = page.read(cx).lines_text();
        // Folded Stages show their ok rows under a filter, and the hand
        // approval's blue dot counts as ok.
        assert!(lines.contains(&"dev-pods".to_owned()), "{lines:?}");
        assert!(lines.contains(&"prod-lon-eligible".to_owned()), "{lines:?}");
        assert!(
            !lines.contains(&"prod-ams-promotion".to_owned()),
            "{lines:?}"
        );
        window.click("change-tally-ok", cx);
        window.render_frame(cx);
        assert!(
            page.read(cx)
                .lines_text()
                .contains(&"# Stage dev".to_owned())
        );
        assert!(!page.read(cx).lines_text().contains(&"dev-pods".to_owned()));
    })
    .unwrap();
}
