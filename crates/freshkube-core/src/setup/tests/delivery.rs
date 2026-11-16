use super::*;

#[test]
fn stage_naming_is_a_template_with_an_optional_project_key() {
    let text = |lines: &str| format!("version: 1\ndelivery:\n  stage_naming:\n{lines}");
    // Off when absent.
    assert_eq!(ok("version: 1\n").delivery, None);
    let setup = ok(&text("    name_template: '{stage}-{project}'\n"));
    let naming = setup.delivery.as_ref().unwrap().stage_naming.as_ref();
    assert_eq!(naming.unwrap().name_template, "{stage}-{project}");
    assert_eq!(naming.unwrap().project_key, None);
    let keyed = ok(&text(
        "    name_template: 'app-{project}-in-{stage}'\n    project_key: example.test/project\n",
    ));
    let naming = keyed.delivery.as_ref().unwrap().stage_naming.as_ref();
    assert_eq!(
        naming.unwrap().project_key.as_deref(),
        Some("example.test/project")
    );
    let written = to_text(&keyed);
    assert!(
        written.contains("project_key: example.test/project"),
        "{written}"
    );
    assert_eq!(ok(&written), keyed);
    // A setup without it writes nothing for it.
    assert!(!to_text(&ok("version: 1\n")).contains("delivery"));
}

#[test]
fn a_stage_naming_template_without_stage_is_a_fault_of_its_own() {
    let text = |lines: &str| format!("version: 1\ndelivery:\n  stage_naming:\n{lines}");
    assert_eq!(
        rule(&text("    name_template: 'app-{project}'\n")),
        "delivery.stage_naming.name_template has no {stage} in it, so it names no Stage"
    );
    assert_eq!(
        rule(&text("    name_template: ''\n")),
        "delivery.stage_naming.name_template is empty"
    );
    assert_eq!(
        rule(&text("    name_template: '{stage}'\n    project_key: ''\n")),
        "delivery.stage_naming.project_key is empty"
    );
    // The template is required once the block is there.
    assert!(matches!(
        refused(&text("    project_key: example.test/project\n")),
        Invalid::WrongShape(_)
    ));
}

