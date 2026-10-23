use super::*;

#[tokio::test]
async fn markers_read_what_they_may_and_name_what_was_refused() {
    let at = "2023-11-14T22:00:00Z";
    let (client, seen) = fake(move |_, path, _| match path {
        "/apis/apps/v1/replicasets" => (403, status(403, "replicasets is forbidden")),
        "/api/v1/nodes" => (
            200,
            json!({"kind": "NodeList", "apiVersion": "v1", "metadata": {}, "items": [{
                "metadata": {"name": "worker-2"},
                "status": {"conditions": [
                    {"type": "Ready", "status": "False", "lastTransitionTime": at}
                ]},
            }]})
            .to_string(),
        ),
        "/api/v1/events" => (
            200,
            json!({"kind": "EventList", "apiVersion": "v1", "metadata": {}, "items": [{
                "metadata": {"name": "cp-1.1", "namespace": "default"},
                "involvedObject": {"kind": "Node", "name": "cp-1"},
                "reason": "Rebooted",
                "lastTimestamp": at,
            }]})
            .to_string(),
        ),
        _ => (404, status(404, "not found")),
    });
    let read = markers::read_markers(client, 0).await;
    let labels: Vec<_> = read.markers.iter().map(|m| m.label.as_str()).collect();
    assert_eq!(labels, ["worker-2 NotReady", "cp-1 rebooted"]);
    assert_eq!(read.unavailable, ["Can't list replicasets: forbidden"]);
    let seen = seen.lock().unwrap();
    assert!(
        seen.iter().all(|request| request.starts_with("GET ")),
        "{seen:?}"
    );
    assert!(
        seen.iter()
            .any(|request| request.contains("/api/v1/events?")
                && request.contains("fieldSelector=involvedObject.kind%3DNode")),
        "{seen:?}"
    );
}

/// A part that fails any other way reads "request failed", and a silent API server "timed out".
#[tokio::test(start_paused = true)]
async fn a_failing_or_silent_marker_read_keeps_its_words() {
    let (client, _) = fake(|_, path, _| match path {
        "/api/v1/nodes" => (500, status(500, "boom")),
        "/api/v1/events" => (0, String::new()),
        _ => (403, status(403, "forbidden")),
    });
    let read = markers::read_markers(client, 0).await;
    assert!(read.markers.is_empty());
    assert_eq!(
        read.unavailable,
        [
            "Can't list replicasets: forbidden",
            "Can't list nodes: request failed",
            "Can't list events: timed out"
        ]
    );
}
