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
            detail.state = "No policy result reported".into();
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
        settle_chain(window, cx);
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
        settle_chain(window, cx);
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

#[gpui_kit::test]
fn selecting_a_link_keeps_the_supplied_unlinked_reason(cx: &mut TestAppContext) {
    let (handle, page) = fixture(cx);
    cx.update_window(handle, |_, window, cx| {
        chain(&page, window, cx);
        let hop = page.read(cx).change.hop("prod-ams-argocd").unwrap();
        let Shows::Hop(detail) = &hop.shows else {
            panic!("Application");
        };
        let why = detail.unlinked.clone().unwrap();
        let edge = page
            .read(cx)
            .trail
            .edges
            .iter()
            .find(|edge| edge.to == "prod-ams-argocd")
            .unwrap()
            .clone();
        assert_eq!(edge.tooltip.as_ref(), why);
        page.update(cx, |page, cx| {
            page.step_to(Cursor::Link(edge.from, edge.to), window, cx)
        });
        window.render_frame(cx);
        assert!(
            window
                .find("change-detail-link")
                .label()
                .unwrap()
                .contains(&why)
        );
        window.click("change-view-list", cx);
        window.render_frame(cx);
        assert!(
            window
                .find("change-detail-link")
                .label()
                .unwrap()
                .contains(&why)
        );
    })
    .unwrap();
}

#[gpui_kit::test]
fn every_example_change_draws_the_supplied_parent_links(cx: &mut TestAppContext) {
    use freshkube_delivery::{changes::example, source::Source};
    use std::collections::BTreeSet;
    let (handle, page) = fixture(cx);
    let now = DateTime::from_timestamp(1_760_000_000, 0).unwrap();
    let Source::Read(projects) = example::read_shaped(now, Shape::Whole).projects else {
        panic!("example projects");
    };
    let mut checked = 0;
    for project in projects {
        let Source::Read(freights) = project.freight else {
            panic!("example Freight");
        };
        for freight in freights {
            let change = example::change_of(&project.name, &freight.name, now).unwrap();
            cx.update_window(handle, |_, window, cx| {
                page.update(cx, |page, cx| page.read_answer(change, window, cx));
                let page = page.read(cx);
                let expected: BTreeSet<_> = page
                    .change
                    .hops
                    .iter()
                    .filter_map(|hop| {
                        let from = page.trail.cards.get(hop.after.as_deref()?)?;
                        let to = &page.trail.cards[hop.key.as_str()];
                        (from != to).then(|| (from.clone(), to.clone()))
                    })
                    .collect();
                let actual: BTreeSet<_> = page
                    .trail
                    .edges
                    .iter()
                    .map(|edge| (edge.from.clone(), edge.to.clone()))
                    .collect();
                assert!(!expected.is_empty(), "{} has a linked trail", freight.name);
                assert_eq!(actual, expected, "{}", freight.name);
                assert_eq!(page.trail.edges.len(), expected.len());
            })
            .unwrap();
            checked += 1;
        }
    }
    assert_eq!(checked, 6);
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
fn background_clicks_never_select_or_scroll_and_outside_clicks_blur(cx: &mut TestAppContext) {
    let (handle, page) = fixture(cx);
    cx.update_window(handle, |_, window, cx| {
        chain(&page, window, cx);
        page.update(cx, |page, cx| page.back(cx));
        window.simulate_next_frame(cx);
        let before = window.find("change-card-pr").bounds();
        window.click_at("change-chain-graph", point(px(4.), px(4.)), cx);
        window.simulate_next_frame(cx);
        assert!(page.read(cx).selected.is_none());
        assert!(page.read(cx).detail.is_none());
        assert_eq!(window.find("change-card-pr").bounds(), before);
        assert!(
            page.read(cx)
                .graph
                .focus_handle()
                .unwrap()
                .is_focused(window)
        );
        window.click_at(
            "change-chain",
            point(
                px(4.),
                window.find("change-chain").bounds().size.height - px(4.),
            ),
            cx,
        );
        window.simulate_next_frame(cx);
        assert!(
            !page
                .read(cx)
                .graph
                .focus_handle()
                .unwrap()
                .is_focused(window)
        );
    })
    .unwrap();
}

#[gpui_kit::test]
fn the_first_switch_to_chain_reveals_the_list_selection_after_layout(cx: &mut TestAppContext) {
    let (handle, page) = fixture(cx);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        page.update(cx, |page, cx| page.select("prod-ams-argocd", window, cx));
        window.click("change-view-chain", cx);
        window.simulate_next_frame(cx);
        window.render_frame(cx);
        window.simulate_next_frame(cx);
        window.render_frame(cx);
        let pane = window.find("change-chain-scroll").bounds();
        let card = window.find("change-card-prod-ams-argocd").bounds();
        assert!(card.left() >= pane.left() - px(1.), "{card:?} in {pane:?}");
        assert!(
            card.right() <= pane.right() + px(1.),
            "{card:?} in {pane:?}"
        );
    })
    .unwrap();
}

