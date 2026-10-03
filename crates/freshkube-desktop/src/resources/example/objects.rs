use super::*;

pub(super) fn certificate_yaml(row: &ResourceRow, ix: usize, created: i64) -> String {
    let (namespace, name, issuer, ready) = CERTIFICATES[ix];
    let mut yaml = metadata(
        "Certificate",
        "cert-manager.io/v1",
        row,
        "  generation: 1\n",
    );
    yaml.push_str(&format!(
        "spec:\n  secretName: {name}\n  dnsNames:\n  - {name}.{namespace}.example.internal\n  issuerRef:\n    group: cert-manager.io\n    kind: ClusterIssuer\n    name: {issuer}\n  privateKey:\n    algorithm: ECDSA\n    size: 256\nstatus:\n  conditions:\n  - type: Ready\n    status: '{}'\n    reason: {}\n    message: {}\n    lastTransitionTime: '{}'\n    observedGeneration: 1\n",
        if ready { "True" } else { "False" },
        if ready { "Ready" } else { "DoesNotExist" },
        certificate_status(ready),
        timestamp(created + 60),
    ));
    if ready {
        yaml.push_str(&format!(
            "  notBefore: '{}'\n  notAfter: '{}'\n  renewalTime: '{}'\n  revision: 1\n",
            timestamp(created + 60),
            timestamp(created + 60 + 90 * 86_400),
            timestamp(created + 60 + 60 * 86_400),
        ));
    } else {
        yaml.push_str(&format!(
            "  - type: Issuing\n    status: 'True'\n    reason: DoesNotExist\n    message: {}\n    lastTransitionTime: '{}'\n    observedGeneration: 1\n",
            certificate_status(ready),
            timestamp(created + 60),
        ));
    }
    yaml
}

/// The example row an identity names, with the position its UID encodes.
pub(super) fn find(identity: &ResourceIdentity, now: i64) -> Option<(ResourceRow, usize)> {
    let context = identity.connection.strip_prefix("example:")?;
    let (_, rows) = read(context, &identity.resource, None, now)?;
    let (position, row) = rows
        .into_iter()
        .enumerate()
        .find(|(_, row)| row.identity == *identity)?;
    let ix = identity
        .uid
        .rsplit('-')
        .next()
        .and_then(|suffix| usize::from_str_radix(suffix, 16).ok())
        .unwrap_or(position);
    Some((row, ix))
}

pub(super) fn time(seconds: i64) -> Option<DateTime<Utc>> {
    DateTime::from_timestamp(seconds, 0)
}

pub(super) fn timestamp(seconds: i64) -> String {
    time(seconds)
        .unwrap_or_default()
        .to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
}

/// Standard base64, for Secret data.
pub(super) fn base64(bytes: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut encoded = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let bits = chunk.iter().enumerate().fold(0u32, |bits, (ix, byte)| {
            bits | u32::from(*byte) << (16 - 8 * ix)
        });
        for ix in 0..4 {
            encoded.push(if ix <= chunk.len() {
                ALPHABET[(bits >> (18 - 6 * ix) & 63) as usize] as char
            } else {
                '='
            });
        }
    }
    encoded
}

/// The metadata block every example object starts with.
pub(super) fn metadata(kind: &str, api_version: &str, row: &ResourceRow, extra: &str) -> String {
    let identity = &row.identity;
    let namespace = if identity.namespace.is_empty() {
        String::new()
    } else {
        format!("  namespace: {}\n", identity.namespace)
    };
    format!(
        "apiVersion: {api_version}\nkind: {kind}\nmetadata:\n  name: {}\n{namespace}  uid: {}\n  resourceVersion: '{}'\n  creationTimestamp: '{}'\n{extra}",
        identity.name,
        identity.uid,
        row.resource_version,
        timestamp(row.created.unwrap_or_default()),
    )
}

