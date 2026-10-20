//! The actual change page on invented data, without the shell or I/O.
use super::{ChangeEvent, ChangePage, Connections, Fetch, HealthIndicator, SessionKey, Shows};
use crate::change_view::ChangeView;
use chrono::{DateTime, Utc};
use freshkube_delivery::change::example::Shape;
use freshkube_delivery::join::Confidence;
use freshkube_ui::ui::Tone;
use freshkube_ui::{
    graph::{Cursor, EdgeStyle, GraphSource},
    layout_check,
};
use gpui_kit::component::Root;
use gpui_kit::test::TestWindowExt;
use gpui_kit::{
    AnyWindowHandle, App, AppContext, Entity, ScrollDelta, SharedString, TestAppContext, Window,
    point, px, size,
};
use std::time::Duration;

fn open(
    cx: &mut TestAppContext,
    width: f32,
    height: f32,
    shape: Shape,
    delay: Duration,
    text: f32,
) -> (AnyWindowHandle, Entity<ChangePage>) {
    open_stage(cx, width, height, shape, delay, text, None)
}

fn open_stage(
    cx: &mut TestAppContext,
    width: f32,
    height: f32,
    shape: Shape,
    delay: Duration,
    text: f32,
    stage: Option<&str>,
) -> (AnyWindowHandle, Entity<ChangePage>) {
    cx.update(|cx| {
        gpui_kit::init(cx);
        freshkube_ui::theme::install(cx);
        freshkube_ui::text_size::install(None, cx);
        freshkube_ui::text_size::set(text, cx);
        cx.set_reduce_motion(true);
        cx.bind_keys(crate::key_bindings());
    });
    let mut page = None;
    let handle = cx.open_window(size(px(width), px(height)), |window, cx| {
        let entity = cx.new(|cx| {
            ChangePage::new(
                Fetch::Example {
                    project: "checkout".into(),
                    freight: "wonky-otter".into(),
                    now: DateTime::from_timestamp(1_760_000_000, 0).unwrap(),
                    shape,
                    delay,
                },
                stage,
                Connections::new("example", [(SessionKey::new("core-fra"), "example".into())]),
                true,
                window,
                cx,
            )
        });
        page = Some(entity.clone());
        Root::new(entity, window, cx)
    });
    cx.run_until_parked();
    (handle.into(), page.unwrap())
}

fn fixture(cx: &mut TestAppContext) -> (AnyWindowHandle, Entity<ChangePage>) {
    open(cx, 1280., 880., Shape::Whole, Duration::ZERO, 13.)
}

fn chain(page: &Entity<ChangePage>, window: &mut Window, cx: &mut App) {
    page.update(cx, |page, cx| page.set_view(ChangeView::Chain, window, cx));
    window.render_frame(cx);
}

#[gpui_kit::test]
fn the_toggle_preserves_rows_cards_links_and_a_gate_selection(cx: &mut TestAppContext) {
    let (handle, page) = fixture(cx);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("change-hop-push-run", cx);
        window.click("change-view-chain", cx);
        window.render_frame(cx);
        assert_eq!(
            page.read(cx).selected().map(|s| s.as_ref()),
            Some("push-run")
        );
        assert!(window.try_find("change-chain").is_some());
        window.click("change-link-signature", cx);
        assert!(page.read(cx).link_selected);
        window.click("change-view-list", cx);
        window.render_frame(cx);
        assert_eq!(
            page.read(cx).selected().map(|s| s.as_ref()),
            Some("signature")
        );
        assert!(page.read(cx).link_selected);
        assert!(window.find("change-hop-signature").visible());
        page.update(cx, |page, cx| page.select("dev-verification", window, cx));
        chain(&page, window, cx);
        assert_eq!(
            page.read(cx).cursor(),
            Some(Cursor::Node("dev-eligible".into()))
        );
        assert_eq!(
            page.read(cx).selected().map(|s| s.as_ref()),
            Some("dev-verification")
        );
        assert!(window.try_find("change-detail-verification").is_some());
        page.update(cx, |page, cx| page.back(cx));
        page.update(cx, |page, cx| page.set_view(ChangeView::List, window, cx));
        chain(&page, window, cx);
        assert!(
            page.read(cx).selected.is_none(),
            "switching never invents a selection"
        );
    })
    .unwrap();
}

