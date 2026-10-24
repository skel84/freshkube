use serde_json::json;

use super::*;
use crate::fake_api::{self, Recorded, Recorder};

fn alice() -> Who {
    Who {
        username: "alice".into(),
        groups: vec!["devs".into()],
    }
}

fn answer(rules: Vec<Rule>, incomplete: bool) -> RulesAnswer {
    RulesAnswer {
        rules,
        incomplete,
        evaluation_error: None,
    }
}

fn ns(name: &str) -> RowKey {
    RowKey::Namespace(name.into())
}

#[test]
fn a_column_is_yes_only_when_every_need_is_granted() {
    let cells = answer(vec![Rule::new(&["get", "list"], &[""], &["pods"])], false).cells();
    // Read also needs watch.
    let Cell::No(missing) = cells.cell(Column::Read) else {
        panic!("{:?}", cells.cell(Column::Read));
    };
    assert_eq!(missing.len(), 1);
    assert_eq!(
        missing[0].phrase(Some("shop")),
        "needs watch on pods in shop"
    );
    let cells = answer(
        vec![Rule::new(&["get", "list", "watch"], &[""], &["pods"])],
        false,
    )
    .cells();
    assert_eq!(cells.cell(Column::Read), &Cell::Yes);
}

#[test]
fn subresources_follow_rbac_matching() {
    let logs = |resource: &str| {
        answer(vec![Rule::new(&["get"], &[""], &[resource])], false)
            .cells()
            .cell(Column::Logs)
            .clone()
    };
    assert_eq!(logs("pods/log"), Cell::Yes);
    assert_eq!(logs("pods/*"), Cell::Yes);
    assert_eq!(logs("*/log"), Cell::Yes);
    assert_eq!(logs("*"), Cell::Yes);
    // The resource alone is not its subresource.
    assert!(matches!(logs("pods"), Cell::No(_)));
    assert!(matches!(logs("pods/exec"), Cell::No(_)));
}

#[test]
fn verbs_and_groups_must_match_and_wildcards_do() {
    let scale = |rule: Rule| {
        answer(vec![rule], false)
            .cells()
            .cell(Column::ScaleRestart)
            .clone()
    };
    let both = &["deployments", "deployments/scale"];
    assert_eq!(scale(Rule::new(&["patch"], &["apps"], both)), Cell::Yes);
    assert_eq!(scale(Rule::new(&["*"], &["*"], &["*"])), Cell::Yes);
    // Another group, or another verb, is not a grant.
    assert!(matches!(
        scale(Rule::new(&["patch"], &["extensions"], both)),
        Cell::No(_)
    ));
    assert!(matches!(
        scale(Rule::new(&["update"], &["apps"], both)),
        Cell::No(_)
    ));
}

#[test]
fn a_rule_naming_objects_is_not_a_grant_on_the_resource() {
    let mut rule = Rule::new(&["delete"], &[""], &["pods"]);
    rule.names = vec!["only-this-one".into()];
    let cells = answer(vec![rule], false).cells();
    assert!(matches!(cells.cell(Column::DeletePods), Cell::No(_)));
}

#[test]
fn an_incomplete_review_asks_instead_of_saying_no() {
    let cells = answer(
        vec![Rule::new(&["get", "list", "watch"], &[""], &["pods"])],
        true,
    )
    .cells();
    assert_eq!(cells.cell(Column::Read), &Cell::Yes, "a grant is a grant");
    assert!(matches!(cells.cell(Column::Shell), Cell::Ask(_)));
    assert!(cells.incomplete);
    assert!(cells.denied().is_empty(), "an ask is not requested as a no");
}

#[test]
fn the_short_form_names_verb_resource_and_where() {
    let need = Column::ScaleRestart.needs()[0];
    assert_eq!(
        need.phrase(Some("shop")),
        "needs patch on deployments in shop"
    );
    let nodes = Column::Edit.node_needs().unwrap()[0];
    assert_eq!(nodes.phrase(None), "needs patch on nodes cluster-wide");
}

#[test]
fn an_access_request_names_the_missing_grants_and_no_secrets() {
    let cells = answer(vec![], false).cells();
    let text = request_text("dev-fra", Some(&alice()), Some("shop"), &cells.denied());
    assert!(text.contains("Cluster context: dev-fra"));
    assert!(text.contains("Who: alice"));
    assert!(text.contains("Namespace: shop"));
    assert!(text.contains("- patch on deployments.apps/scale"));
    assert!(text.contains("- create on pods/exec"));
}