/// The full object behind an example row, written as the server would
/// return it. `None` for anything the example cluster doesn't have.
pub(crate) fn document(identity: &ResourceIdentity, now: i64) -> Option<ObjectDocument> {
    if identity.resource == "secrets" {
        crate::desktop::probe::hit("example.secret-document");
    }
    let (row, ix) = find(identity, now)?;
    let created = row.created.unwrap_or_default();
    let yaml = match identity.resource.as_str() {
        "replicasets.apps" | "events" | "persistentvolumeclaims" | "persistentvolumes" => {
            let context = identity.connection.strip_prefix("example:")?;
            let object = super::summary::extra_objects(context, &identity.resource, now)
                .into_iter()
                .find(|object| {
                    object["metadata"]["name"].as_str() == Some(&identity.name)
                        && object["metadata"]["namespace"].as_str().unwrap_or_default()
                            == identity.namespace
                })?;
            serde_yaml::to_string(&object).ok()?
        }
        "pods" => pod_yaml(&row, ix, created, now),
        "deployments.apps" => deployment_yaml(&row, ix, created),
        "services" => service_yaml(&row, ix),
        "nodes" => node_yaml(&row, created),
        "namespaces" => format!(
            "{}spec:\n  finalizers:\n  - kubernetes\nstatus:\n  phase: Active\n",
            metadata(
                "Namespace",
                "v1",
                &row,
                &format!(
                    "  labels:\n    kubernetes.io/metadata.name: {}\n",
                    row.identity.name
                ),
            )
        ),
        "secrets" => {
            let (_, _, secret_type, data) = SECRETS.get(ix)?;
            let mut yaml = metadata("Secret", "v1", &row, "");
            yaml.push_str(&format!("type: {secret_type}\ndata:\n"));
            for (key, value) in *data {
                yaml.push_str(&format!("  {key}: {}\n", base64(value)));
            }
            yaml
        }
        "certificates.cert-manager.io" => certificate_yaml(&row, ix, created),
        _ => return None,
    };
    object_from_yaml(&kind(&identity.resource)?, &yaml).ok()
}

