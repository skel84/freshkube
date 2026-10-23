use super::*;

#[gpui_kit::test]
fn each_source_state_keeps_its_chain_and_note(cx: &mut TestAppContext) {
    let (handle, page) = fixture(cx);
    cx.update_window(handle, |_, window, cx| {
        for shape in [
            Shape::Whole,
            Shape::Spread,
            Shape::ArgoRefused,
            Shape::DestinationUnreachable,
        ] {
            let change = freshkube_delivery::changes::example::change_of(
                "checkout",
                "wonky-otter",
                Utc::now(),
            )
            .unwrap();
            let change = shape.apply(change).unwrap();
            page.update(cx, |page, cx| page.read_answer(change, window, cx));
            chain(&page, window, cx);
            let p = page.read(cx);
            for row in &p.rows {
                assert!(p.trail.cards.contains_key(&row.key));
            }
            let cards = p
                .change
                .hops
                .iter()
                .filter(|hop| !matches!(hop.shows, Shows::Stage(_)))
                .count()
                + p.change.stages.len();
            assert_eq!(
                p.graph.nodes().len(),
                cards,
                "no downstream card is invented"
            );
            for stage in &p.change.stages {
                assert!(
                    p.graph
                        .nodes()
                        .iter()
                        .any(|node| node.label.as_ref() == stage.name)
                );
            }
            if shape != Shape::Whole {
                assert!(window.try_find("change-missing").is_some());
            }
        }
    })
    .unwrap();
}

#[gpui_kit::test]
fn missing_policy_and_an_anchored_unserved_source_are_unknown_in_both_views(
    cx: &mut TestAppContext,
) {
    use freshkube_delivery::change::{Gap, Missing};
    let (handle, page) = fixture(cx);
    cx.update_window(handle, |_, window, cx| {
        let mut change = page.read(cx).change.clone();
        let policy = change
            .hops
            .iter_mut()
            .find(|hop| hop.key == "policy")
            .unwrap();
        policy.state = HealthIndicator::Unknown;
        policy.detail = "Not found".into();
        policy.link = Some(Confidence::Unknown);
        if let Shows::Hop(detail) = &mut policy.shows {
            detail.state = "No policy result reported".into();
            detail.link = None;
        }
        change.missing.push(
            Missing::new(Gap::NotServed, "Signature results not served on cluster-a")
                .after("push-run"),
        );
        page.update(cx, |page, cx| page.read_answer(change, window, cx));
        chain(&page, window, cx);
        let p = page.read(cx);
        assert!(p.counts[3] >= 1, "the start can't be confirmed");
        assert!(
            p.graph
                .nodes()
                .iter()
                .find(|n| n.key == "policy")
                .unwrap()
                .ghost
        );
        assert_eq!(
            p.trail
                .edges
                .iter()
                .find(|e| e.to == "policy")
                .unwrap()
                .style,
            EdgeStyle::Dotted
        );
        let branch = p
            .trail
            .edges
            .iter()
            .find(|e| e.to == "policy")
            .unwrap()
            .clone();
        assert_eq!(branch.from.as_ref(), "signature");
        assert!(!p.rows.iter().any(|row| row.key == "missing-0"));
        assert!(p.rows.iter().any(|r| r.key == branch.to));
        page.read(cx).graph.reveal_node(&branch.to, window);
        window.render_frame(cx);
        window.click(SharedString::from(format!("change-card-{}", branch.to)), cx);
        assert_eq!(page.read(cx).selected(), Some(&branch.to));
        page.update(cx, |page, cx| page.set_view(ChangeView::List, window, cx));
        window.render_frame(cx);
        assert!(
            window
                .find(SharedString::from(format!("change-hop-{}", branch.to)))
                .visible()
        );
    })
    .unwrap();
}

#[gpui_kit::test]
fn chain_uses_loading_rows_failure_retry_empty_and_stale_states(cx: &mut TestAppContext) {
    let (handle, page) = open(cx, 1280., 880., Shape::Whole, Duration::from_secs(2), 13.);
    cx.update_window(handle, |_, window, cx| {
        chain(&page, window, cx);
        assert!(window.try_find("change-loading").is_some());
        assert!(window.try_find("change-chain").is_none());
        page.update(cx, |page, cx| {
            page.fail_read("cluster-a refused", window, cx)
        });
        window.render_frame(cx);
        assert!(window.try_find("change-failed").is_some());
        for _ in 0..20 {
            if window.find("change-retry").focused() == Some(true) {
                break;
            }
            window.press("tab", cx);
        }
        assert_eq!(window.find("change-retry").focused(), Some(true));
        window.press("enter", cx);
        assert!(
            page.read(cx).is_reading(),
            "Enter activates the focused Retry button"
        );
    })
    .unwrap();
    cx.executor().advance_clock(Duration::from_secs(2));
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(window.try_find("change-chain").is_some());
        page.update(cx, |page, cx| page.focus(window, cx));
        window.simulate_next_frame(cx);
        assert!(
            page.read(cx)
                .graph
                .focus_handle()
                .unwrap()
                .is_focused(window)
        );
        window.press("down", cx);
        assert_eq!(page.read(cx).selected().map(|s| s.as_ref()), Some("pr"));
        page.update(cx, |page, cx| {
            page.fail_read("cluster-a unreachable", window, cx)
        });
        window.render_frame(cx);
        assert!(window.try_find("change-stale").is_some());
        assert!(window.try_find("change-chain").is_some());
        let mut empty = page.read(cx).change.clone();
        empty.hops.clear();
        empty.groups.clear();
        empty.stages.clear();
        page.update(cx, |page, cx| page.read_answer(empty, window, cx));
        window.render_frame(cx);
        assert!(window.try_find("change-chain").is_none());
        window.simulate_next_frame(cx);
        assert!(page.read(cx).focus.is_focused(window));
        window.press("v", cx);
        assert_eq!(page.read(cx).view, ChangeView::List);
        window.press("v", cx);
        assert_eq!(page.read(cx).view, ChangeView::Chain);
        window.press("escape", cx);
    })
    .unwrap();
}

