use super::*;

/// A live read's objects carry the connection's key; the footer names its
/// context instead.
#[gpui_kit::test]
fn the_footer_names_the_context_not_the_connection_key(cx: &mut TestAppContext) {
    use crate::applications::testing::change::Fetch;
    use crate::resources::ClusterAccess;
    use freshkube_delivery::change::live::Place;
    let (runtime, handle, view) = open(cx, 1280., 880.);
    cx.update_window(handle, |_, window, cx| {
        change(&view, cx).unwrap().update(cx, |page, _| {
            page.fetch = Fetch::Live {
                runtime: runtime.handle().clone(),
                access: ClusterAccess::EXAMPLE,
                secrets: None,
                setup_revision: 0,
                place: Box::new(Place {
                    // The open example cluster's key, as live objects carry.
                    cluster: "core-fra".into(),
                    label: "home-lab".into(),
                    project: "checkout".into(),
                    freight: "f".into(),
                    argocd_namespace: "argocd".into(),
                    mapping: Default::default(),
                    sources: Default::default(),
                    public_key_file: None,
                    transparency_log: false,
                    transparency_url: None,
                    evidence_result: None,
                    build_namespace: None,
                    stage_naming: None,
                }),
                kubeconfig: None,
            };
        });
        // Argo CD's Application, read on core-fra, the open cluster here.
        window.click(hop("prod-ams-argocd"), cx);
        window.render_frame(cx);
        let labels = change(&view, cx).unwrap().read(cx).action_labels();
        assert!(
            labels.iter().any(|label| label.ends_with("· home-lab")),
            "{labels:?}"
        );
        assert!(
            labels.iter().all(|label| !label.contains("core-fra")),
            "{labels:?}"
        );
    })
    .unwrap();
}

