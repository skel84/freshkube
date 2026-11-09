use super::*;
use crate::failure::FailureKind;
use k8s_openapi::api::core::v1::Pod;
use k8s_openapi::api::networking::v1::NetworkPolicy;
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::net::IpAddr;

fn policy(name: &str, spec: Value) -> PolicyFacts {
    policy_in("shop", name, spec)
}

fn policy_in(namespace: &str, name: &str, spec: Value) -> PolicyFacts {
    let policy: NetworkPolicy = serde_json::from_value(json!({
        "metadata": {"name": name, "namespace": namespace, "uid": format!("{name}-uid")},
        "spec": spec,
    }))
    .unwrap();
    PolicyFacts::from_policy(&policy)
}

fn labels(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
    pairs
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect()
}

fn subject(namespace: &str, pairs: &[(&str, &str)]) -> Subject {
    Subject {
        namespace: namespace.into(),
        labels: labels(pairs),
        named_ports: Some(vec![NamedPort {
            name: "http".into(),
            number: 8080,
            protocol: "TCP".into(),
        }]),
    }
}

fn pod_remote(namespace: &str, pairs: &[(&str, &str)], ip: Option<&str>) -> Remote {
    Remote::Pod {
        namespace: namespace.into(),
        labels: labels(pairs),
        ip: ip.map(|ip| ip.parse().unwrap()),
        named_ports: Some(vec![NamedPort {
            name: "http".into(),
            number: 8080,
            protocol: "TCP".into(),
        }]),
    }
}

fn external(ip: &str) -> Remote {
    Remote::External {
        ip: Some(ip.parse::<IpAddr>().unwrap()),
    }
}

fn namespaces() -> Vec<NamespaceLabels> {
    vec![
        NamespaceLabels {
            name: "shop".into(),
            labels: labels(&[("kubernetes.io/metadata.name", "shop"), ("env", "prod")]),
        },
        NamespaceLabels {
            name: "monitoring".into(),
            labels: labels(&[
                ("kubernetes.io/metadata.name", "monitoring"),
                ("team", "ops"),
            ]),
        },
        NamespaceLabels {
            name: "scratch".into(),
            labels: labels(&[("kubernetes.io/metadata.name", "scratch"), ("env", "dev")]),
        },
    ]
}

fn ingress(policies: &[PolicyFacts], remote: &Remote, port: i32) -> Answer {
    decide(
        Direction::Ingress,
        policies,
        &subject("shop", &[("app", "api")]),
        remote,
        port,
        "TCP",
        Some(&namespaces()),
    )
    .answer
}

fn egress(policies: &[PolicyFacts], remote: &Remote, port: i32) -> Answer {
    decide(
        Direction::Egress,
        policies,
        &subject("shop", &[("app", "api")]),
        remote,
        port,
        "TCP",
        Some(&namespaces()),
    )
    .answer
}

// ---- selectors

fn selector(value: Value) -> k8s_openapi::apimachinery::pkg::apis::meta::v1::LabelSelector {
    serde_json::from_value(value).unwrap()
}

#[test]
fn an_empty_selector_selects_everything_and_labels_are_anded() {
    let l = labels(&[("app", "api"), ("tier", "backend")]);
    assert_eq!(selector_matches(&selector(json!({})), &l), Match::Yes);
    assert_eq!(
        selector_matches(&selector(json!({})), &BTreeMap::new()),
        Match::Yes
    );
    assert_eq!(
        selector_matches(
            &selector(json!({"matchLabels": {"app": "api", "tier": "backend"}})),
            &l
        ),
        Match::Yes
    );
    assert_eq!(
        selector_matches(
            &selector(json!({"matchLabels": {"app": "api", "tier": "web"}})),
            &l
        ),
        Match::No
    );
}

#[test]
fn expressions_cover_every_operator() {
    let l = labels(&[("env", "prod")]);
    let m = |expr: Value| selector_matches(&selector(json!({"matchExpressions": [expr]})), &l);
    assert_eq!(
        m(json!({"key": "env", "operator": "In", "values": ["prod", "stage"]})),
        Match::Yes
    );
    assert_eq!(
        m(json!({"key": "env", "operator": "In", "values": ["dev"]})),
        Match::No
    );
    assert_eq!(
        m(json!({"key": "gone", "operator": "In", "values": ["x"]})),
        Match::No
    );
    assert_eq!(
        m(json!({"key": "env", "operator": "NotIn", "values": ["dev"]})),
        Match::Yes
    );
    assert_eq!(
        m(json!({"key": "gone", "operator": "NotIn", "values": ["dev"]})),
        Match::Yes
    );
    assert_eq!(
        m(json!({"key": "env", "operator": "NotIn", "values": ["prod"]})),
        Match::No
    );
    assert_eq!(m(json!({"key": "env", "operator": "Exists"})), Match::Yes);
    assert_eq!(m(json!({"key": "gone", "operator": "Exists"})), Match::No);
    assert_eq!(
        m(json!({"key": "gone", "operator": "DoesNotExist"})),
        Match::Yes
    );
    assert_eq!(
        m(json!({"key": "env", "operator": "DoesNotExist"})),
        Match::No
    );
}