#[gpui_kit::test]
fn the_short_example_has_the_same_disabled_page_actions_in_list_and_chain(cx: &mut TestAppContext) {
    let (handle, page) = fixture(cx);
    let change = freshkube_delivery::changes::example::change_of(
        "checkout",
        "brave-lynx",
        DateTime::from_timestamp(1_760_000_000, 0).unwrap(),
    )
    .unwrap();
    cx.update_window(handle, |_, window, cx| {
        page.update(cx, |page, cx| page.read_answer(change, window, cx));
        for view in [ChangeView::List, ChangeView::Chain] {
            page.update(cx, |page, cx| page.set_view(view, window, cx));
            window.render_frame(cx);
            assert!(page.read(cx).page_link.is_err());
            window.click("change-copy-link", cx);
            window.click("change-open-in-kargo", cx);
            assert!(!page.read(cx).copied);
        }
    })
    .unwrap();
}

#[gpui_kit::test]
fn list_keeps_known_unread_applications_and_pods_with_their_reason(cx: &mut TestAppContext) {
    for (shape, key) in [
        (Shape::ArgoRefused, "dev-argocd"),
        (
            Shape::DestinationUnreachable,
            "dev-reported-checkout-dev-0-pods",
        ),
    ] {
        let (handle, page) = open(cx, 1280., 880., shape, Duration::ZERO, 13.);
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            let supplied = page.read(cx).shown().hop(key).unwrap().clone();
            let reason = page.read(cx).shown().missing.last().unwrap().text.clone();
            assert_eq!(supplied.state, HealthIndicator::Unknown);
            assert!(supplied.detail.contains(&reason));
            page.update(cx, |page, cx| page.select(key, window, cx));
            window.render_frame(cx);
            let id: SharedString = format!("change-hop-{key}").into();
            let row = window.find(id.clone());
            assert!(row.visible());
            assert!(row.label().unwrap().contains(&reason));
            assert!(row.label().unwrap().contains("link Can't read"));
            window.click(id, cx);
            window.render_frame(cx);
            assert_eq!(page.read(cx).selected().map(|key| key.as_ref()), Some(key));
            assert!(
                window
                    .find("change-detail-link")
                    .label()
                    .unwrap()
                    .starts_with("Can't read")
            );
            assert!(page.read(cx).action_labels().is_empty());
        })
        .unwrap();
    }
}

#[gpui_kit::test]
fn unread_applications_and_pods_are_drawn_as_parented_ghost_cards(cx: &mut TestAppContext) {
    for (shape, key) in [
        (Shape::ArgoRefused, "dev-argocd"),
        (
            Shape::DestinationUnreachable,
            "dev-reported-checkout-dev-0-pods",
        ),
    ] {
        let (handle, page) = open(cx, 1280., 880., shape, Duration::ZERO, 13.);
        cx.update_window(handle, |_, window, cx| {
            chain(&page, window, cx);
            let p = page.read(cx);
            let hop = p.shown().hop(key).unwrap();
            let parent = &p.trail.cards[hop.after.as_deref().unwrap()];
            let card = p.graph.nodes().iter().find(|card| card.key == key).unwrap();
            assert!(card.ghost, "{key} is a ghost card");
            assert_eq!(card.id.as_ref(), format!("change-card-{key}"));
            let edge = p.graph.edges().iter().find(|edge| edge.to == key).unwrap();
            assert_eq!(&edge.from, parent);
            assert_eq!(edge.style, EdgeStyle::Dotted);
            let edge_id = edge.marker_id.clone();
            page.update(cx, |page, cx| page.select(key, window, cx));
            window.render_frame(cx);
            window.simulate_next_frame(cx);
            window.render_frame(cx);
            let id: SharedString = format!("change-card-{key}").into();
            assert!(window.find(id).visible());
            assert!(
                window.try_find(edge_id).is_some(),
                "the parent edge is drawn"
            );
        })
        .unwrap();
    }
}
