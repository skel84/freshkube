use super::*;

const UID: &str = "0f8fad5b-d9cb-469f-a165-70867728950e";
const STRONG: &str = "abcdefghijklmnopqrstuvwxyz";
const CA: &str = "234567abcdefghijklmnopqrst";

fn resources_view() -> ViewState {
    ViewState {
        kind: Some(KindKey::parse("deployments.apps").unwrap()),
        ns: Some(Namespace::parse("payments").unwrap()),
        object: Some(ViewObject {
            name: Name::parse("checkout").unwrap(),
            uid: Some(ObjectUid::parse(UID).unwrap()),
        }),
        tab: ViewTab::Logs,
        ..ViewState::page(LinkPage::Resources)
    }
}

fn url(query: &str) -> String {
    format!("freshkube://open?{query}")
}

#[test]
fn a_link_round_trips_in_its_canonical_form() {
    let link = Link {
        cluster: ClusterRef {
            strong: Some(FingerprintToken::parse(STRONG).unwrap()),
            ca: Some(FingerprintToken::parse(CA).unwrap()),
        },
        view: resources_view(),
    };
    let text = link.to_url();
    assert_eq!(
        text,
        format!(
            "freshkube://open?cluster=u-{STRONG}&ca={CA}&page=resources&kind=deployments.apps\
             &ns=payments&name=checkout&uid={UID}&tab=logs"
        )
    );
    assert_eq!(Link::parse(&text), Ok(link));
}

#[test]
fn every_page_round_trips_and_the_default_tab_is_left_out() {
    for page in LinkPage::ALL {
        let link = Link::here(ViewState::page(page));
        assert_eq!(
            link.to_url(),
            format!("freshkube://open?page={}", page.slug())
        );
        assert_eq!(Link::parse(&link.to_url()), Ok(link));
        assert_eq!(LinkPage::from_slug(page.slug()), Some(page));
    }
    let details = Link::parse(&url("page=resources&kind=pods&name=api&tab=details")).unwrap();
    assert_eq!(details.view.tab, ViewTab::Details);
    assert!(!details.to_url().contains("tab="));
}

#[test]
fn a_node_link_carries_a_node_and_its_tab() {
    let view = ViewState {
        node: Some(Name::parse("cp-1").unwrap()),
        node_tab: NodeTab::Diagnostics,
        ..ViewState::page(LinkPage::Nodes)
    };
    let text = Link::here(view.clone()).to_url();
    assert_eq!(
        text,
        "freshkube://open?page=nodes&node=cp-1&ntab=diagnostics"
    );
    assert_eq!(Link::parse(&text).unwrap().view, view);
    for tab in [
        NodeTab::Overview,
        NodeTab::Pods,
        NodeTab::Services,
        NodeTab::Processes,
        NodeTab::Storage,
        NodeTab::Network,
        NodeTab::Diagnostics,
        NodeTab::Logs,
        NodeTab::Events,
        NodeTab::Yaml,
    ] {
        assert_eq!(NodeTab::from_slug(tab.slug()), Some(tab));
    }
}

#[test]
fn view_tabs_map_to_pane_tabs_both_ways() {
    for tab in [
        ViewTab::Details,
        ViewTab::Yaml,
        ViewTab::Events,
        ViewTab::Ports,
        ViewTab::Logs,
    ] {
        assert_eq!(ViewTab::from_tab(tab.tab()), tab);
    }
}

#[test]
fn hostile_keys_never_reach_the_view() {
    let link = Link::parse(&url(
        "page=overview&action=delete&route=operations&shell=bash",
    ))
    .unwrap();
    assert_eq!(link, Link::here(ViewState::page(LinkPage::Overview)));
    assert_eq!(link.to_url(), "freshkube://open?page=overview");
}

#[test]
fn pages_that_are_not_plain_places_are_refused() {
    for page in [
        "settings",
        "operations",
        "maintenance",
        "exec",
        "shell",
        "ports",
        "",
    ] {
        assert_eq!(
            Link::parse(&url(&format!("page={page}"))),
            Err(LinkError::Invalid("page")),
            "{page}"
        );
    }
    assert_eq!(
        Link::parse(&url("kind=pods")),
        Err(LinkError::Missing("page"))
    );
}

#[test]
fn the_scheme_and_host_are_exact() {
    for text in [
        "",
        "freshkube:open?page=overview",
        "freshkube://open",
        "freshkube://open#page=overview",
        "Freshkube://open?page=overview",
        "freshkube://opens?page=overview",
        "freshkube://open.evil?page=overview",
        "freshkube://user@open?page=overview",
        "https://open?page=overview",
        "freshkube://open?page=overview#x",
        "xfreshkube://open?page=overview",
    ] {
        assert_eq!(Link::parse(text), Err(LinkError::NotALink), "{text:?}");
    }
    assert!(Link::parse("freshkube://open/?page=overview").is_ok());
    assert!(Link::parse("  freshkube://open?page=overview \n").is_ok());
}