pub(super) fn pod_yaml(row: &ResourceRow, ix: usize, created: i64, now: i64) -> String {
    let (_, app, image, _) = WORKLOADS[ix % WORKLOADS.len()];
    let hash = format!("{:x}", 0x6c4f_8d9b + ix % WORKLOADS.len());
    let status = row.cells[2].as_str();
    let ready = row.cells[1].starts_with("1/");
    let restarts: u32 = row.cells[3]
        .split_whitespace()
        .next()
        .and_then(|count| count.parse().ok())
        .unwrap_or(0);
    let labels = format!(
        "  generateName: {app}-{hash}-\n  labels:\n    app: {app}\n    pod-template-hash: '{hash}'\n  ownerReferences:\n  - apiVersion: apps/v1\n    kind: ReplicaSet\n    name: {app}-{hash}\n    uid: replicasets.apps-{}-{app}-{hash}\n    controller: true\n    blockOwnerDeletion: true\n",
        row.identity.namespace
    );
    let scheduled = status != "Pending";
    let init = has_init_container(ix);
    let mut yaml = metadata("Pod", "v1", row, &labels);
    yaml.push_str(&format!(
        "spec:\n{}  containers:\n  - name: {app}\n    image: {image}\n    ports:\n    - containerPort: 8080\n      protocol: TCP\n    resources:\n      requests:\n        cpu: 100m\n        memory: 128Mi\n      limits:\n        memory: 256Mi\n",
        if init {
            format!("  initContainers:\n  - name: {INIT_CONTAINER}\n    image: busybox:1.37\n")
        } else {
            String::new()
        }
    ));
    if scheduled {
        yaml.push_str(&format!("  nodeName: {}\n", row.cells[6]));
    }
    yaml.push_str("  restartPolicy: Always\n  serviceAccountName: default\nstatus:\n");
    let phase = match status {
        "Completed" => "Succeeded",
        "Pending" | "ContainerCreating" => "Pending",
        _ => "Running",
    };
    yaml.push_str(&format!("  phase: {phase}\n  conditions:\n"));
    if scheduled {
        yaml.push_str(&format!(
            "  - type: Ready\n    status: '{}'\n    lastTransitionTime: '{}'\n{}  - type: PodScheduled\n    status: 'True'\n    lastTransitionTime: '{}'\n  podIP: {}\n  startTime: '{}'\n{}  containerStatuses:\n  - name: {app}\n    image: {image}\n    imageID: example://app\n    ready: {ready}\n    restartCount: {restarts}\n{}    state:\n",
            if ready { "True" } else { "False" },
            timestamp(created + 40),
            if ready {
                String::new()
            } else {
                format!("    reason: ContainersNotReady\n    message: 'containers with unready status: [{app}]'\n")
            },
            timestamp(created),
            row.cells[5],
            timestamp(created),
            if init {
                format!(
                    "  initContainerStatuses:\n  - name: {INIT_CONTAINER}\n    image: busybox:1.37\n    imageID: example://init\n    ready: true\n    restartCount: 0\n    state:\n      terminated:\n        exitCode: 0\n        reason: Completed\n        finishedAt: '{}'\n",
                    timestamp(created + 25)
                )
            } else {
                String::new()
            },
            match last_termination(status, restarts, ix, now) {
                Some(last) => format!(
                    "    lastState:\n      terminated:\n        exitCode: {}\n        reason: {}\n        finishedAt: '{}'\n",
                    last.exit_code,
                    last.reason,
                    last.finished.unwrap_or_default().to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
                ),
                None => String::new(),
            },
        ));
        yaml.push_str(&match status {
            "Running" => format!("      running:\n        startedAt: '{}'\n", timestamp(created + 30)),
            "Completed" => format!(
                "      terminated:\n        exitCode: 0\n        reason: Completed\n        finishedAt: '{}'\n",
                timestamp(created + 300)
            ),
            "CrashLoopBackOff" => "      waiting:\n        reason: CrashLoopBackOff\n        message: back-off 5m0s restarting failed container\n".into(),
            other => format!("      waiting:\n        reason: {other}\n"),
        });
    } else {
        yaml.push_str(&format!(
            "  - type: PodScheduled\n    status: 'False'\n    reason: Unschedulable\n    message: '0/3 nodes are available: 3 Insufficient memory.'\n    lastTransitionTime: '{}'\n",
            timestamp(created)
        ));
    }
    yaml
}

/// The init container some example pods run first.
const INIT_CONTAINER: &str = "init-config";

pub(super) fn has_init_container(ix: usize) -> bool {
    ix.is_multiple_of(3)
}

/// How an example pod's app container ended before its current instance.
pub(super) fn last_termination(
    status: &str,
    restarts: u32,
    ix: usize,
    now: i64,
) -> Option<Termination> {
    let (exit_code, reason, finished) = match status {
        "CrashLoopBackOff" => (137, "OOMKilled", now - 180),
        "Running" if restarts > 0 => (1, "Error", now - (ix as i64 % 9 + 1) * 86_400),
        _ => return None,
    };
    Some(Termination {
        exit_code,
        reason: reason.into(),
        finished: time(finished),
    })
}

/// Lines an example container writes, cycling through these.
const LOG_MESSAGES: [&str; 8] = [
    "level=info msg=\"GET /healthz 200\" duration=1.2ms",
    "level=info msg=\"GET /api/v1/items 200\" duration=14ms",
    "level=debug msg=\"cache hit\" key=items:page=1",
    "level=info msg=\"POST /api/v1/orders 201\" duration=38ms",
    "level=warn msg=\"slow query\" duration=812ms table=orders",
    "level=info msg=\"GET /api/v1/items 200\" duration=11ms",
    "level=error msg=\"upstream timeout\" upstream=ledger:8080 attempt=1",
    "level=info msg=\"GET /healthz 200\" duration=0.9ms",
];

