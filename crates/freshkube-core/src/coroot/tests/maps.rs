use super::*;

#[tokio::test]
async fn maps_keep_unknown_links_and_empty_maps_are_successful() {
    let server=server(vec![(200,serde_json::json!({"map":[
        {"id":"c:ns:Deployment:a","status":"ok","upstreams":[{"id":"c:ns:Deployment:b","status":"unknown"}]},
        {"id":"c:ns:Deployment:b","status":"unknown"}
    ]})),(200,serde_json::json!({"map":[]}))]).await;
    let provider = Provider::new(&server.url, Credentials::Session("fake-session".into())).unwrap();
    let source = source(&provider);
    let map = provider.service_map(&source, range()).await.unwrap();
    assert_eq!(map.nodes.len(), 2);
    assert_eq!(map.edges[0].status, Status::Unknown);
    assert!(map.edges[0].rps.is_none());
    assert!(
        provider
            .service_map(&source, range())
            .await
            .unwrap()
            .nodes
            .is_empty()
    );
}

#[test]
fn a_cluster_sized_map_is_accepted_and_a_runaway_one_is_not() {
    let node = |ix: usize| MapNode {
        id: AppId::new(format!("c:ns:Deployment:app-{ix}")),
        cluster: String::new(),
        category: String::new(),
        status: Status::Ok,
        custom: false,
        labels: Default::default(),
        indicators: Default::default(),
        distance: None,
    };
    let nodes: Vec<_> = (0..400).map(node).collect();
    let edges: Vec<_> = (0..1_200)
        .map(|ix| MapEdge {
            from: nodes[ix % 400].id.clone(),
            to: nodes[(ix / 400 + ix + 1) % 400].id.clone(),
            status: Status::Ok,
            rps: None,
            latency_seconds: None,
            sent_bytes_per_second: None,
            received_bytes_per_second: None,
            issue: String::new(),
        })
        .collect();
    let map = ServiceMap { nodes, edges };
    assert_eq!(limits::map(&map), Ok(()));
    let runaway = ServiceMap {
        nodes: (0..2_001).map(node).collect(),
        edges: Vec::new(),
    };
    assert_eq!(limits::map(&runaway), Err(ReadError::Limit));
}
