//! A canned cluster with one realistic fault, so the agent has something to
//! investigate offline and in a live run without touching a real cluster.
//!
//! The fault: an ExternalSecret rotated `shop/checkout-db` to a new database
//! password, Reloader rolled the Deployment, and the new pods crash because
//! the CloudNativePG role still takes its password from a different Secret,
//! `checkout-db-role`, which was not rotated. The old pods keep working on
//! connections they opened before the rotation.

use std::fmt;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ObjectRef {
    pub kind: &'static str,
    pub namespace: &'static str,
    pub name: &'static str,
}

impl fmt::Display for ObjectRef {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}/{}/{}", self.kind, self.namespace, self.name)
    }
}

pub struct Object {
    pub id: ObjectRef,
    pub status: &'static str,
    pub document: &'static str,
    /// Secret keys. A Secret's values never reach a tool result.
    pub secret_keys: Option<&'static [&'static str]>,
}

pub struct ContainerLog {
    pub pod: &'static str,
    pub container: &'static str,
    pub current: &'static [&'static str],
    pub previous: &'static [&'static str],
}

pub struct Event {
    pub object: ObjectRef,
    pub kind: &'static str,
    pub reason: &'static str,
    pub age: &'static str,
    pub message: &'static str,
}

pub struct Cluster {
    pub objects: Vec<Object>,
    pub logs: Vec<ContainerLog>,
    pub events: Vec<Event>,
}

const NS: &str = "shop";

fn id(kind: &'static str, name: &'static str) -> ObjectRef {
    ObjectRef {
        kind,
        namespace: NS,
        name,
    }
}

