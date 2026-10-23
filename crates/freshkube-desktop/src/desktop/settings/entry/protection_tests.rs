//! The Protection group (#852): what an unset and a set entry show, that a
//! choice is a draft until Save, what Save writes, and the group in the
//! smallest window at the largest text.
use super::tests::{rich, select, to_top};
use crate::desktop::layout_check;
use crate::desktop::settings::edit_tests::{open_settings, read_workspace, saved, write};
use crate::desktop::settings::tests::launch;
use crate::desktop::testing::mount;
use crate::{DebugOptions, GpuiOptions};
use freshkube_core::setup;
use freshkube_core::write::Protection;
use gpui_kit::{AnyWindowHandle, AppContext, Entity, TestAppContext, test::TestWindowExt};

const THREE: &str = r#"{"version":1,"clusters":[
    {"id":"mgmt","role":"core","context":"acme-mgmt"},
    {"id":"prod","role":"environment","context":"acme-prod","protection":"protected"}]}"#;

type Pilot = Entity<crate::desktop::Pilot>;

/// The draft's level and what the file holds, as the group sees them.
fn draft(cx: &mut TestAppContext, view: &Pilot) -> (Option<Protection>, Option<Protection>) {
    cx.read(|cx| {
        let page = view.read(cx).settings_page.read(cx);
        let level = &page.entry.as_ref().unwrap().protection;
        (level.values.level, level.baseline.level)
    })
}

/// Scrolls the Inspector, a step at a time, until the group's Save shows:
/// Identity's fields sit above it.
fn reveal(cx: &mut TestAppContext, handle: AnyWindowHandle) {
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        for _ in 0..40 {
            if window
                .try_find("settings-protection-save")
                .is_some_and(|save| save.visible())
            {
                break;
            }
            window.scroll(
                "settings-detail-body",
                gpui_kit::ScrollDelta::Pixels(gpui_kit::point(
                    gpui_kit::px(0.),
                    gpui_kit::px(-60.),
                )),
                cx,
            );
            window.render_frame(cx);
        }
    })
    .unwrap();
}

fn choose(cx: &mut TestAppContext, handle: AnyWindowHandle, level: Protection) {
    let ix = Protection::ALL.iter().position(|l| *l == level).unwrap();
    reveal(cx, handle);
    cx.update_window(handle, |_, window, cx| {
        window.click(("settings-protection-level", ix), cx);
        window.render_frame(cx);
    })
    .unwrap();
}

#[gpui_kit::test]
async fn an_unset_entry_shows_guarded_and_a_set_one_its_level(cx: &mut TestAppContext) {
    cx.executor().allow_parking();
    let guard = tempfile::tempdir().unwrap();
    write(guard.path(), THREE);
    let (_runtime, handle, view) = launch(cx, guard.path());
    open_settings(cx, handle, &view).await;
    select(cx, handle, "mgmt");
    assert_eq!(draft(cx, &view), (None, None));
    reveal(cx, handle);
    cx.update_window(handle, |_, window, _| {
        for id in [
            "settings-protection",
            "settings-protection-save",
            "settings-protection-revert",
            "settings-protection-saved-in",
        ] {
            assert!(window.find(id).visible(), "{id}");
        }
        assert!(window.try_find("settings-protection-dirty").is_none());
    })
    .unwrap();
    select(cx, handle, "prod");
    assert_eq!(
        draft(cx, &view),
        (Some(Protection::Protected), Some(Protection::Protected))
    );
}

