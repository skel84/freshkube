use chrono::{DateTime, Duration, Utc};
use serde_json::json;

use super::*;
use crate::change::example::Shape;
use crate::changes::live::Where;
use crate::fixtures::FixtureReader;
use crate::github::fake::{Reply, Server};
use crate::github::{Handle, PullState};
use crate::kargo::GROUP;

fn now() -> DateTime<Utc> {
    DateTime::from_timestamp(1_760_000_000, 0).unwrap()
}

fn example() -> Waiting {
    derive(&example::read(now()))
}

fn app<'a>(waiting: &'a Waiting, name: &str) -> &'a App {
    waiting
        .apps
        .iter()
        .find(|app| app.name == name)
        .unwrap_or_else(|| panic!("{name} is listed"))
}

fn freights(app: &App) -> Vec<&str> {
    app.items.iter().map(|item| item.freight.as_str()).collect()
}

#[test]
fn production_is_every_stage_with_nothing_downstream() {
    let waiting = example();
    let checkout = app(&waiting, "checkout");
    assert_eq!(checkout.production(), ["prod-ams", "prod-lon"]);
    assert_eq!(app(&waiting, "catalog").production(), ["prod"]);
    let stages: Vec<&str> = checkout.stages.iter().map(|s| s.name.as_str()).collect();
    assert_eq!(stages, ["dev", "stage", "prod-ams", "prod-lon"]);
}

#[test]
fn a_fan_out_has_a_gap_per_final_stage_and_the_group_shows_the_furthest_behind() {
    let waiting = example();
    let checkout = app(&waiting, "checkout");
    let gaps: Vec<(&str, usize)> = checkout
        .finals
        .iter()
        .map(|gap| (gap.stage.as_str(), gap.ahead.len()))
        .collect();
    // prod-lon runs quiet-heron, so it is three behind; prod-ams, four.
    assert_eq!(gaps, [("prod-ams", 4), ("prod-lon", 3)]);
    assert_eq!(checkout.furthest.as_deref(), Some("prod-ams"));
    assert_eq!(checkout.state, State::Waiting);
    assert_eq!(
        checkout.items.len(),
        4,
        "the group's rows are prod-ams's gap"
    );
    let lon = &checkout.finals[1];
    assert_eq!(lon.ahead, ["merry-owl", "wonky-otter", "tidy-badger"]);
}

#[test]
fn the_wait_runs_from_the_stage_before_production_not_the_merge() {
    let waiting = example();
    let checkout = app(&waiting, "checkout");
    let heron = checkout
        .items
        .iter()
        .find(|item| item.freight == "quiet-heron")
        .unwrap();
    assert_eq!(heron.in_stage.as_deref(), Some("stage"));
    // Its verification in `stage` started 3 days less 70 minutes ago.
    assert_eq!(
        heron.waited(now()),
        Some(Duration::days(3) - Duration::minutes(70))
    );
    // Merged is its own fact, from GitHub, and earlier than the wait began.
    let merged = heron.pull().and_then(|pull| pull.merged_at).unwrap();
    assert!(merged < heron.reached.unwrap());
}

#[test]
fn a_gap_is_oldest_first_and_an_undated_wait_comes_last() {
    let waiting = example();
    assert_eq!(
        freights(app(&waiting, "checkout")),
        ["quiet-heron", "merry-owl", "wonky-otter", "tidy-badger"]
    );
    let tidy = &app(&waiting, "checkout").items[3];
    assert_eq!(
        tidy.reached, None,
        "no Stage has run it, so nothing dates it"
    );
    assert_eq!(tidy.waited(now()), None);
}

#[test]
fn why_is_in_the_change_pages_cause_words() {
    let waiting = example();
    let why = |app: &str, freight: &str| {
        let item = app_item(&waiting, app, freight);
        (item.why.word(), item.why.needs().to_owned())
    };
    assert_eq!(
        why("checkout", "quiet-heron"),
        ("Waiting", "a promotion from stage to prod-ams".into())
    );
    assert_eq!(
        why("checkout", "merry-owl"),
        (
            "Waiting",
            "stage's verification to pass; its last one failed".into()
        )
    );
    assert!(app_item(&waiting, "checkout", "merry-owl").why.blocked);
    assert!(!app_item(&waiting, "checkout", "quiet-heron").why.blocked);
    assert_eq!(
        why("checkout", "wonky-otter"),
        ("Waiting", "a promotion from dev to stage".into())
    );
    assert_eq!(
        why("checkout", "tidy-badger"),
        ("Waiting", "its first promotion, into dev".into())
    );
}