#[test]
fn an_invalid_expression_is_unknown_never_a_match() {
    let l = labels(&[("env", "prod")]);
    let m = |expr: Value| selector_matches(&selector(json!({"matchExpressions": [expr]})), &l);
    assert_eq!(m(json!({"key": "env", "operator": "In"})), Match::Unknown);
    assert_eq!(
        m(json!({"key": "env", "operator": "NotIn", "values": []})),
        Match::Unknown
    );
    assert_eq!(
        m(json!({"key": "env", "operator": "Gt", "values": ["1"]})),
        Match::Unknown
    );
    // A definite No elsewhere still wins.
    let both = selector(json!({
        "matchLabels": {"app": "x"},
        "matchExpressions": [{"key": "env", "operator": "Gt", "values": ["1"]}],
    }));
    assert_eq!(selector_matches(&both, &l), Match::No);
}

#[test]
fn selectors_read_in_words() {
    assert_eq!(selector_text(&selector(json!({}))), None);
    assert_eq!(
        selector_text(&selector(json!({
            "matchLabels": {"app": "api"},
            "matchExpressions": [
                {"key": "env", "operator": "In", "values": ["a", "b"]},
                {"key": "x", "operator": "DoesNotExist"},
            ],
        }))),
        Some("app=api, env in (a, b), !x".into())
    );
}

// ---- facts and verdicts

#[test]
fn policy_types_default_like_the_api_server() {
    let none = policy("a", json!({"podSelector": {}}));
    assert!(none.governs_ingress && !none.governs_egress);
    let with_egress = policy("b", json!({"podSelector": {}, "egress": [{}]}));
    assert!(with_egress.governs_ingress && with_egress.governs_egress);
    let empty_egress = policy("c", json!({"podSelector": {}, "egress": []}));
    assert!(empty_egress.governs_ingress && !empty_egress.governs_egress);
    let only_egress = policy("d", json!({"podSelector": {}, "policyTypes": ["Egress"]}));
    assert!(!only_egress.governs_ingress && only_egress.governs_egress);
}

#[test]
fn each_direction_has_its_own_verdict() {
    let deny = policy(
        "deny",
        json!({"podSelector": {}, "policyTypes": ["Ingress", "Egress"]}),
    );
    assert_eq!(
        deny.verdict(Direction::Ingress),
        DirectionVerdict::DeniesAll
    );
    assert_eq!(deny.verdict(Direction::Egress), DirectionVerdict::DeniesAll);
    let open = policy("open", json!({"podSelector": {}, "ingress": [{}]}));
    assert_eq!(
        open.verdict(Direction::Ingress),
        DirectionVerdict::OpensToAll
    );
    assert_eq!(
        open.verdict(Direction::Egress),
        DirectionVerdict::NotGoverned
    );
    let restricts = policy(
        "r",
        json!({"podSelector": {}, "ingress": [{"ports": [{"port": 80}]}]}),
    );
    assert_eq!(
        restricts.verdict(Direction::Ingress),
        DirectionVerdict::Restricts
    );
    // Empty `from` and `ports` lists are the same as absent ones: everything.
    let empty_lists = policy(
        "e",
        json!({"podSelector": {}, "ingress": [{"from": [], "ports": []}]}),
    );
    assert_eq!(
        empty_lists.verdict(Direction::Ingress),
        DirectionVerdict::OpensToAll
    );
}

