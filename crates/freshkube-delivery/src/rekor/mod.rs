//! The transparency log, when the person turned it on: whether the log holds
//! an entry for the signature that was verified. It never changes a
//! signature check; it adds what the log says, and with the switch on the
//! join requires it ([`docs/SUPPLY_CHAIN.md`](../../../docs/SUPPLY_CHAIN.md),
//! "The transparency log").
//!
//! Where the log is comes from Chains' configuration or the setting, never
//! from an annotation on a run: whoever can patch a TaskRun can write the
//! annotation, and a read must not be steered to a host they choose. From
//! the annotation only the entry's index is taken, as digits.
//!
//! What is checked is that the log has the signature: the entry holds the
//! signature bytes that verified and the key the person named. The log's own
//! signature over its checkpoint and the inclusion proof need the Sigstore
//! trust root and are not checked; the words say so.
//!
//! The entry shapes read are `hashedrekord` and `intoto` (0.0.2), from
//! Rekor's documentation. Any other shape is `Differs`, never a match.

#![cfg_attr(
    not(test),
    allow(
        dead_code,
        reason = "the lookup is called from `collect` in the next step"
    )
)]

use std::collections::BTreeMap;
use std::future::Future;

use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use freshkube_core::resources::{Failure, FailureKind};
use serde_json::Value;

use crate::verify::{TrustedKey, Verified};

/// The most an entry's response may be read to: an entry is a few KiB.
pub(crate) const MOST_ENTRY_BYTES: usize = 256 * 1024;

/// What a GET of the log answered.
pub(crate) struct Fetched {
    pub status: u16,
    /// The body, already cut at [`MOST_ENTRY_BYTES`] by the transport.
    pub body: Vec<u8>,
}

/// The one request this module makes, behind a seam so the matching can be
/// tested without a socket. A GET, under the read's deadline and cap.
pub(crate) trait Fetch {
    fn get(&self, url: &str) -> impl Future<Output = Result<Fetched, Failure>>;
}

/// What the log says of one verified signature.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Outcome {
    /// The entry at `index` holds the signature and the key.
    Matches { index: u64 },
    /// The run carries no entry index, or one that is not a plain number.
    NoIndex,
    /// The log has no entry at that index.
    NotInLog { index: u64 },
    /// The entry is there and is not this signature's, or is a shape this
    /// build does not read.
    Differs { index: u64, why: &'static str },
    /// The log did not answer.
    Unreachable(String),
    /// The log answered, and refused.
    Refused(String),
}

impl Outcome {
    /// Whether the log holds the verified signature: the one outcome that
    /// lets a link stand with the switch on.
    pub(crate) fn matches(&self) -> bool {
        matches!(self, Self::Matches { .. })
    }
}

impl std::fmt::Display for Outcome {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Matches { index } => write!(
                f,
                "the transparency log has it as entry {index}; the log's own signature was not checked"
            ),
            Self::NoIndex => f.write_str("the run names no transparency log entry"),
            Self::NotInLog { index } => {
                write!(f, "the transparency log has no entry {index}")
            }
            Self::Differs { index, why } => {
                write!(f, "the transparency log's entry {index} differs: {why}")
            }
            Self::Unreachable(why) => write!(f, "the transparency log didn't answer: {why}"),
            Self::Refused(why) => write!(f, "the transparency log refused: {why}"),
        }
    }
}

/// Which attestation a lookup is of: the namespace and name of the run or
/// task that carries it, and the attestation's annotation key.
pub(crate) type EntryKey = (Option<String>, String, String);

/// What was read of the transparency log for one change.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) enum LogRead {
    /// The switch is off: nothing was fetched, and confirmation does not
    /// wait for the log.
    #[default]
    Off,
    /// The switch is on and the log's URL is not known: Chains' configuration
    /// could not be read, and the setting names none.
    NoUrl(String),
    /// The switch is on: what the log said of each verified attestation.
    Read(BTreeMap<EntryKey, Outcome>),
}

/// What the log means for a link whose signature verified.
pub(crate) enum Standing {
    /// The link may stand; the words say what the log did.
    Stands(String),
    /// The link is Claimed, for this reason.
    Claims(String),
}

impl LogRead {
    /// With the switch on the log is required: only a matching entry lets a
    /// verified signature stand.
    pub(crate) fn standing(&self, key: &EntryKey) -> Standing {
        match self {
            Self::Off => Standing::Stands("transparency log not read (off)".to_owned()),
            Self::NoUrl(why) => Standing::Claims(format!(
                "the transparency log is on and its address is not known: {why}"
            )),
            Self::Read(entries) => match entries.get(key) {
                Some(outcome) if outcome.matches() => Standing::Stands(outcome.to_string()),
                Some(outcome) => Standing::Claims(outcome.to_string()),
                None => Standing::Claims("the transparency log was not asked".to_owned()),
            },
        }
    }
}