#[test]
fn a_repeated_known_key_refuses_the_link() {
    assert_eq!(
        Link::parse(&url("page=overview&page=nodes")),
        Err(LinkError::Duplicate("page"))
    );
    assert_eq!(
        Link::parse(&url("page=resources&kind=pods&name=a&name=b")),
        Err(LinkError::Duplicate("name"))
    );
    // A repeated unknown key is only dropped.
    assert!(Link::parse(&url("page=overview&x=1&x=2")).is_ok());
}

#[test]
fn the_url_and_the_parameters_are_bounded() {
    let long = format!(
        "page=resources&kind=pods&name={}",
        "a".repeat(LINK_MAX_BYTES)
    );
    assert_eq!(Link::parse(&url(&long)), Err(LinkError::TooLong));

    let pairs = |n: usize| {
        let mut parts = vec!["page=overview".to_owned()];
        parts.extend((1..n).map(|i| format!("x{i}=1")));
        parts.join("&")
    };
    assert!(Link::parse(&url(&pairs(LINK_MAX_PARAMETERS))).is_ok());
    assert_eq!(
        Link::parse(&url(&pairs(LINK_MAX_PARAMETERS + 1))),
        Err(LinkError::TooManyParameters)
    );
}

#[test]
fn a_url_at_the_length_bound_is_read_and_one_over_is_not() {
    let prefix = "freshkube://open?page=overview&pad=";
    let at = format!("{prefix}{}", "x".repeat(LINK_MAX_BYTES - prefix.len()));
    assert_eq!(at.len(), LINK_MAX_BYTES);
    assert!(Link::parse(&at).is_ok());
    assert_eq!(Link::parse(&format!("{at}x")), Err(LinkError::TooLong));
}

#[test]
fn every_field_is_bounded_at_over_and_under() {
    let resources = |extra: &str| url(&format!("page=resources&kind=pods{extra}"));
    // Namespace: a DNS label of at most 63.
    assert!(Link::parse(&resources(&format!("&ns={}", "a".repeat(63)))).is_ok());
    assert_eq!(
        Link::parse(&resources(&format!("&ns={}", "a".repeat(64)))),
        Err(LinkError::Invalid("ns"))
    );
    for bad in ["", "-a", "a-", "A", "a.b", "a_b", "a b"] {
        assert_eq!(
            Link::parse(&resources(&format!("&ns={bad}"))),
            Err(LinkError::Invalid("ns")),
            "{bad:?}"
        );
    }
    // Name: a DNS subdomain of at most 253.
    assert!(Link::parse(&resources(&format!("&name={}", "a".repeat(253)))).is_ok());
    assert_eq!(
        Link::parse(&resources(&format!("&name={}", "a".repeat(254)))),
        Err(LinkError::Invalid("name"))
    );
    for bad in ["", ".a", "a.", "-a", "A", "a_b", "a:b", "a/b", "a..b/"] {
        assert_eq!(
            Link::parse(&resources(&format!("&name={bad}"))),
            Err(LinkError::Invalid("name")),
            "{bad:?}"
        );
    }
    assert!(Link::parse(&resources("&name=a.b-c")).is_ok());
    // Kind: [a-z0-9.-], 1 to 253.
    assert!(Link::parse(&url(&format!("page=resources&kind={}", "a".repeat(253)))).is_ok());
    for bad in [
        String::new(),
        "a".repeat(254),
        "Pods".into(),
        "a_b".into(),
        "a/b".into(),
    ] {
        assert_eq!(
            Link::parse(&url(&format!("page=resources&kind={bad}"))),
            Err(LinkError::Invalid("kind")),
            "{bad:?}"
        );
    }
}

#[test]
fn a_uid_is_a_canonical_lower_case_uuid_or_absent() {
    let with = |uid: &str| url(&format!("page=resources&kind=pods&name=api&uid={uid}"));
    assert!(Link::parse(&with(UID)).is_ok());
    for bad in [
        "",
        "0F8FAD5B-D9CB-469F-A165-70867728950E",
        "0f8fad5bd9cb469fa16570867728950e",
        "0f8fad5b-d9cb-469f-a165-70867728950",
        "0f8fad5b-d9cb-469f-a165-70867728950ee",
        "0f8fad5b-d9cb-469f-a165-7086772895gg",
    ] {
        assert_eq!(
            Link::parse(&with(bad)),
            Err(LinkError::Invalid("uid")),
            "{bad:?}"
        );
    }
    assert_eq!(
        Link::parse(&url(&format!("page=resources&kind=pods&uid={UID}"))),
        Err(LinkError::Missing("name"))
    );
}

