//! What a restart keeps of a team (#849 slice 5): the membership beside the
//! preferences, in a live window only. Nothing reaches a network.
use super::team_join_tests::{open_on, page, settle};
use freshkube_core::team::Choice;
use gpui_kit::{AppContext as _, TestAppContext};

fn preferences(directory: &tempfile::TempDir) -> std::path::PathBuf {
    directory.path().join("preferences.json")
}

#[gpui_kit::test]
fn a_restart_keeps_the_source_the_mapping_and_the_reviewed_version(cx: &mut TestAppContext) {
    let directory = tempfile::tempdir().unwrap();
    let (runtime, handle, view) = open_on(cx, "settings-team");
    let settings = page(cx, &view);
    cx.update(|cx| {
        settings.update(cx, |page, cx| {
            page.team_use_store_for_test(&preferences(&directory));
            page.team_join_and_apply_for_test(cx);
        })
    });
    settle(cx, handle);
    assert!(directory.path().join("team.json").is_file());
    let (before_source, before_version, before_mapping) = cx.update(|cx| {
        let team = settings.read(cx).team.membership().unwrap();
        (
            team.source().clone(),
            team.applied().unwrap().id().to_string(),
            team.mapping().chosen(),
        )
    });
    assert!(
        before_mapping
            .values()
            .any(|choice| matches!(choice, Choice::Context(_)))
    );

    // A new window on the same preferences starts in the team.
    let again = cx.new(|cx| {
        super::SettingsPage::new(
            runtime.handle().clone(),
            None,
            Some(&preferences(&directory)),
            cx,
        )
    });
    cx.update(|cx| {
        let team = again.read(cx).team.membership().expect("kept");
        assert_eq!(team.source(), &before_source);
        assert_eq!(team.applied().unwrap().id(), before_version);
        assert_eq!(team.mapping().chosen(), before_mapping);
        assert!(team.pending().is_none(), "what waited is fetched again");
        assert!(again.read(cx).team_notice().is_none());
    });
}

#[gpui_kit::test]
fn leaving_forgets_the_team_in_the_file_too(cx: &mut TestAppContext) {
    let directory = tempfile::tempdir().unwrap();
    let (runtime, handle, view) = open_on(cx, "settings-team");
    let settings = page(cx, &view);
    cx.update(|cx| {
        settings.update(cx, |page, cx| {
            page.team_use_store_for_test(&preferences(&directory));
            page.team_join_and_apply_for_test(cx);
            page.team_leave_for_test(freshkube_core::team::Leave::GoBack, cx);
        })
    });
    settle(cx, handle);
    let again = cx.new(|cx| {
        super::SettingsPage::new(
            runtime.handle().clone(),
            None,
            Some(&preferences(&directory)),
            cx,
        )
    });
    cx.update(|cx| {
        assert!(again.read(cx).team.membership().is_none());
        assert!(again.read(cx).team_notice().is_none());
    });
}

#[gpui_kit::test]
fn a_file_that_cannot_be_read_says_so_and_starts_without_a_team(cx: &mut TestAppContext) {
    let directory = tempfile::tempdir().unwrap();
    std::fs::write(directory.path().join("team.json"), "{ not json").unwrap();
    let (runtime, _handle, _view) = open_on(cx, "settings-team");
    let again = cx.new(|cx| {
        super::SettingsPage::new(
            runtime.handle().clone(),
            None,
            Some(&preferences(&directory)),
            cx,
        )
    });
    cx.update(|cx| {
        let page = again.read(cx);
        assert!(page.team.membership().is_none());
        let notice = page.team_notice().expect("said");
        // The words name neither the file's place nor an address.
        assert!(!notice.contains(directory.path().to_str().unwrap()));
    });
}

