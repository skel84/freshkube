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

fn app(policies: &[PolicyFacts], namespace: &str, labels: &BTreeMap<String, String>) -> Applying {
    applying(
        policies,
        &PodsOf {
            namespace,
            labels,
            template: false,
            host_network: false,
        },
    )
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
        host_network: false,
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
        host_network: false,
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
        Some(policies),
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
        Some(policies),
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
    let a = app(&policies, "shop", &labels(&[("app", "api")]));
    assert!(a.policies.is_empty());
    assert_eq!(a.ingress, DirectionVerdict::NotGoverned);
    assert_eq!(a.egress, DirectionVerdict::NotGoverned);
    assert_eq!(
        app(&[], "shop", &labels(&[])).ingress,
        DirectionVerdict::NotGoverned
    );
}

#[test]
fn a_default_deny_policy_denies_what_it_selects_and_only_there() {
    let policies = vec![policy(
        "default-deny",
        json!({"podSelector": {}, "policyTypes": ["Ingress", "Egress"]}),
    )];
    let a = app(&policies, "shop", &labels(&[("app", "api")]));
    assert_eq!(a.ingress, DirectionVerdict::DeniesAll);
    assert_eq!(a.egress, DirectionVerdict::DeniesAll);
    assert_eq!(a.policies.len(), 1);
    // Another namespace's policy never applies.
    assert!(app(&policies, "payments", &labels(&[])).policies.is_empty());
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
        app(std::slice::from_ref(&deny), "shop", &l).ingress,
        DirectionVerdict::DeniesAll
    );
    assert_eq!(
        app(&[deny.clone(), allow.clone()], "shop", &l).ingress,
        DirectionVerdict::Restricts
    );
    assert_eq!(
        app(&[deny.clone(), allow.clone(), open], "shop", &l).ingress,
        DirectionVerdict::OpensToAll
    );
    // Egress isn't governed by any of them.
    assert_eq!(
        app(&[deny, allow], "shop", &l).egress,
        DirectionVerdict::NotGoverned
    );
}

#[test]
fn an_unjudgeable_selector_makes_the_verdict_unknown() {
    let bad = policy(
        "bad",
        json!({"podSelector": {"matchExpressions": [{"key": "app", "operator": "Weird"}]}}),
    );
    let a = app(&[bad], "shop", &labels(&[("app", "api")]));
    assert_eq!(a.ingress, DirectionVerdict::Unknown);
    assert!(a.policies[0].uncertain);
    // But a definite open rule is still open.
    let open = policy("open", json!({"podSelector": {}, "ingress": [{}]}));
    let bad = policy(
        "bad",
        json!({"podSelector": {"matchExpressions": [{"key": "app", "operator": "Weird"}]}}),
    );
    assert_eq!(
        app(&[bad, open], "shop", &labels(&[])).ingress,
        DirectionVerdict::OpensToAll
    );
}

// ---- decide