fn settle_chain(window: &mut Window, cx: &mut App) {
    assert!(
        window.simulate_next_frame(cx) > 0,
        "reveal requests a frame"
    );
    window.render_frame(cx);
    // The stacked Inspector can change the canvas's height after it opens.
    for _ in 0..6 {
        if window.simulate_next_frame(cx) == 0 {
            break;
        }
        window.render_frame(cx);
    }
}

fn assert_card_in_canvas(key: &str, window: &Window) {
    let horizontal = window.find("change-chain-scroll").bounds();
    let vertical = window.find("change-chain-canvas").bounds();
    let card = window
        .find(SharedString::from(format!("change-card-{key}")))
        .bounds();
    assert!(
        card.left() >= horizontal.left() - px(1.),
        "{key}: {card:?} in {horizontal:?}"
    );
    assert!(
        card.right() <= horizontal.right() + px(1.),
        "{key}: {card:?} in {horizontal:?}"
    );
    assert!(
        card.top() >= vertical.top() - px(1.),
        "{key}: {card:?} in {vertical:?}"
    );
    assert!(
        card.bottom() <= vertical.bottom() + px(1.),
        "{key}: {card:?} in {vertical:?}"
    );
}

fn assert_stacked_chain_reveal(cx: &mut TestAppContext, width: f32, height: f32, text: f32) {
    let (handle, page) = open(cx, width, height, Shape::Whole, Duration::ZERO, text);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        page.update(cx, |page, cx| page.select("prod-ams-eligible", window, cx));
        page.update(cx, |page, cx| page.focus(window, cx));
        // The header folds the toggle into its menu at narrow widths.
        window.press("v", cx);
        settle_chain(window, cx);
        assert_card_in_canvas("prod-ams-eligible", window);
        assert!(
            window.find("change-detail").bounds().top()
                >= window.find("change-chain").bounds().bottom() - px(1.)
        );

        // Selection in Chain and keyboard navigation also reveal both axes.
        page.update(cx, |page, cx| page.select("prod-lon-eligible", window, cx));
        window.render_frame(cx);
        settle_chain(window, cx);
        assert_card_in_canvas("prod-lon-eligible", window);
        page.update(cx, |page, cx| page.focus(window, cx));
        window.press("up", cx);
        settle_chain(window, cx);
        assert_eq!(
            page.read(cx).cursor(),
            Some(Cursor::Node("prod-ams-eligible".into()))
        );
        assert_card_in_canvas("prod-ams-eligible", window);
    })
    .unwrap();
}

#[gpui_kit::test]
fn narrow_chain_reveals_both_axes_after_layout(cx: &mut TestAppContext) {
    assert_stacked_chain_reveal(cx, 760., 560., 13.);
}

#[gpui_kit::test]
fn narrow_chain_at_text_20_reveals_both_axes_after_layout(cx: &mut TestAppContext) {
    assert_stacked_chain_reveal(cx, 760., 560., 20.);
}

#[gpui_kit::test]
fn wide_chain_at_text_20_reveals_both_axes_after_layout(cx: &mut TestAppContext) {
    assert_stacked_chain_reveal(cx, 1280., 880., 20.);
}

