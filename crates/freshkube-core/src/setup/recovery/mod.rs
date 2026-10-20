//! Recovery is only for YAML disk reads. Strict parse/save/migration input
//! still refuses an invalid row. Raw credential checks precede every decode.
use serde::de::DeserializeOwned;
use serde_yaml::Value;

use super::{Fault, Invalid, Read, Setup, place, validate};
use crate::workspace::{
    self, Destination, Entry, MAX_BYTES, MAX_DESTINATIONS, MAX_ENTRIES, Workspace,
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Recovered {
    pub setup: Setup,
    pub warnings: Vec<Warning>,
    /// An explicit, unusable cluster list must never become an open connection.
    pub all_clusters_skipped: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Row {
    Cluster,
    Destination,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RowReason {
    Shape,
    Workspace(workspace::Invalid),
    Rule(Fault),
    Destination(workspace::Fault),
    CorootWithoutProject,
    CorootTwice { first: String },
}

/// Only row identities and typed, safe reasons survive decoding. Never store
/// parser messages, rejected values, endpoints or credential/path values here.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Warning {
    pub row: Row,
    pub index: usize,
    pub id: Option<String>,
    pub field: String,
    pub reason: RowReason,
}

impl std::fmt::Display for Warning {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let collection = match self.row {
            Row::Cluster => "clusters",
            Row::Destination => "destinations",
        };
        write!(f, "skipped {collection}[{}]", self.index)?;
        if let Some(id) = &self.id {
            write!(f, " ({id})")?;
        }
        write!(f, ": {}: ", self.field)?;
        match &self.reason {
            RowReason::Shape => {
                f.write_str("a required value is missing or has the wrong type or shape")
            }
            RowReason::Workspace(why) => match why {
                workspace::Invalid::EmptyId
                | workspace::Invalid::EmptyContext(_)
                | workspace::Invalid::EmptyTalosContext(_) => f.write_str("is empty"),
                workspace::Invalid::LongId(_) => f.write_str("is longer than 64 bytes"),
                workspace::Invalid::TalosContextWithoutConfig(_) => {
                    f.write_str("requires talosconfig")
                }
                workspace::Invalid::RelativePath(_) => f.write_str("isn’t an absolute path"),
                _ => unreachable!("only row-local workspace faults become warnings"),
            },
            RowReason::Rule(why) => {
                let words = why.to_string();
                f.write_str(
                    words
                        .strip_prefix(&self.field)
                        .unwrap_or(&words)
                        .trim_start_matches(':')
                        .trim_start(),
                )
            }
            RowReason::Destination(why) => f.write_str(&why.words()),
            RowReason::CorootWithoutProject => {
                f.write_str("neither this link nor coroot names a project")
            }
            RowReason::CorootTwice { first } => {
                write!(f, "duplicates the Coroot link of cluster “{first}”")
            }
        }
    }
}

impl Recovered {
    pub fn warning_words(&self) -> Option<String> {
        warning_words(&self.warnings)
    }
}

pub fn warning_words(warnings: &[Warning]) -> Option<String> {
    (!warnings.is_empty()).then(|| {
        format!(
            "freshkube.yaml is used with warnings: {}. The rest of the file stays in use.",
            warnings
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>()
                .join("; ")
        )
    })
}

/// Disk-read policy: retain healthy rows, preserving original indexes and bytes
/// at the caller. Every shared refusal takes precedence over row recovery.
pub fn read(bytes: &[u8]) -> Result<Recovered, Invalid> {
    if bytes.len() as u64 > MAX_BYTES {
        return Err(Invalid::TooLarge);
    }
    let value: Value = serde_yaml::from_slice(bytes).map_err(|e| Invalid::NotYaml(place(&e)))?;
    credential_preflight(&value)?;
    if !value.is_mapping() {
        return Err(Invalid::WrongShape(None));
    }
    super::check_version(&value)?;
    let shared: Read<Value, Value> =
        serde_yaml::from_slice(bytes).map_err(|e| Invalid::WrongShape(place(&e)))?;
    let clusters = shared.clusters;
    let destinations = shared.destinations;
    if clusters.len() > MAX_ENTRIES {
        return Err(Invalid::Workspace(workspace::Invalid::TooManyEntries));
    }
    if destinations.len() > MAX_DESTINATIONS {
        return Err(Invalid::Workspace(workspace::Invalid::TooManyDestinations));
    }
    // Duplicate readable ids are fatal even when either row cannot decode.
    let mut ids = std::collections::HashSet::new();
    for row in &clusters {
        if let Some(id) = row.get("id").and_then(Value::as_str)
            && !ids.insert(id)
        {
            return Err(Invalid::Workspace(workspace::Invalid::DuplicateId(
                id.to_owned(),
            )));
        }
    }
    // Strictly parse all shared configuration, including version and pins.
    // Rows decode as raw values here, so no local failure masks a shared one.
    let mut workspace = Workspace::new(Vec::new());
    workspace.kubeconfig = shared.kubeconfig;
    workspace.sources = shared.sources;
    let mut setup = Setup {
        workspace,
        metrics: shared.metrics,
        coroot: shared.coroot,
        supply_chain: shared.supply_chain,
        extra: shared.extra,
    };
    setup.validate()?;
    let mut warnings = Vec::new();
    for (index, row) in clusters.iter().enumerate() {
        let id = safe_id(row, "id");
        let result = decode_cluster(row)
            .and_then(|entry| cluster_fault(&setup, &entry).map_or(Ok(entry), Err));
        match result {
            Ok(entry) => setup.workspace.clusters.push(entry),
            Err((field, reason)) => warnings.push(Warning {
                row: Row::Cluster,
                index,
                id,
                field,
                reason,
            }),
        }
    }
    let all_clusters_skipped = !clusters.is_empty() && setup.workspace.clusters.is_empty();
    for (index, row) in destinations.into_iter().enumerate() {
        let id = safe_id(&row, "entry");
        let result = serde_yaml::from_value::<Destination>(row.clone())
            .map_err(|_| (destination_shape_field(&row).into(), RowReason::Shape))
            .and_then(|destination| {
                if let Some(fault) = destination.fault_in(&setup.workspace) {
                    let field = match &fault {
                        workspace::Fault::ServerAndName | workspace::Fault::Neither => {
                            "server/name"
                        }
                        workspace::Fault::EmptyServer | workspace::Fault::NotHttp => "server",
                        workspace::Fault::EmptyName => "name",
                        workspace::Fault::Spaces => "server/name",
                        _ => "entry",
                    };
                    Err((field.into(), RowReason::Destination(fault)))
                } else {
                    Ok(destination)
                }
            });
        match result {
            Ok(destination) => setup.workspace.destinations.push(destination),
            Err((field, reason)) => warnings.push(Warning {
                row: Row::Destination,
                index,
                id,
                field,
                reason,
            }),
        }
    }
    // Arrays retain original indexes even when the collections appear in a
    // different order in a hand-edited document.
    let destinations_first = value
        .as_mapping()
        .unwrap()
        .keys()
        .filter_map(Value::as_str)
        .find(|key| matches!(*key, "clusters" | "destinations"))
        == Some("destinations");
    if destinations_first {
        warnings.sort_by_key(|warning| (warning.row == Row::Cluster, warning.index));
    }
    Ok(Recovered {
        setup,
        warnings,
        all_clusters_skipped,
    })
}

fn safe_id(row: &Value, key: &str) -> Option<String> {
    row.get(key)
        .and_then(Value::as_str)
        .filter(|id| !id.trim().is_empty() && id.len() <= 64 && !id.chars().any(char::is_control))
        .map(str::to_owned)
}

type RowFault = (String, RowReason);

fn cluster_fault(setup: &Setup, entry: &Entry) -> Option<RowFault> {
    if let Err(why) = Workspace::new(vec![entry.clone()]).validate() {
        let field = match &why {
            workspace::Invalid::EmptyId | workspace::Invalid::LongId(_) => "id",
            workspace::Invalid::EmptyContext(_) => "context",
            workspace::Invalid::EmptyTalosContext(_)
            | workspace::Invalid::TalosContextWithoutConfig(_) => "talos_context",
            workspace::Invalid::RelativePath(_) => "talosconfig",
            _ => unreachable!("a single entry has only row-local workspace faults"),
        };
        // The id is already carried safely on Warning, never in its reason.
        let why = match why {
            workspace::Invalid::LongId(_) => workspace::Invalid::LongId(String::new()),
            workspace::Invalid::EmptyContext(_) => workspace::Invalid::EmptyContext(String::new()),
            workspace::Invalid::EmptyTalosContext(_) => {
                workspace::Invalid::EmptyTalosContext(String::new())
            }
            workspace::Invalid::TalosContextWithoutConfig(_) => {
                workspace::Invalid::TalosContextWithoutConfig(String::new())
            }
            workspace::Invalid::RelativePath(_) => workspace::Invalid::RelativePath(String::new()),
            why => why,
        };
        return Some((field.into(), RowReason::Workspace(why)));
    }
    // Use field-only validation words; never embed an unvalidated id in a reason.
    if let Err(why) = validate::entry_fields(entry) {
        let field = match &why {
            Fault::Empty(at)
            | Fault::NotUrl(at)
            | Fault::NonLoopbackHttp(at)
            | Fault::RelativePath(at)
            | Fault::ZeroPort(at) => at,
            Fault::OneOf { at, .. } | Fault::Only { at, .. } => at,
            _ => unreachable!("entry field validation"),
        }
        .clone();
        return Some((field, RowReason::Rule(why)));
    }
    let mut entries = setup.workspace.clusters.clone();
    entries.push(entry.clone());
    match validate::coroot_links(
        &entries,
        setup.coroot.as_ref().and_then(|c| c.project.as_deref()),
    ) {
        Err(Fault::CorootWithoutProject(_)) => {
            Some(("coroot.project".into(), RowReason::CorootWithoutProject))
        }
        Err(Fault::CorootTwice { first, .. }) => {
            Some(("coroot".into(), RowReason::CorootTwice { first }))
        }
        _ => None,
    }
}

fn credential_preflight(value: &Value) -> Result<(), Invalid> {
    let check = |block: &Value, token: &str, endpoint: &str| -> Result<(), Invalid> {
        if block
            .get(token)
            .is_some_and(|v| !v.is_null() && v.as_str() != Some("keys.json"))
        {
            return Err(Invalid::KeyValue(None));
        }
        if let Some(address) = block.get(endpoint).and_then(Value::as_str) {
            // Check authority even if the rest of a malformed URL won't parse.
            let authority = address
                .split_once("://")
                .map(|(_, rest)| rest.split(['/', '?', '#']).next().unwrap_or_default());
            if authority.is_some_and(|authority| authority.contains('@')) {
                return Err(Invalid::KeyValue(None));
            }
        }
        Ok(())
    };
    if let Some(coroot) = value.get("coroot") {
        check(coroot, "key", "server")?;
    }
    if let Some(store) = value.get("metrics").and_then(|v| v.get("store")) {
        check(store, "token", "url")?;
    }
    if let Some(rows) = value.get("clusters").and_then(Value::as_sequence) {
        for row in rows {
            if let Some(results) = row.get("results") {
                check(results, "token", "address")?;
            }
            if let Some(source) = row.get("metrics").and_then(|v| v.get("source")) {
                check(source, "token", "url")?;
            }
        }
    }
    if let Some(rows) = value.get("destinations").and_then(Value::as_sequence) {
        for row in rows {
            check(row, "__no_token_field", "server")?;
        }
    }
    Ok(())
}

// The type-directed shape checks name only schema fields. Parser messages can
// contain rejected values, so they are never inspected or retained.
fn wrong<T: DeserializeOwned>(row: &Value, key: &str, required: bool) -> bool {
    match row.get(key) {
        None => required,
        Some(value) if value.is_null() && !required => false,
        Some(value) => serde_yaml::from_value::<T>(value.clone()).is_err(),
    }
}

fn decode_cluster(row: &Value) -> Result<Entry, RowFault> {
    serde_yaml::from_value(row.clone()).map_err(|_| (cluster_shape_field(row), RowReason::Shape))
}

fn cluster_shape_field(row: &Value) -> String {
    for key in ["id", "context"] {
        if wrong::<String>(row, key, true) {
            return key.into();
        }
    }
    if wrong::<workspace::Role>(row, "role", true) {
        return "role".into();
    }
    for key in ["talosconfig", "talos_context"] {
        if wrong::<String>(row, key, false) {
            return key.into();
        }
    }
    if wrong::<super::Results>(row, "results", false) {
        return block_field(
            row,
            "results",
            &[("address", false), ("ca_file", false)],
            Some(("forward", &["namespace", "service"])),
        );
    }
    if wrong::<super::EntryCoroot>(row, "coroot", false) {
        return block_field(
            row,
            "coroot",
            &[("cluster", true), ("project", false)],
            None,
        );
    }
    if wrong::<super::EntryMetrics>(row, "metrics", false) {
        if let Some(metrics) = row.get("metrics") {
            if wrong::<String>(metrics, "value", false) {
                return "metrics.value".into();
            }
            if wrong::<super::Source>(metrics, "source", false) {
                return format!(
                    "metrics.{}",
                    block_field(
                        metrics,
                        "source",
                        &[("url", false)],
                        Some(("service", &["namespace", "name"]))
                    )
                );
            }
        }
        return "metrics".into();
    }
    "row".into()
}

fn block_field(
    row: &Value,
    key: &str,
    strings: &[(&str, bool)],
    nested: Option<(&str, &[&str])>,
) -> String {
    let Some(block) = row.get(key).filter(|v| v.is_mapping()) else {
        return key.into();
    };
    for &(field, required) in strings {
        if wrong::<String>(block, field, required) {
            return format!("{key}.{field}");
        }
    }
    if let Some((child, required)) = nested
        && let Some(value) = block.get(child).filter(|v| !v.is_null())
    {
        if !value.is_mapping() {
            return format!("{key}.{child}");
        }
        for field in required {
            if wrong::<String>(value, field, true) {
                return format!("{key}.{child}.{field}");
            }
        }
        if wrong::<u16>(value, "port", true) {
            return format!("{key}.{child}.port");
        }
        if wrong::<bool>(value, "https", false) {
            return format!("{key}.{child}.https");
        }
        if wrong::<String>(value, "path", false) {
            return format!("{key}.{child}.path");
        }
    }
    key.into()
}

fn destination_shape_field(row: &Value) -> &'static str {
    for key in ["server", "name", "entry"] {
        if wrong::<String>(row, key, false) {
            return key;
        }
    }
    "row"
}

#[cfg(test)]
mod tests;