/// The index in Chains' `chains.tekton.dev/transparency` annotation, which
/// reads `<log url>/api/v1/log/entries?logIndex=<n>`. Only the digits are
/// taken; the annotation's host is never used.
pub(crate) fn log_index(annotation: &str) -> Option<u64> {
    let (_, index) = annotation.rsplit_once("logIndex=")?;
    let digits: String = index.chars().take_while(char::is_ascii_digit).collect();
    // A trailing `&…` is allowed; anything else after the digits is not a
    // plain index.
    let rest = &index[digits.len()..];
    if digits.is_empty() || digits.len() > 18 || !(rest.is_empty() || rest.starts_with('&')) {
        return None;
    }
    digits.parse().ok()
}

/// The entry of `log` at `index`, and whether it holds what verified.
pub(crate) async fn look_up<F: Fetch>(
    fetch: &F,
    log: &str,
    index: Option<u64>,
    verified: &Verified,
) -> Outcome {
    let Some(index) = index else {
        return Outcome::NoIndex;
    };
    let url = format!(
        "{}/api/v1/log/entries?logIndex={index}",
        log.trim_end_matches('/')
    );
    let fetched = match fetch.get(&url).await {
        Ok(fetched) => fetched,
        Err(failure) => {
            return match failure.kind {
                FailureKind::Unauthorized | FailureKind::Forbidden => {
                    Outcome::Refused(failure.message)
                }
                _ => Outcome::Unreachable(failure.message),
            };
        }
    };
    match fetched.status {
        200 => entry_outcome(&fetched.body, index, verified),
        404 => Outcome::NotInLog { index },
        status => Outcome::Refused(format!("it answered {status}")),
    }
}

fn entry_outcome(body: &[u8], index: u64, verified: &Verified) -> Outcome {
    let differs = |why| Outcome::Differs { index, why };
    if body.len() >= MOST_ENTRY_BYTES {
        return differs("the answer is larger than this build reads");
    }
    let Ok(Value::Object(entries)) = serde_json::from_slice::<Value>(body) else {
        return differs("the answer is not a log entry");
    };
    // The answer is a map from the entry's UUID to the entry.
    let mut entries = entries.values();
    let (Some(entry), None) = (entries.next(), entries.next()) else {
        return differs("the answer is not exactly one entry");
    };
    if entry.get("logIndex").and_then(Value::as_u64) != Some(index) {
        return differs("it is not the entry asked for");
    }
    let Some(decoded) = entry
        .get("body")
        .and_then(Value::as_str)
        .and_then(|body| STANDARD.decode(body).ok())
        .and_then(|body| serde_json::from_slice::<Value>(&body).ok())
    else {
        return differs("its body can't be read");
    };
    match holds(&decoded, verified) {
        Some(true) => Outcome::Matches { index },
        Some(false) => differs("it holds another signature or key"),
        None => differs("it is a kind of entry this build does not read"),
    }
}

/// Whether the entry body holds the verified signature and the key; `None`
/// for a shape that is not read.
fn holds(body: &Value, verified: &Verified) -> Option<bool> {
    let spec = body.get("spec")?;
    let pairs: Vec<(&str, &str)> = match body.get("kind").and_then(Value::as_str)? {
        "hashedrekord" => vec![(
            spec.pointer("/signature/content")?.as_str()?,
            spec.pointer("/signature/publicKey/content")?.as_str()?,
        )],
        "intoto" => spec
            .pointer("/content/envelope/signatures")?
            .as_array()?
            .iter()
            .filter_map(|signature| {
                Some((
                    signature.get("sig")?.as_str()?,
                    signature.get("publicKey")?.as_str()?,
                ))
            })
            .collect(),
        _ => return None,
    };
    Some(
        pairs
            .into_iter()
            .any(|(sig, key)| same_signature(sig, &verified.signature) && same_key(key, verified)),
    )
}

/// The entry's signature is the one that verified. Rekor's `intoto` entries
/// base64 the envelope's already-base64 `sig` once more, so both are read.
fn same_signature(stored: &str, signature: &[u8]) -> bool {
    let Ok(once) = STANDARD.decode(stored) else {
        return false;
    };
    once == signature
        || std::str::from_utf8(&once)
            .ok()
            .and_then(|text| STANDARD.decode(text).ok())
            .is_some_and(|twice| twice == signature)
}

/// The entry's key, a base64 PEM, is the key the person named.
fn same_key(stored: &str, verified: &Verified) -> bool {
    STANDARD
        .decode(stored)
        .ok()
        .and_then(|pem| String::from_utf8(pem).ok())
        .and_then(|pem| TrustedKey::from_pem(&pem).ok())
        .is_some_and(|key| key.fingerprint() == verified.key)
}

#[cfg(test)]
mod tests;