fn app_item<'a>(waiting: &'a Waiting, app_name: &str, freight: &str) -> &'a Item {
    app(waiting, app_name)
        .items
        .iter()
        .find(|item| item.freight == freight)
        .unwrap()
}

#[test]
fn what_promoting_now_would_ship_is_the_newest_the_stage_before_verified() {
    let waiting = example();
    let checkout = app(&waiting, "checkout");
    let next = checkout.finals[0]
        .next
        .as_ref()
        .expect("quiet-heron is verified");
    assert_eq!(next.freight, "quiet-heron");
    assert_eq!(next.brings, ["quiet-heron"]);
    // prod-lon takes the same Freight from `stage`, but already runs it.
    assert_eq!(checkout.finals[1].next, None);
    let catalog = app(&waiting, "catalog");
    assert_eq!(
        catalog.finals[0].next.as_ref().unwrap().freight,
        "plucky-newt"
    );
}

#[test]
fn apps_that_were_not_read_are_shown_with_why_never_left_out() {
    let waiting = example();
    for (name, gap) in [("payments", Gap::Refused), ("billing", Gap::Unreadable)] {
        let app = app(&waiting, name);
        assert_eq!(app.fold(), Fold::NotRead, "{name}");
        match &app.state {
            State::NotRead {
                gap: held,
                cause,
                text,
            } => {
                assert_eq!(*held, gap);
                assert_eq!(cause.word(), "Can't read");
                assert!(text.contains(name), "{text}");
            }
            other => panic!("{other:?}"),
        }
    }
    let refused = app(&waiting, "payments");
    let State::NotRead { cause, .. } = &refused.state else {
        unreachable!()
    };
    assert!(matches!(
        cause,
        Cause::CantRead {
            retry: false,
            role: Some(_),
            ..
        }
    ));
    assert!(
        waiting
            .notes
            .iter()
            .any(|note| note.project.as_deref() == Some("payments"))
    );
}

#[test]
fn the_example_has_a_capped_project_and_a_limit_that_skipped_the_newest() {
    let waiting = example();
    assert_eq!(app(&waiting, "catalog").older, 4);
    assert_eq!(app(&waiting, "checkout").older, 0);
    let limited = derive(&example::read_shaped(now(), Shape::GitHubLimited));
    let skipped = |freight| {
        matches!(
            app_item(&limited, "checkout", freight).pull,
            Some(PullRead::Unread {
                rate_limit: true,
                ..
            })
        )
    };
    assert!(skipped("tidy-badger") && skipped("wonky-otter"));
    assert!(
        !skipped("quiet-heron"),
        "older pull requests are kept, stale"
    );
}

#[test]
fn caught_up_apps_fold_into_one_group_at_the_bottom() {
    let waiting = example();
    let order: Vec<(Fold, &str)> = waiting
        .apps
        .iter()
        .map(|app| (app.fold(), app.name.as_str()))
        .collect();
    assert_eq!(
        order,
        [
            (Fold::Waiting, "checkout"),
            (Fold::Waiting, "catalog"),
            (Fold::NotRead, "billing"),
            (Fold::NotRead, "payments"),
            (Fold::CaughtUp, "cart"),
            (Fold::CaughtUp, "search"),
        ]
    );
    assert!(app(&waiting, "cart").items.is_empty());
}

#[test]
fn groups_come_longest_wait_first() {
    let waiting = example();
    let now = waiting.observed_at.unwrap();
    let waits: Vec<Option<Duration>> = waiting
        .apps
        .iter()
        .filter(|app| app.fold() == Fold::Waiting)
        .map(|app| app.longest(now))
        .collect();
    assert!(waits[0] > waits[1], "{waits:?}");
}

#[test]
fn the_find_box_reads_a_pull_request_number_or_a_commit() {
    assert_eq!(Query::parse("418"), Some(Query::Pull(418)));
    assert_eq!(Query::parse(" #418 "), Some(Query::Pull(418)));
    assert_eq!(
        Query::parse("https://github.com/acme/checkout/pull/418/files"),
        Some(Query::Pull(418))
    );
    assert_eq!(
        Query::parse("A1F3c9E"),
        Some(Query::Commit("a1f3c9e".into()))
    );
    assert_eq!(Query::parse("a1f"), None, "too short for a commit");
    assert_eq!(Query::parse("retry keys"), None);
    assert_eq!(Query::parse(""), None);
}

