use super::*;

#[test]
fn the_workspace_rules_still_apply_and_name_the_entry() {
    let text = with_cluster("  - id: cluster-a\n    role: cicd\n    context: other\n");
    assert_eq!(
        refused(&text).to_string(),
        "the id “cluster-a” is used twice"
    );
    let text = with_cluster("    talos_context: cluster-a\n");
    assert_eq!(
        refused(&text),
        Invalid::Workspace(workspace::Invalid::TalosContextWithoutConfig(
            "cluster-a".into()
        ))
    );
    let text = "version: 1\nkubeconfig: kube/config\n";
    assert_eq!(
        refused(text),
        Invalid::Workspace(workspace::Invalid::RelativePath("the workspace".into()))
    );
}

#[test]
fn the_workspace_json_shape_is_unchanged_by_the_new_entry_keys() {
    let entry: Entry = serde_json::from_str(
        r#"{"id":"a","role":"core","context":"a","results":{"address":"https://r.example.test"}}"#,
    )
    .unwrap();
    assert!(entry.results.is_some());
    let plain = serde_json::to_value(Entry::new("a", workspace::Role::Core, "a")).unwrap();
    assert_eq!(
        plain,
        serde_json::json!({"id": "a", "role": "core", "context": "a"})
    );
}
