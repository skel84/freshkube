//! Read-only probe of the resource browsing backend against a real cluster:
//! connects to an explicitly named context, lists one kind as a server-side
//! table, watches it for a few seconds and fetches one object's YAML. It only
//! lists, watches and gets, and prints no credentials, no file contents and
//! no YAML; for the YAML it prints only checks on it.
//!
//! ```sh
//! FRESHKUBE_KUBECONFIG=<file> FRESHKUBE_CONTEXT=<name> \
//!     cargo run -p freshkube-core --example probe -- pods [seconds]
//! ```
//!
//! `FRESHKUBE_NAMESPACE` limits the listing to one namespace.

use std::collections::BTreeSet;
use std::time::Duration;

use freshkube_core::resources::{
    WatchEvent, builtin, connect, discover_contexts, get_object_yaml, kubeconfig_sources,
    watch_collection,
};

#[tokio::main]
async fn main() {
    let mut args = std::env::args().skip(1);
    let key = args.next().unwrap_or_else(|| "pods".into());
    let seconds: u64 = args
        .next()
        .map_or(6, |value| value.parse().expect("seconds"));
    let kind = builtin(&key).unwrap_or_else(|| panic!("unknown kind {key}"));
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
    let yaml = get_object_yaml(
        &connection.client,
        &kind,
        object.namespace.as_deref(),
        &object.name,
    )
    .await
    .expect("get yaml");
    let parsed: serde_yaml::Value = serde_yaml::from_str(&yaml).expect("yaml parses");
    let values_redacted = ["data", "stringData"].iter().all(|field| {
        parsed
            .get(field)
            .and_then(|map| map.as_mapping())
            .is_none_or(|map| {
                map.values()
                    .all(|value| value.as_str() == Some("<redacted>"))
            })
    });
    println!(
        "yaml of {}: {} line(s), managedFields absent {}, values redacted or none {}",
        object.name,
        yaml.lines().count(),
        !yaml.contains("managedFields"),
        values_redacted
    );
}