/// One example log line as Kubernetes writes it with timestamps.
pub(crate) fn pod_log_line(sequence: u64, at: DateTime<Utc>) -> String {
    format!(
        "{} {}",
        at.to_rfc3339_opts(chrono::SecondsFormat::Nanos, true),
        LOG_MESSAGES[sequence as usize % LOG_MESSAGES.len()]
    )
}

/// `count` example lines, one every `step` seconds, the last at `end`.
pub(super) fn log_lines(count: i64, step: i64, end: i64) -> impl Iterator<Item = PodLogUpdate> {
    (0..count).map(move |ix| {
        let at = time(end - (count - 1 - ix) * step).unwrap_or_default();
        PodLogUpdate::Line(pod_log_line(ix as u64, at))
    })
}

/// What following an example container's log reports at once, and whether
/// it goes on writing. The metrics server refuses its logs, as a viewer
/// without `pods/log` would see.
pub(crate) fn pod_log(
    identity: &ResourceIdentity,
    container: &str,
    previous: bool,
    now: i64,
) -> (Vec<PodLogUpdate>, bool) {
    let failed = |kind, message: String| {
        (
            vec![PodLogUpdate::Failed(Failure::new(kind, message))],
            false,
        )
    };
    let Some((row, ix)) = find(identity, now) else {
        return failed(FailureKind::NotFound, "Example data has no such pod".into());
    };
    let (namespace, app, ..) = WORKLOADS[ix % WORKLOADS.len()];
    let name = &identity.name;
    if app == "metrics-server" {
        return failed(
            FailureKind::Forbidden,
            format!(
                "pods \"{name}\" is forbidden: User \"viewer\" cannot get resource \"pods/log\" in API group \"\" in the namespace \"{namespace}\""
            ),
        );
    }
    let status = row.cells[2].as_str();
    let created = row.created.unwrap_or(now);
    let streaming = PodLogUpdate::Streaming;
    if container == INIT_CONTAINER && has_init_container(ix) {
        if status == "Pending" {
            return (vec![PodLogUpdate::Waiting("Not started".into())], false);
        }
        let mut updates = vec![streaming];
        updates.extend(log_lines(3, 2, created + 24));
        updates.push(PodLogUpdate::Ended(Some(Termination {
            exit_code: 0,
            reason: "Completed".into(),
            finished: time(created + 25),
        })));
        return (updates, false);
    }
    if container != app {
        return failed(
            FailureKind::NotFound,
            format!("The pod has no container named {container}"),
        );
    }
    let restarts = row.cells[3]
        .split_whitespace()
        .next()
        .and_then(|count| count.parse().ok())
        .unwrap_or(0);
    let last = last_termination(status, restarts, ix, now);
    if previous {
        let Some(last) = last else {
            return failed(
                FailureKind::Other,
                format!(
                    "previous terminated container \"{container}\" in pod \"{name}\" not found"
                ),
            );
        };
        let end = last.finished.map_or(now, |finished| finished.timestamp()) - 1;
        let mut updates = vec![streaming];
        updates.extend(log_lines(24, 5, end));
        updates.push(PodLogUpdate::Ended(None));
        return (updates, false);
    }
    match status {
        "Running" => {
            let mut updates = vec![streaming];
            updates.extend(log_lines(40, 7, now - 1));
            (updates, true)
        }
        "CrashLoopBackOff" => {
            // The instance that just ended, then the wait to start again.
            let mut updates = vec![streaming];
            updates.extend(log_lines(12, 5, now - 181));
            updates.push(PodLogUpdate::Restarting(last));
            updates.push(PodLogUpdate::Waiting("CrashLoopBackOff".into()));
            (updates, false)
        }
        "Completed" => {
            let mut updates = vec![streaming];
            updates.extend(log_lines(15, 18, created + 299));
            updates.push(PodLogUpdate::Ended(Some(Termination {
                exit_code: 0,
                reason: "Completed".into(),
                finished: time(created + 300),
            })));
            (updates, false)
        }
        "Pending" => (vec![PodLogUpdate::Waiting("Not started".into())], false),
        other => (vec![PodLogUpdate::Waiting(other.into())], false),
    }
}