// The board.

#[test]
fn the_board_hands_out_only_the_rows_it_is_asked_about() {
    let mut board = Board::default();
    board.reset("a");
    let tickets = board.wants(&[ns("shop"), ns("default")]);
    assert_eq!(tickets.len(), 2);
    // Asking again while they are in flight hands out nothing.
    assert!(board.wants(&[ns("shop"), ns("default")]).is_empty());
    assert!(matches!(
        board.view(&ns("shop")),
        RowView::Checking { earlier: None }
    ));
    assert!(matches!(
        board.view(&ns("never-shown")),
        RowView::Checking { .. }
    ));
    assert_eq!(board.in_flight(), 2);
}

#[test]
fn the_board_keeps_to_its_parallel_limit() {
    let mut board = Board::default();
    board.reset("a");
    let rows: Vec<RowKey> = (0..10).map(|n| ns(&format!("ns-{n}"))).collect();
    let tickets = board.wants(&rows);
    assert_eq!(tickets.len(), PARALLEL);
    board.land(&tickets[0], Ok(answer(vec![], false).cells()));
    assert_eq!(board.wants(&rows).len(), 1, "one slot freed");
}

#[test]
fn an_answer_from_before_an_identity_change_is_ignored() {
    let mut board = Board::default();
    board.reset("a");
    let ticket = board.wants(&[ns("shop")]).remove(0);
    assert!(board.reset("b"));
    assert!(!board.land(&ticket, Ok(answer(vec![], false).cells())));
    assert_eq!(board.answered(), 0);
    assert!(matches!(
        board.view(&ns("shop")),
        RowView::Checking { earlier: None }
    ));
}

#[test]
fn a_new_access_identity_drops_every_answer_at_once() {
    let mut board = Board::default();
    board.reset("a");
    board.set_who(alice());
    let ticket = board.wants(&[ns("shop")]).remove(0);
    board.land(&ticket, Ok(answer(vec![], false).cells()));
    assert_eq!(board.answered(), 1);
    assert!(!board.reset("a"), "the same identity changes nothing");
    assert_eq!(board.answered(), 1);
    assert!(board.reset("b"));
    assert_eq!(board.answered(), 0);
    assert!(board.who().is_none());
    assert_eq!(board.changed().unwrap().from.as_deref(), Some("alice"));
}

#[test]
fn another_person_on_the_same_access_drops_the_answers() {
    let mut board = Board::default();
    board.reset("a");
    assert!(!board.set_who(alice()), "the first answer is a baseline");
    let ticket = board.wants(&[ns("shop")]).remove(0);
    board.land(&ticket, Ok(answer(vec![], false).cells()));
    assert!(!board.set_who(alice()), "the same person changes nothing");
    assert_eq!(board.answered(), 1);
    let bob = Who {
        username: "bob".into(),
        groups: vec![],
    };
    assert!(board.set_who(bob));
    assert_eq!(board.answered(), 0);
    let changed = board.changed().unwrap();
    assert_eq!(
        (changed.from.as_deref(), changed.to.as_deref()),
        (Some("alice"), Some("bob"))
    );
    board.dismiss_changed();
    assert!(board.changed().is_none());
}

#[test]
fn a_failed_row_is_asked_again_only_by_a_recheck() {
    let mut board = Board::default();
    board.reset("a");
    let ticket = board.wants(&[ns("shop")]).remove(0);
    board.land(&ticket, Err("refused".into()));
    assert!(matches!(
        board.view(&ns("shop")),
        RowView::Failed("refused")
    ));
    assert!(board.wants(&[ns("shop")]).is_empty(), "no retry loop");
    board.recheck(&ns("shop"));
    let again = board.wants(&[ns("shop")]);
    assert_eq!(again.len(), 1);
}

#[test]
fn a_recheck_keeps_the_earlier_answer_on_show_until_the_new_one_lands() {
    let mut board = Board::default();
    board.reset("a");
    let ticket = board.wants(&[ns("shop")]).remove(0);
    let cells = answer(vec![], false).cells();
    board.land(&ticket, Ok(cells.clone()));
    board.recheck_all();
    let ticket = board.wants(&[ns("shop")]).remove(0);
    assert_eq!(
        board.view(&ns("shop")),
        RowView::Checking {
            earlier: Some(&cells)
        }
    );
    board.land(&ticket, Ok(cells.clone()));
    assert_eq!(board.view(&ns("shop")), RowView::Known(&cells));
}

