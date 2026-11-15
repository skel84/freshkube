use super::*;

const KINDS: [OperationKind; 5] = [
    OperationKind::Cordon,
    OperationKind::Uncordon,
    OperationKind::Drain,
    OperationKind::Reboot,
    OperationKind::Shutdown,
];

const LEVELS: [Protection; 4] = [
    Protection::ReadOnly,
    Protection::Protected,
    Protection::Guarded,
    Protection::Open,
];

fn node(name: &str) -> NodeTarget {
    NodeTarget::new(name, "10.0.0.1")
}

#[test]
fn the_tiers_are_cordon_and_uncordon_reversible_and_the_rest_destructive() {
    assert_eq!(OperationKind::Cordon.tier(), Tier::Reversible);
    assert_eq!(OperationKind::Uncordon.tier(), Tier::Reversible);
    for kind in [
        OperationKind::Drain,
        OperationKind::Reboot,
        OperationKind::Shutdown,
    ] {
        assert_eq!(kind.tier(), Tier::Destructive);
    }
}

/// The gate is the write model's table, row by row, and never asks less than
/// the operation's own confirmation.
#[test]
fn the_gate_follows_the_tier_table_at_every_level() {
    let one = [node("worker-1")];
    for kind in KINDS {
        for level in LEVELS {
            let needs = requirement(kind.tier(), level);
            let gate = operation_gate(kind, level, &one, "cluster-a");
            match (needs.blocked, needs.type_name) {
                (Some(reason), _) => assert_eq!(gate, OperationGate::Blocked(reason)),
                (None, true) => assert_eq!(
                    gate,
                    OperationGate::TypeName {
                        expected: "worker-1".into()
                    },
                    "{kind:?} at {level:?}"
                ),
                (None, false) => assert_eq!(gate, OperationGate::Confirm, "{kind:?} at {level:?}"),
            }
        }
    }
    // Operations keep today's confirmation at `open`, even for the
    // reversible tier whose framework confirmation `open` skips.
    for kind in KINDS {
        assert_eq!(
            operation_gate(kind, Protection::Open, &one, "cluster-a"),
            OperationGate::Confirm
        );
        assert_eq!(
            operation_gate(kind, Protection::Guarded, &one, "cluster-a"),
            OperationGate::Confirm
        );
    }
}

#[test]
fn several_nodes_are_typed_once_by_the_entry_id() {
    let two = [node("worker-1"), node("worker-2")];
    assert_eq!(
        operation_gate(
            OperationKind::Drain,
            Protection::Protected,
            &two,
            "cluster-a"
        ),
        OperationGate::TypeName {
            expected: "cluster-a".into()
        }
    );
}

#[test]
fn safety_g25_protected_operations_refuse_a_missing_or_wrong_name_and_accept_the_exact_one() {
    let one = [node("worker-1")];
    let two = [node("worker-1"), node("worker-2")];
    for kind in KINDS {
        let at = |level, targets: &[NodeTarget], typed: &str| {
            confirm_operation(kind, level, targets, "cluster-a", typed)
        };
        // Protected: nothing, a wrong case, a neighbour, the entry id for one
        // node, and a name with a space all refuse.
        for typed in [
            "",
            "Worker-1",
            "worker-2",
            "cluster-a",
            "worker-1 ",
            " worker-1",
        ] {
            assert_eq!(
                at(Protection::Protected, &one, typed),
                Err(OperationRefusal::NameMismatch),
                "{kind:?} typed {typed:?}"
            );
        }
        assert!(at(Protection::Protected, &one, "worker-1").is_ok());
        // Several nodes: a node's name no longer does; the entry id does.
        for typed in ["", "worker-1", "Cluster-A"] {
            assert_eq!(
                at(Protection::Protected, &two, typed),
                Err(OperationRefusal::NameMismatch)
            );
        }
        assert!(at(Protection::Protected, &two, "cluster-a").is_ok());
        // Read-only refuses whatever was typed, even the exact name.
        for typed in ["", "worker-1"] {
            assert!(matches!(
                at(Protection::ReadOnly, &one, typed),
                Err(OperationRefusal::Blocked(_))
            ));
        }
        // Guarded and open need no name.
        assert!(at(Protection::Guarded, &one, "").is_ok());
        assert!(at(Protection::Open, &one, "").is_ok());
        // No node named, no confirmation at any level.
        for level in LEVELS {
            assert_eq!(at(level, &[], "x"), Err(OperationRefusal::NoTargets));
        }
    }
}

#[test]
fn an_empty_expected_name_is_never_matched_by_an_empty_entry() {
    let two = [node("a"), node("b")];
    assert_eq!(
        confirm_operation(OperationKind::Reboot, Protection::Protected, &two, "", ""),
        Err(OperationRefusal::NameMismatch)
    );
}