#[test]
fn fingerprints_are_26_characters_of_base32() {
    let with = |cluster: &str| url(&format!("cluster={cluster}&page=overview"));
    assert!(Link::parse(&with(&format!("u-{STRONG}"))).is_ok());
    for bad in [
        STRONG.to_owned(),
        format!("u-{}", &STRONG[..25]),
        format!("u-{STRONG}a"),
        format!("u-{}1", &STRONG[..25]),
        format!("u-{}A", &STRONG[..25]),
        format!("x-{STRONG}"),
    ] {
        assert_eq!(
            Link::parse(&with(&bad)),
            Err(LinkError::Invalid("cluster")),
            "{bad:?}"
        );
    }
    assert_eq!(
        Link::parse(&url(&format!("ca={}&page=overview", &CA[..25]))),
        Err(LinkError::Invalid("ca"))
    );
    // A link may name only the CA.
    let ca_only = Link::parse(&url(&format!("ca={CA}&page=overview"))).unwrap();
    assert_eq!(ca_only.cluster.strong, None);
    assert!(ca_only.cluster.ca.is_some());
}

#[test]
fn a_page_must_be_able_to_carry_what_the_link_names() {
    for (query, key) in [
        ("page=overview&kind=pods", "kind"),
        ("page=nodes&ns=payments", "ns"),
        ("page=health&name=api", "name"),
        ("page=overview&tab=yaml", "tab"),
        ("page=resources&node=cp-1", "node"),
        ("page=resources&ntab=pods", "ntab"),
    ] {
        assert_eq!(
            Link::parse(&url(query)),
            Err(LinkError::NotCarried(key)),
            "{query}"
        );
    }
    assert_eq!(
        Link::parse(&url("page=resources&ns=payments")),
        Err(LinkError::Missing("kind"))
    );
    assert_eq!(
        Link::parse(&url("page=resources&kind=pods&tab=yaml")),
        Err(LinkError::Missing("name"))
    );
    assert_eq!(
        Link::parse(&url("page=nodes&ntab=pods")),
        Err(LinkError::Missing("node"))
    );
    assert_eq!(
        Link::parse(&url("page=resources&kind=pods&tab=shell")),
        Err(LinkError::Invalid("tab"))
    );
}

#[test]
fn percent_encoding_takes_utf8_only_and_no_control_characters() {
    assert!(Link::parse(&url("page=%6Fverview")).is_ok());
    assert!(Link::parse(&url("pa%67e=overview")).is_ok());
    for bad in [
        "page=overview%",
        "page=overview%4",
        "page=overview%zz",
        "page=over%FFview",
        "page=%C3%28",
        "page=overview&x=%00",
        "page=overview&x=%0A",
        "page=overview&x=%1B%5B",
        "page=overview&x=%7F",
    ] {
        assert_eq!(Link::parse(&url(bad)), Err(LinkError::BadEncoding), "{bad}");
    }
    // A control character in a raw link is refused too.
    assert_eq!(
        Link::parse("freshkube://open?page=overview&x=a\u{7}b"),
        Err(LinkError::BadEncoding)
    );
    // An encoded slash does not smuggle into a name.
    assert_eq!(
        Link::parse(&url("page=resources&kind=pods&name=a%2Fb")),
        Err(LinkError::Invalid("name"))
    );
}

#[test]
fn a_newer_version_is_refused_as_such() {
    assert!(Link::parse(&url("v=1&page=overview")).is_ok());
    assert_eq!(
        Link::parse(&url("v=2&page=overview")),
        Err(LinkError::NewerVersion(2))
    );
    assert_eq!(
        Link::parse(&url("v=255&page=overview")),
        Err(LinkError::NewerVersion(255))
    );
    for bad in ["0", "", "x", "-1", "256"] {
        assert_eq!(
            Link::parse(&url(&format!("v={bad}&page=overview"))),
            Err(LinkError::Invalid("v")),
            "{bad:?}"
        );
    }
}

#[test]
fn parsing_is_total_over_a_table_of_junk() {
    let junk = [
        "freshkube://open?",
        "freshkube://open?&&&",
        "freshkube://open?=",
        "freshkube://open?page",
        "freshkube://open?page=",
        "freshkube://open?%=%",
        "freshkube://open??page=overview",
        "freshkube://open?page=overview&&page=nodes",
        "freshkube://open?page=over view",
        "freshkube://open?\u{0}",
        "freshkube://open?page=\u{1F980}",
        "\u{FEFF}freshkube://open?page=overview",
        "freshkube://",
        "freshkube",
        "://",
    ];
    for text in junk {
        assert!(Link::parse(text).is_err(), "{text:?}");
    }
}

#[test]
fn a_view_state_serializes_as_its_query_and_checks_on_the_way_back() {
    let view = resources_view();
    let json = serde_json::to_string(&view).unwrap();
    assert_eq!(json, format!("\"{}\"", view.to_query()));
    assert_eq!(serde_json::from_str::<ViewState>(&json).unwrap(), view);
    assert!(serde_json::from_str::<ViewState>("\"page=settings\"").is_err());
    assert!(serde_json::from_str::<ViewState>("\"page=overview&kind=pods\"").is_err());
    assert!(serde_json::from_str::<ViewState>("42").is_err());
}

#[test]
fn a_view_built_by_hand_is_checked_before_it_is_written() {
    let mut view = ViewState::page(LinkPage::Overview);
    assert_eq!(view.check(), Ok(()));
    view.kind = Some(KindKey::parse("pods").unwrap());
    assert_eq!(view.check(), Err(LinkError::NotCarried("kind")));
}
