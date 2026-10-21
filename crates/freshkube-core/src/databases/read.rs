//! GET-only, bounded API discovery and typed lists. No raw objects escape.
use super::*;
use chrono::{DateTime, Utc};
use kube::Client;
use serde::{Deserialize, de::DeserializeOwned};
use std::time::Duration;

const PAGE_SIZE: usize = 200;
const MAX_PAGES: usize = 5;
const REQUEST_DEADLINE: Duration = Duration::from_secs(5);
const KIND_DEADLINE: Duration = Duration::from_secs(15);
/// Two discovery requests plus independent, concurrent per-kind budgets.
pub const READ_BUDGET: Duration =
    Duration::from_secs(REQUEST_DEADLINE.as_secs() * 2 + KIND_DEADLINE.as_secs());
#[derive(Deserialize)]
struct Group {
    versions: Vec<Version>,
}
#[derive(Deserialize)]
struct Version {
    version: String,
}
#[derive(Deserialize)]
struct Resources {
    resources: Vec<Resource>,
}
#[derive(Deserialize)]
struct Resource {
    name: String,
    kind: String,
    namespaced: bool,
    verbs: Vec<String>,
}
#[derive(Deserialize)]
struct List<T> {
    items: Vec<T>,
    #[serde(default)]
    metadata: ListMeta,
}
#[derive(Default, Deserialize)]
struct ListMeta {
    #[serde(rename = "continue", default)]
    next: String,
}

pub(super) async fn get<T: DeserializeOwned>(client: &Client, path: String) -> Result<T, Coverage> {
    let request = http::Request::builder()
        .method("GET")
        .uri(path)
        .body(Vec::new())
        .map_err(|_| Coverage::Failed)?;
    tokio::time::timeout(REQUEST_DEADLINE, client.request(request))
        .await
        .map_err(|_| Coverage::TimedOut)?
        .map_err(|error| match error {
            kube::Error::Api(ref status) => match status.code {
                401 | 403 => Coverage::Refused,
                404 => Coverage::NotServed,
                _ => Coverage::Failed,
            },
            kube::Error::HyperError(_) | kube::Error::Service(_) => Coverage::Unreachable,
            _ => Coverage::Failed,
        })
}
async fn list<T: DeserializeOwned, U>(
    client: &Client,
    path: &str,
    at: DateTime<Utc>,
    project: fn(T) -> Option<U>,
) -> Read<U> {
    let mut answer = Read::empty(Coverage::Complete, at);
    let mut next = String::new();
    let deadline = tokio::time::Instant::now() + KIND_DEADLINE;
    for _ in 0..MAX_PAGES {
        let path = {
            let mut query = url::form_urlencoded::Serializer::new(String::new());
            query.append_pair("limit", &PAGE_SIZE.to_string());
            if !next.is_empty() {
                query.append_pair("continue", &next);
            }
            format!("{path}?{}", query.finish())
        };
        let result = tokio::time::timeout_at(deadline, get::<List<T>>(client, path))
            .await
            .unwrap_or(Err(Coverage::TimedOut));
        let page = match result {
            Ok(page) => page,
            Err(coverage) => {
                answer.coverage = coverage;
                return answer;
            }
        };
        if page.items.len() > PAGE_SIZE {
            answer.coverage = Coverage::Partial;
        }
        for item in page.items.into_iter().take(PAGE_SIZE) {
            match project(item) {
                Some(item) => answer.items.push(item),
                None => answer.coverage = Coverage::Partial,
            }
        }
        next = page.metadata.next;
        if next.is_empty() {
            return answer;
        }
    }
    answer.coverage = Coverage::Partial;
    answer
}
/// The group and version one operator's reader supports.
struct Operator {
    group: &'static str,
    version: &'static str,
}
const CNPG: Operator = Operator {
    group: cnpg::GROUP,
    version: cnpg::VERSION,
};
const MARIADB: Operator = Operator {
    group: mariadb::GROUP,
    version: mariadb::VERSION,
};

/// What discovery found of one operator's API.
struct Served {
    resources: Resources,
    prefix: String,
}
impl Served {
    fn has(&self, plural: &str, kind: &str) -> bool {
        self.resources.resources.iter().any(|r| {
            r.name == plural
                && r.kind == kind
                && r.namespaced
                && r.verbs.iter().any(|v| v == "list")
        })
    }
    fn path(&self, plural: &str) -> String {
        format!("{}/{plural}", self.prefix)
    }
}