#[gpui_kit::test]
fn the_chain_keys_walk_links_and_cards_open_clear_back_and_switch(cx: &mut TestAppContext) {
    let (handle, page) = fixture(cx);
    let events = std::rc::Rc::new(std::cell::RefCell::new(vec![]));
    let recorded = events.clone();
    let _subscription = cx.update(|cx| {
        cx.subscribe(&page, move |_, event, _| {
            recorded.borrow_mut().push(match event {
                ChangeEvent::Back => "back",
                ChangeEvent::Open(_) | ChangeEvent::Switch(..) => "open",
                _ => "other",
            })
        })
    });
    cx.update_window(handle, |_, window, cx| {
        chain(&page, window, cx);
        window.press("down", cx);
        assert_eq!(page.read(cx).selected().map(|s| s.as_ref()), Some("pr"));
        window.press("right", cx);
        assert_eq!(
            page.read(cx).cursor(),
            Some(Cursor::Link("pr".into(), "pr-run".into()))
        );
        window.press("right", cx);
        assert_eq!(page.read(cx).selected().map(|s| s.as_ref()), Some("pr-run"));
        window.press("left", cx);
        window.press("left", cx);
        assert_eq!(page.read(cx).selected().map(|s| s.as_ref()), Some("pr"));
        window.press("down", cx);
        window.press("up", cx);
        assert_eq!(page.read(cx).selected().map(|s| s.as_ref()), Some("pr"));
        page.update(cx, |page, cx| page.select("freight", window, cx));
        window.press("o", cx);
        window.press("enter", cx);
        window.press("v", cx);
        assert_eq!(page.read(cx).view, ChangeView::List);
        window.press("v", cx);
        assert_eq!(page.read(cx).view, ChangeView::Chain);
        window.press("escape", cx);
        assert!(page.read(cx).selected.is_none());
        window.press("escape", cx);
    })
    .unwrap();
    cx.run_until_parked();
    assert_eq!(&*events.borrow(), &["open", "open", "back"]);
}

#[gpui_kit::test]
fn confirmed_claimed_unknown_and_a_failing_confirmed_hop_keep_separate_meanings(
    cx: &mut TestAppContext,
) {
    let (handle, page) = fixture(cx);
    cx.update_window(handle, |_, window, cx| {
        chain(&page, window, cx);
        let edges = page.read(cx).graph.edges().to_vec();
        for style in [EdgeStyle::Solid, EdgeStyle::Dashed, EdgeStyle::Dotted] {
            let edge = edges
                .iter()
                .find(|edge| edge.style == style)
                .expect("fixture has every confidence");
            page.read(cx).graph.reveal_node(&edge.to, window);
            window.render_frame(cx);
            window.click(edge.marker_id.clone(), cx);
            window.render_frame(cx);
            assert_eq!(page.read(cx).selected(), Some(&edge.to));
            assert!(page.read(cx).link_selected);
            assert!(window.try_find("change-detail-link").is_some());
        }
        assert!(
            page.read(cx)
                .graph
                .edges()
                .iter()
                .all(|edge| edge.tone.is_none())
        );
        assert!(
            page.read(cx)
                .graph
                .nodes()
                .iter()
                .any(|node| node.tone == Some(Tone::Crit))
        );
    })
    .unwrap();
}

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
            detail.state = "Not found".into();
            detail.link = None;
        }
        change.missing.push(
            Missing::new(Gap::NotServed, "Signature results not served on cluster-a")
                .after("push-run"),
        );
        let waiting = page.read(cx).counts[2];
        page.update(cx, |page, cx| page.read_answer(change, window, cx));
        chain(&page, window, cx);
        let p = page.read(cx);
        assert_eq!(p.counts[2], waiting + 1);
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
        window.click("change-retry", cx);
        assert!(page.read(cx).is_reading());
    })
    .unwrap();
    cx.executor().advance_clock(Duration::from_secs(2));
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(window.try_find("change-chain").is_some());
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
    })
    .unwrap();
}

#[gpui_kit::test]
fn hide_show_keeps_view_selection_and_freshness_lifetime(cx: &mut TestAppContext) {
    let (handle, page) = fixture(cx);
    cx.update_window(handle, |_, window, cx| {
        chain(&page, window, cx);
        page.update(cx, |page, cx| {
            page.select("policy", window, cx);
            page.set_visible(false, cx);
        });
        page.update(cx, |page, cx| page.refresh(cx));
        assert!(!page.read(cx).is_reading());
        page.update(cx, |page, cx| page.set_visible(true, cx));
        assert!(!page.read(cx).is_reading());
        assert_eq!(page.read(cx).view, ChangeView::Chain);
        assert_eq!(page.read(cx).selected().map(|s| s.as_ref()), Some("policy"));
    })
    .unwrap();
    cx.executor().advance_clock(Duration::from_secs(31));
    cx.update_window(handle, |_, _, cx| {
        page.update(cx, |page, cx| {
            page.set_visible(false, cx);
            page.set_visible(true, cx);
        });
        assert!(page.read(cx).is_reading());
    })
    .unwrap();
    cx.run_until_parked();
    assert_eq!(page.read_with(cx, |page, _| page.view), ChangeView::Chain);
}

