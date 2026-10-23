use super::*;

/// The `node` and `nodes` metadata `with_nodes` puts on a request.
fn targeting(client: &TalosClient) -> (Option<String>, Vec<String>) {
    let request = client.with_nodes(Request::new(()));
    let metadata = request.metadata();
    let node = metadata
        .get("node")
        .map(|value| value.to_str().unwrap().to_string());
    let nodes = metadata
        .get_all("nodes")
        .iter()
        .map(|value| value.to_str().unwrap().to_string())
        .collect();
    (node, nodes)
}

fn targeting_one(node: &str) -> (Option<String>, Vec<String>) {
    targeting(&create_test_client(vec![node.to_string()]))
}

#[tokio::test]
async fn targets_an_ipv4_host_without_its_port() {
    assert_eq!(
        targeting_one("10.5.0.2:50000"),
        (Some("10.5.0.2".to_string()), vec![])
    );
}

#[tokio::test]
async fn targets_a_bare_ipv6_address_whole() {
    assert_eq!(
        targeting_one("2001:db8::5"),
        (Some("2001:db8::5".to_string()), vec![])
    );
}

#[tokio::test]
async fn targets_a_bracketed_ipv6_address_without_its_port() {
    assert_eq!(
        targeting_one("[2001:db8::5]:50000"),
        (Some("2001:db8::5".to_string()), vec![])
    );
}

#[tokio::test]
async fn targets_a_hostname_as_given() {
    assert_eq!(
        targeting_one("node1.example.com"),
        (Some("node1.example.com".to_string()), vec![])
    );
}

#[tokio::test]
async fn blank_targets_leave_the_endpoint_to_answer() {
    for node in ["", " ", ":50000"] {
        assert_eq!(targeting_one(node), (None, vec![]), "{node:?}");
    }
}

#[tokio::test]
async fn a_mutation_without_acknowledgements_names_its_target() {
    let client = create_test_client(vec!["[2001:db8::5]:50000".to_string()]);
    let Err(TalosError::Grpc(status)) =
        client.decode_reboot(crate::proto::machine::RebootResponse { messages: vec![] })
    else {
        panic!("expected an error");
    };
    assert_eq!(status.code(), tonic::Code::DataLoss);
    assert_eq!(
        status.message(),
        "reboot on 2001:db8::5: no acknowledgements returned"
    );
}

#[tokio::test]
async fn loopback_targets_leave_the_endpoint_to_answer() {
    for node in [
        "127.0.0.1:50000",
        "localhost:50000",
        "localhost",
        "[::1]:50000",
    ] {
        assert_eq!(targeting_one(node), (None, vec![]), "{node}");
    }
}

#[tokio::test]
async fn loopback_targets_are_dropped_from_a_list() {
    let client = create_test_client(vec![
        "127.0.0.1:50000".to_string(),
        "node1".to_string(),
        "[2001:db8::6]:50000".to_string(),
    ]);
    assert_eq!(
        targeting(&client),
        (None, vec!["node1".to_string(), "2001:db8::6".to_string()])
    );
}

#[tokio::test]
async fn mutations_refuse_when_no_node_is_targeted() {
    for nodes in [
        vec![],
        vec!["127.0.0.1:50000".to_string()],
        vec![String::new()],
        vec![" ".to_string()],
        vec![":50000".to_string()],
    ] {
        let client = create_test_client(nodes);
        let refused = |result: Result<(), TalosError>| match result {
            Err(TalosError::Grpc(status)) => {
                assert_eq!(status.code(), tonic::Code::FailedPrecondition);
                status.message().to_string()
            }
            other => panic!("expected a refusal, got {other:?}"),
        };
        assert!(refused(client.shutdown(false).await.map(drop)).contains("shutdown"));
        assert!(refused(client.reboot().await.map(drop)).contains("reboot"));
        assert!(
            refused(client.service_restart("kubelet").await.map(drop))
                .contains("restart service kubelet")
        );
        assert!(
            refused(
                client
                    .apply_configuration("", ApplyMode::Auto, true)
                    .await
                    .map(drop)
            )
            .contains("apply configuration")
        );
    }
}

