mod common;

use common::capture;

use common::*;
use freshkube_agent::Access;
use freshkube_agent::saved::AuditLine;
use freshkube_core::agent_reads::{
    AgentAccess, AgentTarget, Call, Code, Source, example_objects, read,
};
use serde_json::json;

fn target() -> AgentTarget {
    AgentTarget {
        entry: ENTRY.into(),
        context: CONTEXT.into(),
        source: Source::Example(example_objects(Objects)),
    }
}

async fn can_read(gate: &AgentAccess) -> bool {
    let call = Call::parse(
        "list_objects",
        &json!({"kind": "pods", "namespace": "shop"}),
    )
    .unwrap();
    match read(gate, &target(), &call.request).await {
        Ok(_) => true,
        Err(refusal) => {
            assert_eq!(refusal.code, Code::NotEnabled);
            false
        }
    }
}

#[cfg(unix)]
fn mode(path: &std::path::Path) -> u32 {
    use std::os::unix::fs::PermissionsExt;
    std::fs::metadata(path).unwrap().permissions().mode() & 0o777
}

fn audit_lines(path: &std::path::Path) -> Vec<AuditLine> {
    std::fs::read_to_string(path)
        .unwrap_or_default()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect()
}

#[tokio::test]
async fn set_access_saves_audits_and_gates_together() {
    capture::install();
    let dir = tempfile::tempdir().unwrap();
    let preferences = dir.path().join("preferences.json");
    let gate = AgentAccess::new();
    let access = Access::open(&preferences, &gate);
    assert!(!can_read(&gate).await, "off by default");

    access
        .set_access(ENTRY, CONTEXT, true, "egress card")
        .unwrap();
    assert!(can_read(&gate).await);
    assert!(preferences.with_file_name("agent.json").exists());
    let lines = audit_lines(&access.audit_path());
    assert_eq!(lines.len(), 1);
    assert_eq!(
        (
            lines[0].entry.as_str(),
            lines[0].context.as_str(),
            lines[0].access.as_str(),
            lines[0].from.as_str()
        ),
        (ENTRY, CONTEXT, "on", "egress card")
    );
    #[cfg(unix)]
    {
        assert_eq!(mode(access.file_path()), 0o600);
        assert_eq!(mode(&access.audit_path()), 0o600);
    }

    // A restart loads the grant back.
    let again = AgentAccess::new();
    let reopened = Access::open(&preferences, &again);
    assert_eq!(reopened.grants().len(), 1);
    assert!(can_read(&again).await);

    access
        .set_access(ENTRY, CONTEXT, false, "settings")
        .unwrap();
    assert!(!can_read(&gate).await);
    assert!(access.grants().is_empty());
    let lines = audit_lines(&access.audit_path());
    assert_eq!(
        lines.iter().map(|l| l.access.as_str()).collect::<Vec<_>>(),
        ["on", "off"]
    );
    let after_off = AgentAccess::new();
    Access::open(&preferences, &after_off);
    assert!(!can_read(&after_off).await, "the file no longer grants it");
}

#[tokio::test]
async fn a_missing_file_grants_nothing_and_an_unparsable_one_grants_nothing_and_logs_once() {
    capture::install();
    let dir = tempfile::tempdir().unwrap();
    let preferences = dir.path().join("preferences.json");
    let gate = AgentAccess::new();
    Access::open(&preferences, &gate);
    assert!(gate.granted().is_empty());

    std::fs::write(preferences.with_file_name("agent.json"), "{ not json").unwrap();
    let before = capture::captured()
        .matches("agent access is off everywhere")
        .count();
    let gate = AgentAccess::new();
    let access = Access::open(&preferences, &gate);
    assert!(gate.granted().is_empty() && access.grants().is_empty());
    assert!(!can_read(&gate).await);
    let after = capture::captured()
        .matches("agent access is off everywhere")
        .count();
    assert_eq!(after - before, 1, "said once");
}

#[tokio::test]
async fn a_failed_audit_changes_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let preferences = dir.path().join("preferences.json");
    // A file where the audit directory should be: the line cannot be written.
    std::fs::write(dir.path().join("audit"), "in the way").unwrap();
    let gate = AgentAccess::new();
    let access = Access::open(&preferences, &gate);
    assert!(access.set_access(ENTRY, CONTEXT, true, "settings").is_err());
    assert!(!can_read(&gate).await, "the gate is unchanged");
    assert!(access.grants().is_empty());
    let reopened = AgentAccess::new();
    Access::open(&preferences, &reopened);
    assert!(reopened.granted().is_empty(), "the file was put back");
}

