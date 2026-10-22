//! A fake cluster and test-only actions for the framework's tests (behind the
//! `testing` feature, or in core's own tests). The fake keeps a few objects,
//! answers access reviews, reads, server-side apply (with `dryRun`) and
//! deletes, and records every request through [`Recorder`] so a test asserts
//! on the list. No page uses these actions: the framework ships none.

use std::{
    collections::BTreeMap,
    io,
    path::Path,
    sync::{Arc, Mutex},
};

use http::Request;
use kube::client::Body;
use serde_json::{Value, json};

use super::{
    ActionDescription, AuditStore, Change, Inverse, ObjectChange, Observation, Protection, Runner,
    Target, Tier, Who,
};
use crate::fake_api::{self, Recorded, Recorder};

pub const DEPLOYMENT: &str = "/apis/apps/v1/namespaces/shop/deployments/web";
pub const SECRET: &str = "/api/v1/namespaces/shop/secrets/db";

#[derive(Default)]
struct State {
    objects: BTreeMap<String, Value>,
    next_version: u64,
    denied: bool,
    access_review_fails: bool,
    /// An answer for every write that is not a dry run.
    fail_write: Option<(u16, String)>,
    /// An answer for every dry run.
    fail_dry_run: Option<(u16, String)>,
    /// A write that is applied but whose status never follows.
    stuck: bool,
    /// A write that gets no answer at all (the object may or may not change).
    no_answer: bool,
}

/// The fake cluster, its recorder and a client over it.
#[derive(Clone)]
pub struct Cluster {
    state: Arc<Mutex<State>>,
    pub recorder: Recorder,
    pub client: kube::Client,
}

fn merge(into: &mut Value, from: &Value) {
    match (into, from) {
        (Value::Object(into), Value::Object(from)) => {
            for (key, value) in from {
                merge(into.entry(key.clone()).or_insert(Value::Null), value);
            }
        }
        (into, from) => *into = from.clone(),
    }
}

fn deployment() -> Value {
    json!({
        "apiVersion": "apps/v1", "kind": "Deployment",
        "metadata": {"name": "web", "namespace": "shop", "uid": "uid-web", "resourceVersion": "100",
            "managedFields": [{"manager": "kubectl", "time": "2026-10-22T09:00:00Z"}]},
        "spec": {"replicas": 2, "template": {"spec": {"containers": [{"name": "web", "image": "registry.example/web:1"}]}}},
        "status": {"readyReplicas": 2}
    })
}

fn secret() -> Value {
    json!({
        "apiVersion": "v1", "kind": "Secret",
        "metadata": {"name": "db", "namespace": "shop", "uid": "uid-db", "resourceVersion": "200"},
        "data": {"password": "aHVudGVyMg=="}
    })
}

impl Cluster {
    pub fn new() -> Self {
        let state = Arc::new(Mutex::new(State {
            next_version: 1000,
            ..State::default()
        }));
        {
            let mut state = state.lock().unwrap();
            state.objects.insert(DEPLOYMENT.into(), deployment());
            state.objects.insert(SECRET.into(), secret());
        }
        let recorder = Recorder::default();
        let (log, shared) = (recorder.clone(), state.clone());
        let service = tower::service_fn(move |request: Request<Body>| {
            let (log, shared) = (log.clone(), shared.clone());
            async move {
                let (parts, body) = request.into_parts();
                let bytes = http_body_util::BodyExt::collect(body)
                    .await
                    .map(|collected| collected.to_bytes())
                    .unwrap_or_default();
                let request = Request::from_parts(parts, ());
                let recorded =
                    log.record_with_body(&request, String::from_utf8_lossy(&bytes).into_owned());
                let (answer, silent) = answer(&mut shared.lock().unwrap(), &recorded);
                if silent {
                    return Err(io::Error::new(
                        io::ErrorKind::ConnectionReset,
                        "connection reset",
                    ));
                }
                Ok(fake_api::response(answer))
            }
        });
        let client = kube::Client::new(service, "default");
        Self {
            state,
            recorder,
            client,
        }
    }

    /// The access review answers no.
    pub fn deny_access(&self) {
        self.state.lock().unwrap().denied = true;
    }

    /// The access review itself fails.
    pub fn break_access_review(&self) {
        self.state.lock().unwrap().access_review_fails = true;
    }

    /// Every write that is not a dry run is answered with this.
    pub fn refuse_writes(&self, code: u16, message: &str) {
        self.state.lock().unwrap().fail_write = Some((code, message.to_owned()));
    }

    /// Every dry run is answered with this.
    pub fn refuse_dry_runs(&self, code: u16, message: &str) {
        self.state.lock().unwrap().fail_dry_run = Some((code, message.to_owned()));
    }