#[test]
fn the_find_box_narrows_by_pull_request_and_by_commit() {
    let waiting = example();
    let checkout = app(&waiting, "checkout");
    assert_eq!(checkout.find(&Query::Pull(418)), ["wonky-otter"]);
    assert_eq!(checkout.find(&Query::Pull(999)), Vec::<&str>::new());
    assert_eq!(
        checkout.find(&Query::Commit("a1f3c9e".into())),
        ["wonky-otter"]
    );
    assert_eq!(
        checkout.find(&Query::Commit("a1f3".into())),
        ["wonky-otter"]
    );
    // Another app has neither.
    assert!(app(&waiting, "catalog").find(&Query::Pull(418)).is_empty());
}

#[test]
fn a_commit_that_already_shipped_is_found_and_traced_to_production() {
    let waiting = example();
    let checkout = app(&waiting, "checkout");
    // calm-wren is in no gap: both regions run it.
    let query = Query::Commit("5d20b1a".into());
    assert_eq!(checkout.find(&query), ["calm-wren"]);
    assert_eq!(checkout.shipped("calm-wren"), Shipped::Everywhere);
    assert_eq!(checkout.shipped("quiet-heron"), Shipped::Partly);
    assert_eq!(checkout.shipped("wonky-otter"), Shipped::No);
    let trace = checkout.trace("quiet-heron");
    let standing: Vec<(&str, Standing)> = trace
        .iter()
        .map(|(stage, how)| (stage.as_str(), *how))
        .collect();
    assert_eq!(
        standing,
        [
            ("dev", Standing::Verified),
            ("stage", Standing::Ran),
            ("prod-ams", Standing::Not),
            ("prod-lon", Standing::Runs),
        ]
    );
    // Pull requests are asked for only for Freight in a gap.
    assert_eq!(checkout.find(&Query::Pull(409)), Vec::<&str>::new());
}

#[test]
fn a_kargo_that_was_not_reached_is_the_pages_own_state() {
    let waiting = derive(&example::read_shaped(now(), Shape::KargoUnreachable));
    assert!(waiting.apps.is_empty());
    assert_eq!(waiting.notes.len(), 1);
    assert_eq!(waiting.notes[0].gap, Gap::Unreadable);
}

#[test]
fn github_trouble_keeps_or_withholds_pull_requests_without_hiding_the_gap() {
    let refused = derive(&example::read_shaped(now(), Shape::GitHubRefused));
    let item = app_item(&refused, "checkout", "wonky-otter");
    assert!(matches!(item.pull, Some(PullRead::Unread { .. })));
    assert_eq!(
        item.title.as_deref(),
        Some("Retry idempotency keys"),
        "the commit's subject"
    );
    assert!(
        refused
            .notes
            .iter()
            .any(|note| note.text.contains("refused"))
    );
    let limited = derive(&example::read_shaped(now(), Shape::GitHubLimited));
    let pull = app_item(&limited, "checkout", "quiet-heron")
        .pull()
        .unwrap();
    assert!(pull.stale.as_deref().unwrap().contains("rate limit"));
    assert_eq!(pull.state, PullState::Merged);
}

#[test]
fn pull_requests_not_read_is_said_once_not_guessed() {
    let mut read = example::read(now());
    read.pulls_read = false;
    if let Source::Read(projects) = &mut read.projects {
        projects
            .iter_mut()
            .for_each(|project| project.pulls.clear());
    }
    let waiting = derive(&read);
    let note = waiting
        .notes
        .iter()
        .filter(|n| n.text.contains("Pull requests aren't read"));
    assert_eq!(note.count(), 1);
    assert!(
        app_item(&waiting, "checkout", "wonky-otter")
            .pull()
            .is_none()
    );
}

fn at(github: Option<Handle>) -> Where {
    Where {
        cluster: "example:core".into(),
        label: "core-fra".into(),
        argocd_namespace: "argocd".into(),
        mapping: Default::default(),
        sources: Default::default(),
        public_key_file: None,
        transparency_log: false,
        transparency_url: None,
        evidence_result: None,
        build_namespace: None,
        stage_naming: None,
        github,
    }
}