pub(super) fn deployment_yaml(row: &ResourceRow, ix: usize, created: i64) -> String {
    let (_, app, image, _) = WORKLOADS[ix % WORKLOADS.len()];
    let (replicas, available) = (&row.cells[2], &row.cells[3]);
    let progressing = if replicas == available {
        "NewReplicaSetAvailable"
    } else {
        "ReplicaSetUpdated"
    };
    format!(
        "{}spec:\n  replicas: {replicas}\n  selector:\n    matchLabels:\n      app: {app}\n  strategy:\n    type: RollingUpdate\n    rollingUpdate:\n      maxSurge: 25%\n      maxUnavailable: 25%\n  template:\n    metadata:\n      labels:\n        app: {app}\n    spec:\n      containers:\n      - name: {app}\n        image: {image}\n        ports:\n        - containerPort: 8080\n          protocol: TCP\nstatus:\n  observedGeneration: 3\n  replicas: {replicas}\n  updatedReplicas: {replicas}\n  readyReplicas: {available}\n  availableReplicas: {available}\n  conditions:\n  - type: Available\n    status: '{}'\n    reason: {}\n    message: Deployment {}.\n    lastUpdateTime: '{}'\n    lastTransitionTime: '{}'\n  - type: Progressing\n    status: 'True'\n    reason: {progressing}\n    message: ReplicaSet \"{app}-{:x}\" has successfully progressed.\n    lastUpdateTime: '{}'\n    lastTransitionTime: '{}'\n",
        metadata(
            "Deployment",
            "apps/v1",
            row,
            &format!(
                "  generation: 3\n  labels:\n    app: {app}\n  annotations:\n    deployment.kubernetes.io/revision: '3'\n"
            ),
        ),
        if replicas == available {
            "True"
        } else {
            "False"
        },
        if replicas == available {
            "MinimumReplicasAvailable"
        } else {
            "MinimumReplicasUnavailable"
        },
        if replicas == available {
            "has minimum availability"
        } else {
            "does not have minimum availability"
        },
        timestamp(created + 120),
        timestamp(created + 120),
        0x6c4f_8d9b + ix % WORKLOADS.len(),
        timestamp(created + 60),
        timestamp(created),
    )
}

pub(super) fn service_yaml(row: &ResourceRow, ix: usize) -> String {
    let (kind, cluster_ip) = (&row.cells[1], &row.cells[2]);
    let selector = match ix.checked_sub(1).map(|ix| WORKLOADS[ix].1) {
        Some(app) => format!("  selector:\n    app: {app}\n"),
        None => String::new(),
    };
    let ports = if kind == "LoadBalancer" {
        "  - name: http\n    port: 80\n    targetPort: 8080\n    nodePort: 31080\n    protocol: TCP\n  - name: https\n    port: 443\n    targetPort: 8443\n    nodePort: 31443\n    protocol: TCP\n"
    } else if ix == 0 {
        "  - name: https\n    port: 443\n    targetPort: 6443\n    protocol: TCP\n"
    } else {
        "  - name: http\n    port: 8080\n    targetPort: 8080\n    protocol: TCP\n"
    };
    let status = if kind == "LoadBalancer" {
        format!(
            "status:\n  loadBalancer:\n    ingress:\n    - ip: {}\n",
            row.cells[3]
        )
    } else {
        "status:\n  loadBalancer: {}\n".into()
    };
    format!(
        "{}spec:\n  type: {kind}\n  clusterIP: {cluster_ip}\n  clusterIPs:\n  - {cluster_ip}\n  ports:\n{ports}{selector}  sessionAffinity: None\n{status}",
        metadata("Service", "v1", row, ""),
    )
}

