use super::*;

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
