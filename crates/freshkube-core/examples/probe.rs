//! Read-only probe of the resource browsing backend against a real cluster:
//! connects to an explicitly named context, lists one kind as a server-side
//! table, watches it for a few seconds, then reads one object in full and
//! watches its events for a few seconds. It only lists, watches and gets, and
//! prints no credentials, no file contents and no YAML; for the document it
//! prints only checks on it.
//!
//! ```sh
//! FRESHKUBE_KUBECONFIG=<file> FRESHKUBE_CONTEXT=<name> \
//!     cargo run -p freshkube-core --example probe -- pods [seconds]
//! ```
//!
//! `FRESHKUBE_NAMESPACE` limits the listing to one namespace. The kind
//! `custom` instead prints the custom API groups and what each offers; a
//! discovered key such as `certificates.cert-manager.io` lists that kind.

use std::collections::BTreeSet;
use std::time::Duration;

use freshkube_core::resources::{
    EventScope, EventUpdate, ResourceKind, WatchEvent, builtin, connect, discover_contexts,
    get_object, kubeconfig_sources, list_custom_groups, list_group_kinds, watch_collection,
    watch_object_events,
};

#[tokio::main]
async fn main() {
    let mut args = std::env::args().skip(1);
    let key = args.next().unwrap_or_else(|| "pods".into());
    let seconds: u64 = args
        .next()
        .map_or(6, |value| value.parse().expect("seconds"));
    let namespace = std::env::var("FRESHKUBE_NAMESPACE").ok();
    // Verification never falls back to the current context.
    let wanted = std::env::var("FRESHKUBE_CONTEXT").expect("set FRESHKUBE_CONTEXT");

    let sources = kubeconfig_sources(None);
    let report = tokio::task::spawn_blocking({
        let sources = sources.clone();
        move || discover_contexts(&sources)
    })
    .await
    .unwrap();
    println!(
        "{} source(s), {} context(s), {} unreadable",
        report.sources.len(),
        report.contexts.len(),
        report.errors.len()
    );
    for context in &report.contexts {
        println!("  context {} (cluster {})", context.name, context.cluster);
    }

    let started = std::time::Instant::now();
    let connection = connect(sources, wanted).await.expect("connect");
    println!(
        "connected to {} in {:?}: server {}",
        connection.context.name,
        started.elapsed(),
        connection.server_version
    );

    // `custom` lists the custom groups and their kinds; other keys are a
    // built-in kind or a discovered `plural.group`.
    let kind = match builtin(&key) {
        Some(kind) => kind,
        None => match discover(&connection.client, &key).await {
            Some(kind) => kind,
            None => return,
        },
    };

    let (sender, mut receiver) = tokio::sync::mpsc::channel(64);
    let watch = tokio::spawn(watch_collection(
        connection.client.clone(),
        kind.clone(),
        namespace.clone(),
        sender,
    ));
    let deadline = tokio::time::Instant::now() + Duration::from_secs(seconds);
    let mut first = None;
    let (mut batches, mut largest, mut upserts, mut deletes) = (0, 0, 0, 0);
    loop {
        let batch = tokio::select! {
            batch = receiver.recv() => batch,
            _ = tokio::time::sleep_until(deadline) => break,
        };
        let Some(batch) = batch else {
            println!("watch ended");
            break;
        };
        batches += 1;
        largest = largest.max(batch.events.len());
        for event in batch.events {
            match event {
                WatchEvent::Reset { columns, rows } => {
                    let columns: Vec<_> = columns
                        .iter()
                        .map(|column| {
                            let wide = if column.priority > 0 { ",wide" } else { "" };
                            format!("{}[{}{wide}]", column.name, column.column_type)
                        })
                        .collect();
                    println!("reset after {:?}: {}", started.elapsed(), columns.join(" "));
                    let namespaces: BTreeSet<_> = rows
                        .iter()
                        .filter_map(|row| row.metadata()?.namespace.clone())
                        .collect();
                    let missing = rows.iter().filter(|row| row.metadata().is_none()).count();
                    println!(
                        "rows: {} in {} namespace(s), {missing} without metadata",
                        rows.len(),
                        namespaces.len()
                    );
                    for row in rows.iter().take(3) {
                        let metadata = row.metadata().unwrap();
                        println!(
                            "  {}/{} uid {}… created {} cells {:?}",
                            metadata.namespace.as_deref().unwrap_or("-"),
                            metadata.name,
                            &metadata.uid[..8.min(metadata.uid.len())],
                            metadata.creation_timestamp.as_deref().unwrap_or("?"),
                            row.cells
                        );
                    }
                    if first.is_none() {
                        first = rows.first().and_then(|row| row.metadata()).cloned();
                    }
                }
                WatchEvent::Upsert(_) => upserts += 1,
                WatchEvent::Delete(_) => deletes += 1,
                WatchEvent::Failed { failure, retrying } => {
                    println!("failed (retrying {retrying}): {failure}")
                }
            }
        }
    }
    watch.abort();
    println!(
        "in {seconds}s: {batches} batch(es), largest {largest}, {upserts} upsert(s), {deletes} delete(s)"
    );

    let Some(object) = first else {
        return;
    };
    let document = get_object(
        &connection.client,
        &kind,
        object.namespace.as_deref(),
        &object.name,
    )
    .await
    .expect("get object");
    let parsed: serde_yaml::Value = serde_yaml::from_str(&document.yaml).expect("yaml parses");
    let values_hidden = ["data", "stringData"].iter().all(|field| {
        parsed
            .get(field)
            .and_then(|map| map.as_mapping())
            .is_none_or(|map| {
                map.values().all(|value| {
                    value
                        .as_str()
                        .is_some_and(|value| value.starts_with("<hidden,"))
                })
            })
    });
    let overview = &document.overview;
    println!(
        "document of {}: {} line(s), uid matches row {}, managedFields absent {}, Secret values hidden or none {}",
        object.name,
        document.yaml.lines().count(),
        document.uid == object.uid,
        !document.yaml.contains("managedFields"),
        values_hidden || !kind.is_secret()
    );
    println!(
        "overview: {} label(s), {} annotation(s), {} owner(s), {} condition(s), {} secret key(s)",
        overview.labels.len(),
        overview.annotations.len(),
        overview.owners.len(),
        overview.conditions.len(),
        overview
            .secret
            .as_ref()
            .map_or(0, |secret| secret.keys.len())
    );

    let scope = EventScope::new(
        &kind,
        object.namespace.as_deref(),
        &object.name,
        &object.uid,
    );
    let (sender, mut receiver) = tokio::sync::mpsc::channel(64);
    let events = tokio::spawn(watch_object_events(
        connection.client.clone(),
        scope,
        sender,
    ));
    let deadline = tokio::time::Instant::now() + Duration::from_secs(seconds);
    loop {
        let update = tokio::select! {
            update = receiver.recv() => update,
            _ = tokio::time::sleep_until(deadline) => break,
        };
        match update {
            None => break,
            Some(EventUpdate::Reset(events)) => {
                let warnings = events.iter().filter(|event| event.is_warning()).count();
                println!(
                    "events: {} listed, {warnings} warning(s), all timed {}",
                    events.len(),
                    events.iter().all(|event| event.last_seen.is_some())
                );
            }
            Some(EventUpdate::Upsert(_)) => println!("event upserted"),
            Some(EventUpdate::Delete(_)) => println!("event deleted"),
            Some(EventUpdate::Failed { failure, retrying }) => {
                println!("events failed (retrying {retrying}): {failure}")
            }
        }
    }
    events.abort();
}