#[test]
fn peers_and_ports_read_in_words() {
    let p = policy(
        "words",
        json!({
            "podSelector": {"matchLabels": {"app": "api"}},
            "ingress": [{
                "from": [
                    {"namespaceSelector": {"matchLabels": {"team": "ops"}}, "podSelector": {"matchLabels": {"app": "prom"}}},
                    {"podSelector": {}},
                    {"namespaceSelector": {}},
                    {"ipBlock": {"cidr": "10.0.0.0/8", "except": ["10.1.0.0/16"]}},
                ],
                "ports": [{"port": 80}, {"port": 8000, "endPort": 8100}, {"port": "http"}, {"protocol": "UDP"}],
            }],
        }),
    );
    assert_eq!(p.selector_words(), "app=api");
    assert_eq!(
        p.ingress[0].describe("from"),
        "from pods app=prom in namespaces team=ops; or any pod in this namespace; or any pod in any namespace; or IP range 10.0.0.0/8 except 10.1.0.0/16 on TCP 80, TCP 8000-8100, TCP port http, any UDP port"
    );
    assert_eq!(
        policy("all", json!({"podSelector": {}, "ingress": [{}]})).ingress[0].describe("from"),
        "from anything on any port"
    );
    assert_eq!(
        policy("all", json!({"podSelector": {}})).selector_words(),
        "every pod in the namespace"
    );
}

// ---- which policies apply

#[test]
fn nothing_selecting_a_pod_is_not_governed() {
    let policies = vec![policy(
        "other",
        json!({"podSelector": {"matchLabels": {"app": "db"}}}),
    )];
    let a = applying(&policies, "shop", &labels(&[("app", "api")]));
    assert!(a.policies.is_empty());
    assert_eq!(a.ingress, DirectionVerdict::NotGoverned);
    assert_eq!(a.egress, DirectionVerdict::NotGoverned);
    assert_eq!(
        applying(&[], "shop", &labels(&[])).ingress,
        DirectionVerdict::NotGoverned
    );
}

#[test]
fn a_default_deny_policy_denies_what_it_selects_and_only_there() {
    let policies = vec![policy(
        "default-deny",
        json!({"podSelector": {}, "policyTypes": ["Ingress", "Egress"]}),
    )];
    let a = applying(&policies, "shop", &labels(&[("app", "api")]));
    assert_eq!(a.ingress, DirectionVerdict::DeniesAll);
    assert_eq!(a.egress, DirectionVerdict::DeniesAll);
    assert_eq!(a.policies.len(), 1);
    // Another namespace's policy never applies.
    assert!(
        applying(&policies, "payments", &labels(&[]))
            .policies
            .is_empty()
    );
}

#[test]
fn policies_add_up() {
    let deny = policy("deny", json!({"podSelector": {}}));
    let allow = policy(
        "allow",
        json!({"podSelector": {"matchLabels": {"app": "api"}}, "ingress": [{"ports": [{"port": 80}]}]}),
    );
    let open = policy(
        "open",
        json!({"podSelector": {"matchLabels": {"app": "api"}}, "ingress": [{}]}),
    );
    let l = labels(&[("app", "api")]);
    assert_eq!(
        applying(std::slice::from_ref(&deny), "shop", &l).ingress,
        DirectionVerdict::DeniesAll
    );
    assert_eq!(
        applying(&[deny.clone(), allow.clone()], "shop", &l).ingress,
        DirectionVerdict::Restricts
    );
    assert_eq!(
        applying(&[deny.clone(), allow.clone(), open], "shop", &l).ingress,
        DirectionVerdict::OpensToAll
    );
    // Egress isn't governed by any of them.
    assert_eq!(
        applying(&[deny, allow], "shop", &l).egress,
        DirectionVerdict::NotGoverned
    );
}

#[test]
fn an_unjudgeable_selector_makes_the_verdict_unknown() {
    let bad = policy(
        "bad",
        json!({"podSelector": {"matchExpressions": [{"key": "app", "operator": "Weird"}]}}),
    );
    let a = applying(&[bad], "shop", &labels(&[("app", "api")]));
    assert_eq!(a.ingress, DirectionVerdict::Unknown);
    assert!(a.policies[0].uncertain);
    // But a definite open rule is still open.
    let open = policy("open", json!({"podSelector": {}, "ingress": [{}]}));
    let bad = policy(
        "bad",
        json!({"podSelector": {"matchExpressions": [{"key": "app", "operator": "Weird"}]}}),
    );
    assert_eq!(
        applying(&[bad, open], "shop", &labels(&[])).ingress,
        DirectionVerdict::OpensToAll
    );
}

// ---- decide

#[test]
fn no_policy_allows_with_its_reason() {
    let d = decide(
        Direction::Ingress,
        &[],
        &subject("shop", &[]),
        &external("203.0.113.9"),
        80,
        "TCP",
        None,
    );
    assert_eq!(d.answer, Answer::Allowed);
    assert!(d.reason.contains("no policy governs"));
}

