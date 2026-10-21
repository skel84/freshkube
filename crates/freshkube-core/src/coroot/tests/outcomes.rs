use super::*;

// What Coroot found after a deploy (`outcome.rs`).

async fn server_with_apps() -> Server {
    server(vec![(200, shop_apps())]).await
}

fn shop_apps() -> serde_json::Value {
    serde_json::json!({"applications":[
        {"id":"c:shop:Deployment:auth","status":"ok","category":"application"}]})
}

/// The application's page with one revision, `9c41e7` started at 1789998000 s.
fn auth_page() -> serde_json::Value {
    let mut wire = app_view_wire();
    let rows = serde_json::json!([deployment_row(
        "9c41e7:1789998000",
        "9c41e7: example.test/shop/auth:1.4.0",
        serde_json::json!([{"report":"SLO","ok":false,"message":"Availability: 97% (objective: 99%)","time":null}])
    )]);
    wire["data"]["reports"]
        .as_array_mut()
        .unwrap()
        .push(deployments_report(rows));
    wire
}

fn entry(id: &str, coroot: Option<(Option<&str>, &str)>) -> crate::workspace::Entry {
    let mut entry = crate::workspace::Entry::new(id, crate::workspace::Role::Core, id);
    entry.coroot = coroot.map(|(project, cluster)| {
        let mut link = crate::setup::EntryCoroot::new(cluster);
        link.project = project.map(str::to_owned);
        Box::new(link)
    });
    entry
}

fn link_of(id: &str, project: &str, cluster: &str) -> Link {
    Link::of(&entry(id, Some((None, cluster))), Some(project)).unwrap()
}

fn query(link: Link) -> Query {
    Query {
        link,
        workload: Workload::Deployment,
        namespace: "shop".into(),
        name: "auth".into(),
        hash: Some("9c41e7".into()),
    }
}

fn now() -> chrono::DateTime<chrono::Utc> {
    chrono::DateTime::from_timestamp(1_790_000_000, 0).unwrap()
}

fn window() -> RevisionWindow {
    RevisionWindow::both(Duration::from_secs(1800))
}

async fn look(responses: Vec<(u16, serde_json::Value)>, query: &Query) -> (Outcome, Vec<String>) {
    let server = server(responses).await;
    let provider = Provider::new(&server.url, Credentials::None).unwrap();
    let answer = provider.outcome(query, None, window(), now()).await;
    assert!(answer.is_for(query, window(), provider.id()));
    let requests = server.requests.lock().unwrap().clone();
    (answer.outcome, requests)
}

#[tokio::test]
async fn a_found_revision_carries_its_window_and_costs_three_requests() {
    let q = query(link_of("cluster-a", "shop-api", "c"));
    let (outcome, requests) = look(
        vec![(200, shop_apps()), (200, auth_page()), (200, auth_page())],
        &q,
    )
    .await;
    let Outcome::Found(view) = outcome else {
        panic!("{outcome:?}");
    };
    assert_eq!(view.revision.hash, "9c41e7");
    assert_eq!(view.revision.findings[0].report, "SLO");
    assert_eq!(view.from.timestamp(), 1_789_996_200);
    assert_eq!(view.to.timestamp(), 1_789_999_800);
    // The applications list, the application's page, then the window; each
    // names the link's project and the application of the link's cluster.
    assert_eq!(requests.len(), 3);
    assert!(
        requests[0].contains("/api/project/shop-api/overview/applications"),
        "{}",
        requests[0]
    );
    assert!(
        requests[1].contains("/api/project/shop-api/app/"),
        "{}",
        requests[1]
    );
    assert!(
        requests[2].contains("from=1789996200000"),
        "{}",
        requests[2]
    );
}

#[tokio::test]
async fn listed_applications_save_a_request() {
    let q = query(link_of("cluster-a", "shop-api", "c"));
    let server = server(vec![
        (200, shop_apps()),
        (200, auth_page()),
        (200, auth_page()),
    ])
    .await;
    let provider = Provider::new(&server.url, Credentials::None).unwrap();
    let source = provider.linked_source("shop-api");
    let apps = provider.applications(&source, range()).await.unwrap();
    let listed = Listed::new(provider.id(), q.link.clone(), apps);
    server.requests.lock().unwrap().clear();
    let answer = provider.outcome(&q, Some(&listed), window(), now()).await;
    assert!(matches!(answer.outcome, Outcome::Found(_)));
    assert_eq!(server.requests.lock().unwrap().len(), 2);
    // A list that holds no such application is an answer.
    let empty = Listed::new(provider.id(), q.link.clone(), vec![]);
    let answer = provider.outcome(&q, Some(&empty), window(), now()).await;
    assert!(matches!(answer.outcome, Outcome::NoApplication));
}