#[tokio::test]
async fn a_targeted_mutation_carries_its_node() {
    let client = create_test_client(vec!["[2001:db8::5]:50000".to_string()]);
    let request = client
        .with_mutation_targets(Request::new(()), "reboot")
        .unwrap();
    assert_eq!(request.metadata().get("node").unwrap(), "2001:db8::5");
}

#[tokio::test]
async fn configured_nodes_are_sent_as_given() {
    // talosctl sends `nodes` as given; an endpoint, VIP or not, only names
    // where the connection goes, so no endpoint removes a node.
    let client = create_test_client(vec![
        "cluster.example.com".to_string(),
        "kubec01".to_string(),
        "kubew01:50000".to_string(),
    ]);
    assert_eq!(
        targeting(&client),
        (
            None,
            vec![
                "cluster.example.com".to_string(),
                "kubec01".to_string(),
                "kubew01".to_string()
            ]
        )
    );
}

#[test]
fn etcd_status_targets_listed_nodes_by_host() {
    let request = with_listed_nodes(
        Request::new(()),
        &[
            "10.5.0.2:50000".to_string(),
            "2001:db8::5".to_string(),
            "[2001:db8::6]:50000".to_string(),
            String::new(),
        ],
    );
    let nodes: Vec<&str> = request
        .metadata()
        .get_all("nodes")
        .iter()
        .map(|value| value.to_str().unwrap())
        .collect();
    assert_eq!(nodes, ["10.5.0.2", "2001:db8::5", "2001:db8::6"]);
    assert!(request.metadata().get("node").is_none());
    let request = with_listed_nodes(Request::new(()), &[]);
    assert!(request.metadata().get("nodes").is_none());
}

#[test]
fn etcd_member_address_keeps_ipv6_whole() {
    let member = |url: &str| EtcdMemberInfo {
        id: 1,
        hostname: "cp1".to_string(),
        peer_urls: vec![url.to_string()],
        client_urls: vec![],
        is_learner: false,
    };
    assert_eq!(
        member("https://10.5.0.2:2380").ip_address().as_deref(),
        Some("10.5.0.2")
    );
    assert_eq!(
        member("https://[2001:db8::5]:2380").ip_address().as_deref(),
        Some("2001:db8::5")
    );
}

// =========================================================================
// gRPC Metadata Format Tests
//
// These tests verify the correct gRPC metadata format for node targeting.
// This was the root cause of VIP + node targeting failures:
//   - WRONG:   insert("node", "node1,node2,node3") - single comma-separated value
//   - CORRECT: append("nodes", "node1"), append("nodes", "node2") - multiple values
//
// Talos Go client behavior:
//   - Single node:   md.Set("node", node)        -> one value
//   - Multiple nodes: md.Set("nodes", nodes...)  -> multiple values for same key
// =========================================================================

#[tokio::test]
async fn test_metadata_single_node_uses_node_header() {
    // Single node should use "node" (singular) header
    let client = create_test_client(vec!["node1".to_string()]);

    let request: Request<()> = Request::new(());
    let request = client.with_nodes(request);

    // Should have "node" header (singular), not "nodes"
    let metadata = request.metadata();
    assert!(
        metadata.get("node").is_some(),
        "Single node should use 'node' header"
    );
    assert!(
        metadata.get("nodes").is_none(),
        "Single node should NOT use 'nodes' header"
    );
    assert_eq!(metadata.get("node").unwrap(), "node1");
}

