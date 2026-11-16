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
        "applied":{"id":"a1","file":"version: 1\n","key_fingerprints":{}},
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

const MGMT: &str = r#"{"version":1,"clusters":[{"id":"mgmt","role":"core","context":"acme-mgmt","protection":"read-only","metrics":{"source":{"url":"https://metrics.acme.example"}}}]}"#;
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
    let key = "clusters[mgmt].metrics.source.url";
    cx.update(|cx| {
        settings.update(cx, |page, cx| {
            page.set_team_for_test(TEAM, cx);
            page.team_set_override_for_test(key, "https://metrics.acme.example".into());
            page.use_team_value(key.to_string(), cx);
        })
    });
    cx.run_until_parked();
    let text = std::fs::read_to_string(dir.path().join("freshkube.yaml")).unwrap();
    assert!(!text.contains("metrics.acme.example"), "{text}");
    assert!(
        text.contains("acme-mgmt"),
        "the rest of the file stays: {text}"
    );
    assert!(
        text.contains("read-only"),
        "the level is not part of it: {text}"
    );
    assert!(!cx.update(|cx| settings.read(cx).team_has_override(key)));
}

#[gpui_kit::test]
async fn use_the_teams_value_never_takes_a_level_out_of_your_file(cx: &mut TestAppContext) {
    use crate::desktop::settings::edit_tests::{open_settings, write};
    use crate::desktop::settings::tests::launch;
    use freshkube_core::write::Protection;
    cx.executor().allow_parking();
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), MGMT);
    let (_runtime, handle, view) = launch(cx, dir.path());
    open_settings(cx, handle, &view).await;
    let settings = page(cx, &view);
    let key = "clusters[mgmt].protection";
    let level = |cx: &mut TestAppContext| {
        cx.update(|cx| {
            let page = settings.read(cx);
            page.workspace()
                .level_of("acme-mgmt", &page.source_recovery)
        })
    };
    assert_eq!(level(cx), Protection::ReadOnly);
    cx.update(|cx| {
        settings.update(cx, |page, cx| {
            page.set_team_for_test(TEAM, cx);
            // An edited `team.json` can bring this override back.
            page.team_set_override_for_test(key, "read-only".into());
            page.use_team_value(key.to_string(), cx);
        })
    });
    cx.run_until_parked();
    // The line is still there, the override stays, the level is what it was.
    let text = std::fs::read_to_string(dir.path().join("freshkube.yaml")).unwrap();
    assert_eq!(text, MGMT);
    assert!(cx.update(|cx| settings.read(cx).team_has_override(key)));
    assert_eq!(level(cx), Protection::ReadOnly);
    // And Settings says it was not done, not that it was.
    cx.update(|cx| {
        let page = settings.read(cx);
        let words = page.notice_failed_words().expect("said");
        assert!(words.contains("level"), "{words}");
    });
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

#[gpui_kit::test]
fn a_file_from_a_newer_app_counts_as_corrupt_and_is_left_as_it_is(cx: &mut TestAppContext) {
    let directory = tempfile::tempdir().unwrap();
    let newer = r#"{"version": 99, "membership": {"source": {"address": "x"}}}"#;
    std::fs::write(directory.path().join("team.json"), newer).unwrap();
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
        assert!(page.team_notice().is_some());
    });
    assert_eq!(
        std::fs::read_to_string(directory.path().join("team.json")).unwrap(),
        newer
    );
}

fn context_levels(
    cx: &mut TestAppContext,
    settings: &gpui_kit::Entity<super::SettingsPage>,
) -> (
    freshkube_core::write::Protection,
    freshkube_core::write::Protection,
) {
    cx.update(|cx| {
        let page = settings.read(cx);
        let level = |context: &str| page.workspace().level_of(context, &page.source_recovery);
        (level("acme-mgmt"), level("a-context-no-entry-names"))
    })
}