/// A project of `count` Freight, newest first by name `f-0`, on GitHub,
/// with one Stage that runs the oldest.
fn crowded(count: usize) -> FixtureReader {
    let freight: Vec<_> = (0..count)
        .map(|n| {
            let made = now() - Duration::minutes(n as i64);
            json!({
                "metadata": {
                    "namespace": "shop-api", "name": format!("f-{n}"),
                    "creationTimestamp": made.to_rfc3339(),
                },
                "commits": [{
                    "repoURL": "https://github.com/acme/shop-api",
                    "id": format!("{n:040x}"),
                }],
            })
        })
        .collect();
    FixtureReader::default()
        .serves(GROUP, "v1alpha1", &["projects", "freights", "stages"])
        .with("projects", vec![json!({"metadata": {"name": "shop-api"}})])
        .with("freights", freight)
        .with(
            "stages",
            vec![json!({
                "metadata": {"namespace": "shop-api", "name": "prod"},
                "spec": {"requestedFreight": [{"sources": {"direct": true}}]},
                "status": {"freightHistory": [{"items": {"W/x": {"name": format!("f-{}", count - 1)}}}]},
            })],
        )
}

fn pulls_for(request_path: &str) -> serde_json::Value {
    let sha = request_path
        .split('/')
        .nth(5)
        .unwrap_or_default()
        .to_owned();
    json!([{
        "number": 7, "title": "A change", "state": "closed",
        "merged_at": "2026-10-01T10:00:00Z", "merge_commit_sha": sha,
        "head": {"sha": "b".repeat(40)}, "base": {"ref": "main"},
        "user": {"login": "ken"},
    }])
}

#[tokio::test]
async fn a_live_read_asks_github_once_per_freight_in_a_gap_and_no_more_than_the_cap() {
    let api = Server::new(|request| Reply::json(200, "", &pulls_for(&request.path)));
    let at = at(Some(Handle::trusting(&api.address, None)));
    let reader = crowded(60);
    let read = live::read(&reader, &at, now(), live::BUDGET).await;
    let waiting = derive(&read);
    let shop = app(&waiting, "shop-api");
    // The Stage runs the oldest of 60, which the newest 50 don't hold: all
    // 50 listed may be ahead of it, and there may be more.
    assert_eq!(shop.items.len(), MAX_FREIGHT);
    assert!(shop.finals[0].at_least);
    assert_eq!(
        api.seen().len(),
        MAX_FREIGHT,
        "one call per Freight in the gap"
    );
    assert!(
        waiting
            .notes
            .iter()
            .any(|note| note.gap == Gap::Capped && note.text.contains("10 older")),
        "{:?}",
        waiting.notes
    );
    assert_eq!(shop.items[0].pull().map(|pull| pull.number), Some(7));
}

#[tokio::test]
async fn a_short_gap_costs_one_call_per_freight_in_it_and_none_for_the_rest() {
    let api = Server::new(|request| Reply::json(200, "", &pulls_for(&request.path)));
    let at = at(Some(Handle::trusting(&api.address, None)));
    // Five Freight; the Stage runs the third newest, so two are ahead.
    let freight: Vec<_> = (0..5)
        .map(|n| {
            json!({
                "metadata": {
                    "namespace": "shop-api", "name": format!("f-{n}"),
                    "creationTimestamp": (now() - Duration::minutes(n)).to_rfc3339(),
                },
                "commits": [{
                    "repoURL": "https://github.com/acme/shop-api",
                    "id": format!("{n:040x}"),
                }],
            })
        })
        .collect();
    let reader = crowded(1).with("freights", freight).with(
        "stages",
        vec![json!({
            "metadata": {"namespace": "shop-api", "name": "prod"},
            "spec": {"requestedFreight": [{"sources": {"direct": true}}]},
            "status": {"freightHistory": [{"items": {"W/x": {"name": "f-2"}}}]},
        })],
    );
    let waiting = derive(&live::read(&reader, &at, now(), live::BUDGET).await);
    let shop = app(&waiting, "shop-api");
    assert_eq!(freights(shop), ["f-1", "f-0"]);
    assert!(!shop.finals[0].at_least);
    assert_eq!(api.seen().len(), 2);
}

#[tokio::test]
async fn a_live_read_with_no_github_asks_nothing_and_says_so() {
    let waiting = derive(&live::read(&crowded(3), &at(None), now(), live::BUDGET).await);
    assert!(
        waiting
            .notes
            .iter()
            .any(|note| note.text.contains("Pull requests aren't read"))
    );
    assert!(
        app(&waiting, "shop-api")
            .items
            .iter()
            .all(|item| item.pull().is_none())
    );
}

