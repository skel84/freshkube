use super::objects::{deployment_yaml, node_yaml, pod_yaml, time, timestamp};
use super::*;

pub(super) fn extra_objects(context: &str, key: &str, now: i64) -> Vec<serde_json::Value> {
    use serde_json::json;
    let meta = |name: &str, namespace: &str| json!({"name":name,"namespace":namespace,"uid":format!("{key}-{namespace}-{name}"),"resourceVersion":"1","creationTimestamp":timestamp(now-720)});
    match key {
        "replicasets.apps" => WORKLOADS.iter().enumerate().map(|(ix, (namespace, app, image, replicas))| {
            let hash=format!("{:x}",0x6c4f_8d9b+ix);
            let mut metadata=meta(&format!("{app}-{hash}"),namespace);
            metadata["ownerReferences"]=json!([{"apiVersion":"apps/v1","kind":"Deployment","name":app,"uid":identity(&connection(context), "deployments.apps", namespace, app, ix).uid,"controller":true}]);
            json!({"apiVersion":"apps/v1","kind":"ReplicaSet","metadata":metadata,"spec":{"replicas":replicas,"selector":{"matchLabels":{"app":app}},"template":{"metadata":{"labels":{"app":app}},"spec":{"containers":[{"name":app,"image":image}]}}},"status":{"replicas":replicas,"readyReplicas":replicas}})
        }).collect(),
        "persistentvolumeclaims" => vec![
            json!({"apiVersion":"v1","kind":"PersistentVolumeClaim","metadata":meta("report-data","batch"),"spec":{"storageClassName":"fast","accessModes":["ReadWriteOnce"],"resources":{"requests":{"storage":"10Gi"}}},"status":{"phase":"Pending"}}),
            json!({"apiVersion":"v1","kind":"PersistentVolumeClaim","metadata":meta("ledger-data","payments"),"spec":{"accessModes":["ReadWriteOnce"],"resources":{"requests":{"storage":"20Gi"}},"volumeName":"ledger-volume"},"status":{"phase":"Bound"}})
        ],
        "persistentvolumes" => (0..3).map(|ix| json!({"apiVersion":"v1","kind":"PersistentVolume","metadata":meta(&format!("volume-{ix}"),""),"spec":{"capacity":{"storage":"20Gi"},"accessModes":["ReadWriteOnce"],"hostPath":{"path":format!("/var/example/{ix}")}},"status":{"phase":if ix==0 {"Bound"} else {"Available"}}})).collect(),
        "events" => {
            let mut result=Vec::new();
            for key in ["pods","deployments.apps"] {
                let (_, rows)=read(context,key,None,now).unwrap();
                for row in rows {
                    for event in events(&row.identity,now) {
                        result.push(json!({"apiVersion":"v1","kind":"Event","metadata":meta(&event.uid,&row.identity.namespace),"involvedObject":{"apiVersion":if key=="pods" {"v1"} else {"apps/v1"},"kind":if key=="pods" {"Pod"} else {"Deployment"},"name":row.identity.name,"namespace":row.identity.namespace,"uid":row.identity.uid},"type":event.event_type,"reason":event.reason,"message":event.message,"count":event.count,"firstTimestamp":event.first_seen,"lastTimestamp":event.last_seen}));
                    }
                }
            }
            result.push(json!({"apiVersion":"v1","kind":"Event","metadata":meta("report-data.provisioning","batch"),"involvedObject":{"apiVersion":"v1","kind":"PersistentVolumeClaim","name":"report-data","namespace":"batch"},"type":"Warning","reason":"ProvisioningFailed","message":"StorageClass fast not found","lastTimestamp":timestamp(now-60)}));
            result
        }
        _=>Vec::new(),
    }
}

