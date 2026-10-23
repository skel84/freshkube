//! A new version waits behind a review in two steps: trust changes first, then
//! the rest. It never applies without both.

use serde_json::Value;

use super::file::{KEY_FILE, Version};
use crate::write::Protection;

/// What kind of trust a change moves.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum TrustKind {
    /// A signing key's fingerprint.
    SigningKey,
    /// An address that receives a sign-in or a token.
    Address,
    /// A protection level looser than before.
    LoosensProtection,
}

impl TrustKind {
    pub fn words(self) -> &'static str {
        match self {
            TrustKind::SigningKey => "Signing key",
            TrustKind::Address => "Receives a sign-in or token",
            TrustKind::LoosensProtection => "Loosens protection",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChangeKind {
    Added,
    Changed,
    Removed,
}

/// One setting a version would set, change or stop setting.
#[derive(Clone, Debug, PartialEq)]
pub struct Change {
    pub at: String,
    pub kind: ChangeKind,
    pub before: Option<Value>,
    pub after: Option<Value>,
    pub trust: Option<TrustKind>,
}

/// The two steps, in order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Step {
    Trust,
    Rest,
}

/// Applying a review that isn't finished.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct NotReviewed(pub Step);

#[derive(Clone, Debug, PartialEq)]
pub struct Review {
    pub changes: Vec<Change>,
    version: Version,
    trust_done: bool,
    rest_done: bool,
}

/// Keys whose value is an address that receives a sign-in or a token.
fn is_address(key: &str) -> bool {
    let last = key.rsplit('.').next().unwrap_or(key);
    matches!(
        last,
        "server" | "url" | "address" | "transparency_url" | "kargo_api" | "argocd_api"
    ) && !key.starts_with("destinations")
}

fn trust_of(key: &str, before: Option<&Value>, after: Option<&Value>) -> Option<TrustKind> {
    if key.starts_with("fingerprint:") {
        return Some(TrustKind::SigningKey);
    }
    if key.ends_with(".protection") && key.starts_with("clusters[") {
        let was: Protection = before
            .and_then(|v| serde_json::from_value(v.clone()).ok())
            .unwrap_or_default();
        let now: Protection = after
            .and_then(|v| serde_json::from_value(v.clone()).ok())
            .unwrap_or_default();
        return (now > was).then_some(TrustKind::LoosensProtection);
    }
    (is_address(key) && after.is_some()).then_some(TrustKind::Address)
}

/// All keys a version sets, with signing key fingerprints as `fingerprint:path`.
fn flat(version: Option<&Version>) -> std::collections::BTreeMap<String, Value> {
    let mut all = std::collections::BTreeMap::new();
    if let Some(version) = version {
        all.extend(version.values.clone());
        for (path, print) in &version.key_fingerprints {
            all.insert(format!("fingerprint:{path}"), Value::String(print.clone()));
        }
    }
    all.remove(KEY_FILE);
    all
}

impl Review {
    pub fn new(applied: Option<&Version>, pending: Version) -> Self {
        let before = flat(applied);
        let after = flat(Some(&pending));
        let mut changes = Vec::new();
        for key in before
            .keys()
            .chain(after.keys())
            .collect::<std::collections::BTreeSet<_>>()
        {
            let (was, now) = (before.get(key), after.get(key));
            if was == now {
                continue;
            }
            let kind = match (was, now) {
                (None, _) => ChangeKind::Added,
                (_, None) => ChangeKind::Removed,
                _ => ChangeKind::Changed,
            };
            changes.push(Change {
                at: key.clone(),
                kind,
                before: was.cloned(),
                after: now.cloned(),
                trust: trust_of(key, was, now),
            });
        }
        // Trust changes first, then by key.
        changes.sort_by(|a, b| {
            b.trust
                .is_some()
                .cmp(&a.trust.is_some())
                .then(a.trust.cmp(&b.trust))
                .then(a.at.cmp(&b.at))
        });
        let has_trust = changes.iter().any(|c| c.trust.is_some());
        Self {
            changes,
            version: pending,
            trust_done: !has_trust,
            rest_done: false,
        }
    }

    /// The step the person is on.
    pub fn step(&self) -> Step {
        if self.trust_done {
            Step::Rest
        } else {
            Step::Trust
        }
    }

    pub fn trust_changes(&self) -> impl Iterator<Item = &Change> {
        self.changes.iter().filter(|c| c.trust.is_some())
    }

    pub fn other_changes(&self) -> impl Iterator<Item = &Change> {
        self.changes.iter().filter(|c| c.trust.is_none())
    }

    /// Confirms the current step. The second step can't be confirmed first.
    pub fn confirm(&mut self) {
        match self.step() {
            Step::Trust => self.trust_done = true,
            Step::Rest => self.rest_done = true,
        }
    }

    pub fn is_finished(&self) -> bool {
        self.trust_done && self.rest_done
    }

    pub(super) fn apply(self) -> Result<Version, NotReviewed> {
        if !self.trust_done {
            return Err(NotReviewed(Step::Trust));
        }
        if !self.rest_done {
            return Err(NotReviewed(Step::Rest));
        }
        Ok(self.version)
    }
}