#[tokio::test]
async fn a_live_read_says_why_kargo_wasnt_read() {
    let unserved = live::read(&FixtureReader::default(), &at(None), now(), live::BUDGET).await;
    assert!(matches!(unserved.projects, Source::NotInstalled(_)));
    let waiting = derive(&unserved);
    assert!(waiting.apps.is_empty());
    assert_eq!(waiting.notes[0].gap, Gap::NotServed);
}

/// A reader that counts the projects it is answering at once, each
/// listing taking `by` on Tokio's clock.
struct Counting {
    inner: FixtureReader,
    busy: std::cell::RefCell<std::collections::BTreeMap<String, usize>>,
    most: std::cell::Cell<usize>,
}

impl Counting {
    fn new(inner: FixtureReader) -> Self {
        Self {
            inner,
            busy: Default::default(),
            most: Default::default(),
        }
    }
}

impl crate::read::Reader for Counting {
    async fn get(
        &self,
        resource: &crate::read::Resource,
        namespace: Option<&str>,
        name: &str,
    ) -> Result<serde_json::Value, freshkube_core::resources::Failure> {
        self.inner.get(resource, namespace, name).await
    }

    async fn list(
        &self,
        request: &crate::read::ListRequest,
    ) -> Result<crate::read::Listing, freshkube_core::resources::Failure> {
        let project = match &request.scope {
            crate::read::Scope::Namespace(project) => Some(project.clone()),
            _ => None,
        };
        if let Some(project) = &project {
            let mut busy = self.busy.borrow_mut();
            *busy.entry(project.clone()).or_default() += 1;
            self.most.set(self.most.get().max(busy.len()));
        }
        let listing = self.inner.list(request).await;
        if let Some(project) = &project {
            let mut busy = self.busy.borrow_mut();
            if let Some(count) = busy.get_mut(project) {
                *count -= 1;
                if *count == 0 {
                    busy.remove(project);
                }
            }
        }
        listing
    }

    async fn groups(
        &self,
    ) -> Result<Vec<crate::read::ServedGroup>, freshkube_core::resources::Failure> {
        self.inner.groups().await
    }

    async fn plurals(
        &self,
        group: &str,
        version: &str,
    ) -> Result<Vec<String>, freshkube_core::resources::Failure> {
        self.inner.plurals(group, version).await
    }
}

fn projects_world(names: &[&str]) -> FixtureReader {
    let freight = |project: &str| {
        json!({
            "metadata": {
                "namespace": project, "name": format!("{project}-f"),
                "creationTimestamp": now().to_rfc3339(),
            },
            "commits": [{"repoURL": "https://git.example.test/acme/x", "id": "abc1234"}],
        })
    };
    let stage = |project: &str| {
        json!({
            "metadata": {"namespace": project, "name": "prod"},
            "spec": {"requestedFreight": [{"sources": {"direct": true}}]},
        })
    };
    FixtureReader::default()
        .serves(GROUP, "v1alpha1", &["projects", "freights", "stages"])
        .with(
            "projects",
            names
                .iter()
                .map(|name| json!({"metadata": {"name": name}}))
                .collect(),
        )
        .with("freights", names.iter().map(|name| freight(name)).collect())
        .with("stages", names.iter().map(|name| stage(name)).collect())
}

#[tokio::test(start_paused = true)]
async fn a_live_read_reads_no_more_than_four_projects_at_once() {
    let names = ["a", "b", "c", "d", "e", "f", "g", "h", "i", "j"];
    let reader = Counting::new(projects_world(&names).slow(std::time::Duration::from_secs(5)));
    let read = live::read(&reader, &at(None), now(), live::BUDGET).await;
    let Source::Read(projects) = &read.projects else {
        panic!("read: {:?}", read.projects)
    };
    assert_eq!(projects.len(), names.len(), "every project was read");
    assert_eq!(reader.most.get(), 4, "four at a time, and all four used");
}