#[test]
fn default_deny_denies_both_directions() {
    let policies = vec![policy(
        "deny",
        json!({"podSelector": {}, "policyTypes": ["Ingress", "Egress"]}),
    )];
    let peer = pod_remote("shop", &[("app", "web")], Some("10.0.0.5"));
    assert_eq!(ingress(&policies, &peer, 80), Answer::Denied);
    assert_eq!(egress(&policies, &peer, 80), Answer::Denied);
}

#[test]
fn an_ingress_only_policy_leaves_egress_open() {
    let policies = vec![policy(
        "deny",
        json!({"podSelector": {}, "policyTypes": ["Ingress"]}),
    )];
    assert_eq!(egress(&policies, &external("1.1.1.1"), 53), Answer::Allowed);
    assert_eq!(ingress(&policies, &external("1.1.1.1"), 53), Answer::Denied);
}

#[test]
fn an_empty_rule_allows_everything() {
    let policies = vec![policy("open", json!({"podSelector": {}, "ingress": [{}]}))];
    assert_eq!(
        ingress(&policies, &external("198.51.100.1"), 9999),
        Answer::Allowed
    );
}

#[test]
fn a_policy_that_selects_other_pods_does_not_govern() {
    let policies = vec![policy(
        "db",
        json!({"podSelector": {"matchLabels": {"app": "db"}}}),
    )];
    assert_eq!(
        ingress(&policies, &external("1.2.3.4"), 80),
        Answer::Allowed
    );
}

#[test]
fn another_namespaces_policy_is_ignored() {
    let policies = vec![policy_in("payments", "deny", json!({"podSelector": {}}))];
    assert_eq!(
        ingress(&policies, &external("1.2.3.4"), 80),
        Answer::Allowed
    );
}

#[test]
fn pod_selector_alone_means_the_policys_own_namespace() {
    let policies = vec![policy(
        "p",
        json!({"podSelector": {}, "ingress": [{"from": [{"podSelector": {"matchLabels": {"app": "web"}}}]}]}),
    )];
    assert_eq!(
        ingress(&policies, &pod_remote("shop", &[("app", "web")], None), 80),
        Answer::Allowed
    );
    assert_eq!(
        ingress(&policies, &pod_remote("shop", &[("app", "cron")], None), 80),
        Answer::Denied
    );
    // Same labels, another namespace: not this one.
    assert_eq!(
        ingress(
            &policies,
            &pod_remote("scratch", &[("app", "web")], None),
            80
        ),
        Answer::Denied
    );
}

#[test]
fn namespace_selector_alone_allows_every_pod_in_matching_namespaces() {
    let policies = vec![policy(
        "p",
        json!({"podSelector": {}, "ingress": [{"from": [{"namespaceSelector": {"matchLabels": {"team": "ops"}}}]}]}),
    )];
    assert_eq!(
        ingress(
            &policies,
            &pod_remote("monitoring", &[("x", "y")], None),
            80
        ),
        Answer::Allowed
    );
    assert_eq!(
        ingress(&policies, &pod_remote("scratch", &[], None), 80),
        Answer::Denied
    );
    // An empty namespace selector is every namespace.
    let all = vec![policy(
        "p",
        json!({"podSelector": {}, "ingress": [{"from": [{"namespaceSelector": {}}]}]}),
    )];
    assert_eq!(
        ingress(&all, &pod_remote("scratch", &[], None), 80),
        Answer::Allowed
    );
    // …but not an address outside the cluster.
    assert_eq!(ingress(&all, &external("203.0.113.1"), 80), Answer::Denied);
}

#[test]
fn namespace_and_pod_selector_in_one_entry_are_anded() {
    let anded = vec![policy(
        "p",
        json!({"podSelector": {}, "ingress": [{"from": [
            {"namespaceSelector": {"matchLabels": {"team": "ops"}}, "podSelector": {"matchLabels": {"app": "prom"}}}
        ]}]}),
    )];
    assert_eq!(
        ingress(
            &anded,
            &pod_remote("monitoring", &[("app", "prom")], None),
            80
        ),
        Answer::Allowed
    );
    assert_eq!(
        ingress(
            &anded,
            &pod_remote("monitoring", &[("app", "other")], None),
            80
        ),
        Answer::Denied
    );
    assert_eq!(
        ingress(&anded, &pod_remote("scratch", &[("app", "prom")], None), 80),
        Answer::Denied
    );
    // Two entries are ORed.
    let ored = vec![policy(
        "p",
        json!({"podSelector": {}, "ingress": [{"from": [
            {"namespaceSelector": {"matchLabels": {"team": "ops"}}},
            {"podSelector": {"matchLabels": {"app": "prom"}}},
        ]}]}),
    )];
    assert_eq!(
        ingress(
            &ored,
            &pod_remote("monitoring", &[("app", "other")], None),
            80
        ),
        Answer::Allowed
    );
    assert_eq!(
        ingress(&ored, &pod_remote("shop", &[("app", "prom")], None), 80),
        Answer::Allowed
    );
    assert_eq!(
        ingress(&ored, &pod_remote("scratch", &[("app", "prom")], None), 80),
        Answer::Denied
    );
}