pub(super) fn node_yaml(row: &ResourceRow, created: i64) -> String {
    let name = &row.identity.name;
    let ready = row.cells[1] == "Ready";
    let role = if row.cells[2] == "control-plane" {
        "    node-role.kubernetes.io/control-plane: ''\n"
    } else {
        ""
    };
    let mut yaml = metadata(
        "Node",
        "v1",
        row,
        &format!(
            "  labels:\n    beta.kubernetes.io/arch: amd64\n    beta.kubernetes.io/os: linux\n    kubernetes.io/arch: amd64\n    kubernetes.io/hostname: {name}\n    kubernetes.io/os: linux\n{role}  annotations:\n    node.alpha.kubernetes.io/ttl: '0'\n    volumes.kubernetes.io/controller-managed-attach-detach: 'true'\n"
        ),
    );
    yaml.push_str(&format!(
        "spec:\n  podCIDR: 10.244.0.0/24\nstatus:\n  addresses:\n  - type: InternalIP\n    address: {}\n  - type: Hostname\n    address: {name}\n  capacity:\n    cpu: '8'\n    memory: 32856156Ki\n    pods: '110'\n  conditions:\n",
        row.cells[5]
    ));
    for (condition, healthy, reason) in [
        ("MemoryPressure", "False", "KubeletHasSufficientMemory"),
        ("DiskPressure", "False", "KubeletHasNoDiskPressure"),
        ("PIDPressure", "False", "KubeletHasSufficientPID"),
    ] {
        yaml.push_str(&format!(
            "  - type: {condition}\n    status: '{healthy}'\n    reason: {reason}\n    lastTransitionTime: '{}'\n",
            timestamp(created + 30)
        ));
    }
    yaml.push_str(&if ready {
        format!(
            "  - type: Ready\n    status: 'True'\n    reason: KubeletReady\n    message: kubelet is posting ready status\n    lastTransitionTime: '{}'\n",
            timestamp(created + 60)
        )
    } else {
        format!(
            "  - type: Ready\n    status: Unknown\n    reason: NodeStatusUnknown\n    message: Kubelet stopped posting node status.\n    lastTransitionTime: '{}'\n",
            timestamp(created + 86_400)
        )
    });
    yaml.push_str(&format!(
        "  nodeInfo:\n    bootID: example\n    machineID: example\n    systemUUID: example\n    architecture: amd64\n    containerRuntimeVersion: containerd://2.1.4\n    kernelVersion: 6.12.48-talos\n    kubeletVersion: {}\n    operatingSystem: linux\n    osImage: Talos (v1.11.2)\n",
        row.cells[4]
    ));
    yaml
}

#[allow(clippy::too_many_arguments)]
pub(super) fn event(
    identity: &ResourceIdentity,
    n: usize,
    warning: bool,
    reason: &str,
    message: String,
    count: u32,
    (first, last): (i64, i64),
    source: String,
) -> ObjectEvent {
    ObjectEvent {
        uid: format!("{}-event-{n}", identity.uid),
        event_type: if warning { "Warning" } else { "Normal" }.into(),
        reason: reason.into(),
        message,
        count,
        first_seen: time(first),
        last_seen: time(last),
        source,
        field_path: String::new(),
    }
}