    /// Writes are applied but the status never follows.
    pub fn never_reaches_the_state(&self) {
        self.state.lock().unwrap().stuck = true;
    }

    /// Writes that are not dry runs get no answer.
    pub fn drop_the_connection_on_writes(&self) {
        self.state.lock().unwrap().no_answer = true;
    }

    pub fn object(&self, path: &str) -> Value {
        self.state.lock().unwrap().objects[path].clone()
    }

    pub fn exists(&self, path: &str) -> bool {
        self.state.lock().unwrap().objects.contains_key(path)
    }

    /// Someone else writes a field: its value, the resourceVersion and
    /// managedFields move.
    pub fn foreign_write(&self, path: &str, manager: &str, patch: &Value) {
        let mut state = self.state.lock().unwrap();
        state.next_version += 1;
        let version = state.next_version;
        let object = state.objects.get_mut(path).unwrap();
        merge(object, patch);
        object["metadata"]["resourceVersion"] = json!(version.to_string());
        object["metadata"]["managedFields"]
            .as_array_mut()
            .unwrap()
            .push(json!({"manager": manager, "time": "2026-10-22T11:00:00Z"}));
    }

    /// The object is deleted and created again: the UID moves.
    pub fn recreate(&self, path: &str) {
        let mut state = self.state.lock().unwrap();
        state.next_version += 1;
        let version = state.next_version;
        let object = state.objects.get_mut(path).unwrap();
        object["metadata"]["uid"] = json!("uid-new");
        object["metadata"]["resourceVersion"] = json!(version.to_string());
    }

    /// Requests that could have changed the cluster: anything but a read,
    /// an access review or a dry run.
    pub fn writes(&self) -> Vec<Recorded> {
        self.recorder
            .requests()
            .into_iter()
            .filter(|r| {
                r.method != http::Method::GET
                    && !r.path.ends_with("/selfsubjectaccessreviews")
                    && !r.query.contains("dryRun")
                    && !r.body.contains("\"dryRun\"")
            })
            .collect()
    }

    /// The reads and reviews are fine; this is what must stay empty when
    /// nothing was confirmed.
    pub fn assert_nothing_written(&self) {
        assert_eq!(
            self.writes().iter().map(Recorded::line).collect::<Vec<_>>(),
            Vec::<String>::new(),
            "no write may be sent; all requests: {:?}",
            self.recorder.lines()
        );
    }
}

impl Default for Cluster {
    fn default() -> Self {
        Self::new()
    }
}

fn status(code: u16, reason: &str, message: &str) -> (u16, String) {
    fake_api::json(
        code,
        json!({"kind": "Status", "apiVersion": "v1", "status": "Failure", "code": code,
               "reason": reason, "message": message}),
    )
}

/// The answer and whether the connection drops instead of answering.
fn answer(state: &mut State, request: &Recorded) -> ((u16, String), bool) {
    let dry = request.query.contains("dryRun") || request.body.contains("\"dryRun\"");
    if request.path.ends_with("/selfsubjectaccessreviews") {
        if state.access_review_fails {
            return (
                status(500, "InternalError", "the review service is down"),
                false,
            );
        }
        return (
            fake_api::json(
                201,
                json!({"apiVersion": "authorization.k8s.io/v1", "kind": "SelfSubjectAccessReview",
                    "metadata": {}, "spec": {},
                    "status": {"allowed": !state.denied,
                               "reason": if state.denied { "RBAC: no role allows it" } else { "" }}}),
            ),
            false,
        );
    }
    let path = request.path.as_str();
    if !state.objects.contains_key(path) {
        return (
            if state.objects.is_empty() {
                fake_api::unknown(request)
            } else {
                status(404, "NotFound", "not found")
            },
            false,
        );
    }
    match request.method.as_str() {
        "GET" => (fake_api::json(200, &state.objects[path]), false),
        "PATCH" | "DELETE" => {
            if dry {
                if let Some((code, message)) = &state.fail_dry_run {
                    return (status(*code, "Invalid", message), false);
                }
            } else {
                if state.no_answer {
                    return ((0, String::new()), true);
                }
                if let Some((code, message)) = &state.fail_write {
                    let reason = if *code == 409 { "Conflict" } else { "Invalid" };
                    return (status(*code, reason, message), false);
                }
            }
            if request.method == "DELETE" {
                if !dry {
                    state.objects.remove(path);
                }
                return (
                    fake_api::json(200, json!({"kind": "Status", "status": "Success"})),
                    false,
                );
            }
            let body: Value = serde_json::from_str(&request.body).unwrap_or(Value::Null);
            let mut next = state.objects[path].clone();
            merge(&mut next, &body);
            state.next_version += 1;
            next["metadata"]["resourceVersion"] = json!(state.next_version.to_string());
            if !state.stuck && next.pointer("/spec/replicas").is_some() {
                next["status"]["readyReplicas"] = next["spec"]["replicas"].clone();
            }
            if !dry {
                state.objects.insert(path.to_owned(), next.clone());
            }
            (fake_api::json(200, next), false)
        }
        _ => (fake_api::unknown(request), false),
    }
}