#[gpui_kit::test]
fn the_view_is_remembered_when_another_change_opens(cx: &mut TestAppContext) {
    let (handle, page) = fixture(cx);
    cx.update_window(handle, |_, window, cx| {
        chain(&page, window, cx);
        let reopened = cx.new(|cx| {
            ChangePage::new(
                page.read(cx).fetch.clone(),
                None,
                Connections::default(),
                true,
                window,
                cx,
            )
        });
        assert_eq!(reopened.read(cx).view, ChangeView::Chain);
    })
    .unwrap();
}

#[gpui_kit::test]
fn chain_and_inspector_keep_the_wide_frame(cx: &mut TestAppContext) {
    let (handle, page) = fixture(cx);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        layout_check::assert_table_page_from(
            window,
            cx,
            &layout_check::TablePage {
                page: "change-page",
                title: "change-title",
                title_text: "wonky-otter",
                table: "change-table-scroll",
                list: "change-list",
            },
            "change-back",
        );
        chain(&page, window, cx);
        page.update(cx, |page, cx| page.select("freight", window, cx));
        window.render_frame(cx);
        layout_check::settle_header(window, cx);
        layout_check::assert_edge_frame_from(
            window,
            cx,
            &layout_check::PageFrame {
                page: "change-page",
                title: "change-title",
                title_text: "wonky-otter",
                content: "change-split",
            },
            "change-back",
        );
        layout_check::assert_inspector(
            window,
            cx,
            "change-split",
            "change-chain",
            "change-detail",
            "change-detail-title",
        );
    })
    .unwrap();
}

#[gpui_kit::test]
fn two_stages_join_the_same_freight_and_the_key_walk_visits_both(cx: &mut TestAppContext) {
    let (handle, page) = fixture(cx);
    cx.update_window(handle, |_, window, cx| {
        let mut change = page.read(cx).change.clone();
        for hop in &mut change.hops {
            if matches!(hop.shows, Shows::Stage(0 | 1)) {
                hop.after = Some("freight".into());
            }
        }
        page.update(cx, |page, cx| page.read_answer(change, window, cx));
        chain(&page, window, cx);
        let p = page.read(cx);
        assert!(
            p.trail
                .edges
                .iter()
                .any(|e| e.from == "freight" && e.to == "dev-eligible")
        );
        assert!(
            p.trail
                .edges
                .iter()
                .any(|e| e.from == "freight" && e.to == "stage-eligible")
        );
        let at = |key: &str| {
            p.graph
                .placed()
                .find(|(n, _, _)| n.key == key)
                .map(|(_, x, y)| (x, y))
                .unwrap()
        };
        assert_eq!(at("dev-eligible").0, at("stage-eligible").0);
        assert_ne!(at("dev-eligible").1, at("stage-eligible").1);
        assert_eq!(
            p.graph.nodes().iter().filter(|n| n.key == "image").count(),
            1
        );
        page.update(cx, |page, cx| page.select("freight", window, cx));
        window.press("right", cx);
        window.press("right", cx);
        assert_eq!(
            page.read(cx).selected().map(|s| s.as_ref()),
            Some("dev-eligible")
        );
        window.press("down", cx);
        assert_eq!(
            page.read(cx).selected().map(|s| s.as_ref()),
            Some("stage-eligible")
        );
    })
    .unwrap();
}

#[gpui_kit::test]
fn the_status_filter_dims_chain_cards_and_a_switch_reveals_its_selection(cx: &mut TestAppContext) {
    let (handle, page) = fixture(cx);
    cx.update_window(handle, |_, window, cx| {
        chain(&page, window, cx);
        page.update(cx, |page, cx| {
            page.toggle_filter(Tone::Crit, cx);
            page.select("dev-verification", window, cx);
        });
        assert_eq!(
            page.read(cx).graph.nodes().len(),
            page.read(cx).trail.nodes.len()
        );
        page.update(cx, |page, cx| page.set_view(ChangeView::List, window, cx));
        window.render_frame(cx);
        assert_eq!(page.read(cx).filter, None);
        assert!(!page.read(cx).folded.contains(&2));
        assert!(window.find("change-hop-dev-verification").visible());
    })
    .unwrap();
}

