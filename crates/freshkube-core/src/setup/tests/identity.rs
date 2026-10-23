use super::*;

#[test]
fn protection_is_one_of_four_words_and_unset_stays_unset() {
    for level in Protection::ALL {
        let setup = ok(&with_cluster(&format!("    protection: {}\n", level.key())));
        let entry = &setup.workspace.clusters[0];
        assert_eq!(entry.protection, Some(level));
        assert_eq!(entry.level(), level);
        let text = to_text(&setup);
        assert!(
            text.contains(&format!("protection: {}\n", level.key())),
            "{text}"
        );
        assert_eq!(ok(&text), setup);
    }
    let setup = ok(&with_cluster(""));
    assert_eq!(setup.workspace.clusters[0].protection, None);
    assert_eq!(setup.workspace.clusters[0].level(), Protection::Guarded);
    assert!(!to_text(&setup).contains("protection"));
    // A literal null is unset, and is written as no key.
    let setup = ok(&with_cluster("    protection: null\n"));
    assert_eq!(setup.workspace.clusters[0].protection, None);
    assert!(!to_text(&setup).contains("protection"));
}

#[test]
fn an_unknown_protection_refuses_a_strict_read_without_repeating_it() {
    for value in ["Protected", "prod", "''", "1", "[guarded]"] {
        let invalid = refused(&with_cluster(&format!("    protection: {value}\n")));
        assert_eq!(invalid, Invalid::WrongShape(at(6, 17)), "{value}");
        assert!(!invalid.to_string().contains("prod"));
    }
}

#[test]
fn a_context_name_never_sets_a_level() {
    let setup = ok(
        "version: 1\nclusters:\n  - id: production\n    role: environment\n    context: prod-fra\n",
    );
    assert_eq!(
        setup.workspace.level_of("prod-fra", &Default::default()),
        Protection::Guarded
    );
}

#[test]
fn a_fingerprint_override_is_kept_and_unset_stays_unset() {
    let setup = ok(&with_cluster("    fingerprint: prod-eu\n"));
    assert_eq!(
        setup.workspace.clusters[0].fingerprint.as_deref(),
        Some("prod-eu")
    );
    let text = to_text(&setup);
    assert!(text.contains("fingerprint: prod-eu\n"), "{text}");
    assert_eq!(ok(&text), setup);
    let setup = ok(&with_cluster(""));
    assert_eq!(setup.workspace.clusters[0].fingerprint, None);
    assert!(!to_text(&setup).contains("fingerprint"));
}

#[test]
fn a_bad_fingerprint_is_refused_without_repeating_it() {
    let long = "a".repeat(129);
    for value in [long.as_str(), "'a b'", "''"] {
        let invalid = refused(&with_cluster(&format!("    fingerprint: {value}\n")));
        let message = invalid.to_string();
        assert!(message.contains("isn’t a fingerprint"), "{message}");
        assert!(
            !message.contains("a b") && !message.contains(&long),
            "{message}"
        );
    }
}
