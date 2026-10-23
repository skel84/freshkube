use super::*;

#[test]
fn a_large_cluster_keeps_the_inspector_bounded_and_discloses_omitted_evidence() {
    let mut inventory = databases::example::inventory(at());
    inventory.databases.items[0].instances = (0..500)
        .map(|index| databases::Instance::named(format!("orders-{index}")))
        .collect();
    let display = display_of(&inventory, Mode::Clusters);
    let row = display
        .rows
        .iter()
        .find(|row| row.identity.name == "orders-db")
        .unwrap();
    let total: usize = row.sections.iter().map(|s| s.fields.len()).sum();
    assert!(total <= 129, "{total}");
    let last = row.sections.last().unwrap();
    assert_eq!(last.title, "More evidence");
    assert!(
        last.fields[0]
            .value
            .contains("additional fields are not shown")
    );
}

#[test]
fn inspector_reports_unknown_schedule_coverage_configuration_and_plain_generations() {
    let mut inv = databases::example::inventory(at());
    inv.schedules = databases::Read::empty(Coverage::Refused, at());
    inv.databases.items[0].backup_configuration = databases::Fact::Known(false);
    inv.databases.items[0].conditions[0].observed_generation = None;
    let display = display_of(&inv, Mode::Clusters);
    assert!(
        field(&display, "orders-db", "Schedules")
            .value
            .starts_with("Unknown")
    );
    assert!(
        field(&display, "orders-db", "Backup configuration")
            .value
            .starts_with("No backup")
    );
    let ready = &field(&display, "orders-db", "Ready").value;
    assert!(ready.contains("generation Not reported"), "{ready}");
    assert!(!ready.contains("Some("));
    assert!(
        field(&display, "orders-db", "Read")
            .value
            .contains(&crate::ui::clock(at().into()))
    );
}

#[test]
fn cluster_inspector_distinguishes_schedule_states_and_uses_prose() {
    for (shape, expected) in [
        ("", "Active · Daily at 02:00"),
        ("suspended", "Suspended · Daily at 02:00"),
    ] {
        let inv = databases::example::shaped(at(), shape);
        let display = display_of(&inv, Mode::Clusters);
        assert_eq!(field(&display, "orders-db", "orders-daily").value, expected);
        // The expression is the operator's, in its own dialect.
        let expression = field(&display, "orders-db", "Expression");
        assert_eq!(expression.value, "0 0 2 * * *");
        assert!(expression.monospace);
        for label in ["Freshness", "Restore prerequisites", "Backup configuration"] {
            assert!(
                !field(&display, "orders-db", label).monospace,
                "{label} is prose"
            );
        }
        assert!(field(&display, "orders-db", "Primary").monospace);
        assert!(field(&display, "orders-db", "Read").monospace);
    }
    // MariaDB's schedule has five fields.
    let inv = databases::example::inventory(at());
    let display = display_of(&inv, Mode::Clusters);
    assert_eq!(
        field(&display, "ledger-db", "ledger-nightly").value,
        "Active · Daily at 03:00"
    );
    assert_eq!(
        field(&display, "ledger-db", "Expression").value,
        "0 3 * * *"
    );
    let mut inv = databases::example::inventory(at());
    inv.schedules.items[0].suspended = databases::Fact::Unknown;
    inv.databases.items[0].current_primary = databases::Fact::Unknown;
    let display = display_of(&inv, Mode::Clusters);
    assert!(
        field(&display, "orders-db", "orders-daily")
            .value
            .starts_with("Suspension unknown")
    );
    assert!(!field(&display, "orders-db", "Primary").monospace);
}

#[test]
fn a_lone_backup_restore_window_and_unknown_readiness_read_plainly() {
    let inv = databases::example::inventory(at());
    let display = display_of(&inv, Mode::Clusters);
    let value = |name: &str, label: &str| field(&display, name, label).value.clone();
    // One completed backup: no span to speak of.
    assert!(
        value("orders-db", "Restore window").ends_with("to now (WAL archived)")
            || value("orders-db", "Restore window").contains("only")
    );
    assert_eq!(
        value("ledger-db", "Instances ready"),
        "Not reported by the operator (3 wanted)"
    );
    assert!(value("ledger-db", "Restore window").contains("Last backup"));
    let none = databases::example::shaped(at(), "no-primary");
    let display = display_of(&none, Mode::Clusters);
    assert_eq!(
        field(&display, "orders-db", "orders-db-1")
            .value
            .split(" · ")
            .next(),
        Some("Role unknown")
    );
}