#[tokio::test]
async fn a_look_hands_back_the_list_it_read_and_none_it_was_given() {
    let q = query(link_of("cluster-a", "shop-api", "c"));
    let server = server(vec![
        (200, shop_apps()),
        (200, auth_page()),
        (200, auth_page()),
        (200, auth_page()),
        (200, auth_page()),
    ])
    .await;
    let provider = Provider::new(&server.url, Credentials::None).unwrap();
    let first = provider.outcome(&q, None, window(), now()).await;
    assert!(matches!(first.outcome, Outcome::Found(_)));
    let listed = first.listed.expect("the look read the list");
    assert_eq!(server.requests.lock().unwrap().len(), 3);
    // Handed back to the next look, it costs two requests and returns none.
    let second = provider.outcome(&q, Some(&listed), window(), now()).await;
    assert!(matches!(second.outcome, Outcome::Found(_)));
    assert!(second.listed.is_none());
    assert_eq!(server.requests.lock().unwrap().len(), 5);
}

#[tokio::test]
async fn a_list_from_another_link_or_connection_is_read_again() {
    let q = query(link_of("cluster-a", "shop-api", "c"));
    let none = serde_json::json!({"applications":[]});
    let server = server(vec![(200, none.clone()), (200, none.clone()), (200, none)]).await;
    let provider = Provider::new(&server.url, Credentials::None).unwrap();
    let other = Provider::new(&server.url, Credentials::None).unwrap();
    let app = {
        let lister = server_with_apps().await;
        let lister_provider = Provider::new(&lister.url, Credentials::None).unwrap();
        let source = lister_provider.linked_source("shop-api");
        lister_provider
            .applications(&source, range())
            .await
            .unwrap()
            .remove(0)
    };
    // Each list holds the application, so trusting it would find it; the
    // fake's own lists hold none, so a re-read says there is none.
    let foreign = [
        Listed::new(
            provider.id(),
            link_of("cluster-c", "shop-api", "c"),
            vec![app.clone()],
        ),
        Listed::new(
            provider.id(),
            link_of("cluster-a", "shop-web", "c"),
            vec![app.clone()],
        ),
        Listed::new(other.id(), q.link.clone(), vec![app]),
    ];
    for listed in &foreign {
        let answer = provider.outcome(&q, Some(listed), window(), now()).await;
        assert!(
            matches!(answer.outcome, Outcome::NoApplication),
            "{:?}",
            answer.outcome
        );
    }
    // Each read the list afresh.
    assert_eq!(server.requests.lock().unwrap().len(), 3);
}

#[tokio::test]
async fn each_way_a_look_can_end_is_its_own_outcome() {
    let q = query(link_of("cluster-a", "shop-api", "c"));
    let apps = || (200, shop_apps());
    // No application of that cluster in the list.
    let (outcome, requests) = look(vec![(200, serde_json::json!({"applications":[]}))], &q).await;
    assert!(matches!(outcome, Outcome::NoApplication));
    assert_eq!(requests.len(), 1);
    // A project the list's 404 says is not there.
    let (outcome, _) = look(vec![(404, serde_json::json!("Not found\n"))], &q).await;
    assert!(matches!(outcome, Outcome::NoProject), "{outcome:?}");
    // The revisions don't hold the hash.
    let mut other = q.clone();
    other.hash = Some("ffffff".into());
    let (outcome, requests) = look(vec![apps(), (200, auth_page())], &other).await;
    assert!(matches!(outcome, Outcome::NoRevision));
    assert_eq!(requests.len(), 2);
    // Refused, by the page and by the window.
    let (outcome, _) = look(vec![(403, serde_json::json!({}))], &q).await;
    assert!(matches!(outcome, Outcome::Refused(Refused::Application)));
    let (outcome, _) = look(
        vec![apps(), (200, auth_page()), (403, serde_json::json!({}))],
        &q,
    )
    .await;
    assert!(matches!(outcome, Outcome::Refused(Refused::Window)));
    // Coroot's own 404 for the window is no data, never a missing application.
    let (outcome, _) = look(
        vec![
            apps(),
            (200, auth_page()),
            (404, serde_json::json!("Application not found\n")),
        ],
        &q,
    )
    .await;
    assert!(matches!(outcome, Outcome::NoData), "{outcome:?}");
    // A failure keeps its kind and says nothing from the server.
    let (outcome, _) = look(
        vec![apps(), (500, serde_json::json!({"error":"secret"}))],
        &q,
    )
    .await;
    assert!(matches!(outcome, Outcome::Failed(ReadError::Failed)));
}

