//! The alert sources' answers as [`model`](super::model) types. Pure: JSON
//! in, values out. A shape a parser doesn't understand is an error naming
//! it, never an empty list, so an answer it can't read is never "no alerts".
use std::collections::{BTreeMap, HashSet};

use chrono::{DateTime, Utc};
use serde_json::Value;

use super::model::{
    Alert, AlertState, Kind, Rule, RuleGroup, RuleHealth, RuleKind, Severity, Silence,
    SilenceMatcher, SilenceState,
};

// String exception: the parsers' messages are shown as the reason an answer can't be read.
type Parsed<T> = Result<T, String>;

fn time(value: Option<&Value>) -> Option<DateTime<Utc>> {
    let at = DateTime::parse_from_rfc3339(value?.as_str()?)
        .ok()?
        .with_timezone(&Utc);
    // Go's zero time, which servers send for "never".
    (at.timestamp() > 0).then_some(at)
}

fn text(value: Option<&Value>) -> Option<String> {
    value
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|text| !text.is_empty())
        .map(str::to_owned)
}

fn strings(value: Option<&Value>) -> BTreeMap<String, String> {
    value
        .and_then(Value::as_object)
        .map(|map| {
            map.iter()
                .filter_map(|(name, value)| Some((name.clone(), value.as_str()?.to_owned())))
                .collect()
        })
        .unwrap_or_default()
}

fn ids(value: Option<&Value>) -> Vec<String> {
    value
        .and_then(Value::as_array)
        .map(|ids| {
            ids.iter()
                .filter_map(|id| id.as_str().map(str::to_owned))
                .collect()
        })
        .unwrap_or_default()
}

/// The labels as one string, in order: the identity of an alert whose
/// server gives it no fingerprint.
fn identity(labels: &BTreeMap<String, String>) -> String {
    labels
        .iter()
        .map(|(name, value)| format!("{name}={value}"))
        .collect::<Vec<_>>()
        .join(",")
}

fn alert(
    labels: BTreeMap<String, String>,
    annotations: &BTreeMap<String, String>,
    state: AlertState,
    since: Option<DateTime<Utc>>,
    fingerprint: Option<String>,
    source: Kind,
) -> Alert {
    Alert {
        name: labels
            .get("alertname")
            .cloned()
            .unwrap_or_else(|| "(unnamed)".into()),
        severity: Severity::parse(labels.get("severity").map(String::as_str)),
        summary: annotations.get("summary").cloned(),
        description: annotations.get("description").cloned(),
        runbook: annotations
            .get("runbook_url")
            .or_else(|| annotations.get("runbook"))
            .cloned(),
        fingerprint: fingerprint.unwrap_or_else(|| identity(&labels)),
        labels,
        state,
        since,
        silenced_by: Vec::new(),
        inhibited_by: Vec::new(),
        source,
    }
}

/// Alertmanager's `GET /api/v2/alerts`.
pub fn alertmanager_alerts(value: &Value) -> Parsed<Vec<Alert>> {
    let items = value
        .as_array()
        .ok_or("Alertmanager's alerts aren't a list")?;
    Ok(items
        .iter()
        .map(|item| {
            let status = item.get("status");
            let silenced_by = ids(status.and_then(|s| s.get("silencedBy")));
            let inhibited_by = ids(status.and_then(|s| s.get("inhibitedBy")));
            let state = if !silenced_by.is_empty() {
                AlertState::Silenced
            } else if !inhibited_by.is_empty() {
                AlertState::Inhibited
            } else {
                AlertState::Firing
            };
            let mut found = alert(
                strings(item.get("labels")),
                &strings(item.get("annotations")),
                state,
                time(item.get("startsAt")),
                text(item.get("fingerprint")),
                Kind::Alertmanager,
            );
            found.silenced_by = silenced_by;
            found.inhibited_by = inhibited_by;
            found
        })
        .collect())
}

