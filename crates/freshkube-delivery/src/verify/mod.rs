//! Whether an attestation's signature verifies with the key the person
//! named, and a subject of it carries the digest the build reported. The one
//! thing that can make the supply-chain link Confirmed, so it is written to
//! have no way to say yes by default:
//!
//! - [`Verification`] has exactly one success variant, [`Verified`], and a
//!   [`Verified`] can be built only in [`verify`], after every check passed
//!   (its `seal` field is private to this module). There is no `Default`, no
//!   `From<bool>` and no skip flag.
//! - [`Trust`] has no default either: [`Trust::Unset`] is a state the
//!   caller must pass on purpose, and it yields [`Verification::NoTrust`].
//! - Every check fails closed: a signature that does not parse does not
//!   verify, a key that cannot be read is [`Verification::KeyUnusable`], and
//!   a statement cut at a cap is [`Verification::NotRead`], not a mismatch.
//!
//! Offline: it reads the attestation already read from the run, and the key
//! file. No network, no clock, no cluster.

mod dsse;
mod key;
#[cfg(test)]
pub(crate) mod tests;

use std::fmt;

pub(crate) use key::{KeyProblem, TrustedKey};

use super::chains::config::Backend;
use super::chains::{Attestation, Envelope, Form, IN_TOTO_PAYLOAD_TYPE, Malformed, Stored};
use super::digest::Digest;

/// What the person named to trust. There is no default.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Trust {
    /// No key named: nothing is verified.
    Unset,
    Key(TrustedKey),
    /// A key was named and cannot be used.
    Unusable(KeyProblem),
}

impl Trust {
    /// The trust a key file gives: its key, or why it is unusable.
    pub(crate) fn from_key_file(path: &std::path::Path) -> Self {
        match TrustedKey::read(path) {
            Ok(key) => Self::Key(key),
            Err(problem) => Self::Unusable(problem),
        }
    }
}

/// The outcome for one attestation. Only [`Verification::Verified`] can make
/// a link Confirmed; every other variant says why not.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Verification {
    Verified(Verified),
    /// No key was named.
    NoTrust,
    /// The key that was named cannot be used.
    KeyUnusable(KeyProblem),
    /// The attestation cannot be read: nothing about it is trusted.
    Malformed(Malformed),
    /// No signature of the envelope verifies with the key. A wrong key and a
    /// changed payload look the same to the math.
    SignatureFailed {
        key: String,
    },
    /// The signature verifies, and no subject carries the build's digest.
    SubjectDiffers {
        key: String,
        built: Digest,
        subjects: Vec<Digest>,
    },
    /// Chains is set to store the attestation where this step does not read.
    #[cfg_attr(
        not(test),
        allow(
            dead_code,
            reason = "Chains' configuration is not read by the join yet"
        )
    )]
    NotStoredHere(Vec<Backend>),
    /// Present, and not checked, with why.
    NotRead(NotRead),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum NotRead {
    /// An image's signature, not an attestation of the run.
    ImageSignature,
    /// A run's attestation that was left unread.
    Unread,
    /// Subjects past the cap were not kept, and none that was kept matches,
    /// so a difference cannot be claimed.
    SubjectsCutAtCap,
}

/// An attestation whose signature verified with the person's key and whose
/// subject carries the build's digest. Built only by [`verify`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Verified {
    /// The key's fingerprint.
    pub(crate) key: String,
    /// The signature that verified, as the envelope holds it: what a
    /// transparency log's entry must also hold.
    pub(crate) signature: Vec<u8>,
    /// The subject digest that matched the build's.
    pub(crate) subject: Digest,
    pub(crate) subject_name: Option<String>,
    pub(crate) predicate_type: Option<String>,
    /// The one commit its provenance names, when it names exactly one.
    pub(crate) commit: Option<String>,
    seal: (),
}

impl Verification {
    /// Whether this is the one outcome that confirms.
    pub(crate) fn confirms(&self) -> bool {
        matches!(self, Self::Verified(_))
    }

    /// Chains is set to store the attestation only where it is not read.
    #[cfg_attr(
        not(test),
        allow(
            dead_code,
            reason = "Chains' configuration is not read by the join yet"
        )
    )]
    pub(crate) fn not_stored_here(backends: &[Backend]) -> Self {
        Self::NotStoredHere(backends.to_vec())
    }
}

