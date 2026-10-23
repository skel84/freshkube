use super::*;

#[gpui_kit::test]
fn each_source_state_keeps_its_overview_and_note(cx: &mut TestAppContext) {
    let (handle, page) = overview_fixture(cx);
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
            window.render_frame(cx);
            let p = page.read(cx);
            assert_eq!(
                p.overview.len(),
                p.change.stages.len(),
                "one row per Stage, none invented"
            );
            for stage in &p.change.stages {
                assert!(
                    window
                        .try_find(format!("change-matrix-row-{}", stage.name))
                        .is_some(),
                    "{shape:?}: {}",
                    stage.name
                );
            }
            for card in ["pr", "build", "freight"] {
                assert!(
                    window.try_find(format!("change-strip-{card}")).is_some(),
                    "{shape:?}: {card}"
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
        let p = page.read(cx);
        assert!(p.counts[3] >= 1, "the start can't be confirmed");
        assert!(!p.rows.iter().any(|row| row.key == "missing-0"));
        let policy = p.rows.iter().find(|row| row.key == "policy").unwrap();
        assert!(policy.ghost);
        page.update(cx, |page, cx| page.select("policy", window, cx));
        assert_eq!(page.read(cx).selected().map(|key| key.as_ref()), Some("policy"));
        page.update(cx, |page, cx| page.set_view(ChangeView::Hops, window, cx));
        window.render_frame(cx);
        assert!(window.find("change-hop-policy").visible());
    })
    .unwrap();
}

#[gpui_kit::test]
fn the_overview_uses_loading_failure_retry_empty_and_stale_states(cx: &mut TestAppContext) {
    let (handle, page) = open_overview(cx, 1280., 880., Shape::Whole, Duration::from_secs(2), 13.);
    cx.update_window(handle, |_, window, cx| {
        // Three skeleton cards in the strip's place, and the matrix's own
        // loading rows under its real header.
        for card in 0..3 {
            assert!(
                window
                    .try_find(format!("change-strip-loading-{card}"))
                    .is_some()
            );
        }
        assert!(window.try_find("change-loading").is_some());
        assert!(window.try_find("change-strip-pr").is_none());
        page.update(cx, |page, cx| page.focus(window, cx));
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
        assert!(window.try_find("change-overview").is_some());
        assert!(window.try_find("change-strip-loading-0").is_none());
        page.update(cx, |page, cx| page.focus(window, cx));
        window.press("down", cx);
        assert_eq!(
            page.read(cx).selected().map(|s| s.as_ref()),
            Some("dev-eligible")
        );
        page.update(cx, |page, cx| {
            page.fail_read("cluster-a unreachable", window, cx)
        });
        window.render_frame(cx);
        assert!(window.try_find("change-stale").is_some());
        assert!(window.try_find("change-overview").is_some());
        let mut empty = page.read(cx).change.clone();
        empty.hops.clear();
        empty.groups.clear();
        empty.stages.clear();
        page.update(cx, |page, cx| page.read_answer(empty, window, cx));
        window.render_frame(cx);
        assert!(window.try_find("change-overview").is_none());
        window.simulate_next_frame(cx);
        assert!(page.read(cx).focus.is_focused(window));
        window.press("v", cx);
        assert_eq!(page.read(cx).view, ChangeView::Hops);
        window.press("v", cx);
        assert_eq!(page.read(cx).view, ChangeView::Overview);
        window.press("escape", cx);
    })
    .unwrap();
}

#[gpui_kit::test]
fn the_short_example_has_the_same_disabled_page_actions_in_both_views(cx: &mut TestAppContext) {
    let (handle, page) = fixture(cx);
    let change = freshkube_delivery::changes::example::change_of(
        "checkout",
        "brave-lynx",
        DateTime::from_timestamp(1_760_000_000, 0).unwrap(),
    )
    .unwrap();
    cx.update_window(handle, |_, window, cx| {
        page.update(cx, |page, cx| page.read_answer(change, window, cx));
        for view in [ChangeView::Hops, ChangeView::Overview] {
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
fn unread_applications_and_pods_say_why_in_their_overview_cells(cx: &mut TestAppContext) {
    use crate::change::overview::cells::Column;
    for (shape, key, column) in [
        (Shape::ArgoRefused, "dev-argocd", Column::Synced),
        (
            Shape::DestinationUnreachable,
            "dev-reported-checkout-dev-0-pods",
            Column::Running,
        ),
    ] {
        let (handle, page) = open_overview(cx, 1280., 880., shape, Duration::ZERO, 13.);
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            let p = page.read(cx);
            let dev = &p.overview[0];
            let at = Column::ALL.iter().position(|c| *c == column).unwrap();
            // Unknown with its cause, never drawn as a failure.
            assert_eq!(dev.cells[at].word.as_ref(), "Can't read", "{key}");
            assert_ne!(dev.cells[at].tone, Tone::Crit);
            assert!(window.try_find(format!("change-cell-dev-{}", column.id())).is_some());
            // The hop it stands for is still reachable from All hops.
            page.update(cx, |page, cx| page.select(key, window, cx));
            assert_eq!(page.read(cx).selected().map(|key| key.as_ref()), Some(key));
        })
        .unwrap();
    }
}