#[test]
fn a_namespace_that_could_not_be_read_is_cant_tell() {
    let policies = vec![policy(
        "p",
        json!({"podSelector": {}, "ingress": [{"from": [{"namespaceSelector": {"matchLabels": {"team": "ops"}}}]}]}),
    )];
    let s = subject("shop", &[]);
    let remote = pod_remote("monitoring", &[], None);
    let d = decide(Direction::Ingress, &policies, &s, &remote, 80, "TCP", None);
    assert_eq!(d.answer, Answer::CantTell);
    // A namespace missing from the list is also unknown, not a no.
    let d = decide(
        Direction::Ingress,
        &policies,
        &s,
        &pod_remote("ghost", &[], None),
        80,
        "TCP",
        Some(&namespaces()),
    );
    assert_eq!(d.answer, Answer::CantTell);
    // A definite allow by another rule is still allowed.
    let both = vec![
        policies[0].clone(),
        policy("open", json!({"podSelector": {}, "ingress": [{}]})),
    ];
    assert_eq!(
        decide(Direction::Ingress, &both, &s, &remote, 80, "TCP", None).answer,
        Answer::Allowed
    );
}

#[test]
fn ip_blocks_honour_except_and_family() {
    let policies = vec![policy(
        "p",
        json!({"podSelector": {}, "ingress": [{"from": [
            {"ipBlock": {"cidr": "10.0.0.0/8", "except": ["10.1.0.0/16"]}},
            {"ipBlock": {"cidr": "2001:db8::/32"}},
        ]}]}),
    )];
    assert_eq!(
        ingress(&policies, &external("10.2.3.4"), 80),
        Answer::Allowed
    );
    assert_eq!(
        ingress(&policies, &external("10.1.3.4"), 80),
        Answer::Denied
    );
    assert_eq!(
        ingress(&policies, &external("192.168.0.1"), 80),
        Answer::Denied
    );
    assert_eq!(
        ingress(&policies, &external("2001:db8::7"), 80),
        Answer::Allowed
    );
    assert_eq!(
        ingress(&policies, &external("2001:dead::7"), 80),
        Answer::Denied
    );
    // /0 matches the whole family; /32 matches one address.
    let all = vec![policy(
        "p",
        json!({"podSelector": {}, "egress": [{"to": [{"ipBlock": {"cidr": "0.0.0.0/0"}}]}]}),
    )];
    assert_eq!(egress(&all, &external("8.8.8.8"), 53), Answer::Allowed);
    let one = vec![policy(
        "p",
        json!({"podSelector": {}, "egress": [{"to": [{"ipBlock": {"cidr": "8.8.8.8/32"}}]}]}),
    )];
    assert_eq!(egress(&one, &external("8.8.8.8"), 53), Answer::Allowed);
    assert_eq!(egress(&one, &external("8.8.4.4"), 53), Answer::Denied);
}

#[test]
fn an_ip_block_with_no_address_or_a_bad_cidr_is_cant_tell() {
    let policies = vec![policy(
        "p",
        json!({"podSelector": {}, "ingress": [{"from": [{"ipBlock": {"cidr": "10.0.0.0/8"}}]}]}),
    )];
    let s = subject("shop", &[("app", "api")]);
    // A pod whose IP wasn't read.
    let d = decide(
        Direction::Ingress,
        &policies,
        &s,
        &pod_remote("shop", &[], None),
        80,
        "TCP",
        None,
    );
    assert_eq!(d.answer, Answer::CantTell);
    let bad = vec![policy(
        "p",
        json!({"podSelector": {}, "ingress": [{"from": [{"ipBlock": {"cidr": "not-a-cidr"}}]}]}),
    )];
    assert_eq!(ingress(&bad, &external("10.0.0.1"), 80), Answer::CantTell);
    let bad_prefix = vec![policy(
        "p",
        json!({"podSelector": {}, "ingress": [{"from": [{"ipBlock": {"cidr": "10.0.0.0/40"}}]}]}),
    )];
    assert_eq!(
        ingress(&bad_prefix, &external("10.0.0.1"), 80),
        Answer::CantTell
    );
    let bad_except = vec![policy(
        "p",
        json!({"podSelector": {}, "ingress": [{"from": [{"ipBlock": {"cidr": "10.0.0.0/8", "except": ["junk"]}}]}]}),
    )];
    assert_eq!(
        ingress(&bad_except, &external("10.0.0.1"), 80),
        Answer::CantTell
    );
}