impl fmt::Display for Verification {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Verified(verified) => write!(
                f,
                "signature verifies with key {}; subject {} matches the build",
                verified.key,
                verified.subject.short()
            ),
            Self::NoTrust => f.write_str("no key is named to verify with"),
            Self::KeyUnusable(problem) => write!(f, "{problem}"),
            Self::Malformed(problem) => write!(f, "the attestation can't be read: {problem}"),
            Self::SignatureFailed { key } => {
                write!(f, "the signature doesn't verify with key {key}")
            }
            Self::SubjectDiffers { key, built, .. } => write!(
                f,
                "signature verifies with key {key}, but no subject is the build's {}",
                built.short()
            ),
            Self::NotStoredHere(_) => {
                f.write_str("stored in the registry or elsewhere; not read in this step")
            }
            Self::NotRead(NotRead::ImageSignature) => f.write_str(
                "an image signature, not an attestation of the run; not read in this step",
            ),
            Self::NotRead(NotRead::Unread) => f.write_str("the attestation was not read"),
            Self::NotRead(NotRead::SubjectsCutAtCap) => {
                f.write_str("subjects past the cap were not read, so no difference is claimed")
            }
        }
    }
}

/// Verifies `attestation` against the key `trust` names and the digest the
/// build reported (`built`).
pub(crate) fn verify(trust: &Trust, attestation: &Attestation, built: &Digest) -> Verification {
    let key = match trust {
        Trust::Unset => return Verification::NoTrust,
        Trust::Unusable(problem) => return Verification::KeyUnusable(problem.clone()),
        Trust::Key(key) => key,
    };
    let envelope = match (&attestation.form, &attestation.stored) {
        (Form::Image, _) => return Verification::NotRead(NotRead::ImageSignature),
        (Form::Run, Stored::Malformed(problem)) => return Verification::Malformed(problem.clone()),
        // A run's attestation is always read or malformed; `Unread` is only
        // ever an image's. Were that to change, nothing here would confirm.
        (Form::Run, Stored::Unread) => return Verification::NotRead(NotRead::Unread),
        (Form::Run, Stored::Envelope(envelope)) => envelope,
    };
    check(key, envelope, built)
}

fn check(key: &TrustedKey, envelope: &Envelope, built: &Digest) -> Verification {
    if envelope.payload_annotation_agrees == Some(false) {
        return Verification::Malformed(Malformed::PayloadDiffers);
    }
    if envelope.payload_type != IN_TOTO_PAYLOAD_TYPE {
        return Verification::Malformed(Malformed::NotAStatement(
            "the envelope is not an in-toto statement",
        ));
    }
    let signed = dsse::pre_authentication_encoding(&envelope.payload_type, &envelope.payload);
    let Some(signature) = envelope
        .signatures
        .iter()
        .find(|signature| key.verifies(&signed, &signature.bytes))
    else {
        return Verification::SignatureFailed {
            key: key.fingerprint().to_owned(),
        };
    };
    // The signature covers these bytes, and the claims were read from them.
    let claims = &envelope.claims;
    if !claims
        .statement_type
        .as_deref()
        .is_some_and(|kind| kind.starts_with("https://in-toto.io/Statement/"))
    {
        return Verification::Malformed(Malformed::NotAStatement(
            "the payload is not an in-toto statement",
        ));
    }
    match claims
        .subjects
        .iter()
        .find(|subject| subject.digest.as_ref() == Some(built))
    {
        Some(subject) => Verification::Verified(Verified {
            key: key.fingerprint().to_owned(),
            signature: signature.bytes.clone(),
            subject: built.clone(),
            subject_name: subject.name.clone(),
            predicate_type: claims.predicate_type.clone(),
            commit: claims.commit.clone(),
            seal: (),
        }),
        None if claims.subjects_omitted > 0 => Verification::NotRead(NotRead::SubjectsCutAtCap),
        None => Verification::SubjectDiffers {
            key: key.fingerprint().to_owned(),
            built: built.clone(),
            subjects: claims
                .subjects
                .iter()
                .filter_map(|subject| subject.digest.clone())
                .collect(),
        },
    }
}
