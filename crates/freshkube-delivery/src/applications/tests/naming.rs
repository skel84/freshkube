use super::*;

#[test]
fn a_name_only_template_puts_an_application_in_the_project_of_its_stage() {
    let mut core = session("core-fra");
    core.kargo = Source::Read(vec![project("checkout", &["dev", "prod"])]);
    core.argo_applications = Source::Read(vec![argo("dev-checkout", json!({}))]);
    let found = derived_named(vec![core], "{stage}-{project}");
    let member = member_of(&found, "checkout", "dev-checkout").expect("a member of checkout");
    assert_eq!(member.basis, Basis::NamedByTemplate);
}

#[test]
fn a_name_only_template_needs_the_whole_name() {
    let mut core = session("core-fra");
    core.kargo = Source::Read(vec![project("checkout", &["dev"])]);
    core.argo_applications = Source::Read(vec![argo("dev-checkout-extra", json!({}))]);
    let found = derived_named(vec![core], "{stage}-{project}");
    assert!(member_of(&found, "checkout", "dev-checkout-extra").is_none());
}

#[test]
fn a_name_that_fits_stages_of_two_projects_claims_none() {
    // `a-b` in project `c` and `a` in project `b-c` both make "a-b-c".
    let mut core = session("core-fra");
    core.kargo = Source::Read(vec![project("c", &["a-b"]), project("b-c", &["a"])]);
    core.argo_applications = Source::Read(vec![argo("a-b-c", json!({}))]);
    let found = derived_named(vec![core], "{stage}-{project}");
    assert!(member_of(&found, "c", "a-b-c").is_none());
    assert!(member_of(&found, "b-c", "a-b-c").is_none());
    assert!(found.find(&app_id("a-b-c")).is_some());
    let own = found.find(&app_id("a-b-c")).unwrap();
    assert!(own.notes.iter().any(|note| matches!(
        note,
        Note::NameFitsSeveralStages { stages, .. } if stages == &["b-c/a", "c/a-b"]
    )));
}

#[test]
fn the_same_stage_read_through_two_sessions_is_not_ambiguous() {
    let mut first = session("core-fra");
    first.kargo = Source::Read(vec![project("checkout", &["dev"])]);
    first.argo_applications = Source::Read(vec![argo("dev-checkout", json!({}))]);
    let mut second = session("cicd-fra");
    second.kargo = Source::Read(vec![project("checkout", &["dev"])]);
    let found = derived_named(vec![first, second], "{stage}-{project}");
    assert!(member_of(&found, "checkout", "dev-checkout").is_some());
}