/// The `web` Deployment as the person saw it at the start.
pub fn web() -> Target {
    Target {
        group: "apps".into(),
        version: "v1".into(),
        kind: "Deployment".into(),
        resource: "deployments".into(),
        namespace: Some("shop".into()),
        name: "web".into(),
        uid: "uid-web".into(),
        resource_version: "100".into(),
        subresource: None,
    }
}

/// The `db` Secret as the person saw it.
pub fn db() -> Target {
    Target {
        group: String::new(),
        version: "v1".into(),
        kind: "Secret".into(),
        resource: "secrets".into(),
        namespace: Some("shop".into()),
        name: "db".into(),
        uid: "uid-db".into(),
        resource_version: "200".into(),
        subresource: None,
    }
}

fn replicas(count: u64) -> Observation {
    Observation::FieldEquals {
        pointer: "/status/readyReplicas".into(),
        value: json!(count),
    }
}

/// A reversible test action: scale `web` (from 2) to `count`.
pub fn scale(count: u64) -> ActionDescription {
    ActionDescription {
        kind: "test-scale".into(),
        verb: "Scale".into(),
        tier: Tier::Reversible,
        sentence: format!("Scale web to {count}"),
        changes: vec![ObjectChange {
            target: web(),
            change: Change::Apply(
                json!({"apiVersion": "apps/v1", "kind": "Deployment", "metadata": {"name": "web"},
                       "spec": {"replicas": count}}),
            ),
            observe: replicas(count),
            inverse: Some(Inverse {
                change: Change::Apply(
                    json!({"apiVersion": "apps/v1", "kind": "Deployment", "metadata": {"name": "web"},
                           "spec": {"replicas": 2}}),
                ),
                observe: replicas(2),
                guard: Observation::FieldEquals {
                    pointer: "/spec/replicas".into(),
                    value: json!(count),
                },
            }),
        }],
    }
}

/// A spec change: set `web`'s image.
pub fn set_image(image: &str) -> ActionDescription {
    ActionDescription {
        kind: "test-set-image".into(),
        verb: "Apply".into(),
        tier: Tier::SpecChange,
        sentence: "Change web's image".into(),
        changes: vec![ObjectChange {
            target: web(),
            change: Change::Apply(json!({"apiVersion": "apps/v1", "kind": "Deployment",
                "metadata": {"name": "web"},
                "spec": {"template": {"spec": {"containers": [{"name": "web", "image": image}]}}}})),
            observe: Observation::FieldEquals {
                pointer: "/spec/template/spec/containers/0/image".into(),
                value: json!(image),
            },
            inverse: None,
        }],
    }
}

/// A destructive action: delete `web`.
pub fn delete_web() -> ActionDescription {
    ActionDescription {
        kind: "test-delete".into(),
        verb: "Delete".into(),
        tier: Tier::Destructive,
        sentence: "Delete web".into(),
        changes: vec![ObjectChange {
            target: web(),
            change: Change::Delete,
            observe: Observation::Gone,
            inverse: None,
        }],
    }
}

/// A spec change on a Secret, whose values must never be shown or recorded.
pub fn rotate_password() -> ActionDescription {
    ActionDescription {
        kind: "test-rotate".into(),
        verb: "Apply".into(),
        tier: Tier::SpecChange,
        sentence: "Rotate db's password".into(),
        changes: vec![ObjectChange {
            target: db(),
            change: Change::Apply(json!({"apiVersion": "v1", "kind": "Secret",
                "metadata": {"name": "db"}, "data": {"password": "bmV3cGFzc3dvcmQ="}})),
            observe: Observation::FieldEquals {
                pointer: "/data/password".into(),
                value: json!("bmV3cGFzc3dvcmQ="),
            },
            inverse: None,
        }],
    }
}

/// Two objects in one action, for the typed entry id.
pub fn scale_and_delete() -> ActionDescription {
    let mut action = scale(3);
    action.tier = Tier::Destructive;
    action.changes.extend(rotate_password().changes);
    action
}

/// A runner over the fake, auditing under `dir`.
pub fn runner(cluster: &Cluster, dir: &Path, protection: Protection) -> Runner {
    Runner::new(
        cluster.client.clone(),
        AuditStore::new(dir.join("audit")),
        Who {
            entry: "prod-fra".into(),
            context: "ctx-a".into(),
            protection,
            actor: "tester".into(),
        },
    )
    .watching(super::Watching {
        wait: std::time::Duration::from_secs(30),
        every: std::time::Duration::from_secs(1),
    })
}