#[test]
fn ports_by_number_range_and_protocol() {
    let policies = vec![policy(
        "p",
        json!({"podSelector": {}, "ingress": [{"ports": [
            {"port": 80},
            {"port": 9000, "endPort": 9010},
            {"port": 53, "protocol": "UDP"},
        ]}]}),
    )];
    let any = external("1.2.3.4");
    assert_eq!(ingress(&policies, &any, 80), Answer::Allowed);
    assert_eq!(ingress(&policies, &any, 81), Answer::Denied);
    assert_eq!(ingress(&policies, &any, 9000), Answer::Allowed);
    assert_eq!(ingress(&policies, &any, 9010), Answer::Allowed);
    assert_eq!(ingress(&policies, &any, 9011), Answer::Denied);
    let udp = |port, protocol| {
        decide(
            Direction::Ingress,
            &policies,
            &subject("shop", &[]),
            &any,
            port,
            protocol,
            None,
        )
        .answer
    };
    assert_eq!(udp(53, "UDP"), Answer::Allowed);
    assert_eq!(udp(53, "TCP"), Answer::Denied);
    assert_eq!(udp(80, "SCTP"), Answer::Denied);
}

#[test]
fn a_port_with_no_number_allows_every_port_of_its_protocol() {
    let policies = vec![policy(
        "p",
        json!({"podSelector": {}, "ingress": [{"ports": [{"protocol": "UDP"}]}]}),
    )];
    let d = |protocol| {
        decide(
            Direction::Ingress,
            &policies,
            &subject("shop", &[]),
            &external("1.2.3.4"),
            5353,
            protocol,
            None,
        )
        .answer
    };
    assert_eq!(d("UDP"), Answer::Allowed);
    assert_eq!(d("TCP"), Answer::Denied);
}

#[test]
fn named_ports_resolve_on_the_destination_pod() {
    let policies = vec![policy(
        "p",
        json!({"podSelector": {}, "ingress": [{"ports": [{"port": "http"}]}]}),
    )];
    let any = external("1.2.3.4");
    // `http` is 8080 on the subject.
    assert_eq!(ingress(&policies, &any, 8080), Answer::Allowed);
    assert_eq!(ingress(&policies, &any, 80), Answer::Denied);
    // A name the pod doesn't declare matches nothing.
    let other = vec![policy(
        "p",
        json!({"podSelector": {}, "ingress": [{"ports": [{"port": "metrics"}]}]}),
    )];
    assert_eq!(ingress(&other, &any, 8080), Answer::Denied);
    // Container ports not read: can't tell.
    let mut unread = subject("shop", &[]);
    unread.named_ports = None;
    assert_eq!(
        decide(
            Direction::Ingress,
            &policies,
            &unread,
            &any,
            8080,
            "TCP",
            None
        )
        .answer,
        Answer::CantTell
    );
}

#[test]
fn egress_named_ports_resolve_on_the_remote_pod_and_never_on_an_address() {
    let policies = vec![policy(
        "p",
        json!({"podSelector": {}, "egress": [{"ports": [{"port": "http"}]}]}),
    )];
    assert_eq!(
        egress(&policies, &pod_remote("shop", &[], None), 8080),
        Answer::Allowed
    );
    assert_eq!(
        egress(&policies, &pod_remote("shop", &[], None), 80),
        Answer::Denied
    );
    assert_eq!(
        egress(&policies, &external("1.2.3.4"), 8080),
        Answer::CantTell
    );
}

#[test]
fn rules_in_one_direction_never_leak_into_the_other() {
    let policies = vec![policy(
        "p",
        json!({"podSelector": {}, "policyTypes": ["Ingress", "Egress"],
        "ingress": [{}]}),
    )];
    assert_eq!(
        ingress(&policies, &external("1.1.1.1"), 80),
        Answer::Allowed
    );
    assert_eq!(egress(&policies, &external("1.1.1.1"), 80), Answer::Denied);
}