/// The change followed from prod-ams in the example reshaped as `variant`,
/// and its May be missing banner's label.
fn missing_in(variant: Variant, cx: &mut TestAppContext) -> String {
    let (_runtime, handle, view) = on_stage_of(cx, 1280., 880., variant);
    cx.update_window(handle, |_, window, cx| {
        window.press("f", cx);
        window.render_frame(cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(change(&view, cx).is_some(), "the change opens");
        assert!(window.try_find("change-failed").is_none());
        window
            .find("change-missing")
            .label()
            .expect("a label")
            .to_owned()
    })
    .unwrap()
}

#[gpui_kit::test]
fn argo_cd_refused_on_its_entry_is_a_banner_in_its_name(cx: &mut TestAppContext) {
    let missing = missing_in(Variant::ArgoRefused, cx);
    assert!(
        missing.starts_with("May be missing: Argo CD Applications were refused on core-fra: "),
        "{missing}"
    );
}

#[gpui_kit::test]
fn an_unreachable_destination_is_a_banner_and_reported_hops(cx: &mut TestAppContext) {
    let missing = missing_in(Variant::Destination, cx);
    assert_eq!(
        missing,
        "May be missing: Couldn't reach dev-fra: connection refused by dev-fra.example.test:6443"
    );
}

#[gpui_kit::test]
fn a_whole_example_has_no_banner(cx: &mut TestAppContext) {
    let (_runtime, handle, _view) = open(cx, 1280., 880.);
    cx.update_window(handle, |_, window, _| {
        assert!(window.try_find("change-missing").is_none());
    })
    .unwrap();
}

/// With Kargo's entry open, the trail's Kargo objects open on it.
#[gpui_kit::test]
fn with_cicd_fra_open_its_freight_opens_and_argo_cds_offer_a_switch(cx: &mut TestAppContext) {
    let (_runtime, handle, view) = open_on(cx, 1280., 880., "cicd-fra");
    cx.update_window(handle, |_, window, cx| {
        window.click(hop("freight"), cx);
        window.render_frame(cx);
        let page = change(&view, cx).unwrap();
        assert!(
            page.read(cx).action_why("Open in Resources").is_none(),
            "{:?}",
            page.read(cx).action_labels()
        );
        window.click(hop("prod-ams-argocd"), cx);
        window.render_frame(cx);
        let labels = page.read(cx).action_labels();
        assert!(
            labels.iter().any(|label| label.starts_with("Switch")),
            "{labels:?}"
        );
    })
    .unwrap();
}

/// The example's Stage `dev`, deploying to `cluster`, with `reach`.
fn dev_with(
    reach: freshkube_delivery::change::Reach,
    cluster: freshkube_delivery::change::Destination,
) -> freshkube_delivery::change::Stage {
    let change = freshkube_delivery::testing::change(chrono::Utc::now());
    let mut dev = change
        .stages
        .into_iter()
        .find(|stage| stage.name == "dev")
        .expect("the Stage dev");
    dev.reach = reach;
    dev.cluster = cluster;
    dev
}

/// A Stage's group row says how sure it is that its pods run the change
/// when that is less than Confirmed, with the weaker glyph.
#[test]
fn a_stage_group_row_names_a_reach_weaker_than_confirmed() {
    use freshkube_delivery::change::{Destination, Reach};
    use freshkube_delivery::join::Confidence;
    use freshkube_ui::ui::Tone;
    let cluster = || Destination::Cluster("dev-fra".into());
    let entry = || Destination::Entry {
        entry: "dev-fra".into(),
        via: "server https://dev-fra.example.test:6443".into(),
    };
    let line = |reach, cluster, rows| {
        crate::applications::testing::change::stage_line(
            "deploys to dev-fra",
            &dev_with(reach, cluster),
            rows,
        )
    };
    let words = |extra: Option<&str>| {
        let mut words = vec!["deploys to dev-fra".to_owned(), "verified".to_owned()];
        words.extend(extra.map(str::to_owned));
        words
    };

    // Confirmed changes nothing.
    assert_eq!(
        line(Reach::Linked(Confidence::Confirmed), cluster(), Tone::Good),
        (words(None), Tone::Good)
    );
    // Claimed is the blue dot over Good, and never hides a warning.
    assert_eq!(
        line(Reach::Linked(Confidence::Claimed), cluster(), Tone::Good),
        (words(Some("pods on dev-fra claimed")), Tone::Info)
    );
    assert_eq!(
        line(Reach::Linked(Confidence::Claimed), cluster(), Tone::Warn),
        (words(Some("pods on dev-fra claimed")), Tone::Warn)
    );
    // Unknown names the entry its destination is mapped to.
    assert_eq!(
        line(Reach::Linked(Confidence::Unknown), entry(), Tone::Good),
        (words(Some("pods on dev-fra unknown")), Tone::Unknown)
    );
    assert_eq!(
        line(Reach::Linked(Confidence::Unknown), entry(), Tone::Crit),
        (words(Some("pods on dev-fra unknown")), Tone::Crit)
    );
    // Without a destination there's no cluster to name.
    assert_eq!(
        line(
            Reach::Linked(Confidence::Unknown),
            Destination::Unknown("no Application found".into()),
            Tone::Good,
        )
        .0
        .last()
        .map(String::as_str),
        Some("pods unknown")
    );
    // Refused names what was refused.
    assert_eq!(
        line(
            Reach::Refused("Argo CD Applications".into()),
            cluster(),
            Tone::Good,
        ),
        (words(Some("Argo CD Applications refused")), Tone::Unknown)
    );
    // A Stage that runs other Freight keeps its rows' words and glyph.
    assert_eq!(
        line(Reach::NotReached, cluster(), Tone::Unknown),
        (words(None), Tone::Unknown)
    );
}

/// With dev-fra unreachable, dev's group row says its pods there are
/// unknown, with the Unknown ring, as the Changes list's cell does.
#[gpui_kit::test]
fn an_unreachable_destinations_stage_row_says_its_pods_are_unknown(cx: &mut TestAppContext) {
    let (_runtime, handle, view) = on_stage_of(cx, 1280., 880., Variant::Destination);
    cx.update_window(handle, |_, window, cx| {
        window.press("f", cx);
        window.render_frame(cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let page = change(&view, cx).expect("the change opens");
        let ix = page
            .read(cx)
            .groups
            .iter()
            .position(|group| group.label.as_ref() == "Stage dev")
            .expect("dev's group");
        assert_eq!(
            page.read(cx).groups[ix].tone,
            freshkube_ui::ui::Tone::Unknown
        );
        assert_eq!(
            window
                .find(SharedString::from(format!("change-group-{ix}")))
                .label(),
            Some("Stage dev · deploys to dev-fra (mapped) · verified · pods on dev-fra unknown")
        );
    })
    .unwrap();
}

/// A Stage whose rows are fine but whose pods are only claimed to run the
/// change doesn't fold as fine.
#[gpui_kit::test]
fn a_stage_whose_pods_are_only_claimed_never_folds(cx: &mut TestAppContext) {
    let (_runtime, handle, view) = open(cx, 1280., 880.);
    cx.update_window(handle, |_, _, cx| {
        let page = change(&view, cx).unwrap();
        page.update(cx, |page, _| {
            let mut change = page.change.clone();
            change
                .stages
                .iter_mut()
                .find(|stage| stage.name == "stage")
                .expect("the Stage stage")
                .reach = freshkube_delivery::change::Reach::Linked(
                freshkube_delivery::join::Confidence::Claimed,
            );
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
        assert!(
            !lines.iter().any(|line| line.starts_with("dev-")),
            "dev still folds"
        );
    })
    .unwrap();
}
