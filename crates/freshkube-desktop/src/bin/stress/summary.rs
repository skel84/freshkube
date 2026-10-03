//! Typed and Table views of one synthetic world, with snapshot pagination and
//! bounded watch replay. Each watch observes the same mutations.
use super::*;
use std::collections::{BTreeMap, VecDeque};

const REPLAY_LIMIT: usize = 100_000;
const SNAPSHOT_LIMIT: usize = 8;

#[derive(Default)]
pub(super) struct State {
    inner: Mutex<History>,
}
#[derive(Default)]
struct History {
    requests: BTreeMap<String, usize>,
    snapshots: BTreeMap<u64, Captured>,
    next_snapshot: u64,
    changes: VecDeque<Change>,
    expired_once: bool,
    reported: bool,
}
struct Captured {
    version: u64,
    rows: Arc<Vec<Pod>>,
}
#[derive(Clone)]
struct Change {
    kind: &'static str,
    pod: Pod,
    version: u64,
}

impl State {
    pub(super) fn count(&self, path: &str, watching: bool, table: bool) {
        let operation = if path == "/version" {
            "version"
        } else if watching {
            "watch"
        } else {
            "list-page"
        };
        let representation = if table { "Table" } else { "Object" };
        let key = format!("{path} {operation} {representation}");
        let mut history = self.inner.lock().unwrap();
        let count = history.requests.entry(key.clone()).or_default();
        *count += 1;
        // Log cumulative counters as reads start too: native quit can bypass
        // main's return, and slow window startup can outlast the reporter.
        eprintln!("stress server requests {key}={count}");
    }
    pub(super) fn report(&self) {
        let mut history = self.inner.lock().unwrap();
        if history.reported {
            return;
        }
        history.reported = true;
        for (key, count) in &history.requests {
            eprintln!("stress server requests {key}={count}");
        }
        eprintln!(
            "stress server retained replay={} snapshots={}",
            history.changes.len(),
            history.snapshots.len()
        );
    }
}

pub(super) fn start(world: Arc<World>) {
    let reporting = world.clone();
    let seconds = std::env::var("FRESHKUBE_STRESS_SECONDS")
        .ok()
        .and_then(|s| s.parse::<u64>().ok())
        .unwrap_or(30);
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_secs(seconds.saturating_sub(1))).await;
        reporting.summary.report();
    });
    let rate = match world.scenario {
        Scenario::Burst { rate, .. } | Scenario::SummaryBurst { rate, .. } => rate,
        _ => 0,
    };
    if rate == 0 {
        return;
    }
    tokio::spawn(async move {
        let started = Instant::now();
        let mut sent = 0u64;
        let mut tick = tokio::time::interval(TICK);
        loop {
            tick.tick().await;
            let due = (started.elapsed().as_secs_f64() * f64::from(rate)) as u64;
            let mut pods = world.pods.lock().unwrap();
            let mut history = world.summary.inner.lock().unwrap();
            for _ in sent..due.min(sent + 2000) {
                sent += 1;
                if pods.rows.is_empty() {
                    break;
                }
                let ix = pods.random() as usize % pods.rows.len();
                if sent.is_multiple_of(10) {
                    pods.version += 1;
                    let mut gone = pods.rows[ix].clone();
                    gone.version = pods.version;
                    history.changes.push_back(Change {
                        kind: "DELETED",
                        version: gone.version,
                        pod: gone,
                    });
                    let new = pods.pod();
                    pods.rows[ix] = new.clone();
                    history.changes.push_back(Change {
                        kind: "ADDED",
                        version: new.version,
                        pod: new,
                    });
                } else {
                    pods.version += 1;
                    let version = pods.version;
                    let pod = &mut pods.rows[ix];
                    pod.version = version;
                    if pod.status == "Running" {
                        pod.status = "CrashLoopBackOff";
                        pod.restarts += 1;
                    } else {
                        pod.status = "Running";
                    }
                    history.changes.push_back(Change {
                        kind: "MODIFIED",
                        version,
                        pod: pod.clone(),
                    });
                }
            }
            while history.changes.len() > REPLAY_LIMIT {
                history.changes.pop_front();
            }
        }
    });
}