/// API discovery is not an operator-health probe.
async fn discover(
    client: &Client,
    operator: &Operator,
    namespace: Option<&str>,
) -> Result<Served, Coverage> {
    let group = get::<Group>(client, format!("/apis/{}", operator.group)).await?;
    if !group.versions.iter().any(|v| v.version == operator.version) {
        return Err(Coverage::Unsupported);
    }
    let resources = get::<Resources>(
        client,
        format!("/apis/{}/{}", operator.group, operator.version),
    )
    .await?;
    let base = format!("/apis/{}/{}", operator.group, operator.version);
    let prefix = match namespace {
        Some(namespace) => format!("{base}/namespaces/{namespace}"),
        None => base,
    };
    Ok(Served { resources, prefix })
}

/// Lists one kind when the API serves it, else NotServed.
async fn kind<T: DeserializeOwned, U>(
    client: &Client,
    served: &Served,
    (plural, kind): (&str, &str),
    at: DateTime<Utc>,
    project: fn(T) -> Option<U>,
) -> Read<U> {
    if served.has(plural, kind) {
        list(client, &served.path(plural), at, project).await
    } else {
        Read::empty(Coverage::NotServed, at)
    }
}

/// Reads CloudNativePG and the MariaDB operator at once; each is installed
/// or not on its own, and one's refusal never hides the other.
pub async fn read(client: &Client, namespace: Option<&str>, at: DateTime<Utc>) -> Inventory {
    if namespace.is_some_and(|namespace| !valid_namespace(namespace)) {
        return Inventory::unavailable(Coverage::Failed, at);
    }
    let (cnpg, mariadb) = tokio::join!(
        read_cnpg(client, namespace, at),
        read_mariadb(client, namespace, at)
    );
    Inventory::merge(
        vec![(Engine::CloudNativePg, cnpg), (Engine::MariaDb, mariadb)],
        at,
    )
}

async fn read_cnpg(client: &Client, namespace: Option<&str>, at: DateTime<Utc>) -> Inventory {
    let served = match discover(client, &CNPG, namespace).await {
        Ok(served) => served,
        Err(coverage) => return Inventory::unavailable(coverage, at),
    };
    let (mut databases, backups, schedules, poolers) = tokio::join!(
        kind(
            client,
            &served,
            ("clusters", "Cluster"),
            at,
            cnpg::Cluster::project
        ),
        kind(
            client,
            &served,
            ("backups", "Backup"),
            at,
            cnpg::BackupObject::project
        ),
        kind(
            client,
            &served,
            ("scheduledbackups", "ScheduledBackup"),
            at,
            cnpg::ScheduleObject::project
        ),
        kind(
            client,
            &served,
            ("poolers", "Pooler"),
            at,
            cnpg::PoolerObject::project
        ),
    );
    for db in &mut databases.items {
        db.proxy = cnpg::proxy(db, &poolers);
    }
    Inventory {
        discovery: Coverage::Complete,
        engines: Vec::new(),
        databases,
        backups,
        schedules,
    }
}

async fn read_mariadb(client: &Client, namespace: Option<&str>, at: DateTime<Utc>) -> Inventory {
    let served = match discover(client, &MARIADB, namespace).await {
        Ok(served) => served,
        Err(coverage) => return Inventory::unavailable(coverage, at),
    };
    let (databases, backups, scalers) = tokio::join!(
        kind(
            client,
            &served,
            ("mariadbs", "MariaDB"),
            at,
            |o: mariadb::MariaDbObject| o.project()
        ),
        kind(
            client,
            &served,
            ("backups", "Backup"),
            at,
            mariadb::BackupObject::project
        ),
        kind(
            client,
            &served,
            ("maxscales", "MaxScale"),
            at,
            mariadb::MaxScaleObject::project
        ),
    );
    let mut projected = Read {
        items: Vec::new(),
        coverage: databases.coverage,
        observed_at: databases.observed_at,
        stale: false,
    };
    for item in databases.items {
        let mut database = item.database;
        database.proxy = mariadb::proxy(item.max_scale.as_ref(), &scalers);
        projected.items.push(database);
    }
    let mut runs = Read::empty(backups.coverage.clone(), at);
    let mut schedules = Read::empty(backups.coverage, at);
    for item in backups.items {
        match item {
            mariadb::Item::Run(backup) => runs.items.push(backup),
            mariadb::Item::Schedule(schedule) => schedules.items.push(schedule),
        }
    }
    Inventory {
        discovery: Coverage::Complete,
        engines: Vec::new(),
        databases: projected,
        backups: runs,
        schedules,
    }
}

/// Kubernetes namespace names are DNS labels, never API paths or queries.
pub fn valid_namespace(namespace: &str) -> bool {
    !namespace.is_empty()
        && namespace.len() <= 63
        && namespace.as_bytes()[0].is_ascii_alphanumeric()
        && namespace.as_bytes()[namespace.len() - 1].is_ascii_alphanumeric()
        && namespace
            .bytes()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == b'-')
}