#[test]
fn an_invalid_peer_or_selector_is_cant_tell_unless_something_else_allows() {
    let invalid = vec![policy(
        "p",
        json!({"podSelector": {}, "ingress": [{"from": [{}]}]}),
    )];
    // `from: [{}]` has an entry with nothing in it.
    assert_eq!(
        ingress(&invalid, &external("1.1.1.1"), 80),
        Answer::CantTell
    );
    let bad_selector = vec![policy(
        "p",
        json!({"podSelector": {"matchExpressions": [{"key": "a", "operator": "Odd"}]}}),
    )];
    assert_eq!(
        ingress(&bad_selector, &external("1.1.1.1"), 80),
        Answer::CantTell
    );
}

#[test]
fn can_reach_needs_both_ends() {
    let from = subject("shop", &[("app", "web")]);
    let to = subject("payments", &[("app", "ledger")]);
    let deny_in = vec![policy_in("payments", "deny", json!({"podSelector": {}}))];
    let allow_in = vec![policy_in(
        "payments",
        "allow",
        json!({"podSelector": {}, "ingress": [{"from": [{"namespaceSelector": {"matchLabels": {"env": "prod"}}}], "ports": [{"port": 8080}]}]}),
    )];
    let deny_out = vec![policy(
        "deny-out",
        json!({"podSelector": {}, "policyTypes": ["Egress"]}),
    )];
    let ns = namespaces();
    let go = |f: Option<&[PolicyFacts]>, t: Option<&[PolicyFacts]>, port| {
        can_reach(
            f,
            t,
            &from,
            Some("10.0.0.1".parse().unwrap()),
            &to,
            Some("10.0.0.2".parse().unwrap()),
            port,
            "TCP",
            Some(&ns),
        )
    };
    // Both open.
    assert_eq!(go(Some(&[]), Some(&[]), 8080).answer, Answer::Allowed);
    // Destination denies.
    assert_eq!(go(Some(&[]), Some(&deny_in), 8080).answer, Answer::Denied);
    // Destination allows prod namespaces on 8080 only.
    assert_eq!(go(Some(&[]), Some(&allow_in), 8080).answer, Answer::Allowed);
    assert_eq!(go(Some(&[]), Some(&allow_in), 9090).answer, Answer::Denied);
    // Source's egress denies even though the destination allows.
    assert_eq!(
        go(Some(&deny_out), Some(&allow_in), 8080).answer,
        Answer::Denied
    );
    // A refused read on either side is can't tell, not allowed.
    assert_eq!(go(None, Some(&allow_in), 8080).answer, Answer::CantTell);
    assert_eq!(go(Some(&[]), None, 8080).answer, Answer::CantTell);
    // …but a definite denial on the other end still stands.
    assert_eq!(go(None, Some(&deny_in), 8080).answer, Answer::Denied);
}

// ---- pod counts

#[test]
fn selected_pods_are_counted_by_the_policys_selector() {
    let pod = |name: &str, app: &str| -> Pod {
        serde_json::from_value(
            json!({"metadata": {"name": name, "namespace": "shop", "labels": {"app": app}}}),
        )
        .unwrap()
    };
    let p = policy("p", json!({"podSelector": {"matchLabels": {"app": "api"}}}));
    let pods = vec![pod("a", "api"), pod("b", "api"), pod("c", "db")];
    let c = count_selected(&p, &pods, false);
    assert_eq!((c.selected, c.unknown, c.total, c.cut), (2, 0, 3, false));
    let all = policy("all", json!({"podSelector": {}}));
    assert_eq!(count_selected(&all, &pods, true).selected, 3);
    assert!(count_selected(&all, &pods, true).cut);
    let weird = policy(
        "w",
        json!({"podSelector": {"matchExpressions": [{"key": "a", "operator": "Odd"}]}}),
    );
    assert_eq!(count_selected(&weird, &pods, false).unknown, 3);
}

// ---- reads against a fake API

fn status(code: u16) -> (u16, String) {
    (code, json!({"kind":"Status","apiVersion":"v1","status":"Failure","code":code,"reason":"x","message":"no"}).to_string())
}

fn ok(body: Value) -> (u16, String) {
    (200, body.to_string())
}

fn only_gets(recorder: &crate::fake_api::Recorder) {
    assert!(!recorder.requests().is_empty());
    for request in recorder.requests() {
        assert_eq!(
            request.method.as_str(),
            "GET",
            "{} {}",
            request.method,
            request.path
        );
    }
}