#[test]
fn unknown_keys_under_delivery_are_kept_and_named() {
    let setup = ok("\
version: 1
delivery:
  delivery_note: 1
  stage_naming: {name_template: '{stage}', naming_note: 2}
");
    assert_eq!(
        setup.unknown_keys(),
        [
            "delivery.delivery_note",
            "delivery.stage_naming.naming_note"
        ]
    );
    assert_eq!(ok(&to_text(&setup)), setup);
}

#[test]
fn a_stage_naming_template_needs_only_a_stage_somewhere_in_it() {
    let text = |template: &str| {
        format!("version: 1\ndelivery:\n  stage_naming:\n    name_template: '{template}'\n")
    };
    for template in [
        "{stage}",
        "{stage}-{stage}",
        "{stage}-{x}",
        "app-{project}-{stage}",
    ] {
        let setup = ok(&text(template));
        let naming = setup.delivery.unwrap().stage_naming.unwrap();
        assert_eq!(naming.name_template, template);
    }
}

#[test]
fn the_build_namespace_is_an_optional_delivery_key() {
    // Off when absent, and a setup without it writes nothing for it.
    assert_eq!(ok("version: 1\n").delivery, None);
    assert!(!to_text(&ok("version: 1\n")).contains("delivery"));
    let setup = ok("version: 1\ndelivery:\n  build_namespace: acme-builds\n");
    assert_eq!(
        setup
            .delivery
            .as_ref()
            .and_then(|delivery| delivery.build_namespace.as_deref()),
        Some("acme-builds")
    );
    let written = to_text(&setup);
    assert!(
        written.contains("build_namespace: acme-builds"),
        "{written}"
    );
    assert_eq!(ok(&written), setup);
    assert_eq!(
        rule("version: 1\ndelivery:\n  build_namespace: ''\n"),
        "delivery.build_namespace is empty"
    );
}

#[test]
fn unknown_keys_under_delivery_are_kept_and_named_beside_the_build_namespace() {
    let setup = ok("version: 1\ndelivery:\n  build_namespace: acme-builds\n  delivery_note: 1\n");
    assert_eq!(setup.unknown_keys(), ["delivery.delivery_note"]);
    assert_eq!(ok(&to_text(&setup)), setup);
}

#[test]
fn the_build_namespace_must_be_a_namespace_name_and_is_never_repeated() {
    let long = "a".repeat(64);
    for bad in [" a/b", "../x", "Acme", "-a", "a-", "a b", long.as_str()] {
        assert_eq!(
            rule(&format!(
                "version: 1\ndelivery:\n  build_namespace: '{bad}'\n"
            )),
            "delivery.build_namespace isn’t a Kubernetes namespace name",
            "{bad:?}"
        );
    }
    for good in ["a", "acme-builds", "x1", &"a".repeat(63)] {
        ok(&format!(
            "version: 1\ndelivery:\n  build_namespace: {good}\n"
        ));
    }
}

#[test]
fn build_namespace_templates_keep_the_placeholder_on_round_trip() {
    for template in [
        "{project}",
        "{project}-ci",
        "ci-{project}",
        "{project}-{project}",
    ] {
        let setup = ok(&format!(
            "version: 1\ndelivery:\n  build_namespace: '{template}'\n"
        ));
        assert_eq!(
            setup.delivery.as_ref().unwrap().build_namespace.as_deref(),
            Some(template)
        );
        assert_eq!(ok(&to_text(&setup)), setup);
    }
}

#[test]
fn build_namespace_templates_refuse_unknown_or_malformed_placeholders() {
    for template in [
        "{stage}",
        "{app}",
        "{project}-{stage}",
        "{project",
        "project}",
        "{{project}}",
    ] {
        assert_eq!(
            rule(&format!(
                "version: 1\ndelivery:\n  build_namespace: '{template}'\n"
            )),
            "delivery.build_namespace isn’t a Kubernetes namespace name"
        );
    }
}

#[test]
fn build_namespace_resolution_checks_each_projects_result() {
    for project in ["shop", "billing"] {
        assert_eq!(
            resolve_build_namespace("{project}", project),
            Ok(project.into())
        );
        assert_eq!(
            resolve_build_namespace("{project}-ci", project),
            Ok(format!("{project}-ci"))
        );
        assert_eq!(
            resolve_build_namespace("builds", project),
            Ok("builds".into())
        );
    }
    let project = "s".repeat(60);
    assert!(resolve_build_namespace("{project}-ci", &project).is_ok());
    for project in [
        "s".repeat(61),
        "Shop".into(),
        "shop.team".into(),
        String::new(),
    ] {
        let name = format!("{project}-ci");
        assert_eq!(
            resolve_build_namespace("{project}-ci", &project),
            Err(format!(
                "The build namespace in the setup file resolves to \"{name}\" for project {project}, which isn't a namespace name"
            ))
        );
    }
    // A valid file can produce a bad name for one project without refusing the file.
    ok("version: 1\ndelivery:\n  build_namespace: '{project}-ci'\n");
}

#[test]
fn applications_name_the_stages_they_are_not_expected_in() {
    let text = "version: 1\ndelivery:\n  applications:\n    - name: checkout\n      not_expected_in: [prod-lon, dev-eu]\n    - name: catalog\n";
    let setup = ok(text);
    let delivery = setup.delivery.as_ref().unwrap();
    assert_eq!(delivery.applications.len(), 2);
    assert_eq!(
        not_expected_in(&delivery.applications, "checkout"),
        ["prod-lon", "dev-eu"]
    );
    // An entry that lists none, and a project with no entry, expect all.
    assert!(not_expected_in(&delivery.applications, "catalog").is_empty());
    assert!(not_expected_in(&delivery.applications, "payments").is_empty());
    // It round-trips, and a setup without it writes nothing for it.
    let written = to_text(&setup);
    assert!(written.contains("not_expected_in:"), "{written}");
    assert_eq!(ok(&written), setup);
    assert!(
        !to_text(&ok("version: 1\ndelivery:\n  build_namespace: builds\n"))
            .contains("applications")
    );
}

#[test]
fn the_first_entry_for_a_project_wins() {
    // A strict parse keeps the rows as written; a disk read drops a second
    // entry for the project and a Stage listed twice (recovery's tests).
    let setup = ok(
        "version: 1\ndelivery:\n  applications:\n    - {name: checkout, not_expected_in: [prod-lon]}\n    - {name: checkout, not_expected_in: [dev]}\n",
    );
    let delivery = setup.delivery.unwrap();
    assert_eq!(
        not_expected_in(&delivery.applications, "checkout"),
        ["prod-lon"]
    );
}

#[test]
fn an_application_needs_a_name_and_no_empty_stage() {
    assert_eq!(
        rule("version: 1\ndelivery:\n  applications:\n    - {name: ''}\n"),
        "delivery.applications[0].name is empty"
    );
    assert_eq!(
        rule(
            "version: 1\ndelivery:\n  applications:\n    - {name: a}\n    - {name: b, not_expected_in: ['']}\n"
        ),
        "delivery.applications[1].not_expected_in[0] is empty"
    );
}

#[test]
fn unknown_keys_under_an_application_are_kept_and_named() {
    let setup = ok("version: 1\ndelivery:\n  applications:\n    - {name: a, not_expected: [x]}\n");
    assert_eq!(
        setup.unknown_keys(),
        ["delivery.applications[a].not_expected"]
    );
    assert!(to_text(&setup).contains("not_expected"));
}

#[test]
fn a_project_or_stage_name_is_short_text_without_control_characters() {
    let long = "a".repeat(65);
    assert_eq!(
        rule(&format!(
            "version: 1\ndelivery:\n  applications:\n    - {{name: {long}}}\n"
        )),
        "delivery.applications[0].name is longer than 64 bytes or has a control character"
    );
    assert_eq!(
        rule("version: 1\ndelivery:\n  applications:\n    - {name: \"a\\nb\"}\n"),
        "delivery.applications[0].name is longer than 64 bytes or has a control character"
    );
    assert_eq!(
        rule(
            "version: 1\ndelivery:\n  applications:\n    - {name: a, not_expected_in: [\"x\\u0007\"]}\n"
        ),
        "delivery.applications[0].not_expected_in[0] is longer than 64 bytes or has a control character"
    );
    // The value itself is never repeated, and 64 bytes is fine.
    let fine = "a".repeat(64);
    assert!(
        ok(&format!(
            "version: 1\ndelivery:\n  applications:\n    - {{name: {fine}}}\n"
        ))
        .delivery
        .is_some()
    );
}

#[test]
fn a_template_is_shown_without_its_placeholders_braces() {
    use crate::setup::plain_template;
    assert_eq!(plain_template("ci-{project}"), "ci-the project");
    assert_eq!(plain_template("{stage}-{project}"), "the stage-the project");
    assert_eq!(plain_template("shop-ci"), "shop-ci");
}
