use super::*;

/// dev-fra's pods run the change: Compare opens Observability's
/// Deployments on Coroot's revision of checkout-api with the hash of the
/// ReplicaSet that runs it.
#[gpui_kit::test]
fn a_deployments_revision_opens_in_observability_by_its_hash(cx: &mut TestAppContext) {
    use freshkube_delivery::change::example::POD_HASH;
    let (_runtime, handle, view) = open_on_dev(cx);
    cx.update_window(handle, |_, window, cx| {
        // dev is fine, so it folds while prod needs a look.
        window.click("change-fold-2", cx);
        window.render_frame(cx);
        window.click(hop("dev-deployment"), cx);
        window.render_frame(cx);
        let compare = action(&view, "Compare in Observability · dev-fra", cx);
        let page = change(&view, cx).unwrap();
        assert_eq!(
            page.read(cx)
                .action_why("Compare in Observability · dev-fra"),
            None
        );
        window.click(compare, cx);
        window.render_frame(cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(view.read(cx).applications().1, Page::Observability);
        let page = view.read(cx).observability();
        let page = page.read(cx);
        assert_eq!(
            page.destination(),
            crate::observability::Destination::Deployments
        );
        assert_eq!(
            page.shown_revision(),
            (
                Some("fixture:checkout:Deployment:checkout-api".into()),
                Some(POD_HASH.into())
            )
        );
        assert!(window.find("obs-wanted-revision").visible());
    })
    .unwrap();
}

/// A switch after the list has read still makes dev-fra the open cluster
/// on the change page, as on the application page.
#[gpui_kit::test]
fn a_switch_after_the_read_opens_the_entrys_cluster_on_the_change_page(cx: &mut TestAppContext) {
    let (_runtime, handle, view) = open_switched(cx, true);
    cx.update_window(handle, |_, window, cx| {
        window.click("change-fold-2", cx);
        window.render_frame(cx);
        window.click(hop("dev-deployment"), cx);
        window.render_frame(cx);
        let page = change(&view, cx).unwrap();
        assert_eq!(
            page.read(cx)
                .action_why("Compare in Observability · dev-fra"),
            None
        );
        assert_eq!(
            page.read(cx)
                .action_why("Open the Deployment in Resources · dev-fra"),
            None
        );
    })
    .unwrap();
}

/// On a cluster that isn't open, Compare is greyed out with why, and
/// opens nothing.
#[gpui_kit::test]
fn a_revision_on_a_cluster_that_is_not_open_does_not_open(cx: &mut TestAppContext) {
    let (_runtime, handle, view) = open(cx, 1280., 880.);
    cx.update_window(handle, |_, window, cx| {
        // dev is fine, so it folds while prod needs a look.
        window.click("change-fold-2", cx);
        window.render_frame(cx);
        window.click(hop("dev-deployment"), cx);
        window.render_frame(cx);
        let label = "Compare in Observability · dev-fra";
        let compare = action(&view, label, cx);
        let page = change(&view, cx).unwrap();
        assert_eq!(
            page.read(cx).action_why(label).as_deref(),
            Some("dev-fra isn't the open cluster, so Observability doesn't read its revisions")
        );
        window.click(compare, cx);
        window.render_frame(cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.read(|cx| assert_eq!(view.read(cx).applications().1, Page::Applications));
}

/// The change, followed from prod-ams with dev-fra's workspace entry open,
/// so dev-fra stands for the open example cluster.
fn open_on_dev(
    cx: &mut TestAppContext,
) -> (tokio::runtime::Runtime, AnyWindowHandle, Entity<Pilot>) {
    open_switched(cx, false)
}

/// The same, with dev-fra's entry opened before the list reads or, with
/// `after_read`, once it has read with no entry open.
fn open_switched(
    cx: &mut TestAppContext,
    after_read: bool,
) -> (tokio::runtime::Runtime, AnyWindowHandle, Entity<Pilot>) {
    open_with(cx, &["dev-fra"], after_read)
}