/// The `data` of a Prometheus-shaped answer, which vmalert gives.
fn data(value: &Value) -> Parsed<&Value> {
    if value.get("status").and_then(Value::as_str) == Some("error") {
        let message = text(value.get("error")).unwrap_or_else(|| "the server refused".into());
        return Err(message);
    }
    value
        .get("data")
        .ok_or_else(|| "the answer has no data".to_owned())
}

/// vmalert's `GET /api/v1/alerts`: active alerts, firing or pending.
pub fn vmalert_alerts(value: &Value) -> Parsed<Vec<Alert>> {
    let items = data(value)?
        .get("alerts")
        .and_then(Value::as_array)
        .ok_or("vmalert's alerts aren't a list")?;
    Ok(items
        .iter()
        .map(|item| {
            let mut labels = strings(item.get("labels"));
            if let Some(name) = text(item.get("name")) {
                labels.entry("alertname".into()).or_insert(name);
            }
            let state = match item.get("state").and_then(Value::as_str) {
                Some("pending") => AlertState::Pending,
                _ => AlertState::Firing,
            };
            alert(
                labels,
                &strings(item.get("annotations")),
                state,
                time(item.get("activeAt")),
                None,
                Kind::Vmalert,
            )
        })
        .collect())
}

/// Keeps the first alert of each fingerprint: replicas of one Alertmanager
/// answer alike, and so do Services that reach the same replicas.
pub fn dedupe(alerts: Vec<Alert>) -> Vec<Alert> {
    let mut seen = HashSet::new();
    alerts
        .into_iter()
        .filter(|alert| seen.insert((alert.source, alert.fingerprint.clone())))
        .collect()
}

/// vmalert's `GET /api/v1/rules`.
pub fn vmalert_rules(value: &Value) -> Parsed<Vec<RuleGroup>> {
    let groups = data(value)?
        .get("groups")
        .and_then(Value::as_array)
        .ok_or("vmalert's rule groups aren't a list")?;
    Ok(groups
        .iter()
        .map(|group| {
            let rules = group
                .get("rules")
                .and_then(Value::as_array)
                .map(|rules| rules.iter().map(rule).collect())
                .unwrap_or_default();
            RuleGroup {
                name: text(group.get("name")).unwrap_or_else(|| "(unnamed)".into()),
                origin: text(group.get("file")),
                interval_seconds: group.get("interval").and_then(Value::as_f64),
                last_evaluation: time(group.get("lastEvaluation")),
                rules,
            }
        })
        .collect())
}

fn rule(item: &Value) -> Rule {
    let kind = match item.get("type").and_then(Value::as_str) {
        Some("recording") => RuleKind::Recording,
        _ => RuleKind::Alerting,
    };
    let alerts = item
        .get("alerts")
        .and_then(Value::as_array)
        .map_or(0, Vec::len);
    Rule {
        name: text(item.get("name")).unwrap_or_else(|| "(unnamed)".into()),
        kind,
        health: RuleHealth::parse(item.get("health").and_then(Value::as_str)),
        last_error: text(item.get("lastError")).or_else(|| text(item.get("error"))),
        last_evaluation: time(item.get("lastEvaluation")),
        query: text(item.get("query")).unwrap_or_default(),
        alerts,
        firing: item.get("state").and_then(Value::as_str) == Some("firing"),
        source: Kind::Vmalert,
    }
}

/// Alertmanager's `GET /api/v2/silences`.
pub fn silences(value: &Value) -> Parsed<Vec<Silence>> {
    let items = value
        .as_array()
        .ok_or("Alertmanager's silences aren't a list")?;
    Ok(items
        .iter()
        .map(|item| Silence {
            id: text(item.get("id")).unwrap_or_default(),
            state: match item
                .get("status")
                .and_then(|status| status.get("state"))
                .and_then(Value::as_str)
            {
                Some("active") => SilenceState::Active,
                Some("pending") => SilenceState::Pending,
                _ => SilenceState::Expired,
            },
            matchers: item
                .get("matchers")
                .and_then(Value::as_array)
                .map(|matchers| {
                    matchers
                        .iter()
                        .map(|matcher| SilenceMatcher {
                            name: text(matcher.get("name")).unwrap_or_default(),
                            value: matcher
                                .get("value")
                                .and_then(Value::as_str)
                                .unwrap_or_default()
                                .to_owned(),
                            regex: matcher.get("isRegex").and_then(Value::as_bool) == Some(true),
                            equal: matcher.get("isEqual").and_then(Value::as_bool) != Some(false),
                        })
                        .collect()
                })
                .unwrap_or_default(),
            created_by: text(item.get("createdBy")).unwrap_or_default(),
            comment: text(item.get("comment")).unwrap_or_default(),
            starts_at: time(item.get("startsAt")),
            ends_at: time(item.get("endsAt")),
        })
        .collect())
}

