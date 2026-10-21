//! GET-only, bounded API discovery and typed lists. No raw objects escape.
use super::{cnpg::*, *};
use chrono::{DateTime, Utc};
use kube::Client;
use serde::{Deserialize, de::DeserializeOwned};
use std::time::Duration;

const PAGE_SIZE: usize = 200;
const MAX_PAGES: usize = 5;
const REQUEST_DEADLINE: Duration = Duration::from_secs(5);
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

async fn get<T: DeserializeOwned>(client: &Client, path: String) -> Result<T, Coverage> {
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
    for _ in 0..MAX_PAGES {
        let path = {
            let mut query = url::form_urlencoded::Serializer::new(String::new());
            query.append_pair("limit", &PAGE_SIZE.to_string());
            if !next.is_empty() {
                query.append_pair("continue", &next);
            }
            format!("{path}?{}", query.finish())
        };
        let result = get::<List<T>>(client, path).await;
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
/// API discovery is not an operator-health probe. Only the v1 projection is supported.
pub async fn read(client: &Client, namespace: Option<&str>, at: DateTime<Utc>) -> Inventory {
    if namespace.is_some_and(|namespace| !valid_namespace(namespace)) {
        return Inventory::unavailable(Coverage::Failed, at);
    }
    let group = match get::<Group>(client, format!("/apis/{GROUP}")).await {
        Ok(group) => group,
        Err(coverage) => return Inventory::unavailable(coverage, at),
    };
    if !group.versions.iter().any(|version| version.version == "v1") {
        return Inventory::unavailable(Coverage::Unsupported, at);
    }
    let resources = match get::<Resources>(client, format!("/apis/{GROUP}/v1")).await {
        Ok(resources) => resources,
        Err(coverage) => return Inventory::unavailable(coverage, at),
    };
    let available = |plural: &str, kind: &str| {
        resources.resources.iter().any(|r| {
            r.name == plural
                && r.kind == kind
                && r.namespaced
                && r.verbs.iter().any(|v| v == "list")
        })
    };
    let prefix = match namespace {
        Some(namespace) => {
            format!("/apis/{GROUP}/v1/namespaces/{namespace}")
        }
        None => format!("/apis/{GROUP}/v1"),
    };
    let databases = async {
        if available("clusters", "Cluster") {
            list(client, &format!("{prefix}/clusters"), at, Cluster::project).await
        } else {
            Read::empty(Coverage::NotServed, at)
        }
    };
    let backups = async {
        if available("backups", "Backup") {
            list(
                client,
                &format!("{prefix}/backups"),
                at,
                BackupObject::project,
            )
            .await
        } else {
            Read::empty(Coverage::NotServed, at)
        }
    };
    let schedules = async {
        if available("scheduledbackups", "ScheduledBackup") {
            list(
                client,
                &format!("{prefix}/scheduledbackups"),
                at,
                ScheduleObject::project,
            )
            .await
        } else {
            Read::empty(Coverage::NotServed, at)
        }
    };
    let (databases, backups, schedules) = tokio::join!(databases, backups, schedules);
    Inventory {
        discovery: Coverage::Complete,
        databases,
        backups,
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