#[tokio::test(start_paused = true)]
async fn a_project_late_past_the_budget_is_not_read_never_dropped() {
    let reader = projects_world(&["fine", "stuck"]).hanging_list("freights", "stuck");
    let read = live::read(&reader, &at(None), now(), live::BUDGET).await;
    let waiting = derive(&read);
    assert_eq!(waiting.apps.len(), 2, "the late project is still listed");
    let stuck = app(&waiting, "stuck");
    let State::NotRead { gap, cause, text } = &stuck.state else {
        panic!("{:?}", stuck.state)
    };
    assert_eq!(*gap, Gap::Unreadable);
    assert_eq!(cause.word(), "Can't read");
    assert!(text.contains("no answer within the read's 90 s"), "{text}");
    assert_eq!(app(&waiting, "fine").fold(), Fold::Waiting);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_pull_request_late_past_the_budget_is_unread_never_dropped() {
    let api = Server::new(|_| {
        std::thread::sleep(std::time::Duration::from_secs(6));
        Reply::json(200, "", &json!([]))
    });
    let mut world = projects_world(&["shop"]);
    world = world.with(
        "freights",
        vec![json!({
            "metadata": {
                "namespace": "shop", "name": "f",
                "creationTimestamp": now().to_rfc3339(),
            },
            "commits": [{"repoURL": "https://github.com/acme/shop", "id": format!("{:040x}", 1)}],
        })],
    );
    let at = at(Some(Handle::trusting(&api.address, None)));
    let budget = std::time::Duration::from_secs(2);
    let waiting = derive(&live::read(&world, &at, now(), budget).await);
    let item = &app(&waiting, "shop").items[0];
    match &item.pull {
        Some(PullRead::Unread { rate_limit, why }) => {
            assert!(!rate_limit);
            // The budget ran out, or the client's own deadline did first.
            assert!(why.starts_with("Pull requests weren't read: "), "{why}");
        }
        other => panic!("{other:?}"),
    }
}

#[tokio::test]
async fn a_low_rate_limit_skips_pull_requests_and_says_so_apart_from_other_failures() {
    // The first answers leave 0 calls, so the calls after them are skipped.
    let reset = (chrono::Utc::now() + chrono::Duration::hours(1)).timestamp();
    let api = Server::new(move |request| {
        let sha = request
            .path
            .split('/')
            .nth(5)
            .unwrap_or_default()
            .to_owned();
        Reply::json(
            200,
            &format!(
                "x-ratelimit-limit: 60\r\nx-ratelimit-remaining: 0\r\nx-ratelimit-reset: {reset}\r\n"
            ),
            &pulls_for(&format!("/a/b/c/d/e/{sha}")),
        )
    });
    let at = at(Some(Handle::trusting(&api.address, None)));
    let waiting = derive(&live::read(&crowded_on_github(8), &at, now(), live::BUDGET).await);
    let shop = app(&waiting, "shop-api");
    let limited = shop
        .items
        .iter()
        .filter(|item| {
            matches!(
                item.pull,
                Some(PullRead::Unread {
                    rate_limit: true,
                    ..
                })
            )
        })
        .count();
    let found = shop
        .items
        .iter()
        .filter(|item| item.pull().is_some())
        .count();
    // Some were in flight before the first answer told the limit; the
    // rest were skipped, and a skipped read sends nothing.
    assert!(
        found >= 1 && limited >= 1,
        "{found} read, {limited} skipped"
    );
    assert_eq!(found + limited, 8);
    assert_eq!(api.seen().len(), found);
    // A failure that isn't the limit is its own kind of unread.
    let other = Server::new(|_| Reply::empty(500, ""));
    let at = super::tests::at(Some(Handle::trusting(&other.address, None)));
    let waiting = derive(&live::read(&crowded_on_github(2), &at, now(), live::BUDGET).await);
    assert!(app(&waiting, "shop-api").items.iter().all(|item| matches!(
        item.pull,
        Some(PullRead::Unread {
            rate_limit: false,
            ..
        })
    )));
}

/// `count` Freight on GitHub, the Stage running none, so all are ahead.
fn crowded_on_github(count: usize) -> FixtureReader {
    crowded(count).with(
        "stages",
        vec![json!({
            "metadata": {"namespace": "shop-api", "name": "prod"},
            "spec": {"requestedFreight": [{"sources": {"direct": true}}]},
        })],
    )
}

#[tokio::test]
async fn the_group_counts_the_freight_the_cap_left_out() {
    let waiting = derive(&live::read(&crowded(60), &at(None), now(), live::BUDGET).await);
    let shop = app(&waiting, "shop-api");
    assert_eq!(shop.older, 10);
    let waiting = derive(&live::read(&crowded(5), &at(None), now(), live::BUDGET).await);
    assert_eq!(app(&waiting, "shop-api").older, 0);
}