/// Discovers custom groups. For `custom`, prints each group's kinds and
/// returns nothing; otherwise returns the kind whose key is `key`.
async fn discover(client: &kube::Client, key: &str) -> Option<ResourceKind> {
    let started = std::time::Instant::now();
    let groups = list_custom_groups(client).await.expect("discover groups");
    println!(
        "{} custom group(s) in {:?}",
        groups.len(),
        started.elapsed()
    );
    for group in &groups {
        if key != "custom" && !key.ends_with(&format!(".{}", group.name)) {
            continue;
        }
        match list_group_kinds(client, group).await {
            Ok(found) => {
                if key == "custom" {
                    let kinds: Vec<_> = found
                        .kinds
                        .iter()
                        .map(|kind| format!("{}@{}", kind.kind, kind.version))
                        .collect();
                    println!(
                        "  {} [{}]: {}; unlistable {:?}; failed versions {:?}",
                        group.name,
                        group.versions.join(","),
                        kinds.join(" "),
                        found.unlistable,
                        found.failures
                    );
                } else if let Some(kind) = found.kinds.into_iter().find(|kind| kind.key() == key) {
                    return Some(kind);
                }
            }
            Err(failure) => println!("  {}: {failure}", group.name),
        }
    }
    if key != "custom" {
        println!("no discovered kind {key}");
    }
    None
}