#[gpui_kit::test]
async fn a_choice_is_a_draft_until_save_and_save_writes_only_its_key(cx: &mut TestAppContext) {
    cx.executor().allow_parking();
    let guard = tempfile::tempdir().unwrap();
    write(guard.path(), &rich());
    let before = std::fs::read(guard.path().join("freshkube.yaml")).unwrap();
    let (_runtime, handle, view) = launch(cx, guard.path());
    open_settings(cx, handle, &view).await;
    select(cx, handle, "prod");
    choose(cx, handle, Protection::Open);
    assert_eq!(draft(cx, &view), (Some(Protection::Open), None));
    cx.update_window(handle, |_, window, _| {
        assert!(window.find("settings-protection-dirty").visible());
    })
    .unwrap();
    // Nothing is written by choosing.
    assert_eq!(
        std::fs::read(guard.path().join("freshkube.yaml")).unwrap(),
        before
    );
    cx.update_window(handle, |_, window, cx| {
        window.click("settings-protection-save", cx);
    })
    .unwrap();
    saved(cx, handle, &view).await;
    let mut expected = setup::parse(rich().as_bytes()).unwrap();
    expected
        .workspace
        .clusters
        .iter_mut()
        .find(|e| e.id == "prod")
        .unwrap()
        .protection = Some(Protection::Open);
    let written =
        setup::parse(&std::fs::read(guard.path().join("freshkube.yaml")).unwrap()).unwrap();
    // Every other key, entry and unknown key is as it was.
    assert_eq!(written, expected);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(window.try_find("settings-protection-dirty").is_none());
    })
    .unwrap();
    assert_eq!(
        draft(cx, &view),
        (Some(Protection::Open), Some(Protection::Open))
    );
}

#[gpui_kit::test]
async fn choosing_guarded_writes_the_word_and_an_untouched_entry_stays_unset(
    cx: &mut TestAppContext,
) {
    cx.executor().allow_parking();
    let guard = tempfile::tempdir().unwrap();
    write(guard.path(), THREE);
    let (_runtime, handle, view) = launch(cx, guard.path());
    open_settings(cx, handle, &view).await;
    select(cx, handle, "mgmt");
    // Saving a clean group writes nothing, so an unset level stays unset.
    reveal(cx, handle);
    cx.update_window(handle, |_, window, cx| {
        window.click("settings-protection-save", cx);
    })
    .unwrap();
    cx.run_until_parked();
    let text = std::fs::read_to_string(guard.path().join("freshkube.yaml")).unwrap();
    assert!(!text.contains("guarded"), "{text}");
    choose(cx, handle, Protection::Guarded);
    assert_eq!(draft(cx, &view), (Some(Protection::Guarded), None));
    cx.update_window(handle, |_, window, cx| {
        window.click("settings-protection-save", cx);
    })
    .unwrap();
    saved(cx, handle, &view).await;
    let written = read_workspace(guard.path());
    let mgmt = written.clusters.iter().find(|e| e.id == "mgmt").unwrap();
    assert_eq!(mgmt.protection, Some(Protection::Guarded));
    let prod = written.clusters.iter().find(|e| e.id == "prod").unwrap();
    assert_eq!(prod.protection, Some(Protection::Protected));
}

#[gpui_kit::test]
async fn revert_drops_the_choice_and_the_file_is_not_touched(cx: &mut TestAppContext) {
    cx.executor().allow_parking();
    let guard = tempfile::tempdir().unwrap();
    write(guard.path(), THREE);
    let before = std::fs::read(guard.path().join("freshkube.yaml")).unwrap();
    let (_runtime, handle, view) = launch(cx, guard.path());
    open_settings(cx, handle, &view).await;
    select(cx, handle, "prod");
    choose(cx, handle, Protection::ReadOnly);
    assert!(draft(cx, &view).0 == Some(Protection::ReadOnly));
    cx.update_window(handle, |_, window, cx| {
        window.click("settings-protection-revert", cx);
        window.render_frame(cx);
        assert!(window.try_find("settings-protection-dirty").is_none());
    })
    .unwrap();
    assert_eq!(
        draft(cx, &view),
        (Some(Protection::Protected), Some(Protection::Protected))
    );
    assert_eq!(
        std::fs::read(guard.path().join("freshkube.yaml")).unwrap(),
        before
    );
}

