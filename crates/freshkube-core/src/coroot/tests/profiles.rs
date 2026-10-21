use super::*;

fn profile_view(flamegraph: serde_json::Value, diff: bool) -> serde_json::Value {
    serde_json::json!({"context":{},"data":{
        "status":"ok","message":"OK",
        "services":[{"name":"api","linked":true}],
        "profiles":[{"type":"go:profile_cpu:nanoseconds","name":"CPU"},{"type":"go:heap_inuse_space:bytes","name":"Memory (in-use bytes)"}],
        "profile":{"type":"go:profile_cpu:nanoseconds","diff":diff,"flamegraph":flamegraph},
        "chart":null,"instances":["api-0","api-1"]}})
}

#[tokio::test]
async fn a_profile_is_flattened_with_narrow_frames_left_out_and_changes_compared() {
    // Previous window: main 60, gc 40. Current: main 90, gc 10.
    let graph = serde_json::json!({"name":"total","total":200,"self":0,"comp":100,"children":[
        {"name":"gc","total":50,"self":50,"comp":10,"children":null},
        {"name":"main","total":150,"self":50,"comp":90,"children":[
            {"name":"handle","total":100,"self":100,"comp":70,"children":[]},
            {"name":"tiny","total":0,"self":0,"comp":0,"children":[]}]}]});
    let server = server(vec![(200, profile_view(graph, true))]).await;
    let provider = Provider::new(&server.url, Credentials::None).unwrap();
    let app = AppId::new("c:prod:Deployment:api");
    let query = ProfileQuery {
        kind: Some("go:profile_cpu:nanoseconds".into()),
        compare: true,
        instance: None,
    };
    let profiling = provider
        .profiling(&source(&provider), range(), &app, &query)
        .await
        .unwrap();
    assert_eq!(profiling.kinds.len(), 2);
    assert_eq!(profiling.instances, vec!["api-0", "api-1"]);
    let graph = profiling.graph.unwrap();
    assert!(graph.compared);
    assert_eq!(graph.omitted, 1);
    let names: Vec<_> = graph.frames.iter().map(|f| f.name.as_str()).collect();
    assert_eq!(names, vec!["total", "gc", "main", "handle"]);
    let main = &graph.frames[2];
    assert_eq!((main.parent, main.depth), (Some(0), 1));
    assert!((main.x - 0.25).abs() < 1e-9 && (main.width - 0.75).abs() < 1e-9);
    assert!((main.change.unwrap() - 30.).abs() < 1e-9);
    assert!((graph.frames[1].change.unwrap() + 30.).abs() < 1e-9);
    assert_eq!(
        ProfileKind::unit("go:heap_inuse_space:bytes"),
        ProfileUnit::Bytes
    );
}

#[test]
fn a_deep_profile_parses_and_a_runaway_one_is_refused() {
    let deep = |levels: usize| {
        let mut body = String::from(
            r#"{"context":{},"data":{"status":"ok","profile":{"type":"t","flamegraph":"#,
        );
        for ix in 0..levels {
            body.push_str(&format!(
                r#"{{"name":"f{ix}","total":10,"self":0,"comp":0,"children":["#
            ));
        }
        body.push_str(&"]}".repeat(levels));
        body.push_str("}}}");
        body.into_bytes()
    };
    let profiling = profiling::decode(deep(1_000)).unwrap();
    let graph = profiling.graph.unwrap();
    assert_eq!(graph.frames.len(), 1_000);
    assert_eq!(graph.frames[999].depth, 999);
    assert_eq!(
        profiling::decode(deep(3_000)).unwrap_err(),
        ReadError::Limit
    );
    assert_eq!(
        profiling::decode(b"{\"data\":".to_vec()).unwrap_err(),
        ReadError::InvalidResponse
    );
}