/// How many peers Alertmanager's `GET /api/v2/status` says its cluster has,
/// and its version.
pub fn alertmanager_status(value: &Value) -> (Option<String>, Option<usize>) {
    let version = text(
        value
            .get("versionInfo")
            .and_then(|info| info.get("version")),
    );
    let peers = value
        .get("cluster")
        .and_then(|cluster| cluster.get("peers"))
        .and_then(Value::as_array)
        .map(Vec::len);
    (version, peers)
}

/// One PrometheusRule object (`monitoring.coreos.com`), as the cluster
/// returns it. Nobody has evaluated it here, so each rule's health is
/// Unknown, never Ok.
pub fn prometheus_rule(object: &Value) -> Vec<RuleGroup> {
    let metadata = object.get("metadata");
    let origin = match (
        text(metadata.and_then(|m| m.get("namespace"))),
        text(metadata.and_then(|m| m.get("name"))),
    ) {
        (Some(namespace), Some(name)) => Some(format!("{namespace}/{name}")),
        (None, Some(name)) => Some(name),
        _ => None,
    };
    object
        .get("spec")
        .and_then(|spec| spec.get("groups"))
        .and_then(Value::as_array)
        .map(|groups| {
            groups
                .iter()
                .map(|group| RuleGroup {
                    name: text(group.get("name")).unwrap_or_else(|| "(unnamed)".into()),
                    origin: origin.clone(),
                    interval_seconds: text(group.get("interval"))
                        .as_deref()
                        .and_then(duration_seconds),
                    last_evaluation: None,
                    rules: group
                        .get("rules")
                        .and_then(Value::as_array)
                        .map(|rules| rules.iter().filter_map(defined_rule).collect())
                        .unwrap_or_default(),
                })
                .collect()
        })
        .unwrap_or_default()
}

fn defined_rule(item: &Value) -> Option<Rule> {
    let (name, kind) = match (text(item.get("alert")), text(item.get("record"))) {
        (Some(name), _) => (name, RuleKind::Alerting),
        (None, Some(name)) => (name, RuleKind::Recording),
        (None, None) => return None,
    };
    Some(Rule {
        name,
        kind,
        health: RuleHealth::Unknown,
        last_error: None,
        last_evaluation: None,
        query: text(item.get("expr")).unwrap_or_default(),
        alerts: 0,
        firing: false,
        source: Kind::PrometheusRule,
    })
}

/// `30s`, `1m`, `1h30m`: the Prometheus duration a rule group's interval
/// is written in.
fn duration_seconds(text: &str) -> Option<f64> {
    let mut total = 0.;
    let mut number = String::new();
    let mut unit = String::new();
    let mut flush = |number: &mut String, unit: &mut String| -> Option<()> {
        if number.is_empty() && unit.is_empty() {
            return Some(());
        }
        let n: f64 = number.parse().ok()?;
        total += n * match unit.as_str() {
            "ms" => 0.001,
            "s" => 1.,
            "m" => 60.,
            "h" => 3600.,
            "d" => 86_400.,
            "w" => 604_800.,
            _ => return None,
        };
        number.clear();
        unit.clear();
        Some(())
    };
    for c in text.chars() {
        if c.is_ascii_digit() || c == '.' {
            if !unit.is_empty() {
                flush(&mut number, &mut unit)?;
            }
            number.push(c);
        } else {
            unit.push(c);
        }
    }
    flush(&mut number, &mut unit)?;
    (total > 0.).then_some(total)
}