#[test]
fn no_policy_allows_with_its_reason() {
    let d = decide(
        Direction::Ingress,
        Some(&[]),
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
    let policies = [policy(
        "p",
        json!({"podSelector": {}, "ingress": [{"from": [{"namespaceSelector": {"matchLabels": {"team": "ops"}}}]}]}),
    )];
    let s = subject("shop", &[]);
    let remote = pod_remote("monitoring", &[], None);
    let d = decide(
        Direction::Ingress,
        Some(&policies[..]),
        &s,
        &remote,
        80,
        "TCP",
        None,
    );
    assert_eq!(d.answer, Answer::CantTell);
    // A namespace missing from the list is also unknown, not a no.
    let d = decide(
        Direction::Ingress,
        Some(&policies[..]),
        &s,
        &pod_remote("ghost", &[], None),
        80,
        "TCP",
        Some(&namespaces()),
    );
    assert_eq!(d.answer, Answer::CantTell);
    // A definite allow by another rule is still allowed.
    let both = [
        policies[0].clone(),
        policy("open", json!({"podSelector": {}, "ingress": [{}]})),
    ];
    assert_eq!(
        decide(
            Direction::Ingress,
            Some(&both[..]),
            &s,
            &remote,
            80,
            "TCP",
            None
        )
        .answer,
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
    let policies = [policy(
        "p",
        json!({"podSelector": {}, "ingress": [{"from": [{"ipBlock": {"cidr": "10.0.0.0/8"}}]}]}),
    )];
    let s = subject("shop", &[("app", "api")]);
    // A pod whose IP wasn't read.
    let d = decide(
        Direction::Ingress,
        Some(&policies[..]),
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
            Some(&policies[..]),
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
    let policies = [policy(
        "p",
        json!({"podSelector": {}, "ingress": [{"ports": [{"protocol": "UDP"}]}]}),
    )];
    let d = |protocol| {
        decide(
            Direction::Ingress,
            Some(&policies[..]),
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
            Some(&policies[..]),
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

fn pods_of(labels: &BTreeMap<String, String>) -> PodsOf<'_> {
    PodsOf {
        namespace: "shop",
        labels,
        template: false,
        host_network: false,
    }
}

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
async fn a_policys_pods_are_listed_without_finished_ones_and_only_gets() {
    let (client, recorder) = crate::fake_api::client(|request| match request.path.as_str() {
        "/api/v1/namespaces/shop/pods" => Some(ok(json!({
            "apiVersion": "v1", "kind": "PodList", "metadata": {},
            "items": [
                {"metadata": {"name": "a", "namespace": "shop"}},
                {"metadata": {"name": "b", "namespace": "shop"}},
                {"metadata": {"name": "net", "namespace": "shop"}, "spec": {"hostNetwork": true, "containers": []}},
                {"metadata": {"name": "done", "namespace": "shop"}, "status": {"phase": "Succeeded"}},
            ],
        }))),
        _ => Some(status(500)),
    });
    let policy = policy("default-deny", json!({"podSelector": {}}));
    let counted = read_selected_pods(&client, &policy).await.unwrap();
    // The finished pod is left out and the host-network one counted apart.
    assert_eq!(
        (counted.selected, counted.total, counted.host_network),
        (2, 2, 1)
    );
    let requests = recorder.requests();
    assert!(
        requests[0].query.contains("fieldSelector"),
        "{}",
        requests[0].query
    );
    assert!(requests[0].query.contains("status.phase"));
    only_gets(&recorder);
}

#[tokio::test]
async fn a_refused_pod_list_leaves_the_count_unknown_not_zero() {
    let (client, _) = crate::fake_api::client(|_| Some(status(403)));
    let policy = policy("p", json!({"podSelector": {}}));
    assert_eq!(
        read_selected_pods(&client, &policy).await.unwrap_err().kind,
        FailureKind::Forbidden
    );
}

#[test]
fn a_policy_is_read_from_the_yaml_on_screen() {
    let facts = PolicyFacts::from_yaml(
        "apiVersion: networking.k8s.io/v1\nkind: NetworkPolicy\nmetadata:\n  name: deny\n  namespace: shop\nspec:\n  podSelector: {}\n",
    )
    .unwrap();
    assert_eq!(
        facts.verdict(Direction::Ingress),
        DirectionVerdict::DeniesAll
    );
    assert!(PolicyFacts::from_yaml("spec: [").is_err());
}

#[tokio::test]
async fn a_refused_policy_list_is_an_error_never_no_policy() {
    let (client, recorder) = crate::fake_api::client(|_| Some(status(403)));
    let err = read_applying(&client, &pods_of(&BTreeMap::new()))
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
    let err = read_applying(&client, &pods_of(&BTreeMap::new()))
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
    let a = read_applying(&client, &pods_of(&labels(&[("app", "api")])))
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

// ---- host-network pods, controller labels, unread policy lists

fn host_subject() -> Subject {
    Subject {
        host_network: true,
        ..subject("shop", &[("app", "node-exporter")])
    }
}

#[test]
fn a_host_network_pod_is_unknown_never_denied() {
    let deny = vec![policy("deny", json!({"podSelector": {}}))];
    let l = labels(&[("app", "node-exporter")]);
    let a = applying(
        &deny,
        &PodsOf {
            namespace: "shop",
            labels: &l,
            template: false,
            host_network: true,
        },
    );
    assert_eq!(a.ingress, DirectionVerdict::Unknown);
    assert_eq!(a.egress, DirectionVerdict::Unknown);
    assert!(a.host_network);
    // The policies that select it are still listed.
    assert_eq!(a.policies.len(), 1);
    // The same policies deny a pod on the pod network.
    assert_eq!(app(&deny, "shop", &l).ingress, DirectionVerdict::DeniesAll);
}

#[test]
fn either_end_on_the_host_network_is_cant_tell() {
    let deny = [policy("deny", json!({"podSelector": {}}))];
    let open = [policy("open", json!({"podSelector": {}, "ingress": [{}]}))];
    let any = pod_remote("shop", &[], None);
    let d = decide(
        Direction::Ingress,
        Some(&deny[..]),
        &host_subject(),
        &any,
        80,
        "TCP",
        None,
    );
    assert_eq!(d.answer, Answer::CantTell);
    assert_eq!(d.reason, HOST_NETWORK_NOTE);
    // Even a rule that would allow it.
    let d = decide(
        Direction::Ingress,
        Some(&open[..]),
        &host_subject(),
        &any,
        80,
        "TCP",
        None,
    );
    assert_eq!(d.answer, Answer::CantTell);
    // The remote being the host-network pod.
    let host_remote = Remote::Pod {
        namespace: "shop".into(),
        labels: BTreeMap::new(),
        ip: None,
        named_ports: None,
        host_network: true,
    };
    let d = decide(
        Direction::Ingress,
        Some(&open[..]),
        &subject("shop", &[]),
        &host_remote,
        80,
        "TCP",
        None,
    );
    assert_eq!(d.answer, Answer::CantTell);
    // can_reach: a host-network end on either side.
    let plain = subject("shop", &[("app", "api")]);
    for (from, to) in [(&host_subject(), &plain), (&plain, &host_subject())] {
        let d = can_reach(Some(&[]), Some(&[]), from, None, to, None, 80, "TCP", None);
        assert_eq!(d.answer, Answer::CantTell);
    }
}

#[test]
fn a_policy_list_that_was_not_read_in_full_is_cant_tell() {
    let d = decide(
        Direction::Ingress,
        None,
        &subject("shop", &[]),
        &external("1.2.3.4"),
        80,
        "TCP",
        None,
    );
    assert_eq!(d.answer, Answer::CantTell);
}

fn template_applying(policies: &[PolicyFacts], pairs: &[(&str, &str)]) -> Applying {
    let l = labels(pairs);
    applying(
        policies,
        &PodsOf {
            namespace: "shop",
            labels: &l,
            template: true,
            host_network: false,
        },
    )
}

#[test]
fn a_template_cant_answer_for_labels_its_controller_adds() {
    let template = [("app", "api")];
    // A per-replica policy on a StatefulSet pod name or a hash.
    for key in CONTROLLER_LABELS {
        let by_label = vec![policy(
            "one",
            json!({"podSelector": {"matchLabels": {key: "x"}}}),
        )];
        let a = template_applying(&by_label, &template);
        assert_eq!(a.ingress, DirectionVerdict::Unknown, "{key}");
        assert!(a.policies[0].uncertain, "{key}");
        // DoesNotExist can't be a yes from labels the template lacks.
        let absent = vec![policy(
            "none",
            json!({"podSelector": {"matchExpressions": [{"key": key, "operator": "DoesNotExist"}]}}),
        )];
        assert_eq!(
            template_applying(&absent, &template).ingress,
            DirectionVerdict::Unknown,
            "{key}"
        );
    }
    // Ordinary labels are still judged, and a pod's own labels are too.
    let plain = vec![policy(
        "p",
        json!({"podSelector": {"matchLabels": {"app": "api"}}}),
    )];
    assert_eq!(
        template_applying(&plain, &template).ingress,
        DirectionVerdict::DeniesAll
    );
    let hash = vec![policy(
        "h",
        json!({"podSelector": {"matchLabels": {"pod-template-hash": "abc"}}}),
    )];
    assert_eq!(
        app(
            &hash,
            "shop",
            &labels(&[("app", "api"), ("pod-template-hash", "abc")])
        )
        .ingress,
        DirectionVerdict::DeniesAll
    );
    // A definite No on another term still wins.
    let both = vec![policy(
        "b",
        json!({"podSelector": {"matchLabels": {"app": "db", "pod-template-hash": "x"}}}),
    )];
    assert_eq!(
        template_applying(&both, &template).ingress,
        DirectionVerdict::NotGoverned
    );
}

#[test]
fn counting_leaves_out_finished_and_host_network_pods() {
    let pod = |name: &str, extra: Value| -> Pod {
        let mut pod = json!({"metadata": {"name": name, "namespace": "shop"}});
        merge(&mut pod, extra);
        serde_json::from_value(pod).unwrap()
    };
    let p = policy("p", json!({"podSelector": {}}));
    let pods = vec![
        pod("a", json!({})),
        pod("done", json!({"status": {"phase": "Succeeded"}})),
        pod("failed", json!({"status": {"phase": "Failed"}})),
        pod(
            "net",
            json!({"spec": {"hostNetwork": true, "containers": []}}),
        ),
    ];
    let c = count_selected(&p, &pods, false);
    assert_eq!((c.selected, c.total, c.host_network), (1, 1, 1));
}

fn merge(into: &mut Value, with: Value) {
    match (into, with) {
        (Value::Object(into), Value::Object(with)) => {
            for (key, value) in with {
                merge(into.entry(key).or_insert(Value::Null), value);
            }
        }
        (into, with) => *into = with,
    }
}
