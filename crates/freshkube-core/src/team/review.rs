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
    /// The trust changes the person has confirmed, each on its own.
    confirmed: std::collections::BTreeSet<String>,
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
        Self {
            changes,
            version: pending,
            confirmed: Default::default(),
            rest_done: false,
        }
    }

    /// The step the person is on: every trust change is confirmed on its own
    /// before the rest can be.
    pub fn step(&self) -> Step {
        if self.trust_pending().next().is_some() {
            Step::Trust
        } else {
            Step::Rest
        }
    }

    pub fn trust_changes(&self) -> impl Iterator<Item = &Change> {
        self.changes.iter().filter(|c| c.trust.is_some())
    }

    pub fn other_changes(&self) -> impl Iterator<Item = &Change> {
        self.changes.iter().filter(|c| c.trust.is_none())
    }

    /// The trust changes not yet confirmed.
    pub fn trust_pending(&self) -> impl Iterator<Item = &Change> {
        self.trust_changes()
            .filter(|c| !self.confirmed.contains(&c.at))
    }

    pub fn is_confirmed(&self, at: &str) -> bool {
        self.confirmed.contains(at)
    }

    /// Confirms one trust change. False when `at` is not a trust change.
    pub fn confirm_trust(&mut self, at: &str) -> bool {
        let known = self.trust_changes().any(|c| c.at == at);
        if known {
            self.confirmed.insert(at.to_string());
        }
        known
    }

    /// Confirms the rest of the diff. Refused while a trust change waits.
    pub fn confirm_rest(&mut self) -> bool {
        let ready = self.step() == Step::Rest;
        if ready {
            self.rest_done = true;
        }
        ready
    }

    pub fn is_finished(&self) -> bool {
        self.step() == Step::Rest && self.rest_done
    }

    pub(super) fn apply(self) -> Result<Version, NotReviewed> {
        if self.step() == Step::Trust {
            return Err(NotReviewed(Step::Trust));
        }
        if !self.rest_done {
            return Err(NotReviewed(Step::Rest));
        }
        Ok(self.version)
    }
}