#[test]
fn stopping_drops_what_is_in_flight_and_keeps_the_answers() {
    let mut board = Board::default();
    board.reset("a");
    let done = board.wants(&[ns("shop")]).remove(0);
    board.land(&done, Ok(answer(vec![], false).cells()));
    let pending = board.wants(&[ns("default")]).remove(0);
    board.stop();
    assert!(!board.land(&pending, Ok(answer(vec![], false).cells())));
    assert_eq!(board.answered(), 1);
    assert_eq!(
        board.wants(&[ns("default")]).len(),
        1,
        "asked again on show"
    );
}

// The reads, on the recording fake.

const RULES: &str = "/apis/authorization.k8s.io/v1/selfsubjectrulesreviews";
const ACCESS: &str = "/apis/authorization.k8s.io/v1/selfsubjectaccessreviews";
const WHO: &str = "/apis/authentication.k8s.io/v1/selfsubjectreviews";

fn status(code: u16, message: &str) -> (u16, String) {
    fake_api::json(
        code,
        json!({"kind":"Status","apiVersion":"v1","status":"Failure","code":code,
            "reason":"Forbidden","message":message}),
    )
}

fn rules_answer(incomplete: bool) -> (u16, String) {
    fake_api::json(
        201,
        json!({"apiVersion":"authorization.k8s.io/v1","kind":"SelfSubjectRulesReview",
            "status":{"resourceRules":[{"verbs":["get","list","watch"],"apiGroups":[""],"resources":["pods"]}],
                "nonResourceRules":[],"incomplete":incomplete,
                "evaluationError": if incomplete {"webhook authorizer did not answer"} else {""}}}),
    )
}

fn only_reviews(recorder: &Recorder) {
    recorder.assert_all("the table only creates reviews", |request| {
        request.method == http::Method::POST
            && (request.path == RULES || request.path == ACCESS || request.path == WHO)
    });
}

#[tokio::test]
async fn one_rules_review_per_asked_namespace_and_nothing_else() {
    let (client, recorder) =
        fake_api::client(|request: &Recorded| (request.path == RULES).then(|| rules_answer(false)));
    for namespace in ["shop", "default"] {
        let cells = rules(&client, namespace).await.unwrap();
        assert_eq!(cells.cell(Column::Read), &Cell::Yes);
        assert!(matches!(cells.cell(Column::Shell), Cell::No(_)));
    }
    let requests = recorder.requests();
    assert_eq!(requests.len(), 2, "{:?}", recorder.lines());
    only_reviews(&recorder);
    let spaces: Vec<String> = requests
        .iter()
        .map(|request| {
            let body: serde_json::Value = serde_json::from_str(&request.body).unwrap();
            body["spec"]["namespace"].as_str().unwrap().to_owned()
        })
        .collect();
    assert_eq!(spaces, ["shop", "default"]);
}

#[tokio::test]
async fn an_incomplete_rules_review_reads_ask_with_the_servers_note() {
    let (client, _) =
        fake_api::client(|request: &Recorded| (request.path == RULES).then(|| rules_answer(true)));
    let cells = rules(&client, "shop").await.unwrap();
    assert!(matches!(cells.cell(Column::Edit), Cell::Ask(_)));
    assert!(cells.incomplete);
    assert_eq!(
        cells.note.as_deref(),
        Some("webhook authorizer did not answer")
    );
}

#[tokio::test]
async fn a_refused_review_is_an_error_never_a_no() {
    let (client, _) = fake_api::client(|request: &Recorded| {
        (request.path == RULES).then(|| {
            status(
                403,
                "forbidden: Bearer abcdefghijklmnopqrstuvwxyz0123456789",
            )
        })
    });
    let error = rules(&client, "shop").await.unwrap_err();
    assert!(!error.is_empty());
    assert!(
        !error.contains("abcdefghijklmnopqrstuvwxyz0123456789"),
        "{error}"
    );
}

#[tokio::test]
async fn who_reads_the_user_and_groups_from_one_review() {
    let (client, recorder) = fake_api::client(|request: &Recorded| {
        (request.path == WHO).then(|| {
            fake_api::json(
                201,
                json!({"apiVersion":"authentication.k8s.io/v1","kind":"SelfSubjectReview",
                    "status":{"userInfo":{"username":"alice","groups":["devs","system:authenticated"]}}}),
            )
        })
    });
    let who = who(&client).await.unwrap();
    assert_eq!(who.username, "alice");
    assert_eq!(who.groups.len(), 2);
    assert_eq!(recorder.requests().len(), 1);
    only_reviews(&recorder);
}

