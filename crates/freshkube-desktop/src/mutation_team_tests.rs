//! The team's floors reach the level a write is sent under (#849, slice 5C):
//! the guard the dialog opens carries the floors the window holds and reads
//! the saved team again at the send step.
use freshkube_core::team::{Choice, Membership, TeamFloors, Version, example};
use freshkube_core::workspace::SourceRecovery;
use freshkube_core::write::Protection;

use crate::mutation::{
    TEAM_FILE, context_level, send_guard, set_active_entry, set_setup_file, set_workspace_levels,
};
use gpui_kit::App;

const SETUP: &str = "version: 1\nclusters:\n  - id: prod\n    role: core\n    context: prod-ctx\n    protection: open\n";
const TEAM: &str =
    "version: 1\nclusters:\n  - id: prod-ams\n    role: core\n    protection: read-only\n";

/// The saved team of a team that holds `prod-ams` on `context`.
fn saved_team(context: &str) -> (TeamFloors, String) {
    let mut team = Membership::join(example::source(), Default::default());
    team.fetched(
        Ok(Version::parse("abc1234", TEAM.as_bytes(), Default::default()).unwrap()),
        chrono::Utc::now(),
    );
    team.map_context(
        "prod-ams",
        Choice::Context(context.into()),
        &[context.into()],
    )
    .unwrap();
    let mut review = team.review().unwrap();
    example::confirm_all(&mut review);
    team.apply(review, chrono::Utc::now()).unwrap();
    let mut file = serde_json::to_value(freshkube_core::team::SavedTeam {
        membership: team.to_saved(),
    })
    .unwrap();
    file["version"] = 1.into();
    (TeamFloors::from_membership(&team), file.to_string())
}

/// Publishes the setup as the shell does, with `floors` in its recovery.
fn publish(file: &std::path::Path, floors: TeamFloors, cx: &mut App) {
    let setup = freshkube_core::setup::parse(SETUP.as_bytes()).unwrap();
    let mut recovery = SourceRecovery::default();
    recovery.set_team_floors(floors);
    set_workspace_levels(&setup.workspace, &recovery, cx);
    set_setup_file(Some(file.to_path_buf()), true, cx);
    set_active_entry(Some("prod".into()), cx);
}

fn read(
    cx: &mut gpui_kit::TestAppContext,
) -> Result<Protection, freshkube_core::write::Unreadable> {
    cx.update(|cx| send_guard("prod-ctx", false, cx).expect("a guard").read())
}

#[gpui_kit::test]
fn safety_g32_the_send_guard_holds_the_floors_the_window_holds(cx: &mut gpui_kit::TestAppContext) {
    let directory = tempfile::tempdir().unwrap();
    let file = directory.path().join("freshkube.yaml");
    std::fs::write(&file, SETUP).unwrap();
    let (floors, _) = saved_team("prod-ctx");
    // The window opened under the floor; the saved team is gone since.
    cx.update(|cx| publish(&file, floors, cx));
    assert_eq!(read(cx), Ok(Protection::ReadOnly));
    // The same window with no floor held reads the file's own level.
    cx.update(|cx| publish(&file, TeamFloors::none(), cx));
    assert_eq!(read(cx), Ok(Protection::Open));
}

#[gpui_kit::test]
fn safety_g32_the_send_step_reads_a_team_saved_since_the_dialog_opened(
    cx: &mut gpui_kit::TestAppContext,
) {
    let directory = tempfile::tempdir().unwrap();
    let file = directory.path().join("freshkube.yaml");
    std::fs::write(&file, SETUP).unwrap();
    cx.update(|cx| publish(&file, TeamFloors::none(), cx));
    assert_eq!(read(cx), Ok(Protection::Open));
    // A team is joined and applied in Settings: `team.json` is saved.
    let (_, saved) = saved_team("prod-ctx");
    std::fs::write(directory.path().join(TEAM_FILE), &saved).unwrap();
    assert_eq!(read(cx), Ok(Protection::ReadOnly));
    // A saved team that can't be read holds the context read-only too.
    std::fs::write(directory.path().join(TEAM_FILE), "{ not json").unwrap();
    assert_eq!(read(cx), Ok(Protection::ReadOnly));
}

#[gpui_kit::test]
fn the_context_level_every_write_is_gated_on_follows_the_floors(cx: &mut gpui_kit::TestAppContext) {
    let directory = tempfile::tempdir().unwrap();
    let file = directory.path().join("freshkube.yaml");
    std::fs::write(&file, SETUP).unwrap();
    let (floors, _) = saved_team("prod-ctx");
    cx.update(|cx| {
        publish(&file, floors, cx);
        assert_eq!(context_level("prod-ctx", cx), Some(Protection::ReadOnly));
        // Another context: nothing holds it, so it is guarded.
        assert_eq!(context_level("elsewhere", cx), Some(Protection::Guarded));
        publish(&file, TeamFloors::unreadable(), cx);
        assert_eq!(context_level("elsewhere", cx), Some(Protection::ReadOnly));
    });
}
