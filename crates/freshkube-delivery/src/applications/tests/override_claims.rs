use super::*;

#[test]
fn an_override_merge_claims_the_moved_application_and_stages() {
    let over = Override {
        merges: vec![
            Merge {
                from: vec![id(Rule::Kargo, "cart")],
                into: app_id("checkout-dev"),
            },
            Merge {
                from: vec![app_id("checkout-dev")],
                into: id(Rule::Kargo, "checkout"),
            },
        ],
        ..Override::default()
    };
    let found = derive(&override_claim_inputs(), &over);
    let checkout = found.find(&id(Rule::Kargo, "checkout")).unwrap();
    let links = claims::claims(&found, checkout);
    let moved: Vec<_> = checkout
        .members
        .iter()
        .filter(|m| m.at.namespace.as_deref() != Some("checkout"))
        .collect();
    assert_eq!(moved.len(), 4, "Application, workload, Stage and Warehouse");
    for member in moved {
        let claim = links.links.iter().find(|c| c.member == member.at).unwrap();
        assert_eq!(claim.confidence, crate::join::Confidence::Claimed);
        assert_eq!(member.basis, Basis::Override);
        assert!(claim.why.contains("your override"), "{}", claim.why);
        assert!(claim.why.contains("merge"), "{}", claim.why);
        assert_ne!(claim.why, "It is the Application itself");
    }
}

#[test]
fn an_override_split_caps_the_workload_left_without_its_application() {
    let inputs = override_claim_inputs();
    let before = derive(&inputs, &Override::default());
    let original = before.find(&app_id("checkout-dev")).unwrap();
    let via = original
        .members
        .iter()
        .find(|m| m.at.kind == MemberKind::ArgoApplication)
        .unwrap()
        .at
        .clone();
    let before_links = claims::claims(&before, original);
    let web = before_links
        .links
        .iter()
        .find(|c| c.member.name == "web")
        .unwrap();
    assert_eq!(web.confidence, crate::join::Confidence::Confirmed);
    let found = derive(
        &inputs,
        &Override {
            splits: vec![Split {
                member: via.clone(),
                name: "separate checkout".into(),
            }],
            ..Override::default()
        },
    );
    let left = found.find(&app_id("checkout-dev")).unwrap();
    assert_eq!(left.members.len(), 1);
    assert_eq!(left.members[0].via.as_ref(), Some(&via));
    let links = claims::claims(&found, left);
    assert_eq!(links.links[0].confidence, crate::join::Confidence::Claimed);
    assert!(
        links.links[0].why.contains("no longer"),
        "{}",
        links.links[0].why
    );
    let split = found.find(&id(Rule::Manual, "separate checkout")).unwrap();
    assert_eq!(split.members[0].basis, Basis::Override);
    assert_eq!(
        claims::claims(&found, split).links[0].confidence,
        crate::join::Confidence::Claimed
    );
}

#[test]
fn an_override_keeps_untouched_parts_basis_and_claims() {
    let inputs = override_claim_inputs();
    let before = derive(&inputs, &Override::default());
    let checkout_id = id(Rule::Kargo, "checkout");
    let original = before.find(&checkout_id).unwrap();
    for over in [
        Override {
            merges: vec![Merge {
                from: vec![app_id("checkout-dev")],
                into: checkout_id.clone(),
            }],
            ..Override::default()
        },
        Override {
            splits: vec![Split {
                member: before.find(&app_id("checkout-dev")).unwrap().members[0]
                    .at
                    .clone(),
                name: "separate checkout".into(),
            }],
            ..Override::default()
        },
    ] {
        let found = derive(&inputs, &over);
        let checkout = found.find(&checkout_id).unwrap();
        let before_links = claims::claims(&before, original);
        let after_links = claims::claims(&found, checkout);
        for member in &original.members {
            assert_eq!(
                checkout.members.iter().find(|m| m.at == member.at),
                Some(member)
            );
            assert_eq!(
                after_links.links.iter().find(|c| c.member == member.at),
                before_links.links.iter().find(|c| c.member == member.at)
            );
        }
    }
}
