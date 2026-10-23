use super::*;

/// The change page opens on prod-ams waiting for promotion, with the
/// Stages that are fully fine folded; a row shows its hop, and an action
/// says where it would lead, naming the cluster.
#[gpui_kit::test]
fn the_change_story_opens_on_the_waiting_stage_and_shows_each_hop(cx: &mut TestAppContext) {
    use super::stories::change::{ChangeStory, FIRST};
    let (handle, workbench) = open(cx, super::stories::find("change-trail").unwrap());
    let story = cx.update(|cx| story::<ChangeStory>(&workbench, cx));
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(story.read(cx).selected(), Some(FIRST));
        assert_eq!(
            window.find("change-trail-detail-title").label(),
            Some("Kargo Stage prod-ams: Waiting for promotion")
        );
        let shown = story.read(cx).shown();
        assert!(!shown.contains(&"dev-pods") && !shown.contains(&"stage-pods"));
        assert!(shown.contains(&"prod-ams-pods") && shown.contains(&"prod-fra-pods"));

        // The pruned run's logs can't be opened: pressing does nothing.
        window.click("change-trail-hop-pr-run", cx);
        window.render_frame(cx);
        assert_eq!(story.read(cx).selected(), Some("pr-run"));
        assert_eq!(
            window.find("change-trail-detail-title").label(),
            Some("PipelineRun checkout-pr-418-m2q8: Pruned")
        );
        window.click("change-trail-action-0", cx);
        assert!(story.read(cx).opened().is_none());

        // The pods prod-ams won't list open in Resources on prod-ams.
        window.click("change-trail-hop-prod-ams-pods", cx);
        window.render_frame(cx);
        window.click("change-trail-action-0", cx);
        assert_eq!(
            story.read(cx).opened().map(|text| text.as_ref()),
            Some("Would open the pods in Resources on prod-ams.")
        );
    })
    .unwrap();
}

/// A Stage's chevron folds and unfolds it, Show all unfolds every Stage,
/// and a status chip shows only its rows, in every group.
#[gpui_kit::test]
fn the_change_story_folds_stages_and_filters_by_status(cx: &mut TestAppContext) {
    use super::stories::change::ChangeStory;
    let (handle, workbench) = open(cx, super::stories::find("change-trail").unwrap());
    let story = cx.update(|cx| story::<ChangeStory>(&workbench, cx));
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let folded = story.read(cx).shown().len();
        // prod-ams is group 4.
        window.click("change-trail-fold-4", cx);
        window.render_frame(cx);
        assert!(!story.read(cx).shown().contains(&"prod-ams-pods"));
        window.click("change-trail-fold-4", cx);
        window.render_frame(cx);
        assert_eq!(story.read(cx).shown().len(), folded);

        window.click("change-trail-hops-show-all", cx);
        window.render_frame(cx);
        assert_eq!(story.read(cx).shown().len(), 27);
        assert!(window.try_find("change-trail-hops-show-all").is_none());

        window.click("change-trail-tally-failing", cx);
        window.render_frame(cx);
        assert_eq!(story.read(cx).shown(), vec!["prod-fra-verification"]);
        // A Freight approved by hand past stage has the blue dot, and
        // counts as ok.
        window.click("change-trail-tally-failing", cx);
        window.click("change-trail-tally-ok", cx);
        window.render_frame(cx);
        assert!(story.read(cx).shown().contains(&"prod-fra-eligible"));
        window.click("change-trail-tally-ok", cx);
        window.click("change-trail-tally-failing", cx);
        window.render_frame(cx);
        window.click("change-trail-tally-waiting", cx);
        window.render_frame(cx);
        assert_eq!(
            story.read(cx).shown(),
            vec![
                "pr-run",
                "prod-ams-promotion",
                "prod-ams-verification",
                "prod-ams-pods"
            ]
        );
        window.click("change-trail-tally-waiting", cx);
        window.render_frame(cx);
        assert_eq!(story.read(cx).shown().len(), 27);
    })
    .unwrap();
}

#[gpui_kit::test]
fn the_change_chain_controls_and_keys_show_each_case(cx: &mut TestAppContext) {
    use super::stories::change::chain::ChainStory;
    use freshkube_ui::graph::{Cursor, GraphSource};
    let (handle, workbench) = open(cx, super::stories::find("change-chain").unwrap());
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let story = workbench
            .read(cx)
            .view()
            .clone()
            .downcast::<ChainStory>()
            .unwrap();
        for case in [
            "confirmed",
            "claimed",
            "unknown",
            "refused",
            "missing",
            "stages",
        ] {
            window.click(format!("change-chain-case-{case}"), cx);
            window.render_frame(cx);
            let focus = story.read(cx).graph().focus_handle().unwrap().clone();
            window.focus(&focus, cx);
            window.press("down", cx);
            assert_eq!(story.read(cx).cursor(), Some(Cursor::Node(0)));
            window.press("right", cx);
            assert_eq!(story.read(cx).cursor(), Some(Cursor::Link(0, 1)));
            window.render_frame(cx);
            assert!(window.try_find("change-chain-detail").is_some());
        }
    })
    .unwrap();
}