#[gpui_kit::test]
async fn a_saved_level_reaches_the_header_and_the_status_bar(cx: &mut TestAppContext) {
    cx.executor().allow_parking();
    let guard = tempfile::tempdir().unwrap();
    write(guard.path(), THREE);
    let (_runtime, handle, view) = launch(cx, guard.path());
    open_settings(cx, handle, &view).await;
    // The window shows acme-mgmt, as a connection to it would leave it.
    cx.update_window(handle, |_, window, cx| {
        view.update(cx, |pilot, cx| {
            pilot.applied.context = Some("acme-mgmt".into());
            pilot.prepare_context_display(window, cx);
            cx.notify();
        });
        window.render_frame(cx);
        assert_eq!(
            window.find("status-protection").label(),
            Some("Protection: Guarded. Each action asks for its own confirmation.")
        );
    })
    .unwrap();
    let id = "mgmt";
    select(cx, handle, id);
    choose(cx, handle, Protection::ReadOnly);
    cx.update_window(handle, |_, window, cx| {
        window.click("settings-protection-save", cx);
    })
    .unwrap();
    saved(cx, handle, &view).await;
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let words = format!(
            "Protection: {}. {}",
            Protection::ReadOnly.label(),
            Protection::ReadOnly.meaning()
        );
        for tag in ["header-protection", "status-protection"] {
            assert_eq!(window.find(tag).label(), Some(words.as_str()), "{tag}");
        }
    })
    .unwrap();
}

/// Every level's group in the smallest window at the largest text: the
/// segment wraps instead of leaving the Inspector, and the group sits
/// inside the window.
#[gpui_kit::test]
fn the_group_fits_the_smallest_window_at_the_largest_text(cx: &mut TestAppContext) {
    let options = GpuiOptions::fixture().with_debug(DebugOptions {
        page: Some("settings-clusters".into()),
        text_size: Some(20.),
        ..DebugOptions::default()
    });
    let (_runtime, handle, _pilot) = mount(cx, options, 760., 560.);
    cx.update_window(handle, |_, window, cx| {
        crate::text_size::set(20., cx);
        window.render_frame(cx);
        window.click("settings-cluster-prod-ams", cx);
        window.render_frame(cx);
        let group = window.find("settings-protection");
        let viewport = window.viewport_size();
        assert!(group.visible() || group.bounds().size.height > gpui_kit::px(0.));
        let bounds = group.bounds();
        assert!(bounds.left() >= gpui_kit::px(0.) && bounds.right() <= viewport.width);
        let _ = layout_check::settle_header;
    })
    .unwrap();
    to_top(cx, handle);
}

/// The Clusters table's Protection column: a tag at the table's size, and
/// "· default" in muted beside Guarded when the entry sets none.
#[gpui_kit::test]
async fn the_table_shows_each_level_and_says_when_guarded_is_the_default(cx: &mut TestAppContext) {
    cx.executor().allow_parking();
    let guard = tempfile::tempdir().unwrap();
    write(guard.path(), THREE);
    let (_runtime, handle, view) = launch(cx, guard.path());
    open_settings(cx, handle, &view).await;
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(
            window.find("settings-cluster-mgmt-protection").label(),
            Some("Guarded · default")
        );
        assert_eq!(
            window
                .find("settings-cluster-mgmt-protection-default")
                .label(),
            Some("· default")
        );
        assert_eq!(
            window.find("settings-cluster-prod-protection").label(),
            Some("Protected")
        );
        assert!(
            window
                .try_find("settings-cluster-prod-protection-default")
                .is_none()
        );
        // The tag fits the row: 18 dp in a 26 dp row.
        let tag = window.find("settings-cluster-prod-protection").bounds();
        let row = window.find("settings-cluster-prod").bounds();
        assert!(tag.size.height < row.size.height);
        assert!(tag.top() >= row.top() && tag.bottom() <= row.bottom());
    })
    .unwrap();
}

/// A team file that names `prod` with a level and `mgmt` with none.
const TEAM: &str = "version: 1\nclusters:\n  - id: prod\n    role: environment\n    protection: protected\n  - id: mgmt\n    role: core\n";

#[gpui_kit::test]
async fn the_teams_level_is_a_floor_the_inspector_will_not_go_under(cx: &mut TestAppContext) {
    cx.executor().allow_parking();
    let guard = tempfile::tempdir().unwrap();
    write(guard.path(), THREE);
    let (_runtime, handle, view) = launch(cx, guard.path());
    open_settings(cx, handle, &view).await;
    cx.update(|cx| {
        view.read(cx)
            .settings_page
            .clone()
            .update(cx, |page, cx| page.set_team_for_test(TEAM, cx))
    });
    // prod: the team's Protected. Looser levels do nothing; stricter choose.
    select(cx, handle, "prod");
    for level in [Protection::Open, Protection::Guarded] {
        choose(cx, handle, level);
        assert_eq!(
            draft(cx, &view),
            (Some(Protection::Protected), Some(Protection::Protected)),
            "{level:?}"
        );
    }
    choose(cx, handle, Protection::ReadOnly);
    assert_eq!(draft(cx, &view).0, Some(Protection::ReadOnly));
}

