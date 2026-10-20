//! What a hop follows: the chain the rows are drawn as, read from the
//! join's links, never guessed from the rows' order or names.

use serde_json::json;

use super::super::{Change, Hop as Row, Shows};
use super::read;
use super::tests::{one_cluster, place, stage_of};
use crate::tests::observed_at;

fn after<'a>(change: &'a Change, key: &str) -> Option<&'a str> {
    change
        .hops
        .iter()
        .find(|hop| hop.key == key)
        .unwrap_or_else(|| panic!("{key} in {:#?}", keys(change)))
        .after
        .as_deref()
}

fn keys(change: &Change) -> Vec<&str> {
    change.hops.iter().map(|hop| hop.key.as_str()).collect()
}

fn detail_key(hop: &Row) -> Option<&str> {
    let Shows::Hop(detail) = &hop.shows else {
        panic!("{} shows a Stage", hop.key);
    };
    detail.link.as_ref().and_then(|link| link.key.as_deref())
}

#[tokio::test]
async fn every_hop_follows_a_hop_that_is_there() {
    let reader = one_cluster(|_| {});
    let change = read(&reader, &place("f-new"), observed_at())
        .await
        .expect("read");

    for hop in &change.hops {
        if let Some(after) = &hop.after {
            assert_ne!(after, &hop.key, "{} follows itself", hop.key);
            assert!(
                keys(&change).contains(&after.as_str()),
                "{} follows {after}, which isn't a hop of {:?}",
                hop.key,
                keys(&change)
            );
        }
    }
}

#[tokio::test]
async fn the_chain_runs_from_the_build_through_the_freight_and_the_stage_to_the_pods() {
    let reader = one_cluster(|_| {});
    let change = read(&reader, &place("f-new"), observed_at())
        .await
        .expect("read");

    // The Freight follows the build that made its image; the Warehouse and
    // the Stage's gates follow the Freight.
    let freight = change.hops.iter().find(|hop| hop.key == "freight").unwrap();
    assert_eq!(freight.after.as_deref(), Some("build-run-0"));
    assert_eq!(after(&change, "build-chain-1"), Some("build-run-0"));
    assert_eq!(after(&change, "warehouse"), Some("freight"));
    for gate in ["eligible", "promotion", "verification"] {
        assert_eq!(after(&change, &format!("dev-{gate}")), Some("freight"));
    }

    // Its Application follows the Stage's card; what runs there follows
    // the Application, one link after another. A build's two hops both
    // follow the run, since the Supply chain is the run's.
    assert_eq!(after(&change, "dev-argocd-0"), Some("dev-eligible"));
    assert_eq!(after(&change, "dev-rollout-1"), Some("dev-argocd-0"));
    assert_eq!(after(&change, "dev-pods-2"), Some("dev-rollout-1"));
}

#[tokio::test]
async fn the_build_hops_follow_the_links_that_made_them() {
    let reader = one_cluster(|_| {});
    let change = read(&reader, &place("f-new"), observed_at())
        .await
        .expect("read");

    let builds: Vec<&Row> = change
        .hops
        .iter()
        .filter(|hop| hop.key.starts_with("build-"))
        .collect();
    assert!(!builds.is_empty());
    // The first has nothing before it in the rows: the pull request and the
    // commit are the change's origin, not hops.
    assert_eq!(builds[0].after, None);
    for hop in &builds[1..] {
        if let Some(after) = &hop.after {
            let earlier = builds.iter().position(|b| &b.key == after);
            let this = builds.iter().position(|b| b.key == hop.key);
            assert!(
                earlier < this,
                "{} follows {after}, which is later",
                hop.key
            );
        }
    }
}

#[tokio::test]
async fn two_stages_that_run_one_commit_each_follow_their_own_card() {
    let reader = one_cluster(|world| {
        world.kargo = std::mem::take(&mut world.kargo).with(
            "stages",
            vec![
                stage_of("dev", &[], &["f-new"]),
                stage_of("east", &["dev"], &["f-old"]),
                stage_of("west", &["dev"], &["f-old"]),
                stage_of("late", &["west"], &["f-old"]),
            ],
        );
    });
    let change = read(&reader, &place("f-new"), observed_at())
        .await
        .expect("read");

    // The Freight and the build are drawn once; each Stage follows the
    // card it takes Freight from, and its three gates share that card.
    for gate in ["eligible", "promotion", "verification"] {
        assert_eq!(after(&change, &format!("dev-{gate}")), Some("freight"));
        assert_eq!(
            after(&change, &format!("east-{gate}")),
            Some("dev-eligible")
        );
        assert_eq!(
            after(&change, &format!("west-{gate}")),
            Some("dev-eligible")
        );
        assert_eq!(
            after(&change, &format!("late-{gate}")),
            Some("west-eligible")
        );
    }
    assert_eq!(keys(&change).iter().filter(|k| **k == "freight").count(), 1);
}

#[tokio::test]
async fn a_stage_that_takes_freight_from_a_stage_not_shown_follows_the_freight() {
    // `other` belongs to another Warehouse, so the Freight can't reach it.
    let reader = one_cluster(|world| {
        let mut other = stage_of("other", &[], &["f-old"]);
        other["spec"]["requestedFreight"][0]["origin"]["name"] = json!("elsewhere");
        world.kargo = std::mem::take(&mut world.kargo).with(
            "stages",
            vec![
                stage_of("dev", &[], &["f-new"]),
                other,
                stage_of("after-other", &["other"], &["f-old"]),
            ],
        );
    });
    let change = read(&reader, &place("f-new"), observed_at())
        .await
        .expect("read");

    assert!(
        !keys(&change).contains(&"other-eligible"),
        "{:?}",
        keys(&change)
    );
    assert_eq!(after(&change, "after-other-eligible"), Some("freight"));
}

#[tokio::test]
async fn a_link_says_what_it_was_joined_on() {
    let reader = one_cluster(|_| {});
    let change = read(&reader, &place("f-new"), observed_at())
        .await
        .expect("read");

    for hop in &change.hops {
        let Shows::Hop(detail) = &hop.shows else {
            continue;
        };
        let Some(link) = &detail.link else { continue };
        // `by` is the join's word for the key; `key` is its value.
        match link.by.as_str() {
            "nothing" => assert_eq!(link.key, None, "{}", hop.key),
            _ => assert!(
                link.key.as_deref().is_some_and(|key| !key.is_empty()),
                "{} joined by {} but names no key",
                hop.key,
                link.by
            ),
        }
    }
    let freight = change.hops.iter().find(|hop| hop.key == "freight").unwrap();
    assert!(detail_key(freight).is_some(), "{freight:?}");
}

#[tokio::test]
async fn the_kargo_pieces_that_couldnt_be_read_hang_from_the_freight() {
    let reader = one_cluster(|world| {
        world.kargo = std::mem::take(&mut world.kargo).refusing("stages");
    });
    let change = read(&reader, &place("f-new"), observed_at())
        .await
        .expect("read");

    let line = change
        .missing
        .iter()
        .find(|line| line.text.contains("Stages"))
        .unwrap_or_else(|| panic!("{:?}", change.missing));
    assert_eq!(line.after.as_deref(), Some("freight"));
    assert_eq!(after(&change, "stages"), Some("freight"));
}