#[gpui_kit::test]
fn a_saved_address_the_rules_refuse_is_not_taken_up(cx: &mut TestAppContext) {
    let directory = tempfile::tempdir().unwrap();
    std::fs::write(
        directory.path().join("team.json"),
        r#"{"membership":{"source":{"address":"https://user:hunter2@git.acme.example/ops/setup.git","git_ref":"","path":""},
        "applied":{"id":"a1","values":{},"ignored":[],"key_fingerprints":{}},
        "applied_at":null,"overrides":{},"before_joining":{},"held":{},"mapping":{}}}"#,
    )
    .unwrap();
    let (runtime, _handle, _view) = open_on(cx, "settings-team");
    let again = cx.new(|cx| {
        super::SettingsPage::new(
            runtime.handle().clone(),
            None,
            Some(&preferences(&directory)),
            cx,
        )
    });
    cx.update(|cx| {
        let page = again.read(cx);
        assert!(page.team.membership().is_none());
        let notice = page.team_notice().expect("said");
        assert!(!notice.contains("hunter2") && !notice.contains("git.acme"));
    });
}

#[gpui_kit::test]
fn a_save_that_fails_is_said_and_the_team_stays_for_the_session(cx: &mut TestAppContext) {
    let directory = tempfile::tempdir().unwrap();
    // A directory where the file goes: the save can't replace it.
    std::fs::create_dir(directory.path().join("team.json")).unwrap();
    let (_runtime, handle, view) = open_on(cx, "settings-team");
    let settings = page(cx, &view);
    cx.update(|cx| {
        settings.update(cx, |page, cx| {
            page.team_use_store_for_test(&preferences(&directory));
            page.team_join_and_apply_for_test(cx);
        })
    });
    settle(cx, handle);
    cx.update(|cx| {
        let page = settings.read(cx);
        assert!(page.team_notice().expect("said").contains("forget"));
        assert!(page.team.membership().is_some());
    });
}

const MGMT: &str = r#"{"version":1,"clusters":[{"id":"mgmt","role":"core","context":"acme-mgmt","protection":"read-only"}]}"#;
const TEAM: &str = "version: 1\nclusters:\n  - id: mgmt\n    role: core\n    protection: guarded\n";

#[gpui_kit::test]
async fn use_the_teams_value_takes_your_value_out_of_your_file(cx: &mut TestAppContext) {
    use crate::desktop::settings::edit_tests::{open_settings, write};
    use crate::desktop::settings::tests::launch;
    cx.executor().allow_parking();
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), MGMT);
    let (_runtime, handle, view) = launch(cx, dir.path());
    open_settings(cx, handle, &view).await;
    let settings = page(cx, &view);
    let key = "clusters[mgmt].protection";
    cx.update(|cx| {
        settings.update(cx, |page, cx| {
            page.set_team_for_test(TEAM, cx);
            page.team_set_override_for_test(key, "read-only".into());
            page.use_team_value(key.to_string(), cx);
        })
    });
    cx.run_until_parked();
    let text = std::fs::read_to_string(dir.path().join("freshkube.yaml")).unwrap();
    assert!(!text.contains("read-only"), "{text}");
    assert!(
        text.contains("acme-mgmt"),
        "the rest of the file stays: {text}"
    );
    assert!(!cx.update(|cx| settings.read(cx).team_has_override(key)));
}

#[gpui_kit::test]
async fn use_the_teams_value_that_would_break_the_file_writes_nothing(cx: &mut TestAppContext) {
    use crate::desktop::settings::edit_tests::{open_settings, write};
    use crate::desktop::settings::tests::launch;
    cx.executor().allow_parking();
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), MGMT);
    let (_runtime, handle, view) = launch(cx, dir.path());
    open_settings(cx, handle, &view).await;
    let settings = page(cx, &view);
    let key = "clusters[mgmt].context";
    cx.update(|cx| {
        settings.update(cx, |page, cx| {
            page.set_team_for_test(TEAM, cx);
            page.team_set_override_for_test(key, "acme-mgmt".into());
            page.use_team_value(key.to_string(), cx);
        })
    });
    cx.run_until_parked();
    assert_eq!(
        std::fs::read_to_string(dir.path().join("freshkube.yaml")).unwrap(),
        MGMT
    );
    assert!(cx.update(|cx| settings.read(cx).team_has_override(key)));
}