pub(super) fn supported(path: &str) -> bool {
    matches!(
        path,
        "/api/v1/pods"
            | "/api/v1/nodes"
            | "/api/v1/events"
            | "/api/v1/namespaces"
            | "/api/v1/persistentvolumeclaims"
            | "/api/v1/persistentvolumes"
            | "/apis/apps/v1/deployments"
            | "/apis/apps/v1/daemonsets"
            | "/apis/apps/v1/statefulsets"
    )
}

fn selected(pod: &Pod, namespace: Option<&str>, param: &dyn Fn(&str) -> Option<String>) -> bool {
    if namespace.is_some_and(|ns| pod.namespace != ns) {
        return false;
    }
    for term in param("fieldSelector")
        .unwrap_or_default()
        .split(',')
        .filter(|s| !s.is_empty())
    {
        let Some((key, value)) = term.split_once('=') else {
            return false;
        };
        let matches = match key {
            "metadata.name" => pod.name == value,
            "metadata.namespace" => pod.namespace == value,
            "spec.nodeName" => node_name(pod) == value,
            _ => false,
        };
        if !matches {
            return false;
        }
    }
    for term in param("labelSelector")
        .unwrap_or_default()
        .split(',')
        .filter(|s| !s.is_empty())
    {
        let Some((key, value)) = term.split_once('=') else {
            return false;
        };
        if key != "app" || pod.name.split('-').take(2).collect::<Vec<_>>().join("-") != value {
            return false;
        }
    }
    true
}
fn node_name(pod: &Pod) -> String {
    format!(
        "worker-{}",
        pod.uid
            .rsplit('-')
            .next()
            .and_then(|s| s.parse::<usize>().ok())
            .unwrap_or(0)
            % 100
    )
}

pub(super) fn pod_value(pod: &Pod) -> Value {
    let running = pod.status == "Running";
    json!({"apiVersion":"v1","kind":"Pod",
        "metadata":{"name":pod.name,"namespace":pod.namespace,"uid":pod.uid,"resourceVersion":pod.version.to_string(),
            "creationTimestamp":"2026-09-27T00:00:00Z","labels":{"app":pod.name.split('-').take(2).collect::<Vec<_>>().join("-")}},
        "spec":{"nodeName":node_name(pod),"containers":[{"name":"app","image":"example:1"}]},
        "status":{"phase":"Running","containerStatuses":[{"name":"app","image":"example:1","imageID":"example",
            "ready":running,"restartCount":pod.restarts,
            "state":if running {json!({"running":{}})} else {json!({"waiting":{"reason":pod.status}})}}]}})
}

pub(super) fn pods(
    world: &World,
    namespace: Option<&str>,
    param: &dyn Fn(&str) -> Option<String>,
    table: bool,
) -> Response<Body> {
    let limit = param("limit")
        .and_then(|s| s.parse::<usize>().ok())
        .unwrap_or(usize::MAX);
    let continuation = param("continue").filter(|s| !s.is_empty());
    let (id, from, version, rows) = if let Some(token) = continuation {
        let Some((id, from)) = token
            .split_once(':')
            .and_then(|(a, b)| Some((a.parse::<u64>().ok()?, b.parse::<usize>().ok()?)))
        else {
            return expired();
        };
        let history = world.summary.inner.lock().unwrap();
        let Some(snapshot) = history.snapshots.get(&id) else {
            return expired();
        };
        (id, from, snapshot.version, snapshot.rows.clone())
    } else {
        let pods = world.pods.lock().unwrap();
        let rows = Arc::new(
            pods.rows
                .iter()
                .filter(|pod| selected(pod, namespace, param))
                .cloned()
                .collect::<Vec<_>>(),
        );
        let version = pods.version;
        let mut history = world.summary.inner.lock().unwrap();
        history.next_snapshot += 1;
        let id = history.next_snapshot;
        if rows.len() > limit {
            history.snapshots.insert(
                id,
                Captured {
                    version,
                    rows: rows.clone(),
                },
            );
            while history.snapshots.len() > SNAPSHOT_LIMIT {
                history.snapshots.pop_first();
            }
        }
        (id, 0, version, rows)
    };
    if from > rows.len() {
        return expired();
    }
    let to = from.saturating_add(limit).min(rows.len());
    let next = if to < rows.len() {
        format!("{id}:{to}")
    } else {
        world.summary.inner.lock().unwrap().snapshots.remove(&id);
        String::new()
    };
    let page: Vec<Value> = rows[from..to]
        .iter()
        .map(|pod| if table { pod_row(pod) } else { pod_value(pod) })
        .collect();
    json_response(if table {
        json!({"kind":"Table","apiVersion":"meta.k8s.io/v1","metadata":{"resourceVersion":version.to_string(),"continue":next},"columnDefinitions":pod_columns(),"rows":page})
    } else {
        json!({"kind":"PodList","apiVersion":"v1","metadata":{"resourceVersion":version.to_string(),"continue":next},"items":page})
    })
}