#[gpui_kit::test]
fn narrow_chain_at_text_20_reveals_a_far_card_and_stacks_the_inspector(cx: &mut TestAppContext) {
    let (handle, page) = open(cx, 760., 560., Shape::Whole, Duration::ZERO, 20.);
    cx.update_window(handle, |_, window, cx| {
        chain(&page, window, cx);
        page.update(cx, |page, cx| page.select("prod-lon-pods", window, cx));
        window.render_frame(cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.render_frame(cx);
        layout_check::settle_header(window, cx);
        layout_check::assert_inspector(
            window,
            cx,
            "change-split",
            "change-chain",
            "change-detail",
            "change-detail-title",
        );
        let pane = window.find("change-chain-scroll").bounds();
        let card = window.find("change-card-prod-lon-pods").bounds();
        assert!(card.left() >= pane.left() - px(1.));
        assert!(card.right() <= pane.right() + px(1.));
        assert!(
            window.find("change-detail").bounds().top()
                >= window.find("change-chain").bounds().bottom() - px(1.)
        );
    })
    .unwrap();
}

#[gpui_kit::test]
fn a_stages_card_uses_its_gates_while_downstream_health_stays_on_its_card(cx: &mut TestAppContext) {
    let (handle, page) = fixture(cx);
    cx.update_window(handle, |_, window, cx| {
        let mut change = page.read(cx).change.clone();
        let words = change.stages[0].words.clone();
        change
            .hops
            .iter_mut()
            .find(|hop| hop.key == "dev-pods")
            .unwrap()
            .state = HealthIndicator::Error;
        page.update(cx, |page, cx| page.read_answer(change, window, cx));
        chain(&page, window, cx);
        let p = page.read(cx);
        let stage = p
            .graph
            .nodes()
            .iter()
            .find(|node| node.key == "dev-eligible")
            .unwrap();
        assert_eq!(stage.tone, Some(Tone::Good));
        assert!(stage.detail.starts_with(&words));
        assert_eq!(
            p.graph
                .nodes()
                .iter()
                .find(|node| node.key == "dev-pods")
                .unwrap()
                .tone,
            Some(Tone::Crit)
        );
    })
    .unwrap();
}

#[gpui_kit::test]
fn two_windows_keep_their_own_view_when_another_change_opens(cx: &mut TestAppContext) {
    let (first, a) = fixture(cx);
    let (second, b) = fixture(cx);
    cx.update_window(first, |_, window, cx| chain(&a, window, cx))
        .unwrap();
    cx.update_window(second, |_, window, cx| {
        chain(&b, window, cx);
        b.update(cx, |page, cx| page.set_view(ChangeView::List, window, cx));
    })
    .unwrap();
    for (handle, page, expected) in [(first, a, ChangeView::Chain), (second, b, ChangeView::List)] {
        cx.update_window(handle, |_, window, cx| {
            let fetch = page.read(cx).fetch.clone();
            let reopened =
                cx.new(|cx| ChangePage::new(fetch, None, Connections::default(), true, window, cx));
            assert_eq!(reopened.read(cx).view, expected);
        })
        .unwrap();
    }
}

#[gpui_kit::test]
fn selecting_a_link_reveals_its_comparison_after_reading_a_long_stage(cx: &mut TestAppContext) {
    let (handle, page) = open(cx, 760., 700., Shape::Whole, Duration::ZERO, 13.);
    cx.update_window(handle, |_, window, cx| {
        chain(&page, window, cx);
        page.update(cx, |page, cx| page.select("dev-eligible", window, cx));
        window.render_frame(cx);
        window.scroll(
            "change-detail-body",
            ScrollDelta::Pixels(point(px(0.), px(-300.))),
            cx,
        );
        window.render_frame(cx);
        assert!(page.read(cx).detail_scroll.offset().y < px(0.));
        window.press("left", cx);
        window.render_frame(cx);
        assert!(page.read(cx).link_selected);
        for view in [ChangeView::Chain, ChangeView::List] {
            page.update(cx, |page, cx| page.set_view(view, window, cx));
            window.render_frame(cx);
            let body = window.find("change-detail-body").bounds();
            let link = window.find("change-detail-link").bounds();
            assert!(link.top() >= body.top());
            assert!(link.bottom() <= body.bottom());
        }
    })
    .unwrap();
}

#[gpui_kit::test]
fn a_change_opened_in_remembered_chain_reveals_its_initial_stage(cx: &mut TestAppContext) {
    let (first, a) = fixture(cx);
    cx.update_window(first, |_, window, cx| chain(&a, window, cx))
        .unwrap();
    let (handle, page) = open_stage(
        cx,
        760.,
        560.,
        Shape::Whole,
        Duration::ZERO,
        20.,
        Some("prod-lon"),
    );
    cx.update_window(handle, |_, window, cx| {
        assert_eq!(page.read(cx).view, ChangeView::Chain);
        window.render_frame(cx);
        window.simulate_next_frame(cx);
        window.render_frame(cx);
        let pane = window.find("change-chain-scroll").bounds();
        let card = window.find("change-card-prod-lon-eligible").bounds();
        assert!(card.left() >= pane.left() - px(1.));
        assert!(card.right() <= pane.right() + px(1.));
    })
    .unwrap();
}