/// Events the example cluster recorded about an object, oldest first.
pub(crate) fn events(identity: &ResourceIdentity, now: i64) -> Vec<ObjectEvent> {
    let Some((row, ix)) = find(identity, now) else {
        return Vec::new();
    };
    let created = row.created.unwrap_or_default();
    let address = identity.address();
    match identity.resource.as_str() {
        "pods" => {
            let (_, app, image, _) = WORKLOADS[ix % WORKLOADS.len()];
            let status = row.cells[2].as_str();
            let node = &row.cells[6];
            let kubelet = format!("kubelet on {node}");
            if status == "Pending" {
                return vec![event(
                    identity,
                    0,
                    true,
                    "FailedScheduling",
                    "0/3 nodes are available: 3 Insufficient memory. preemption: 0/3 nodes are available: 3 No preemption victims found for incoming pod.".into(),
                    6,
                    (created, now - 240),
                    "default-scheduler".into(),
                )];
            }
            let mut events = vec![event(
                identity,
                0,
                false,
                "Scheduled",
                format!("Successfully assigned {address} to {node}"),
                1,
                (created, created),
                "default-scheduler".into(),
            )];
            if status == "ContainerCreating" {
                events.push(event(
                    identity,
                    1,
                    false,
                    "Pulling",
                    format!("Pulling image \"{image}\""),
                    1,
                    (created + 2, created + 2),
                    kubelet,
                ));
                return events;
            }
            for (n, (reason, message)) in [
                (
                    "Pulled",
                    format!("Container image \"{image}\" already present on machine"),
                ),
                ("Created", format!("Created container: {app}")),
                ("Started", format!("Started container {app}")),
            ]
            .into_iter()
            .enumerate()
            {
                let at = created + 5 + n as i64;
                let mut event = event(
                    identity,
                    n + 1,
                    false,
                    reason,
                    message,
                    1,
                    (at, at),
                    kubelet.clone(),
                );
                event.field_path = format!("spec.containers{{{app}}}");
                events.push(event);
            }
            if status == "CrashLoopBackOff" {
                let mut event = event(
                    identity,
                    4,
                    true,
                    "BackOff",
                    format!(
                        "Back-off restarting failed container {app} in pod {}_{}({})",
                        identity.name, identity.namespace, identity.uid
                    ),
                    14,
                    (created + 600, now - 180),
                    kubelet,
                );
                event.field_path = format!("spec.containers{{{app}}}");
                events.push(event);
            }
            events
        }
        "deployments.apps" => {
            let (_, app, ..) = WORKLOADS[ix % WORKLOADS.len()];
            vec![event(
                identity,
                0,
                false,
                "ScalingReplicaSet",
                format!(
                    "Scaled up replica set {app}-{:x} from 0 to {}",
                    0x6c4f_8d9b + ix % WORKLOADS.len(),
                    row.cells[2]
                ),
                1,
                (created + 60, created + 60),
                "deployment-controller".into(),
            )]
        }
        "nodes" if row.cells[1] != "Ready" => vec![event(
            identity,
            0,
            true,
            "NodeNotReady",
            format!("Node {} status is now: NodeNotReady", identity.name),
            1,
            (now - 3_000, now - 3_000),
            "node-controller".into(),
        )],
        "certificates.cert-manager.io" => {
            let (_, name, ..) = CERTIFICATES[ix];
            let issued = if row.cells[1] == "True" {
                (
                    "Issuing",
                    "The certificate has been successfully issued".to_owned(),
                )
            } else {
                (
                    "Requested",
                    format!("Created new CertificateRequest resource \"{name}-1\""),
                )
            };
            [
                ("Issuing", certificate_status(false).to_owned()),
                (
                    "Generated",
                    "Stored new private key in temporary Secret resource".to_owned(),
                ),
                issued,
            ]
            .into_iter()
            .enumerate()
            .map(|(n, (reason, message))| {
                let at = created + 60 + n as i64;
                event(
                    identity,
                    n,
                    false,
                    reason,
                    message,
                    1,
                    (at, at),
                    "cert-manager-certificates-issuing".into(),
                )
            })
            .collect()
        }
        _ => Vec::new(),
    }
}

/// One value of an example Secret, as revealing it would read it.
pub(crate) fn secret_value(
    identity: &ResourceIdentity,
    key: &str,
    now: i64,
) -> Option<SecretValue> {
    let (_, ix) = find(identity, now)?;
    let (_, _, _, data) = SECRETS.get(ix)?;
    let (_, value) = data.iter().find(|(name, _)| *name == key)?;
    Some(match String::from_utf8(value.to_vec()) {
        Ok(text) => SecretValue::Text(text),
        Err(error) => SecretValue::Binary(error.into_bytes().len()),
    })
}