#[tokio::test]
async fn test_metadata_multiple_nodes_uses_nodes_header() {
    // Multiple nodes should use "nodes" (plural) header
    let client = create_test_client(vec![
        "node1".to_string(),
        "node2".to_string(),
        "node3".to_string(),
    ]);

    let request: Request<()> = Request::new(());
    let request = client.with_nodes(request);

    // Should have "nodes" header (plural), not "node"
    let metadata = request.metadata();
    assert!(
        metadata.get("node").is_none(),
        "Multiple nodes should NOT use 'node' header"
    );
    assert!(
        metadata.get("nodes").is_some(),
        "Multiple nodes should use 'nodes' header"
    );
}

#[tokio::test]
async fn test_metadata_multiple_nodes_are_separate_values_not_comma_separated() {
    // CRITICAL: This is the root cause test
    // Multiple nodes must be separate metadata values, NOT comma-separated
    // tonic's append() creates multiple values for the same key
    // This matches Talos Go client's md.Set("nodes", nodes...) behavior
    let client = create_test_client(vec![
        "node1".to_string(),
        "node2".to_string(),
        "node3".to_string(),
    ]);

    let request: Request<()> = Request::new(());
    let request = client.with_nodes(request);

    let metadata = request.metadata();

    // Get all values for "nodes" header
    let nodes_values: Vec<&str> = metadata
        .get_all("nodes")
        .iter()
        .filter_map(|v| v.to_str().ok())
        .collect();

    // Should have 3 separate values, not 1 comma-separated value
    assert_eq!(
        nodes_values.len(),
        3,
        "Expected 3 separate metadata values, got {}: {:?}",
        nodes_values.len(),
        nodes_values
    );

    // Each value should be a single node, not comma-separated
    for value in &nodes_values {
        assert!(
            !value.contains(','),
            "Node metadata values should NOT be comma-separated: {}",
            value
        );
    }

    // Verify the actual values
    assert!(nodes_values.contains(&"node1"));
    assert!(nodes_values.contains(&"node2"));
    assert!(nodes_values.contains(&"node3"));
}

#[tokio::test]
async fn test_metadata_empty_nodes_no_header() {
    // Empty nodes should not add any header
    let client = create_test_client(vec![]);

    let request: Request<()> = Request::new(());
    let request = client.with_nodes(request);

    let metadata = request.metadata();
    assert!(
        metadata.get("node").is_none(),
        "Empty nodes should not add 'node' header"
    );
    assert!(
        metadata.get("nodes").is_none(),
        "Empty nodes should not add 'nodes' header"
    );
}

#[tokio::test]
async fn test_metadata_two_nodes_uses_nodes_header() {
    // Two nodes should use "nodes" (plural) header, not "node"
    // This is a boundary condition - even 2 nodes should use the plural form
    let client = create_test_client(vec!["node1".to_string(), "node2".to_string()]);

    let request: Request<()> = Request::new(());
    let request = client.with_nodes(request);

    let metadata = request.metadata();
    assert!(
        metadata.get("node").is_none(),
        "Two nodes should NOT use 'node' header"
    );
    assert!(
        metadata.get("nodes").is_some(),
        "Two nodes should use 'nodes' header"
    );

    let nodes_values: Vec<&str> = metadata
        .get_all("nodes")
        .iter()
        .filter_map(|v| v.to_str().ok())
        .collect();
    assert_eq!(nodes_values.len(), 2);
}

#[tokio::test]
async fn test_metadata_preserves_node_names_without_ports() {
    // Port stripping happens before metadata is set
    let client = create_test_client(vec!["node1:50000".to_string(), "node2:50000".to_string()]);

    let request: Request<()> = Request::new(());
    let request = client.with_nodes(request);

    let metadata = request.metadata();
    let nodes_values: Vec<&str> = metadata
        .get_all("nodes")
        .iter()
        .filter_map(|v| v.to_str().ok())
        .collect();

    // Ports should be stripped
    assert!(nodes_values.contains(&"node1"));
    assert!(nodes_values.contains(&"node2"));
    assert!(!nodes_values.iter().any(|v| v.contains(':')));
}
