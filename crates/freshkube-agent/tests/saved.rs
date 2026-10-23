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