#[gpui_kit::test]
fn safety_g32_an_unreadable_team_holds_every_context_read_only_until_it_is_forgotten(
    cx: &mut TestAppContext,
) {
    use freshkube_core::write::Protection;
    let directory = tempfile::tempdir().unwrap();
    std::fs::write(directory.path().join("team.json"), "{ not json").unwrap();
    let (runtime, handle, _view) = open_on(cx, "settings-team");
    let again = cx.new(|cx| {
        super::SettingsPage::new(
            runtime.handle().clone(),
            None,
            Some(&preferences(&directory)),
            cx,
        )
    });
    // The floors can't be known, so nothing is looser than read-only, whatever
    // the entry says or no entry says.
    assert_eq!(
        context_levels(cx, &again),
        (Protection::ReadOnly, Protection::ReadOnly)
    );
    cx.update(|cx| {
        let notice = again.read(cx).team_notice().expect("said");
        assert!(notice.contains("read-only"), "{notice}");
    });
    // Forgetting it writes no team over it, and the hold ends.
    cx.update(|cx| again.update(cx, |page, cx| page.team_forget_unreadable_for_test(cx)));
    settle(cx, handle);
    assert_eq!(
        context_levels(cx, &again),
        (Protection::Guarded, Protection::Guarded)
    );
    cx.update(|cx| assert!(again.read(cx).team_notice().is_none()));
    // And the next start finds no team.
    let after = cx.new(|cx| {
        super::SettingsPage::new(
            runtime.handle().clone(),
            None,
            Some(&preferences(&directory)),
            cx,
        )
    });
    assert_eq!(
        context_levels(cx, &after),
        (Protection::Guarded, Protection::Guarded)
    );
}

#[gpui_kit::test]
fn an_unreadable_team_is_held_until_a_join_writes_a_team_over_it(cx: &mut TestAppContext) {
    use freshkube_core::write::Protection;
    let directory = tempfile::tempdir().unwrap();
    std::fs::write(directory.path().join("team.json"), "{ not json").unwrap();
    let (runtime, handle, _view) = open_on(cx, "settings-team");
    let again = cx.new(|cx| {
        super::SettingsPage::new(
            runtime.handle().clone(),
            None,
            Some(&preferences(&directory)),
            cx,
        )
    });
    assert_eq!(context_levels(cx, &again).1, Protection::ReadOnly);
    // A join that applies saves the team, which is read again from now on.
    cx.update(|cx| {
        again.update(cx, |page, cx| {
            page.set_team_example(freshkube_core::team::example::Shape::NoTeam, cx);
            page.team_use_store_for_test(&preferences(&directory));
            page.team_mark_unreadable_for_test();
            page.team_join_and_apply_for_test(cx);
        })
    });
    settle(cx, handle);
    // Not read-only everywhere any more: only what the team's floors hold.
    assert_eq!(context_levels(cx, &again).1, Protection::Guarded);
}

#[gpui_kit::test]
fn the_unreadable_example_holds_every_context_read_only_until_it_is_forgotten(
    cx: &mut TestAppContext,
) {
    use crate::desktop::settings::team_join_tests::{open, visible};
    use freshkube_core::team::example::Shape;
    use freshkube_core::write::Protection;
    let (_runtime, handle, view) = open(cx, Shape::Unreadable);
    let settings = page(cx, &view);
    assert!(visible(cx, handle, "team-notice"));
    assert!(visible(cx, handle, "team-forget-unreadable"));
    assert!(
        visible(cx, handle, "team-join-address-input"),
        "joining again is offered"
    );
    // Every context the fixture's setup has is held read-only.
    cx.update(|cx| {
        let page = settings.read(cx);
        assert!(page.rows.iter().all(|row| {
            page.workspace()
                .level_of(row.context.as_ref(), &page.source_recovery)
                == Protection::ReadOnly
        }));
        assert_eq!(
            page.team_notice().map(|n| n.contains("read-only")),
            Some(true)
        );
    });
    super::team_join_tests::click_for_saved(cx, handle, "team-forget-unreadable");
    assert!(!visible(cx, handle, "team-notice"));
    cx.update(|cx| {
        let page = settings.read(cx);
        assert!(page.rows.iter().all(|row| {
            page.workspace()
                .level_of(row.context.as_ref(), &page.source_recovery)
                != Protection::ReadOnly
                || row.protection == Some(Protection::ReadOnly)
        }));
    });
}