#[tokio::test]
async fn reading_a_policy_lists_pods_and_only_gets() {
    let (client, recorder) = crate::fake_api::client(|request| match request.path.as_str() {
        "/apis/networking.k8s.io/v1/namespaces/shop/networkpolicies/default-deny" => {
            Some(ok(json!({
                "apiVersion": "networking.k8s.io/v1", "kind": "NetworkPolicy",
                "metadata": {"name": "default-deny", "namespace": "shop"},
                "spec": {"podSelector": {}},
            })))
        }
        "/api/v1/namespaces/shop/pods" => Some(ok(json!({
            "apiVersion": "v1", "kind": "PodList", "metadata": {},
            "items": [
                {"metadata": {"name": "a", "namespace": "shop"}},
                {"metadata": {"name": "b", "namespace": "shop"}},
            ],
        }))),
        _ => Some(status(500)),
    });
    let reading = read_policy_by_name(&client, "shop", "default-deny")
        .await
        .unwrap();
    assert_eq!(
        reading.policy.verdict(Direction::Ingress),
        DirectionVerdict::DeniesAll
    );
    assert_eq!(reading.pods.unwrap().selected, 2);
    only_gets(&recorder);
}

#[tokio::test]
async fn a_refused_pod_list_leaves_the_count_unknown_not_zero() {
    let (client, _) = crate::fake_api::client(|request| match request.path.as_str() {
        "/apis/networking.k8s.io/v1/namespaces/shop/networkpolicies/p" => Some(ok(json!({
            "apiVersion": "networking.k8s.io/v1", "kind": "NetworkPolicy",
            "metadata": {"name": "p", "namespace": "shop"}, "spec": {"podSelector": {}},
        }))),
        _ => Some(status(403)),
    });
    let reading = read_policy_by_name(&client, "shop", "p").await.unwrap();
    assert_eq!(reading.pods.unwrap_err().kind, FailureKind::Forbidden);
}

#[tokio::test]
async fn a_refused_policy_list_is_an_error_never_no_policy() {
    let (client, recorder) = crate::fake_api::client(|_| Some(status(403)));
    let err = read_applying(&client, "shop", &BTreeMap::new())
        .await
        .unwrap_err();
    assert_eq!(err.kind, FailureKind::Forbidden);
    only_gets(&recorder);
}

#[tokio::test]
async fn a_cut_policy_list_is_an_error() {
    let (client, _) = crate::fake_api::client(|_| {
        Some(ok(json!({
            "apiVersion": "networking.k8s.io/v1", "kind": "NetworkPolicyList",
            "metadata": {"continue": "more"}, "items": [],
        })))
    });
    let err = read_applying(&client, "shop", &BTreeMap::new())
        .await
        .unwrap_err();
    assert!(err.message.contains("cut"));
}

#[tokio::test]
async fn applying_reads_one_namespace_and_reports_the_verdicts() {
    let (client, recorder) = crate::fake_api::client(|request| match request.path.as_str() {
        "/apis/networking.k8s.io/v1/namespaces/shop/networkpolicies" => Some(ok(json!({
            "apiVersion": "networking.k8s.io/v1", "kind": "NetworkPolicyList", "metadata": {},
            "items": [{"metadata": {"name": "deny", "namespace": "shop"}, "spec": {"podSelector": {}}}],
        }))),
        _ => Some(status(500)),
    });
    let a = read_applying(&client, "shop", &labels(&[("app", "api")]))
        .await
        .unwrap();
    assert_eq!(a.ingress, DirectionVerdict::DeniesAll);
    assert_eq!(a.egress, DirectionVerdict::NotGoverned);
    assert_eq!(recorder.requests().len(), 1);
    only_gets(&recorder);
}

#[tokio::test]
async fn namespaces_are_read_for_selectors_and_a_cut_is_reported() {
    let (client, recorder) = crate::fake_api::client(|request| match request.path.as_str() {
        "/api/v1/namespaces" => Some(ok(json!({
            "apiVersion": "v1", "kind": "NamespaceList", "metadata": {"continue": "x"},
            "items": [{"metadata": {"name": "shop", "labels": {"env": "prod"}}}],
        }))),
        _ => Some(status(500)),
    });
    let read = read_namespaces(&client).await.unwrap();
    assert!(read.cut);
    assert_eq!(read.namespaces[0].labels["env"], "prod");
    only_gets(&recorder);
}