#[tokio::test]
async fn charts_that_stop_before_the_window_ends_say_where() {
    let q = query(link_of("cluster-a", "shop-api", "c"));
    let mut early = auth_page();
    // Coroot's data ends at 1789998600 s, 20 minutes before the window does.
    let ctx = serde_json::json!({"from":1789996400000_i64,"to":1789998600000_i64,"step":60000,"raw_step":60000,"truncated":false});
    let text = early.to_string().replace(
        &serde_json::json!({"from":1789996400000_i64,"to":1790000000000_i64,"step":60000,"raw_step":60000,"truncated":false}).to_string(),
        &ctx.to_string(),
    );
    early = serde_json::from_str(&text).unwrap();
    let (outcome, _) = look(
        vec![(200, shop_apps()), (200, auth_page()), (200, early)],
        &q,
    )
    .await;
    let Outcome::EndsEarly(view, at) = outcome else {
        panic!("{outcome:?}");
    };
    assert!(at < view.to, "{at} {}", view.to);
}

#[tokio::test]
async fn a_page_without_charts_is_no_data() {
    let q = query(link_of("cluster-a", "shop-api", "c"));
    let mut bare = auth_page();
    bare["data"]["reports"] = bare["data"]["reports"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|r| r["name"] == "Deployments")
        .cloned()
        .collect();
    let (outcome, _) = look(
        vec![(200, shop_apps()), (200, auth_page()), (200, bare)],
        &q,
    )
    .await;
    assert!(matches!(outcome, Outcome::NoData), "{outcome:?}");
}

#[tokio::test]
async fn nothing_is_read_for_a_rollout_or_an_unconfirmed_replica_set() {
    let server = server(vec![]).await;
    let provider = Provider::new(&server.url, Credentials::None).unwrap();
    let mut q = query(link_of("cluster-a", "shop-api", "c"));
    q.workload = Workload::Rollout;
    let answer = provider.outcome(&q, None, window(), now()).await;
    assert!(matches!(answer.outcome, Outcome::NotDeployment));
    q.workload = Workload::Deployment;
    for hash in [None, Some(String::new())] {
        q.hash = hash;
        let answer = provider.outcome(&q, None, window(), now()).await;
        assert!(matches!(answer.outcome, Outcome::NotLooked));
    }
    assert!(server.requests.lock().unwrap().is_empty());
}

#[tokio::test]
async fn another_application_s_page_never_answers_for_this_one() {
    let q = query(link_of("cluster-a", "shop-api", "c"));
    let mut foreign = auth_page();
    foreign["data"]["app_map"]["application"]["id"] =
        serde_json::json!("c:shop:Deployment:billing");
    let (outcome, _) = look(vec![(200, shop_apps()), (200, foreign)], &q).await;
    assert!(
        matches!(outcome, Outcome::Failed(ReadError::InvalidResponse)),
        "{outcome:?}"
    );
}

#[test]
fn a_link_is_the_entry_s_own_and_matching_names_link_nothing() {
    // Its own project wins over the connection's.
    let own = Link::of(
        &entry("cluster-c", Some((Some("shop-web"), "c"))),
        Some("shop-api"),
    );
    assert_eq!(own.unwrap().project(), "shop-web");
    let inherited = Link::of(&entry("cluster-a", Some((None, "c"))), Some("shop-api")).unwrap();
    assert_eq!(
        (inherited.entry(), inherited.project(), inherited.cluster()),
        ("cluster-a", "shop-api", "c")
    );
    // A context called like a Coroot cluster is not linked until it says so.
    let unlinked = Link::of(&entry("cluster-a", None), Some("shop-api"));
    assert!(matches!(unlinked, Err(Outcome::NotLinked)));
    let no_project = Link::of(&entry("cluster-a", Some((None, "c"))), None);
    assert!(matches!(no_project, Err(Outcome::NoProjectForEntry)));
    // An explicit blank project leaves a named cluster without its project,
    // even when the connection names one; a blank cluster is no link.
    for connection in [None, Some("shop-api")] {
        assert!(matches!(
            Link::of(&entry("cluster-a", Some((Some(" "), "c"))), connection),
            Err(Outcome::NoProjectForEntry)
        ));
    }
    assert!(matches!(
        Link::of(&entry("cluster-a", Some((None, " "))), Some("shop-api")),
        Err(Outcome::NotLinked)
    ));
}

#[test]
fn an_answer_is_for_its_entry_link_window_and_connection_only() {
    let server_url = "http://127.0.0.1:1";
    let provider = Provider::new(server_url, Credentials::None).unwrap();
    let reconnected = Provider::new(server_url, Credentials::None).unwrap();
    let a = query(link_of("cluster-a", "shop-api", "c"));
    let answer = Answer {
        query: a.clone(),
        window: window(),
        provider: provider.id(),
        outcome: Outcome::NoRevision,
        listed: None,
    };
    assert!(answer.is_for(&a, window(), provider.id()));
    // The same Coroot application asked for by another entry.
    let c = query(link_of("cluster-c", "shop-api", "c"));
    assert!(!answer.is_for(&c, window(), provider.id()));
    // The link edited: another cluster, another project, another hash.
    for edited in [
        query(link_of("cluster-a", "shop-api", "d")),
        query(link_of("cluster-a", "shop-web", "c")),
    ] {
        assert!(!answer.is_for(&edited, window(), provider.id()));
    }
    let mut rehashed = a.clone();
    rehashed.hash = Some("ffffff".into());
    assert!(!answer.is_for(&rehashed, window(), provider.id()));
    // Another window, and a reconnected server with the same project.
    let wider = RevisionWindow::both(Duration::from_secs(3600));
    assert!(!answer.is_for(&a, wider, provider.id()));
    assert!(!answer.is_for(&a, window(), reconnected.id()));
}