fn gone() -> Value {
    json!({"kind":"Status","apiVersion":"v1","status":"Failure","reason":"Expired","code":410,"message":"synthetic watch history expired"})
}
fn expired() -> Response<Body> {
    let mut response = json_response(gone());
    *response.status_mut() = StatusCode::GONE;
    response
}

pub(super) fn watch(
    world: &Arc<World>,
    path: &str,
    param: &dyn Fn(&str) -> Option<String>,
    table: bool,
) -> Response<Body> {
    if path != "/api/v1/pods"
        || !matches!(
            world.scenario,
            Scenario::Burst { .. } | Scenario::SummaryBurst { .. }
        )
    {
        return idle_stream();
    }
    let mut version = param("resourceVersion")
        .and_then(|s| s.parse::<u64>().ok())
        .unwrap_or(0);
    let fields = param("fieldSelector");
    let labels = param("labelSelector");
    let (sender, receiver) = mpsc::channel(16);
    let world = world.clone();
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(TICK);
        loop {
            tokio::select! { _ = sender.closed() => return, _ = tick.tick() => {} }
            let (changes, expired) = {
                let mut history = world.summary.inner.lock().unwrap();
                let forced = matches!(world.scenario, Scenario::SummaryBurst { expire: true, .. })
                    && !history.expired_once
                    && world.started.elapsed() > Duration::from_secs(10);
                if forced {
                    history.expired_once = true;
                }
                let expired = forced
                    || history
                        .changes
                        .front()
                        .is_some_and(|change| change.version > version + 1);
                (
                    history
                        .changes
                        .iter()
                        .skip(
                            history
                                .changes
                                .partition_point(|change| change.version <= version),
                        )
                        .take(512)
                        .cloned()
                        .collect::<Vec<_>>(),
                    expired,
                )
            };
            if expired {
                let _ = sender
                    .send(Bytes::from(format!(
                        "{}\n",
                        json!({"type":"ERROR","object":gone()})
                    )))
                    .await;
                return;
            }
            let mut chunk = String::new();
            for change in changes {
                version = change.version;
                if !selected(&change.pod, None, &|name| match name {
                    "fieldSelector" => fields.clone(),
                    "labelSelector" => labels.clone(),
                    _ => None,
                }) {
                    continue;
                }
                let object = if table {
                    json!({"kind":"Table","apiVersion":"meta.k8s.io/v1","metadata":{"resourceVersion":version.to_string()},"columnDefinitions":null,"rows":[pod_row(&change.pod)]})
                } else {
                    pod_value(&change.pod)
                };
                chunk.push_str(&json!({"type":change.kind,"object":object}).to_string());
                chunk.push('\n');
            }
            if !chunk.is_empty() && sender.send(Bytes::from(chunk)).await.is_err() {
                return;
            }
        }
    });
    stream_response(receiver, "application/json")
}