pub(super) fn extra_rows(
    connection: &str,
    context: &str,
    key: &str,
    now: i64,
) -> (Vec<ResourceColumn>, Vec<ResourceRow>) {
    let columns = vec![
        ResourceColumn::new("Name", ColumnKind::Text, false),
        ResourceColumn::new(
            if key == "events" { "Type" } else { "Status" },
            ColumnKind::Text,
            false,
        ),
        ResourceColumn::new(
            if key == "events" { "Reason" } else { "Details" },
            ColumnKind::Text,
            false,
        ),
    ];
    let rows = extra_objects(context, key, now)
        .into_iter()
        .enumerate()
        .map(|(ix, object)| {
            let name = object["metadata"]["name"].as_str().unwrap();
            let namespace = object["metadata"]["namespace"].as_str().unwrap_or_default();
            let status = if key == "events" {
                object["type"].as_str().unwrap_or_default().to_owned()
            } else {
                object["status"]["phase"]
                    .as_str()
                    .unwrap_or("Ready")
                    .to_owned()
            };
            let details = if key == "events" {
                object["reason"].as_str().unwrap_or_default().to_owned()
            } else {
                object["spec"]["storageClassName"]
                    .as_str()
                    .unwrap_or_default()
                    .to_owned()
            };
            let mut id = identity(connection, key, namespace, name, ix);
            id.uid = object["metadata"]["uid"].as_str().unwrap().to_owned();
            ResourceRow {
                identity: id,
                cells: vec![name.into(), status, details],
                created: Some(now - 720),
                terminating: false,
                resource_version: EXAMPLE_VERSION.into(),
                owner: None,
                generated: None,
                pod: None,
            }
        })
        .collect();
    (columns, rows)
}

/// Unit-test snapshots use the same reducer as the running fixture.
#[cfg(test)]
pub(crate) fn summary(
    context: &str,
    now: i64,
) -> freshkube_core::kubernetes_summary::KubernetesSummary {
    use freshkube_core::kubernetes_summary::{Session, SessionIdentity};
    let session = Session::new(SessionIdentity::new(connection(context), 0));
    seed_summary(&session, context, now);
    (*session.derive(time(now).unwrap()).summary).clone()
}

pub(crate) fn summary_objects<T: serde::de::DeserializeOwned>(
    context: &str,
    key: &str,
    now: i64,
) -> Vec<T> {
    if matches!(
        key,
        "events" | "replicasets.apps" | "persistentvolumeclaims" | "persistentvolumes"
    ) {
        return extra_objects(context, key, now)
            .into_iter()
            .map(|object| serde_json::from_value(object).expect("typed example object"))
            .collect();
    }
    let objects = read(context, key, None, now)
        .unwrap()
        .1
        .into_iter()
        .enumerate()
        .map(|(ix, row)| {
            let created = row.created.unwrap_or_default();
            let yaml = match key {
                "pods" => pod_yaml(&row, ix, created, now),
                "nodes" => node_yaml(&row, created),
                "deployments.apps" => deployment_yaml(&row, ix, created),
                _ => document(&row.identity, now).expect("example document").yaml,
            };
            serde_yaml::from_str(&yaml).expect("typed example object")
        })
        .collect();
    objects
}

/// Seed exactly the reducer used by live list/watch streams. The caller keeps
/// `now` fixed for the lifetime of a fixture session.
pub(crate) fn seed_summary(
    session: &freshkube_core::kubernetes_summary::Session,
    context: &str,
    now: i64,
) {
    use freshkube_core::kubernetes_summary::SummaryResource;
    use k8s_openapi::api::{
        apps::v1::{DaemonSet, Deployment, StatefulSet},
        core::v1::{Event, Namespace, Node, PersistentVolume, PersistentVolumeClaim, Pod},
    };
    use kube::runtime::watcher::Event as Change;
    fn seed<K: SummaryResource>(
        session: &freshkube_core::kubernetes_summary::Session,
        objects: Vec<K>,
        now: i64,
    ) {
        let generation = session.generation();
        session
            .apply::<K>(generation, Change::Init, time(now).unwrap())
            .unwrap();
        for object in objects {
            session
                .apply(generation, Change::InitApply(object), time(now).unwrap())
                .unwrap();
        }
        session
            .apply::<K>(generation, Change::InitDone, time(now).unwrap())
            .unwrap();
    }
    seed::<Node>(session, summary_objects(context, "nodes", now), now);
    seed::<Pod>(session, summary_objects(context, "pods", now), now);
    seed::<Deployment>(
        session,
        summary_objects(context, "deployments.apps", now),
        now,
    );
    seed::<StatefulSet>(session, vec![], now);
    seed::<DaemonSet>(session, vec![], now);
    seed::<Namespace>(session, summary_objects(context, "namespaces", now), now);
    seed::<PersistentVolumeClaim>(
        session,
        summary_objects(context, "persistentvolumeclaims", now),
        now,
    );
    seed::<PersistentVolume>(
        session,
        summary_objects(context, "persistentvolumes", now),
        now,
    );
    seed::<Event>(session, summary_objects(context, "events", now), now);
    session.version(session.generation(), "v1.32.3".into(), time(now).unwrap());
}
