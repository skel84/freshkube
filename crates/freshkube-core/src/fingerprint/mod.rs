//! A cluster's fingerprint for deep links (#873): what lets a link made in
//! one person's Freshkube find the same cluster in another's, though their
//! context names differ.
//!
//! A link carries up to two values, each `base32(sha256(SALT ‖ domain ‖
//! input)[..16])`, so a link is never a raw namespace UID or certificate:
//!
//! - the **strong** one, from the `kube-system` namespace's UID (one metadata
//!   GET), or from a setup-file `fingerprint:` override, which has a domain of
//!   its own so the override string itself never appears in a link;
//! - the **CA** one, from the cluster's CA certificates in the local
//!   kubeconfig (no network).
//!
//! Matching ([`matches`]) never lets a CA stand for a cluster whose strong
//! value says otherwise: a TLS-terminating proxy can share one CA across
//! clusters.

use std::collections::BTreeSet;
use std::time::Duration;

use kube::Client;
use kube::config::Kubeconfig;
use sha2::{Digest, Sha256};

use crate::constants::FINGERPRINT_SALT;
use crate::resources::{Failure, FailureKind, builtin, get_metadata};

/// How long the one `kube-system` read may take.
pub const READ_DEADLINE: Duration = Duration::from_secs(10);

const UID_DOMAIN: &str = "kube-system-uid";
const OVERRIDE_DOMAIN: &str = "override";
const CA_DOMAIN: &str = "ca";
/// An override: 1 to 128 characters, no whitespace.
const OVERRIDE_MAX: usize = 128;
/// A kubeconfig's CA bundle that is larger is not read.
const CA_MAX_BYTES: usize = 1 << 20;

/// The two values a link or a context has: either may be unknown.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct Fingerprint {
    /// From the `kube-system` UID or an override: 26 characters of base32.
    pub strong: Option<String>,
    /// From the CA certificates: 26 characters of base32.
    pub ca: Option<String>,
}

impl Fingerprint {
    pub fn is_empty(&self) -> bool {
        self.strong.is_none() && self.ca.is_none()
    }
}

/// How well a context fits a link.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Strength {
    /// Both sides have a strong value and they are equal.
    Exact,
    /// A side lacks a strong value and the CA values are equal. Never opens
    /// without asking: one CA can serve several clusters.
    CaOnly,
}

/// Matches a link's fingerprint to a context's:
///
/// 1. both have a strong value, equal → [`Strength::Exact`];
/// 2. both have a strong value, different → no match, whatever the CA says;
/// 3. a side lacks a strong value and the CA values are equal →
///    [`Strength::CaOnly`];
/// 4. otherwise no match.
pub fn matches(link: &Fingerprint, context: &Fingerprint) -> Option<Strength> {
    match (&link.strong, &context.strong) {
        (Some(wanted), Some(found)) => (wanted == found).then_some(Strength::Exact),
        _ => match (&link.ca, &context.ca) {
            (Some(wanted), Some(found)) if wanted == found => Some(Strength::CaOnly),
            _ => None,
        },
    }
}

/// The strong value of a cluster whose `kube-system` namespace has `uid`.
pub fn strong_from_uid(uid: &str) -> String {
    token(UID_DOMAIN, uid.as_bytes())
}

/// The strong value of a setup-file override, or `None` for one that is empty,
/// longer than 128 characters or holds whitespace.
pub fn strong_from_override(text: &str) -> Option<String> {
    valid_override(text).then(|| token(OVERRIDE_DOMAIN, text.as_bytes()))
}

/// Whether `text` may be a `fingerprint:` override.
pub fn valid_override(text: &str) -> bool {
    !text.is_empty()
        && text.chars().count() <= OVERRIDE_MAX
        && !text.chars().any(char::is_whitespace)
}