#[tokio::test]
async fn turning_a_cluster_off_ends_a_session_endpoints_reads() {
    let dir = tempfile::tempdir().unwrap();
    let preferences = dir.path().join("preferences.json");
    let gate = AgentAccess::new();
    let access = Access::open(&preferences, &gate);
    access.set_access(ENTRY, CONTEXT, true, "settings").unwrap();
    let (endpoint, _events) = start(&gate).await;
    let client = client(&endpoint).await;
    let ask = || {
        let serde_json::Value::Object(arguments) = json!({"kind": "pods", "namespace": "shop"})
        else {
            unreachable!()
        };
        rmcp::model::CallToolRequestParams::new("list_objects").with_arguments(arguments)
    };
    // The target's context for the session entry is `ctx-a`, as granted.
    assert_eq!(client.call_tool(ask()).await.unwrap().is_error, Some(false));
    access
        .set_access(ENTRY, CONTEXT, false, "settings")
        .unwrap();
    let refused = client.call_tool(ask()).await.unwrap();
    assert_eq!(refused.is_error, Some(true));
    assert_eq!(
        refused.structured_content.unwrap()["error"]["code"],
        "not_enabled"
    );
}

#[tokio::test]
async fn turning_access_off_fails_closed_even_when_the_save_or_the_audit_fails() {
    // The audit line cannot be written.
    let dir = tempfile::tempdir().unwrap();
    let preferences = dir.path().join("preferences.json");
    let gate = AgentAccess::new();
    let access = Access::open(&preferences, &gate);
    access.set_access(ENTRY, CONTEXT, true, "settings").unwrap();
    assert!(can_read(&gate).await);
    std::fs::remove_dir_all(dir.path().join("audit")).unwrap();
    std::fs::write(dir.path().join("audit"), "in the way").unwrap();
    assert!(
        access
            .set_access(ENTRY, CONTEXT, false, "settings")
            .is_err()
    );
    assert!(
        !can_read(&gate).await,
        "a failed audit does not keep access on"
    );
    assert!(access.grants().is_empty());

    // The file cannot be saved: a directory sits where it should be.
    let dir = tempfile::tempdir().unwrap();
    let preferences = dir.path().join("preferences.json");
    let gate = AgentAccess::new();
    let access = Access::open(&preferences, &gate);
    access.set_access(ENTRY, CONTEXT, true, "settings").unwrap();
    std::fs::remove_file(access.file_path()).unwrap();
    std::fs::create_dir(access.file_path()).unwrap();
    assert!(
        access
            .set_access(ENTRY, CONTEXT, false, "settings")
            .is_err()
    );
    assert!(
        !can_read(&gate).await,
        "a failed save does not keep access on"
    );
    assert!(access.grants().is_empty());
}

#[tokio::test]
async fn a_failed_save_of_turning_off_is_pending_until_it_is_written() {
    let dir = tempfile::tempdir().unwrap();
    let preferences = dir.path().join("preferences.json");
    let gate = AgentAccess::new();
    let access = Access::open(&preferences, &gate);
    access
        .set_access(ENTRY, CONTEXT, true, "egress card")
        .unwrap();
    assert!(!access.save_pending());

    // A directory sits where the file should be, so the save fails.
    std::fs::remove_file(access.file_path()).unwrap();
    std::fs::create_dir(access.file_path()).unwrap();
    assert!(
        access
            .set_access(ENTRY, CONTEXT, false, "settings")
            .is_err()
    );
    assert!(
        !can_read(&gate).await,
        "the endpoint refuses from that moment"
    );
    assert!(access.save_pending(), "the file may still hold the grant");
    assert!(access.retry_save().is_err(), "still in the way");
    assert!(access.save_pending());

    std::fs::remove_dir(access.file_path()).unwrap();
    access.retry_save().unwrap();
    assert!(!access.save_pending());
    let gate = AgentAccess::new();
    assert!(
        access.pending_revocations().is_empty(),
        "the marker is cleared"
    );
    let again = Access::open(&preferences, &gate);
    assert!(
        again.grants().is_empty(),
        "nothing comes back at the next start"
    );
    assert!(!can_read(&gate).await);
}