#[gpui_kit::test]
fn chain_reveals_again_when_the_stacked_canvas_shrinks_on_a_later_frame(cx: &mut TestAppContext) {
    let (handle, page) = open(cx, 760., 880., Shape::Whole, Duration::ZERO, 13.);
    cx.update_window(handle, |_, window, cx| {
        chain(&page, window, cx);
        page.update(cx, |page, cx| page.select("prod-lon-eligible", window, cx));
        window.render_frame(cx);
        assert!(window.simulate_next_frame(cx) > 0);
        window.render_frame(cx);
        let before = window.find("change-chain-canvas").bounds().size.height;
        // Model the split applying a smaller height after the first reveal.
        let state = page.read(cx).split.stacked_state().clone();
        state.update(cx, |state, cx| state.resize_panel(0, px(180.), window, cx));
        window.render_frame(cx);
        settle_chain(window, cx);
        assert!(window.find("change-chain-canvas").bounds().size.height < before);
        assert_card_in_canvas("prod-lon-eligible", window);
        assert_eq!(
            window.simulate_next_frame(cx),
            0,
            "settled reveal stops requesting frames"
        );
    })
    .unwrap();
}

#[gpui_kit::test]
fn remembered_chain_reveals_its_initial_stage_on_both_axes(cx: &mut TestAppContext) {
    let (first, a) = fixture(cx);
    cx.update_window(first, |_, window, cx| chain(&a, window, cx))
        .unwrap();
    for (width, height) in [(760., 560.), (1280., 880.)] {
        let (handle, page) = open_stage(
            cx,
            width,
            height,
            Shape::Whole,
            Duration::ZERO,
            20.,
            Some("prod-ams"),
        );
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            settle_chain(window, cx);
            assert_eq!(page.read(cx).view, ChangeView::Chain);
            assert_card_in_canvas("prod-ams-eligible", window);
        })
        .unwrap();
    }
}

#[gpui_kit::test]
fn every_example_change_keeps_list_order_within_each_chain_column(cx: &mut TestAppContext) {
    use freshkube_delivery::{changes::example, source::Source};
    let (handle, page) = fixture(cx);
    let now = DateTime::from_timestamp(1_760_000_000, 0).unwrap();
    let Source::Read(projects) = example::read(now).projects else {
        panic!("example projects")
    };
    for project in projects {
        let Source::Read(freight) = project.freight else {
            panic!("example Freight")
        };
        for freight in freight {
            let change = example::change_of(&project.name, &freight.name, now).unwrap();
            cx.update_window(handle, |_, window, cx| {
                page.update(cx, |page, cx| page.read_answer(change, window, cx));
                let p = page.read(cx);
                let mut columns = std::collections::BTreeMap::<i32, Vec<_>>::new();
                for (node, x, y) in p.graph.placed() {
                    columns
                        .entry(x.round() as i32)
                        .or_default()
                        .push((node.key.clone(), y));
                }
                for column in columns.values_mut() {
                    let mut expected = vec![];
                    for row in &p.rows {
                        let card = &p.trail.cards[&row.key];
                        if column.iter().any(|(key, _)| key == card) && !expected.contains(card) {
                            expected.push(card.clone());
                        }
                    }
                    column.sort_by(|a, b| a.1.total_cmp(&b.1));
                    let actual: Vec<_> = column.iter().map(|(key, _)| key.clone()).collect();
                    assert_eq!(actual, expected, "{} / {}", project.name, freight.name);
                }
            })
            .unwrap();
        }
    }
}

fn has_cursor_ring(key: &str, window: &Window) -> bool {
    let bounds = window
        .find(SharedString::from(format!("change-card-{key}")))
        .bounds()
        .scale(window.scale_factor());
    window.painted_quads().iter().any(|quad| {
        quad.bounds == bounds && quad.border_widths.left.0 >= 2. * window.scale_factor()
    })
}

