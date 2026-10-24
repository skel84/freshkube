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
async fn safety_g08_a_missing_file_grants_nothing_and_an_unparsable_one_grants_nothing_and_logs_once()
 {
    capture::install();
    let dir = tempfile::tempdir().unwrap();
    let preferences = dir.path().join("preferences.json");
    let gate = AgentAccess::new();
    Access::open(&preferences, &gate);
    assert!(gate.granted().is_empty());

    let file = preferences.with_file_name("agent.json");
    std::fs::write(&file, "{ not json").unwrap();
    let gate = AgentAccess::new();
    let access = Access::open(&preferences, &gate);
    assert!(gate.granted().is_empty() && access.grants().is_empty());
    assert!(!can_read(&gate).await);
    // The capture is shared by every test in this binary, so count only the
    // warnings that name this test's own file.
    let said = capture::captured()
        .lines()
        .filter(|line| {
            line.contains("agent access is off everywhere")
                && line.contains(&*file.to_string_lossy())
        })
        .count();
    assert_eq!(said, 1, "said once");
}

/// A file with a grant in it, then damaged three ways: cut short, not JSON,
/// and written by a newer build. None of them grants anything, and the
/// newer one is never written over. The revocation marker beside a granting
/// file, corrupt or newer, holds every entry off too.
#[tokio::test]
async fn safety_g08_a_truncated_corrupt_or_newer_file_grants_nothing_and_a_newer_one_is_not_written()
 {
    let granted = tempfile::tempdir().unwrap();
    let source = granted.path().join("preferences.json");
    let gate = AgentAccess::new();
    let access = Access::open(&source, &gate);
    access.set_access(ENTRY, CONTEXT, true, "settings").unwrap();
    assert!(
        can_read(&gate).await,
        "the grant is real before it is damaged"
    );
    let whole = std::fs::read_to_string(access.file_path()).unwrap();
    let mut later: serde_json::Value = serde_json::from_str(&whole).unwrap();
    later["version"] = json!(2);
    let cases = [
        ("truncated", whole[..whole.len() / 2].to_owned()),
        ("corrupt", "{ not json".to_owned()),
        ("newer", later.to_string()),
    ];
    for (name, text) in cases {
        let dir = tempfile::tempdir().unwrap();
        let preferences = dir.path().join("preferences.json");
        let file = preferences.with_file_name("agent.json");
        std::fs::write(&file, &text).unwrap();
        let gate = AgentAccess::new();
        let access = Access::open(&preferences, &gate);
        assert!(
            gate.granted().is_empty() && access.grants().is_empty(),
            "{name}"
        );
        assert!(!can_read(&gate).await, "{name} grants nothing");

        let turned_on = access.set_access(ENTRY, CONTEXT, true, "settings");
        if name == "newer" {
            assert!(turned_on.is_err(), "a newer file is not written");
            assert!(!can_read(&gate).await, "and nothing is on");
            assert_eq!(std::fs::read_to_string(&file).unwrap(), text);
        } else {
            // The damaged file is kept, then replaced by the person's choice.
            turned_on.unwrap();
            let kept = std::fs::read_dir(dir.path())
                .unwrap()
                .filter_map(Result::ok)
                .filter(|e| {
                    e.file_name()
                        .to_string_lossy()
                        .starts_with("agent.corrupt-")
                })
                .map(|e| std::fs::read_to_string(e.path()).unwrap())
                .collect::<Vec<_>>();
            assert_eq!(kept, [text], "{name} is kept");
        }
    }
}

#[tokio::test]
async fn safety_g08_a_corrupt_or_newer_marker_beside_a_grant_grants_nothing() {
    capture::install();
    let granted = tempfile::tempdir().unwrap();
    let gate = AgentAccess::new();
    let access = Access::open(&granted.path().join("preferences.json"), &gate);
    access.set_access(ENTRY, CONTEXT, true, "settings").unwrap();
    let grant = std::fs::read_to_string(access.file_path()).unwrap();
    for (name, marker) in [
        ("corrupt", "{ not json"),
        ("truncated", "{\"revoked\":[{\"entry\":"),
        ("newer", "{\"version\":2,\"revoked\":[]}"),
    ] {
        let dir = tempfile::tempdir().unwrap();
        let preferences = dir.path().join("preferences.json");
        std::fs::write(preferences.with_file_name("agent.json"), &grant).unwrap();
        std::fs::write(preferences.with_file_name("agent-pending.json"), marker).unwrap();
        let gate = AgentAccess::new();
        let access = Access::open(&preferences, &gate);
        assert!(gate.granted().is_empty(), "{name}: the gate is off");
        assert!(access.grants().is_empty(), "{name}: nothing is held");
        assert!(!can_read(&gate).await, "{name}");
        assert!(!access.held_off().is_empty(), "{name}: the dock says why");
    }
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
async fn safety_g08_turning_access_off_fails_closed_even_when_the_save_or_the_audit_fails() {
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

#[tokio::test]
async fn when_neither_the_file_nor_the_marker_can_be_written_it_says_so() {
    let dir = tempfile::tempdir().unwrap();
    let preferences = dir.path().join("preferences.json");
    let gate = AgentAccess::new();
    let access = Access::open(&preferences, &gate);
    access
        .set_access(ENTRY, CONTEXT, true, "egress card")
        .unwrap();
    let file = access.file_path().to_path_buf();
    let marker = dir.path().join("agent-pending.json");
    assert!(access.revocation_remembered());

    // A directory sits at both files, as a folder that can't be written
    // would stop both.
    std::fs::remove_file(&file).unwrap();
    std::fs::create_dir(&file).unwrap();
    std::fs::create_dir(&marker).unwrap();
    assert!(
        access
            .set_access(ENTRY, CONTEXT, false, "agent tab")
            .is_err()
    );
    assert!(
        !can_read(&gate).await,
        "off now, whatever happens to the files"
    );
    assert!(access.save_pending());
    assert!(
        !access.revocation_remembered(),
        "the marker could not be written, so access can come back at the next start"
    );

    // Once the file can be written, the save clears the worry.
    std::fs::remove_dir(&file).unwrap();
    std::fs::remove_dir(&marker).unwrap();
    access.retry_save().unwrap();
    assert!(access.revocation_remembered());
    assert!(!access.save_pending());
}

#[tokio::test]
async fn the_marker_is_written_before_the_file_is_saved() {
    // A crash between the revoke and the save leaves the marker behind: the
    // file is blocked here, so the marker is all there is.
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
    let _ = access.set_access(ENTRY, CONTEXT, false, "agent tab");
    assert!(dir.path().join("agent-pending.json").exists());
    assert!(access.revocation_remembered());
    drop(access);
    std::fs::remove_dir(&file).unwrap();
    std::fs::write(&file, kept).unwrap();
    let gate = AgentAccess::new();
    let _again = Access::open(&preferences, &gate);
    assert!(!can_read(&gate).await);
}

#[test]
fn the_chosen_model_is_saved_beside_the_grants_and_read_back() {
    let dir = tempfile::tempdir().unwrap();
    let preferences = dir.path().join("freshkube.yaml");
    let access = Access::open(&preferences, &AgentAccess::default());
    assert_eq!(access.model(), None);
    access.set_model(Some("hosted/large".into())).unwrap();
    let reopened = Access::open(&preferences, &AgentAccess::default());
    assert_eq!(reopened.model().as_deref(), Some("hosted/large"));
    assert!(!preferences.exists());
}