#[tokio::test(start_paused = true)]
async fn a_look_that_hears_nothing_ends_in_a_timeout_on_the_test_clock() {
    // A server that accepts and never answers.
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let held = tokio::spawn(async move {
        let mut sockets = vec![];
        loop {
            sockets.push(listener.accept().await.unwrap());
        }
    });
    let provider = Provider::new(&url, Credentials::None).unwrap();
    let q = query(link_of("cluster-a", "shop-api", "c"));
    let started = tokio::time::Instant::now();
    // Below the client's 5 s connect timeout: the paused clock jumps ahead
    // while the connect is still in flight, and a longer limit let that
    // timeout win as Unreachable.
    let limit = Duration::from_secs(3);
    let answer = provider
        .outcome_within(limit, &q, None, window(), now())
        .await;
    assert!(
        matches!(answer.outcome, Outcome::Failed(ReadError::Timeout)),
        "{:?}",
        answer.outcome
    );
    assert_eq!(started.elapsed(), limit);
    // With the whole look's limit, a server that says nothing ends the look
    // at the provider's read deadline, as a failure either way.
    let answer = provider.outcome(&q, None, window(), now()).await;
    assert!(matches!(answer.outcome, Outcome::Failed(_)));
    assert!(started.elapsed() <= limit + Duration::from_secs(50));
    held.abort();
}

#[tokio::test]
async fn dropping_the_future_cancels_the_look() {
    use std::sync::atomic::{AtomicBool, Ordering};
    // A server that notes when the connection it holds is closed.
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let closed = Arc::new(AtomicBool::new(false));
    let flag = closed.clone();
    let (arrived, request) = tokio::sync::oneshot::channel();
    let held = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        let mut bytes = vec![0; 16384];
        let mut arrived = Some(arrived);
        // The request, then nothing until the client goes away.
        while socket.read(&mut bytes).await.is_ok_and(|n| n > 0) {
            if let Some(arrived) = arrived.take() {
                let _ = arrived.send(());
            }
        }
        flag.store(true, Ordering::SeqCst);
    });
    let provider = Provider::new(&url, Credentials::None).unwrap();
    let q = query(link_of("cluster-a", "shop-api", "c"));
    {
        let look = provider.outcome(&q, None, window(), now());
        // Drop the look once the server holds its request, not after a
        // timer: a timer could fire before the client had connected, and
        // then nothing would ever close.
        tokio::select! {
            _ = look => panic!("the server never answers"),
            _ = request => {}
        }
    }
    for _ in 0..400 {
        if closed.load(Ordering::SeqCst) {
            break;
        }
        tokio::task::yield_now().await;
        std::thread::sleep(Duration::from_millis(5));
    }
    assert!(
        closed.load(Ordering::SeqCst),
        "the look kept its connection"
    );
    held.abort();
}

#[tokio::test]
async fn after_a_rollback_the_newest_revision_of_the_hash_is_the_one() {
    let q = query(link_of("cluster-a", "shop-api", "c"));
    // Rolling back to an old ReplicaSet re-uses its hash: Coroot keeps a
    // revision per rollout, newest first, and the running ReplicaSet is the
    // latest of them.
    let finding =
        || serde_json::json!([{"report":"SLO","ok":true,"message":"No change","time":null}]);
    let mut page = auth_page();
    let rows = serde_json::json!([
        deployment_row("9c41e7:1789998000", "9c41e7: auth:1.3.9", finding()),
        deployment_row("2b70aa:1789994000", "2b70aa: auth:1.4.0", finding()),
        deployment_row("9c41e7:1789990000", "9c41e7: auth:1.3.9", finding()),
    ]);
    let reports = page["data"]["reports"].as_array_mut().unwrap();
    reports.retain(|r| r["name"] != "Deployments");
    reports.push(deployments_report(rows));
    let (outcome, _) = look(
        vec![(200, shop_apps()), (200, page.clone()), (200, page)],
        &q,
    )
    .await;
    let Outcome::Found(view) = outcome else {
        panic!("{outcome:?}");
    };
    assert_eq!(view.revision.id, "9c41e7:1789998000");
}