impl Cluster {
    pub fn kinds() -> &'static [&'static str] {
        &[
            "Deployment",
            "ReplicaSet",
            "Pod",
            "Service",
            "Secret",
            "ExternalSecret",
            "Cluster.postgresql.cnpg.io",
        ]
    }

    /// Matches a kind the way a model is likely to name it: any case, plural
    /// or the bare custom kind.
    pub fn canonical_kind(asked: &str) -> Option<&'static str> {
        let asked = asked.trim().to_ascii_lowercase();
        let asked = asked.trim_end_matches('s');
        Self::kinds().iter().copied().find(|kind| {
            let lower = kind.to_ascii_lowercase();
            lower == asked
                || lower.split('.').next() == Some(asked)
                || lower.starts_with(&format!("{asked}."))
        })
    }

    pub fn find(&self, kind: &str, namespace: &str, name: &str) -> Option<&Object> {
        let kind = Self::canonical_kind(kind)?;
        self.objects
            .iter()
            .find(|o| o.id.kind == kind && o.id.namespace == namespace && o.id.name == name)
    }

    pub fn checkout_crash() -> Self {
        let objects = vec![
            Object {
                id: id("Deployment", "checkout"),
                status: "2/3 ready, 1 updated, revision 14",
                document: r#"metadata:
  annotations:
    deployment.kubernetes.io/revision: "14"
    secret.reloader.stakater.com/reload: checkout-db
spec:
  replicas: 3
  strategy: {type: RollingUpdate, rollingUpdate: {maxUnavailable: 0, maxSurge: 1}}
  template:
    metadata:
      annotations:
        reloader.stakater.com/last-reloaded-from: '{"type":"SECRET","name":"checkout-db","hash":"9f2c…"}'
    spec:
      containers:
      - name: checkout
        image: registry.example.com/shop/checkout:2.31.0
        env:
        - {name: DATABASE_HOST, value: postgres-rw.shop.svc}
        - {name: DATABASE_USER, valueFrom: {secretKeyRef: {name: checkout-db, key: username}}}
        - {name: DATABASE_PASSWORD, valueFrom: {secretKeyRef: {name: checkout-db, key: password}}}
        readinessProbe: {httpGet: {path: /ready, port: 8080}}
status:
  replicas: 4
  updatedReplicas: 1
  readyReplicas: 2
  availableReplicas: 2
  unavailableReplicas: 1
  conditions:
  - {type: Available, status: "False", reason: MinimumReplicasUnavailable, lastTransitionTime: 11m ago}
  - {type: Progressing, status: "True", reason: ReplicaSetUpdated, message: ReplicaSet "checkout-7c9d8f6b4" is progressing., lastTransitionTime: 11m ago}"#,
                secret_keys: None,
            },
            Object {
                id: id("ReplicaSet", "checkout-7c9d8f6b4"),
                status: "0/1 ready, revision 14, created 11m ago",
                document: r#"metadata:
  annotations: {deployment.kubernetes.io/revision: "14"}
  ownerReferences: [{kind: Deployment, name: checkout, controller: true}]
spec: {replicas: 1}
status: {replicas: 1, readyReplicas: 0, availableReplicas: 0}"#,
                secret_keys: None,
            },
            Object {
                id: id("ReplicaSet", "checkout-5b7f9c8d2"),
                status: "2/2 ready, revision 13, created 6d ago",
                document: r#"metadata:
  annotations: {deployment.kubernetes.io/revision: "13"}
  ownerReferences: [{kind: Deployment, name: checkout, controller: true}]
spec: {replicas: 2}
status: {replicas: 2, readyReplicas: 2, availableReplicas: 2}"#,
                secret_keys: None,
            },
            Object {
                id: id("Pod", "checkout-7c9d8f6b4-q8zxk"),
                status: "CrashLoopBackOff, 0/1 ready, 6 restarts, 11m",
                document: r#"metadata:
  ownerReferences: [{kind: ReplicaSet, name: checkout-7c9d8f6b4, controller: true}]
spec:
  nodeName: worker-2
  containers: [{name: checkout, image: registry.example.com/shop/checkout:2.31.0}]
status:
  phase: Running
  conditions:
  - {type: Ready, status: "False", reason: ContainersNotReady}
  containerStatuses:
  - name: checkout
    ready: false
    restartCount: 6
    state: {waiting: {reason: CrashLoopBackOff, message: back-off 2m40s restarting failed container}}
    lastState: {terminated: {exitCode: 1, reason: Error, startedAt: 3m ago, finishedAt: 3m ago}}"#,
                secret_keys: None,
            },
            Object {
                id: id("Pod", "checkout-5b7f9c8d2-4mtrn"),
                status: "Running, 1/1 ready, 0 restarts, 6d",
                document: r#"metadata:
  ownerReferences: [{kind: ReplicaSet, name: checkout-5b7f9c8d2, controller: true}]
spec: {nodeName: worker-1, containers: [{name: checkout, image: registry.example.com/shop/checkout:2.30.4}]}
status:
  phase: Running
  containerStatuses: [{name: checkout, ready: true, restartCount: 0, state: {running: {startedAt: 6d ago}}}]"#,
                secret_keys: None,
            },
            Object {
                id: id("Pod", "checkout-5b7f9c8d2-h2p9l"),
                status: "Running, 1/1 ready, 0 restarts, 6d",
                document: r#"metadata:
  ownerReferences: [{kind: ReplicaSet, name: checkout-5b7f9c8d2, controller: true}]
spec: {nodeName: worker-3, containers: [{name: checkout, image: registry.example.com/shop/checkout:2.30.4}]}
status:
  phase: Running
  containerStatuses: [{name: checkout, ready: true, restartCount: 0, state: {running: {startedAt: 6d ago}}}]"#,
                secret_keys: None,
            },
            Object {
                id: id("Service", "postgres-rw"),
                status: "ClusterIP 10.96.41.7:5432, 1 endpoint",
                document: r#"spec:
  selector: {cnpg.io/cluster: postgres, cnpg.io/instanceRole: primary}
  ports: [{name: postgres, port: 5432, targetPort: 5432}]
endpoints: [postgres-1 10.244.1.23:5432 ready]"#,
                secret_keys: None,
            },
            Object {
                id: id("Secret", "checkout-db"),
                status: "Opaque, 2 keys, updated 11m ago",
                document: r#"metadata:
  resourceVersion: "88412097"
  labels: {reconcile.external-secrets.io/managed: "true"}
  annotations: {reconcile.external-secrets.io/data-hash: 51ad…}
  ownerReferences: [{kind: ExternalSecret, name: checkout-db, controller: true}]
  managedFields:
  - {manager: external-secrets, operation: Update, time: 11m ago}
type: Opaque"#,
                secret_keys: Some(&["username", "password"]),
            },
            Object {
                id: id("Secret", "checkout-db-role"),
                status: "kubernetes.io/basic-auth, 2 keys, updated 41d ago",
                document: r#"metadata:
  resourceVersion: "61200413"
  labels: {cnpg.io/reload: "true"}
  managedFields:
  - {manager: kubectl-client-side-apply, operation: Update, time: 41d ago}
type: kubernetes.io/basic-auth"#,
                secret_keys: Some(&["username", "password"]),
            },
            Object {
                id: id("ExternalSecret", "checkout-db"),
                status: "SecretSynced, refreshed 11m ago",
                document: r#"spec:
  refreshInterval: 1h
  secretStoreRef: {kind: ClusterSecretStore, name: vault}
  target: {name: checkout-db, creationPolicy: Owner}
  data:
  - {secretKey: username, remoteRef: {key: prod/shop/checkout-db, property: username}}
  - {secretKey: password, remoteRef: {key: prod/shop/checkout-db, property: password}}
status:
  refreshTime: 11m ago
  syncedResourceVersion: 1-7e1c…
  conditions:
  - {type: Ready, status: "True", reason: SecretSynced, message: Secret was synced, lastTransitionTime: 11m ago}"#,
                secret_keys: None,
            },
            Object {
                id: id("Cluster.postgresql.cnpg.io", "postgres"),
                status: "Cluster in healthy state, 3/3 instances ready",
                document: r#"spec:
  instances: 3
  managed:
    roles:
    - name: checkout
      ensure: present
      login: true
      passwordSecret: {name: checkout-db-role}
status:
  phase: Cluster in healthy state
  readyInstances: 3
  currentPrimary: postgres-1
  managedRolesStatus:
    byStatus: {reconciled: [checkout, reporting]}
    passwordStatus:
      checkout: {transactionID: 9811, resourceVersion: "61200413"}"#,
                secret_keys: None,
            },
            Object {
                id: id("Pod", "postgres-1"),
                status: "Running, 1/1 ready, primary, 41d",
                document: r#"metadata:
  labels: {cnpg.io/cluster: postgres, cnpg.io/instanceRole: primary}
spec: {nodeName: worker-1, containers: [{name: postgres, image: ghcr.io/cloudnative-pg/postgresql:16.4}]}
status: {phase: Running, containerStatuses: [{name: postgres, ready: true, restartCount: 0}]}"#,
                secret_keys: None,
            },
        ];

        let logs = vec![
            ContainerLog {
                pod: "checkout-7c9d8f6b4-q8zxk",
                container: "checkout",
                current: &[
                    "2026-10-03T09:41:02Z INFO  checkout 2.31.0 starting",
                    "2026-10-03T09:41:02Z INFO  config: db host=postgres-rw.shop.svc port=5432 user=checkout",
                ],
                previous: &[
                    "2026-10-03T09:38:20Z INFO  checkout 2.31.0 starting",
                    "2026-10-03T09:38:20Z INFO  config: db host=postgres-rw.shop.svc port=5432 user=checkout",
                    "2026-10-03T09:38:21Z ERROR db: connect failed: FATAL: password authentication failed for user \"checkout\"",
                    "2026-10-03T09:38:21Z ERROR startup aborted: database unavailable",
                ],
            },
            ContainerLog {
                pod: "checkout-5b7f9c8d2-4mtrn",
                container: "checkout",
                current: &[
                    "2026-10-03T09:40:55Z INFO  POST /api/orders 201 38ms",
                    "2026-10-03T09:41:01Z INFO  db pool: 10/10 connections healthy",
                    "2026-10-03T09:41:07Z INFO  GET /api/cart 200 12ms",
                ],
                previous: &[],
            },
            ContainerLog {
                pod: "checkout-5b7f9c8d2-h2p9l",
                container: "checkout",
                current: &[
                    "2026-10-03T09:40:58Z INFO  GET /api/cart 200 9ms",
                    "2026-10-03T09:41:01Z INFO  db pool: 10/10 connections healthy",
                ],
                previous: &[],
            },
            ContainerLog {
                pod: "postgres-1",
                container: "postgres",
                current: &[
                    "2026-10-03 09:38:21.104 UTC [48211] checkout@shop FATAL:  password authentication failed for user \"checkout\"",
                    "2026-10-03 09:38:21.104 UTC [48211] checkout@shop DETAIL:  Connection matched pg_hba.conf line 7: \"host all all all scram-sha-256\"",
                    "2026-10-03 09:41:03.512 UTC [48390] checkout@shop FATAL:  password authentication failed for user \"checkout\"",
                ],
                previous: &[],
            },
        ];

        let events = vec![
            Event {
                object: id("ExternalSecret", "checkout-db"),
                kind: "Normal",
                reason: "Updated",
                age: "11m",
                message: "Updated Secret",
            },
            Event {
                object: id("Deployment", "checkout"),
                kind: "Normal",
                reason: "ScalingReplicaSet",
                age: "11m",
                message: "Scaled up replica set checkout-7c9d8f6b4 to 1",
            },
            Event {
                object: id("Pod", "checkout-7c9d8f6b4-q8zxk"),
                kind: "Warning",
                reason: "BackOff",
                age: "40s (x31 over 10m)",
                message: "Back-off restarting failed container checkout in pod checkout-7c9d8f6b4-q8zxk",
            },
            Event {
                object: id("Pod", "checkout-7c9d8f6b4-q8zxk"),
                kind: "Warning",
                reason: "Unhealthy",
                age: "3m (x12 over 10m)",
                message: "Readiness probe failed: connect: connection refused",
            },
        ];

        Self {
            objects,
            logs,
            events,
        }
    }
}
