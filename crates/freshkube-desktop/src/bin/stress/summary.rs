//! Typed synthetic summary responses.
use super::*;

/// Typed cache-backed answers, separate from the browsing Table API.
pub(super) fn summary_list(world: &World, path: &str) -> Option<Response<Body>> {
    let (kind, items) = match path {
        "/api/v1/pods" => {
            let pods = world.pods.lock().expect("pods");
            let items = pods.rows.iter().enumerate().map(|(ix,pod)| json!({"metadata":{"name":pod.name,"namespace":pod.namespace,"uid":pod.uid,"creationTimestamp":"2026-09-27T00:00:00Z"},"spec":{"nodeName":format!("worker-{}",ix%100),"containers":[{"name":"app","image":"example:1"}]},"status":{"phase":"Running","containerStatuses":[{"name":"app","image":"example:1","imageID":"example","ready":true,"restartCount":pod.restarts,"state":if ix%20==0 {json!({"waiting":{"reason":"CrashLoopBackOff"}})} else {json!({"running":{}})}}]}})).collect();
            ("PodList",items)
        }
        "/apis/apps/v1/deployments" => ("DeploymentList",(0..if matches!(world.scenario,Scenario::Summary) {2000} else {20}).map(|ix|json!({"metadata":{"name":format!("deploy-{ix}"),"namespace":format!("ns-{:02}",ix%NAMESPACES)},"status":{"replicas":3,"readyReplicas":if ix%10==0 {0} else {3},"availableReplicas":if ix%10==0 {0} else {3}}})).collect()),
        "/apis/apps/v1/statefulsets" => ("StatefulSetList",Vec::new()),
        "/apis/apps/v1/daemonsets" => ("DaemonSetList",Vec::new()),
        "/api/v1/nodes" => ("NodeList",(0..100).map(|ix|json!({"metadata":{"name":format!("worker-{ix}")},"status":{"conditions":[{"type":"Ready","status":if ix%10==0 {"False"} else {"True"}}]}})).collect()),
        "/api/v1/namespaces" => ("NamespaceList",(0..NAMESPACES).map(|ix|json!({"metadata":{"name":format!("ns-{ix:02}")}})).collect()),
        "/api/v1/persistentvolumeclaims" => ("PersistentVolumeClaimList",Vec::new()),
        "/api/v1/persistentvolumes" => ("PersistentVolumeList",Vec::new()),
        "/api/v1/events" => {
            let now=Utc::now().to_rfc3339();
            ("EventList",(0..if matches!(world.scenario,Scenario::Summary) {5000} else {0}).map(|ix|json!({"metadata":{"name":format!("warning-{ix}"),"namespace":format!("ns-{:02}",ix%NAMESPACES)},"involvedObject":{"kind":"Pod","name":format!("pod-{ix}"),"namespace":format!("ns-{:02}",ix%NAMESPACES)},"type":"Warning","reason":"BackOff","message":"Example warning","lastTimestamp":now})).collect())
        }
        _=>return None,
    };
    Some(json_response(
        json!({"apiVersion":"v1","kind":kind,"metadata":{"resourceVersion":"1"},"items":items}),
    ))
}