#[test]
fn the_chosen_opencode_and_the_data_mode_are_kept_beside_the_grants() {
    let dir = tempfile::tempdir().unwrap();
    let preferences = dir.path().join("preferences.json");
    let gate = AgentAccess::new();
    let access = Access::open(&preferences, &gate);
    assert_eq!(access.data_mode(), freshkube_agent::DataMode::Own);
    assert_eq!(access.opencode_path(), None);
    access
        .set_opencode_path(Some("/opt/tools/opencode".into()))
        .unwrap();
    access.set_access(ENTRY, CONTEXT, true, "settings").unwrap();
    let again = Access::open(&preferences, &AgentAccess::new());
    assert_eq!(again.opencode_path(), Some("/opt/tools/opencode".into()));
    assert_eq!(again.grants().len(), 1, "a grant keeps the chosen file");

    // `data: shared` is a setting the person writes in the file.
    let text = std::fs::read_to_string(again.file_path()).unwrap();
    std::fs::write(
        again.file_path(),
        text.replace("\"data\": \"own\"", "\"data\": \"shared\""),
    )
    .unwrap();
    let shared = Access::open(&preferences, &AgentAccess::new());
    assert_eq!(shared.data_mode(), freshkube_agent::DataMode::Shared);
}

#[tokio::test]
async fn a_revocation_that_was_not_saved_survives_a_restart() {
    let dir = tempfile::tempdir().unwrap();
    let preferences = dir.path().join("preferences.json");
    let gate = AgentAccess::new();
    let access = Access::open(&preferences, &gate);
    access
        .set_access(ENTRY, CONTEXT, true, "egress card")
        .unwrap();
    let file = access.file_path().to_path_buf();
    let kept = std::fs::read(&file).unwrap();

    // Turning it off can't be saved: the file still says on.
    std::fs::remove_file(&file).unwrap();
    std::fs::create_dir(&file).unwrap();
    assert!(
        access
            .set_access(ENTRY, CONTEXT, false, "agent tab")
            .is_err()
    );
    assert!(!can_read(&gate).await);
    let marker = access.pending_revocations();
    assert_eq!(marker.len(), 1);
    assert_eq!(marker[0].entry, ENTRY);
    drop(access);

    // After a restart the file is as it was, and says on. It is not trusted:
    // the marker beside it says that turning it off was never saved.
    std::fs::remove_dir(&file).unwrap();
    std::fs::write(&file, &kept).unwrap();
    let gate = AgentAccess::new();
    let again = Access::open(&preferences, &gate);
    assert!(!can_read(&gate).await, "the cluster is still off");
    assert!(again.grants().is_empty());
    assert_eq!(again.held_off(), [ENTRY.to_owned()]);
    // The save succeeded this time, so the marker is cleared and the file
    // says off.
    assert!(!again.save_pending());
    assert!(again.pending_revocations().is_empty());
    drop(again);
    let gate = AgentAccess::new();
    let third = Access::open(&preferences, &gate);
    assert!(!can_read(&gate).await);
    assert!(third.held_off().is_empty());

    // Turning it on again is the person's choice, and works.
    third
        .set_access(ENTRY, CONTEXT, true, "egress card")
        .unwrap();
    assert!(can_read(&gate).await);
}

#[tokio::test]
async fn a_revocation_that_still_cannot_be_saved_stays_off_across_restarts() {
    let dir = tempfile::tempdir().unwrap();
    let preferences = dir.path().join("preferences.json");
    let gate = AgentAccess::new();
    let access = Access::open(&preferences, &gate);
    access
        .set_access(ENTRY, CONTEXT, true, "egress card")
        .unwrap();
    let file = access.file_path().to_path_buf();
    let kept = std::fs::read(&file).unwrap();
    std::fs::remove_file(&file).unwrap();
    std::fs::create_dir(&file).unwrap();
    assert!(
        access
            .set_access(ENTRY, CONTEXT, false, "agent tab")
            .is_err()
    );
    drop(access);

    // Two restarts with the file still unwritable: off both times, marker kept.
    for _ in 0..2 {
        let gate = AgentAccess::new();
        let again = Access::open(&preferences, &gate);
        assert!(!can_read(&gate).await);
        assert!(again.save_pending());
        assert_eq!(again.pending_revocations().len(), 1);
    }

    // Once the file can be written the marker is cleared, and only then.
    std::fs::remove_dir(&file).unwrap();
    std::fs::write(&file, &kept).unwrap();
    let gate = AgentAccess::new();
    let access = Access::open(&preferences, &gate);
    assert!(!can_read(&gate).await);
    assert!(access.pending_revocations().is_empty());
}

#[tokio::test]
async fn a_marker_that_cannot_be_read_grants_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let preferences = dir.path().join("preferences.json");
    let gate = AgentAccess::new();
    let access = Access::open(&preferences, &gate);
    access
        .set_access(ENTRY, CONTEXT, true, "egress card")
        .unwrap();
    drop(access);
    std::fs::write(dir.path().join("agent-pending.json"), "{ not json").unwrap();
    let gate = AgentAccess::new();
    let again = Access::open(&preferences, &gate);
    assert!(!can_read(&gate).await, "what can't be known is off");
    assert!(again.grants().is_empty());
}
