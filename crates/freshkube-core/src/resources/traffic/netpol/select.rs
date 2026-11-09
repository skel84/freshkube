//! Label selectors, matched without ever guessing: a selector that can't be
//! evaluated is `Unknown`, which no caller reads as a match.
use k8s_openapi::apimachinery::pkg::apis::meta::v1::LabelSelector;
use std::collections::BTreeMap;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Match {
    Yes,
    No,
    Unknown,
}

impl Match {
    /// Both must hold: a definite No wins over Unknown.
    pub fn and(self, other: Match) -> Match {
        match (self, other) {
            (Match::No, _) | (_, Match::No) => Match::No,
            (Match::Unknown, _) | (_, Match::Unknown) => Match::Unknown,
            _ => Match::Yes,
        }
    }

    /// Either may hold: a definite Yes wins over Unknown.
    pub fn or(self, other: Match) -> Match {
        match (self, other) {
            (Match::Yes, _) | (_, Match::Yes) => Match::Yes,
            (Match::Unknown, _) | (_, Match::Unknown) => Match::Unknown,
            _ => Match::No,
        }
    }
}

/// Whether `labels` satisfy `selector`. An empty selector selects everything.
/// `In` and `NotIn` with no values, and an operator Kubernetes doesn't have,
/// are invalid and give `Unknown`.
pub fn selector_matches(selector: &LabelSelector, labels: &BTreeMap<String, String>) -> Match {
    let mut result = Match::Yes;
    for (key, value) in selector.match_labels.iter().flatten() {
        result = result.and(if labels.get(key) == Some(value) {
            Match::Yes
        } else {
            Match::No
        });
    }
    for requirement in selector.match_expressions.iter().flatten() {
        let values = requirement.values.as_deref().unwrap_or_default();
        let found = labels.get(&requirement.key);
        result = result.and(match requirement.operator.as_str() {
            "In" if !values.is_empty() => found.is_some_and(|v| values.contains(v)).into(),
            "NotIn" if !values.is_empty() => (!found.is_some_and(|v| values.contains(v))).into(),
            "Exists" => found.is_some().into(),
            "DoesNotExist" => found.is_none().into(),
            _ => Match::Unknown,
        });
    }
    result
}

impl From<bool> for Match {
    fn from(yes: bool) -> Self {
        if yes { Match::Yes } else { Match::No }
    }
}

/// A selector in words: `all`, or its terms joined by commas.
pub fn selector_text(selector: &LabelSelector) -> Option<String> {
    let mut terms = selector
        .match_labels
        .iter()
        .flatten()
        .map(|(key, value)| format!("{key}={value}"))
        .collect::<Vec<_>>();
    for requirement in selector.match_expressions.iter().flatten() {
        let values = requirement.values.as_deref().unwrap_or_default().join(", ");
        terms.push(match requirement.operator.as_str() {
            "In" => format!("{} in ({values})", requirement.key),
            "NotIn" => format!("{} not in ({values})", requirement.key),
            "Exists" => requirement.key.clone(),
            "DoesNotExist" => format!("!{}", requirement.key),
            other => format!("{} {other} ({values})", requirement.key),
        });
    }
    (!terms.is_empty()).then(|| terms.join(", "))
}