#[gpui_kit::test]
fn mouse_selection_keeps_the_selected_border_and_only_keyboard_navigation_rings_it(
    cx: &mut TestAppContext,
) {
    let (handle, page) = fixture(cx);
    cx.update_window(handle, |_, window, cx| {
        window.activate_window();
        chain(&page, window, cx);
        window.click("change-card-pr", cx);
        settle_chain(window, cx);
        assert_eq!(page.read(cx).selected().map(|key| key.as_ref()), Some("pr"));
        assert!(page.read(cx).focus_handle().is_focused(window));
        assert!(
            !has_cursor_ring("pr", window),
            "mouse selection has no keyboard ring"
        );
        let bounds = window
            .find("change-card-pr")
            .bounds()
            .scale(window.scale_factor());
        let accent = freshkube_ui::palette::palette(cx).accent_line;
        assert!(
            window
                .painted_quads()
                .iter()
                .any(|q| q.bounds == bounds && q.border_color == accent)
        );
        window.press("down", cx);
        settle_chain(window, cx);
        assert!(
            has_cursor_ring("pr-run", window),
            "keyboard navigation rings its card"
        );
        window.click("change-card-pr", cx);
        settle_chain(window, cx);
        assert!(
            !has_cursor_ring("pr", window),
            "mouse selection clears the ring"
        );
        window.press("down", cx);
        settle_chain(window, cx);
        assert!(has_cursor_ring("pr-run", window));
    })
    .unwrap();
    // Focus subscriptions become active after the render's update ends.
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| window.blur(cx))
        .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.simulate_next_frame(cx);
        window.render_frame(cx);
        assert!(!has_cursor_ring("pr-run", window), "blur clears the ring");
        page.update(cx, |page, cx| page.focus(window, cx));
        window.render_frame(cx);
        assert!(
            !has_cursor_ring("pr-run", window),
            "focus alone does not restore the ring"
        );
    })
    .unwrap();
}

fn assert_crossing_pill_clearance(cx: &mut TestAppContext, width: f32, height: f32, text: f32) {
    let (handle, page) = open(cx, width, height, Shape::Whole, Duration::ZERO, text);
    cx.update_window(handle, |_, window, cx| {
        chain(&page, window, cx);
        let ams = window.find("change-link-prod-ams-eligible").bounds();
        let lon = window.find("change-link-prod-lon-eligible").bounds();
        let ams_card = window.find("change-card-prod-ams-eligible").bounds();
        let lon_card = window.find("change-card-prod-lon-eligible").bounds();
        assert!(
            (ams.center().y - ams_card.center().y).abs()
                < (ams.center().y - lon_card.center().y).abs()
        );
        assert!(
            (lon.center().y - lon_card.center().y).abs()
                < (lon.center().y - ams_card.center().y).abs()
        );
        assert!(ams.bottom() < lon.top() || lon.bottom() < ams.top());
        assert!((ams.center().y - ams_card.center().y).abs() < px(1.));
        assert!((lon.center().y - lon_card.center().y).abs() < px(1.));
        let origin = window.find("change-chain-graph").bounds().origin;
        let unit = freshkube_ui::ui::dp_px(1., window);
        let p = page.read(cx);
        for (key, pill) in [("prod-ams-eligible", lon), ("prod-lon-eligible", ams)] {
        let (_, route) = p.graph.shown().find(|(edge, _)| edge.to == key).unwrap();
        // Both curves, including their strokes, clear the other pill.
        for segment in &route.segments {
            for step in 0..=1000 {
                let t = step as f32 / 1000.;
                let u = 1. - t;
                let weights = [u * u * u, 3. * u * u * t, 3. * u * t * t, t * t * t];
                let (x, y) = weights
                    .iter()
                    .zip(segment)
                    .fold((0., 0.), |(x, y), (w, p)| (x + w * p.0, y + w * p.1));
                let at = origin + point(unit * x, unit * y);
                let margin = unit * 4.;
                assert!(
                    at.x < pill.left() - margin
                        || at.x > pill.right() + margin
                        || at.y < pill.top() - margin
                        || at.y > pill.bottom() + margin,
                    "{key} at {at:?} needs clearance from the other pill {pill:?}; route {route:?}"
                );
            }
        }
        }
    })
    .unwrap();
}

#[gpui_kit::test]
fn crossing_links_keep_their_pills_beside_their_own_targets(cx: &mut TestAppContext) {
    assert_crossing_pill_clearance(cx, 1600., 1000., 13.);
}

#[gpui_kit::test]
fn narrow_chain_clears_pills_in_both_directions(cx: &mut TestAppContext) {
    assert_crossing_pill_clearance(cx, 760., 560., 13.);
}

#[gpui_kit::test]
fn narrow_chain_at_text_20_clears_pills_in_both_directions(cx: &mut TestAppContext) {
    assert_crossing_pill_clearance(cx, 760., 560., 20.);
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
            assert!(row.label().unwrap().contains("link Unknown"));
            window.click(id, cx);
            window.render_frame(cx);
            assert_eq!(page.read(cx).selected().map(|key| key.as_ref()), Some(key));
            assert!(
                window
                    .find("change-detail-link")
                    .label()
                    .unwrap()
                    .starts_with("Unknown")
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