#[tokio::test]
async fn the_nodes_row_asks_exact_reviews_with_no_namespace() {
    let (client, recorder) = fake_api::client(|request: &Recorded| {
        (request.path == ACCESS).then(|| {
            let denied = request.body.contains("\"patch\"");
            fake_api::json(
                201,
                json!({"apiVersion":"authorization.k8s.io/v1","kind":"SelfSubjectAccessReview",
                    "status":{"allowed": !denied, "reason": ""}}),
            )
        })
    });
    let cells = nodes(&client).await.unwrap();
    assert_eq!(cells.cell(Column::Read), &Cell::Yes);
    let Cell::No(missing) = cells.cell(Column::Edit) else {
        panic!("{:?}", cells.cell(Column::Edit));
    };
    assert_eq!(missing[0].phrase(None), "needs patch on nodes cluster-wide");
    assert_eq!(cells.cell(Column::Shell), &Cell::NotApplicable);
    // Read asks get and list, Edit asks patch.
    assert_eq!(recorder.requests().len(), 3);
    only_reviews(&recorder);
    for request in recorder.requests() {
        assert!(!request.body.contains("\"namespace\""), "{}", request.body);
    }
}

#[tokio::test]
async fn namespaces_are_one_metadata_list_with_a_cap() {
    let (client, recorder) = fake_api::client(|request: &Recorded| {
        (request.path == "/api/v1/namespaces").then(|| {
            fake_api::json(
                200,
                json!({"apiVersion":"v1","kind":"NamespaceList","metadata":{},
                    "items":[{"metadata":{"name":"shop"}},{"metadata":{"name":"default"}}]}),
            )
        })
    });
    let listed = list_namespaces(&client).await.unwrap();
    assert_eq!(listed.names, ["default", "shop"]);
    assert!(!listed.more && listed.refused.is_none());
    assert_eq!(recorder.requests().len(), 1);
    recorder.assert_all("a read", |request| request.method == http::Method::GET);
    assert!(recorder.requests()[0].query.contains("limit=501"));
}

#[test]
fn the_example_shapes_answer_through_the_same_matching() {
    use example::{Shape, answer};
    let shop = ns("shop");
    let limited = answer(Shape::Limited, 0, &shop).unwrap();
    assert_eq!(limited.cell(Column::ScaleRestart), &Cell::Yes);
    assert!(matches!(limited.cell(Column::Forward), Cell::No(_)));
    assert!(matches!(
        answer(Shape::Incomplete, 0, &shop)
            .unwrap()
            .cell(Column::Edit),
        Cell::Ask(_)
    ));
    assert!(answer(Shape::Failed, 0, &shop).is_err());
    assert!(answer(Shape::Failed, 0, &RowKey::Nodes).is_err());
    let full = answer(Shape::Full, 0, &shop).unwrap();
    assert!(full.cells.iter().all(|cell| *cell == Cell::Yes));
    assert_ne!(
        example::who(Shape::IdentityChanged, 0),
        example::who(Shape::IdentityChanged, 1)
    );
    assert_eq!(
        example::who(Shape::Limited, 0),
        example::who(Shape::Limited, 3)
    );
}

#[tokio::test]
async fn a_refused_namespace_list_reviews_the_contexts_own_namespace_alone() {
    let (client, recorder) = fake_api::client(|request: &Recorded| {
        (request.path == "/api/v1/namespaces").then(|| {
            status(
                403,
                "namespaces is forbidden: User \"alice\" cannot list resource",
            )
        })
    });
    let listed = list_namespaces(&client).await.unwrap();
    // The client's own default namespace, never a guess or a sweep.
    assert_eq!(listed.names, [client.default_namespace()]);
    assert!(listed.refused.as_deref().unwrap().contains("forbidden"));
    assert_eq!(recorder.requests().len(), 1);
}

#[tokio::test]
async fn a_namespace_list_that_fails_otherwise_is_an_error_not_a_refusal() {
    let (client, _) = fake_api::client(|request: &Recorded| {
        (request.path == "/api/v1/namespaces").then(|| status(500, "internal error"))
    });
    assert!(list_namespaces(&client).await.is_err());
}

#[test]
fn the_ns_refused_example_lists_one_namespace_and_why() {
    let listed = example::namespaces(example::Shape::NsRefused);
    assert_eq!(listed.names, ["shop"]);
    assert!(listed.refused.is_some());
    assert!(
        example::namespaces(example::Shape::Limited)
            .refused
            .is_none()
    );
}