pub(super) fn summary_list(
    world: &World,
    path: &str,
    param: &dyn Fn(&str) -> Option<String>,
) -> Option<Response<Body>> {
    if path == "/api/v1/pods" {
        return Some(pods(world, None, param, false));
    }
    let large = matches!(
        world.scenario,
        Scenario::Summary | Scenario::SummaryBurst { .. }
    );
    let meta = |name: String, namespace: Option<String>| json!({"name":name,"namespace":namespace,"uid":format!("synthetic-{path}-{name}"),"resourceVersion":"1"});
    let (kind, mut items): (&str,Vec<Value>) = match path {
        "/apis/apps/v1/deployments" => ("Deployment",(0..if large {2000} else {20}).map(|ix|json!({"metadata":meta(format!("deploy-{ix}"),Some(format!("ns-{:02}",ix%NAMESPACES))),"status":{"replicas":3,"readyReplicas":if ix%10==0 {0} else {3},"availableReplicas":if ix%10==0 {0} else {3}}})).collect()),
        "/apis/apps/v1/statefulsets" => ("StatefulSet",vec![]), "/apis/apps/v1/daemonsets" => ("DaemonSet",vec![]),
        "/api/v1/nodes" => ("Node",(0..100).map(|ix|json!({"metadata":meta(format!("worker-{ix}"),None),"status":{"conditions":[{"type":"Ready","status":if ix%10==0 {"False"} else {"True"}}]}})).collect()),
        "/api/v1/namespaces" => ("Namespace",(0..NAMESPACES).map(|ix|json!({"metadata":meta(format!("ns-{ix:02}"),None)})).collect()),
        "/api/v1/persistentvolumeclaims" => ("PersistentVolumeClaim",vec![]), "/api/v1/persistentvolumes" => ("PersistentVolume",vec![]),
        "/api/v1/events" => ("Event",(0..if large {5000} else {0}).map(|ix|json!({"metadata":meta(format!("warning-{ix}"),Some(format!("ns-{:02}",ix%NAMESPACES))),"involvedObject":{"kind":"Pod","name":format!("pod-{ix}"),"namespace":format!("ns-{:02}",ix%NAMESPACES)},"type":"Warning","reason":"BackOff","message":"Example warning","lastTimestamp":world.created.to_rfc3339()})).collect()),
        _ => return None,
    };
    for item in &mut items {
        item["apiVersion"] = if path.starts_with("/apis/apps/") {
            "apps/v1"
        } else {
            "v1"
        }
        .into();
        item["kind"] = kind.into();
    }
    let from = param("continue")
        .and_then(|s| s.parse::<usize>().ok())
        .unwrap_or(0);
    let limit = param("limit")
        .and_then(|s| s.parse::<usize>().ok())
        .unwrap_or(usize::MAX);
    let to = from.saturating_add(limit).min(items.len());
    let next = if to < items.len() {
        to.to_string()
    } else {
        String::new()
    };
    Some(json_response(
        json!({"apiVersion":if path.starts_with("/apis/apps/") {"apps/v1"} else {"v1"},"kind":format!("{kind}List"),"metadata":{"resourceVersion":"1","continue":next},"items":items[from.min(to)..to]}),
    ))
}

pub(super) fn decode(value: &str) -> String {
    let bytes = value.as_bytes();
    let mut out = Vec::new();
    let mut ix = 0;
    while ix < bytes.len() {
        if bytes[ix] == b'%'
            && ix + 2 < bytes.len()
            && let Ok(value) = u8::from_str_radix(&value[ix + 1..ix + 3], 16)
        {
            out.push(value);
            ix += 3;
        } else {
            out.push(if bytes[ix] == b'+' { b' ' } else { bytes[ix] });
            ix += 1;
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn value(response: Response<Body>) -> Value {
        serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes()).unwrap()
    }

    #[tokio::test]
    async fn paginated_lists_keep_one_snapshot_while_the_world_changes() {
        let world = World::new(Scenario::Table { pods: 3 });
        let first = value(pods(
            &world,
            None,
            &|name| (name == "limit").then(|| "1".into()),
            false,
        ))
        .await;
        let continuation = first["metadata"]["continue"].as_str().unwrap();
        let expected = world.pods.lock().unwrap().rows[1].clone();
        {
            let mut pods = world.pods.lock().unwrap();
            pods.version += 1;
            pods.rows[1].status = "NewState";
            pods.rows[1].version = pods.version;
        }
        let second = value(pods(
            &world,
            None,
            &|name| match name {
                "limit" => Some("1".into()),
                "continue" => Some(continuation.into()),
                _ => None,
            },
            false,
        ))
        .await;
        assert_eq!(
            first["metadata"]["resourceVersion"],
            second["metadata"]["resourceVersion"]
        );
        assert_eq!(second["items"][0], pod_value(&expected));
    }

    #[tokio::test]
    async fn typed_and_table_selectors_return_the_same_identity_and_status() {
        let world = World::new(Scenario::Table { pods: 10 });
        let params = |name: &str| match name {
            "fieldSelector" => Some("spec.nodeName=worker-1".into()),
            "labelSelector" => Some("app=app-1".into()),
            _ => None,
        };
        let typed = value(pods(&world, None, &params, false)).await;
        let table = value(pods(&world, None, &params, true)).await;
        assert_eq!(typed["items"].as_array().unwrap().len(), 1);
        assert_eq!(typed["items"][0], table["rows"][0]["object"]);
        assert_eq!(decode("type%3DWarning"), "type=Warning");
    }
}