#[gpui_kit::test]
async fn a_team_cluster_with_no_level_has_the_guarded_floor(cx: &mut TestAppContext) {
    cx.executor().allow_parking();
    let guard = tempfile::tempdir().unwrap();
    write(guard.path(), THREE);
    let (_runtime, handle, view) = launch(cx, guard.path());
    open_settings(cx, handle, &view).await;
    cx.update(|cx| {
        view.read(cx)
            .settings_page
            .clone()
            .update(cx, |page, cx| page.set_team_for_test(TEAM, cx))
    });
    select(cx, handle, "mgmt");
    choose(cx, handle, Protection::Open);
    assert_eq!(draft(cx, &view), (None, None));
    choose(cx, handle, Protection::Protected);
    assert_eq!(draft(cx, &view).0, Some(Protection::Protected));
}

#[gpui_kit::test]
async fn without_a_team_every_level_still_chooses(cx: &mut TestAppContext) {
    cx.executor().allow_parking();
    let guard = tempfile::tempdir().unwrap();
    write(guard.path(), THREE);
    let (_runtime, handle, view) = launch(cx, guard.path());
    open_settings(cx, handle, &view).await;
    select(cx, handle, "prod");
    choose(cx, handle, Protection::Open);
    assert_eq!(draft(cx, &view).0, Some(Protection::Open));
}

/// A team file that makes `protected` the floor of three clusters.
const FLOORS: &str = "version: 1\nclusters:\n  - id: unset\n    role: core\n    protection: protected\n  - id: loose\n    role: environment\n    protection: protected\n  - id: strict\n    role: environment\n    protection: protected\n";

#[gpui_kit::test]
async fn an_own_or_default_level_looser_than_the_floor_shows_the_floor(cx: &mut TestAppContext) {
    cx.executor().allow_parking();
    let guard = tempfile::tempdir().unwrap();
    write(
        guard.path(),
        r#"{"version":1,"clusters":[
            {"id":"unset","role":"core","context":"acme-unset"},
            {"id":"loose","role":"environment","context":"acme-loose","protection":"open"},
            {"id":"strict","role":"environment","context":"acme-strict","protection":"read-only"}]}"#,
    );
    let (_runtime, handle, view) = launch(cx, guard.path());
    open_settings(cx, handle, &view).await;
    cx.update(|cx| {
        view.read(cx)
            .settings_page
            .clone()
            .update(cx, |page, cx| page.set_team_for_test(FLOORS, cx))
    });
    cx.update_window(handle, |_, window, cx| window.render_frame(cx))
        .unwrap();
    cx.update_window(handle, |_, window, _| {
        for id in ["unset", "loose"] {
            let tag = window.find(format!("settings-cluster-{id}-protection"));
            assert_eq!(tag.label(), Some("Protected · team floor"), "{id}");
            assert!(
                window
                    .try_find(format!("settings-cluster-{id}-protection-floor"))
                    .is_some(),
                "{id}"
            );
            assert!(
                window
                    .try_find(format!("settings-cluster-{id}-protection-default"))
                    .is_none(),
                "{id}"
            );
        }
        // A stricter level of your own stays.
        let strict = window.find("settings-cluster-strict-protection");
        assert_eq!(strict.label(), Some("Read-only"));
    })
    .unwrap();
    // The Inspector shows the same, and the draft keeps what the file holds.
    select(cx, handle, "unset");
    reveal(cx, handle);
    cx.update_window(handle, |_, window, _| {
        assert_eq!(
            window.find("settings-protection-raised").label(),
            Some("Protected · team floor")
        );
    })
    .unwrap();
    assert_eq!(draft(cx, &view), (None, None));
}