/// The CA value of a PEM bundle: the sorted, de-duplicated DER of its
/// certificates. `None` when it holds none.
pub fn ca_from_pem(bundle: &[u8]) -> Option<String> {
    if bundle.len() > CA_MAX_BYTES {
        return None;
    }
    let certificates: BTreeSet<Vec<u8>> = x509_parser::pem::Pem::iter_from_buffer(bundle)
        .filter_map(Result::ok)
        .filter(|pem| pem.label == "CERTIFICATE")
        .map(|pem| pem.contents)
        .collect();
    if certificates.is_empty() {
        return None;
    }
    let mut input = Vec::new();
    for der in &certificates {
        // Length-prefixed, so two bundles never hash alike by concatenation.
        input.extend((der.len() as u64).to_be_bytes());
        input.extend(der);
    }
    Some(token(CA_DOMAIN, &input))
}

/// The CA value of the cluster `context` names in `kubeconfig`: its inline
/// `certificate-authority-data`, else the file `certificate-authority` names.
/// A context with neither, or a file that can't be read, has none. No
/// credential is read.
pub fn ca_of_context(kubeconfig: &Kubeconfig, context: &str) -> Option<String> {
    let wanted = &kubeconfig
        .contexts
        .iter()
        .find(|named| named.name == context)?
        .context
        .as_ref()?
        .cluster;
    let cluster = kubeconfig
        .clusters
        .iter()
        .find(|named| &named.name == wanted)?
        .cluster
        .as_ref()?;
    if let Some(data) = cluster
        .certificate_authority_data
        .as_deref()
        .filter(|data| !data.trim().is_empty())
    {
        use base64::Engine as _;
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(data.trim())
            .ok()?;
        return ca_from_pem(&bytes);
    }
    let file = cluster.certificate_authority.as_deref()?;
    crate::read_bounded_regular_file(std::path::Path::new(file), CA_MAX_BYTES as u64)
        .ok()
        .and_then(|bytes| ca_from_pem(&bytes))
}

/// What the one `kube-system` read found.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum StrongRead {
    /// The namespace's UID.
    Uid(String),
    /// 401 or 403: the reader's access can't see it. The cluster has the CA
    /// value alone, marked "CA only".
    Refused,
    /// Anything else: not a fallback, so the reason is shown.
    Failed(String),
}

/// Reads `kube-system`'s UID: one metadata GET of that namespace under
/// [`READ_DEADLINE`], nothing else.
pub async fn read_strong(client: &Client) -> StrongRead {
    let namespaces = builtin("namespaces").expect("namespaces is a built-in kind");
    let read = get_metadata(client, &namespaces, None, "kube-system");
    match tokio::time::timeout(READ_DEADLINE, read).await {
        Err(_) => StrongRead::Failed("The read of kube-system timed out".into()),
        Ok(Ok(metadata)) => match metadata.uid {
            Some(uid) if !uid.is_empty() => StrongRead::Uid(uid),
            _ => StrongRead::Failed("The API returned no UID for kube-system".into()),
        },
        Ok(Err(Failure {
            kind: FailureKind::Forbidden | FailureKind::Unauthorized,
            ..
        })) => StrongRead::Refused,
        Ok(Err(failure)) => StrongRead::Failed(failure.message),
    }
}

/// `base32(sha256(SALT ‖ 0 ‖ domain ‖ 0 ‖ input)[..16])`, lower case, no
/// padding: 26 characters.
fn token(domain: &str, input: &[u8]) -> String {
    let mut hash = Sha256::new();
    hash.update(FINGERPRINT_SALT.as_bytes());
    hash.update([0]);
    hash.update(domain.as_bytes());
    hash.update([0]);
    hash.update(input);
    base32(&hash.finalize()[..16])
}

/// RFC 4648 base32, lower case, unpadded.
fn base32(bytes: &[u8]) -> String {
    const ALPHABET: &[u8; 32] = b"abcdefghijklmnopqrstuvwxyz234567";
    let mut out = String::with_capacity((bytes.len() * 8).div_ceil(5));
    let (mut buffer, mut bits) = (0u32, 0u32);
    for byte in bytes {
        buffer = (buffer << 8) | u32::from(*byte);
        bits += 8;
        while bits >= 5 {
            bits -= 5;
            out.push(char::from(ALPHABET[((buffer >> bits) & 31) as usize]));
        }
    }
    if bits > 0 {
        out.push(char::from(ALPHABET[((buffer << (5 - bits)) & 31) as usize]));
    }
    out
}

#[cfg(test)]
mod tests;
