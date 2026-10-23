use super::*;

#[test]
fn an_entrys_coroot_and_metrics_are_checked() {
    assert_eq!(
        rule(&with_cluster("    coroot: {cluster: ''}\n")),
        "the cluster “cluster-a” coroot.cluster is empty"
    );
    assert_eq!(
        rule(&with_cluster("    coroot: {cluster: c, project: ''}\n")),
        "the cluster “cluster-a” coroot.project is empty"
    );
    assert_eq!(
        rule(&with_cluster("    metrics: {value: ''}\n")),
        "the cluster “cluster-a” metrics.value is empty"
    );
    assert_eq!(
        rule(&with_cluster("    metrics: {source: {}}\n")),
        "the cluster “cluster-a” metrics.source must set exactly one of service or url"
    );
    assert_eq!(
        rule(&with_cluster(
            "    metrics: {source: {service: {namespace: m, name: p, port: 9090}, token: keys.json}}\n"
        )),
        "the cluster “cluster-a” metrics.source.token only goes with url"
    );
    // An entry's own source is checked under every mode.
    for mode in ["automatic", "central", "per-entry"] {
        let text = format!(
            "version: 1\nmetrics:\n  mode: {mode}\n  store: {{url: 'https://m.example.test'}}\nclusters:\n  - id: a\n    role: core\n    context: a\n    metrics: {{source: {{url: nonsense}}}}\n"
        );
        assert_eq!(
            rule(&text),
            "the cluster “a” metrics.source.url isn’t an http:// or https:// URL"
        );
    }
}

#[test]
fn the_coroot_connection_is_checked() {
    let text = |lines: &str| format!("version: 1\ncoroot:\n{lines}  sign_in: anonymous\n");
    assert_eq!(
        rule(&text("  server: coroot.example.test\n")),
        "coroot.server isn’t an http:// or https:// URL"
    );
    assert_eq!(
        rule(&text(
            "  server: https://coroot.example.test\n  project: ''\n"
        )),
        "coroot.project is empty"
    );
    for sign_in in ["api-key", "session", "anonymous"] {
        let text = format!(
            "version: 1\ncoroot:\n  server: https://coroot.example.test\n  sign_in: {sign_in}\n"
        );
        let setup = ok(&text);
        assert_eq!(ok(&to_text(&setup)), setup);
    }
}

/// Two clusters, each with `coroot` lines after its context.
fn with_links(connection: &str, a: &str, b: &str) -> String {
    format!(
        "version: 1\nclusters:\n  - id: cluster-a\n    role: core\n    context: cluster-a\n{a}  - id: cluster-c\n    role: core\n    context: cluster-c\n{b}{connection}"
    )
}

const SHOP: &str =
    "coroot:\n  server: https://coroot.example.test\n  sign_in: anonymous\n  project: shop-api\n";

#[test]
fn two_entries_may_not_link_one_coroot_cluster() {
    let link = "    coroot:\n      cluster: cluster-a\n";
    // Both take the connection's project.
    assert_eq!(
        rule(&with_links(SHOP, link, link)),
        "the Coroot cluster “cluster-a” of project “shop-api” is linked to “cluster-a” and “cluster-c”"
    );
    // One names that project itself: still the same project.
    let own = "    coroot:\n      project: shop-api\n      cluster: cluster-a\n";
    assert_eq!(
        refused(&with_links(SHOP, link, own)),
        Invalid::Rule(Fault::CorootTwice {
            first: "cluster-a".into(),
            second: "cluster-c".into(),
            project: "shop-api".into(),
            cluster: "cluster-a".into(),
        })
    );
}

#[test]
fn the_same_cluster_name_in_another_project_is_another_link() {
    let link = "    coroot:\n      cluster: cluster-a\n";
    let other = "    coroot:\n      project: shop-web\n      cluster: cluster-a\n";
    ok(&with_links(SHOP, link, other));
    // Two clusters of one project are two links.
    let second = "    coroot:\n      cluster: cluster-c\n";
    ok(&with_links(SHOP, link, second));
}

#[test]
fn a_linked_entry_needs_a_project_from_somewhere() {
    let link = "    coroot:\n      cluster: cluster-a\n";
    let own = "    coroot:\n      project: shop-api\n      cluster: cluster-c\n";
    // Neither the entry nor the connection names one.
    assert_eq!(
        rule(&with_links("", link, own)),
        "the cluster “cluster-a” names a Coroot cluster, and neither it nor coroot names a project"
    );
    // A connection without a project is no project either.
    let bare = "coroot:\n  server: https://coroot.example.test\n  sign_in: anonymous\n";
    assert_eq!(
        refused(&with_links(bare, link, own)),
        Invalid::Rule(Fault::CorootWithoutProject("cluster-a".into()))
    );
    // Each one's own project is enough, with no connection at all.
    let both = "    coroot:\n      project: shop-api\n      cluster: cluster-a\n";
    ok(&with_links("", both, own));
    // And so is the connection's, for an entry that names none.
    ok(&with_links(SHOP, link, own));
}

#[test]
fn an_unlinked_entry_needs_nothing_and_a_refusal_repeats_no_value() {
    ok(&with_links("", "", ""));
    let link = "    coroot:\n      cluster: cluster-a\n";
    assert!(!rule(&with_links(SHOP, link, link)).contains("coroot.example.test"));
}

#[test]
fn the_example_links_are_distinct_and_survive_a_save() {
    let setup = ok(EXAMPLE);
    assert_eq!(setup.validate(), Ok(()));
    assert_eq!(setup.check_saveable(), Ok(()));
    // Linking cluster-c to cluster-a's Coroot cluster of the same project
    // breaks the rule, and the edited setup is not saveable.
    let mut edited = setup.clone();
    edited.workspace.clusters[2].coroot = Some(EntryCoroot::new("cluster-a").into());
    assert!(edited.validate().is_err());
    assert!(edited.check_saveable().is_err());
}
