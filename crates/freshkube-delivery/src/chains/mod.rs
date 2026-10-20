//! What Tekton Chains stores: the attestations it writes as annotations on
//! the TaskRuns and PipelineRuns it signed, and the configuration that says
//! where it stores them. Read, parsed and bounded here; nothing is verified.
//! Every fact in an [`Attestation`] is a claim of whoever could patch the
//! run until a later step checks its signature against a key the person
//! named ([`docs/SUPPLY_CHAIN.md`](../../../docs/SUPPLY_CHAIN.md)).
//!
//! The layout is Chains' `tekton` storage backend, read from upstream's
//! source: for each stored artifact it adds four annotations keyed by the
//! artifact, `chains.tekton.dev/payload-<key>`, `signature-<key>`,
//! `cert-<key>` and `chain-<key>`, each the standard base64 of its bytes.
//! For a run `<key>` is `taskrun-<uid>` or `pipelinerun-<uid>`, and the
//! in-toto formats wrap what they sign in a DSSE envelope: the `signature`
//! annotation holds the envelope (JSON), the `payload` annotation the
//! statement it carries. An image Chains signed is keyed by the first 12
//! digits of its digest instead, and is a simple-signing payload, not an
//! attestation of the run.

use std::collections::BTreeSet;

use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use serde_json::Value;

use super::digest::{Digest, is_full_sha, text};
use super::observation::{Fact, ObjectRef, Observation, pointer_segment, role};

const PAYLOAD: &str = "chains.tekton.dev/payload-";
const SIGNATURE: &str = "chains.tekton.dev/signature-";
const CERTIFICATE: &str = "chains.tekton.dev/cert-";
/// Chains writes the transparency log entry it uploaded to here, as a URL.
const TRANSPARENCY: &str = "chains.tekton.dev/transparency";

/// The DSSE payload type of an in-toto statement.
pub(crate) const IN_TOTO_PAYLOAD_TYPE: &str = "application/vnd.in-toto+json";

/// The most a payload or a signature annotation may decode to. Chains'
/// statements are a few KiB; 256 KiB leaves room for a pipeline of hundreds
/// of tasks, and anything larger is named, never truncated and trusted.
pub(crate) const MOST_DECODED_BYTES: usize = 256 * 1024;
/// The most attestations one run keeps. A PipelineRun's tasks each carry
/// their own, so this is per object; the rest are counted in
/// [`Attestations::omitted`].
pub(crate) const MOST_ATTESTATIONS: usize = 16;
/// The most subjects of one statement that are kept.
pub(crate) const MOST_SUBJECTS: usize = 64;

/// What one object carries: the attestations read, and how many past
/// [`MOST_ATTESTATIONS`] were left unread.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Attestations {
    pub read: Vec<Attestation>,
    pub omitted: usize,
}

/// One stored artifact's signature and payload, as the annotations hold
/// them.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Attestation {
    /// The annotations' suffix: `taskrun-<uid>`, `pipelinerun-<uid>` or an
    /// image's 12 digest digits.
    pub key: String,
    pub form: Form,
    pub stored: Stored,
    /// Whether a certificate annotation was present. Its bytes are not kept:
    /// trust is a key the person names, not a certificate a run carries.
    pub certificate: bool,
    /// The transparency log entry Chains annotated the object with, as
    /// written. A claim: never fetched from, since an annotation must not
    /// steer where a read goes.
    pub log_entry: Option<String>,
}

/// What the key says was signed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Form {
    /// The run itself: an in-toto statement in a DSSE envelope.
    Run,
    /// An image, signed with cosign's simple signing. Named and left unread
    /// until the registry step.
    Image,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Stored {
    Envelope(Envelope),
    /// Present, and not an attestation of the run.
    Unread,
    Malformed(Malformed),
}

/// A DSSE envelope as stored, with what its statement says.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Envelope {
    pub payload_type: String,
    /// The bytes the signatures cover with the payload type: the decoded
    /// `payload` of the envelope.
    pub payload: Vec<u8>,
    pub signatures: Vec<EnvelopeSignature>,
    /// Whether the `payload` annotation is the same statement the envelope
    /// carries. `None` when the annotation is absent.
    pub payload_annotation_agrees: Option<bool>,
    /// What the statement says. Unverified.
    pub claims: Claims,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EnvelopeSignature {
    /// The `keyid` the signer wrote, if any: a hint, never a match.
    pub key_id: Option<String>,
    pub bytes: Vec<u8>,
}

/// What an in-toto statement says, read before anything is verified.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Claims {
    pub predicate_type: Option<String>,
    pub subjects: Vec<Subject>,
    /// Subjects past [`MOST_SUBJECTS`].
    pub subjects_omitted: usize,
    /// The one commit the provenance's source materials name, when they
    /// name exactly one.
    pub commit: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Subject {
    pub name: Option<String>,
    /// `digest.sha256`, when the subject has one.
    pub digest: Option<Digest>,
}

/// Why an attestation could not be read.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Malformed {
    /// The payload or signature annotation is missing while the other is
    /// there.
    Missing(&'static str),
    NotBase64(&'static str),
    /// Decodes to more than [`MOST_DECODED_BYTES`].
    TooLarge(&'static str),
    NotAnEnvelope(&'static str),
    /// The envelope's payload is not an in-toto statement.
    NotAStatement(&'static str),
}

impl std::fmt::Display for Malformed {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Missing(which) => write!(f, "the {which} annotation is missing"),
            Self::NotBase64(which) => write!(f, "{which} is not base64"),
            Self::TooLarge(which) => {
                write!(
                    f,
                    "{which} is larger than {} KiB",
                    MOST_DECODED_BYTES / 1024
                )
            }
            Self::NotAnEnvelope(why) | Self::NotAStatement(why) => f.write_str(why),
        }
    }
}

/// The attestations an object's annotations hold, up to
/// [`MOST_ATTESTATIONS`]: the run's own first, then images', each in key order.
pub(crate) fn parse_attestations(object: &Value) -> Attestations {
    let Some(annotations) = object
        .pointer("/metadata/annotations")
        .and_then(Value::as_object)
    else {
        return Attestations::default();
    };
    let keys: BTreeSet<&str> = annotations
        .keys()
        .filter_map(|name| {
            name.strip_prefix(PAYLOAD)
                .or_else(|| name.strip_prefix(SIGNATURE))
        })
        .collect();
    let log_entry = annotations
        .get(TRANSPARENCY)
        .and_then(Value::as_str)
        .filter(|entry| !entry.is_empty())
        .map(str::to_owned);
    let get = |prefix: &str, key: &str| {
        annotations
            .get(&format!("{prefix}{key}"))
            .and_then(Value::as_str)
            .filter(|value| !value.is_empty())
    };
    // The run's own keys come first: an image's key starts with hex digits
    // and sorts before `taskrun-`, so in key order a TaskRun with many image
    // signatures would lose the one attestation that matters.
    let (own, images): (Vec<&str>, Vec<&str>) =
        keys.into_iter().partition(|key| form_of(key) == Form::Run);
    let mut found = Attestations::default();
    for key in own.into_iter().chain(images) {
        if found.read.len() == MOST_ATTESTATIONS {
            found.omitted += 1;
            continue;
        }
        let form = form_of(key);
        let stored = match form {
            Form::Image => Stored::Unread,
            Form::Run => match read_envelope(get(SIGNATURE, key), get(PAYLOAD, key)) {
                Ok(envelope) => Stored::Envelope(envelope),
                Err(problem) => Stored::Malformed(problem),
            },
        };
        found.read.push(Attestation {
            key: key.to_owned(),
            form,
            stored,
            certificate: get(CERTIFICATE, key).is_some(),
            log_entry: log_entry.clone(),
        });
    }
    found
}

fn form_of(key: &str) -> Form {
    if key.starts_with("taskrun-") || key.starts_with("pipelinerun-") {
        Form::Run
    } else {
        Form::Image
    }
}

fn decode(encoded: &str, which: &'static str) -> Result<Vec<u8>, Malformed> {
    // base64 expands by 4/3, so a longer text cannot decode to the cap.
    if encoded.len() > MOST_DECODED_BYTES / 3 * 4 + 4 {
        return Err(Malformed::TooLarge(which));
    }
    let bytes = STANDARD
        .decode(encoded.trim())
        .map_err(|_| Malformed::NotBase64(which))?;
    if bytes.len() > MOST_DECODED_BYTES {
        return Err(Malformed::TooLarge(which));
    }
    Ok(bytes)
}

fn read_envelope(signature: Option<&str>, payload: Option<&str>) -> Result<Envelope, Malformed> {
    let signature = signature.ok_or(Malformed::Missing("signature"))?;
    let envelope: Value = serde_json::from_slice(&decode(signature, "the signature")?)
        .map_err(|_| Malformed::NotAnEnvelope("the signature is not a DSSE envelope"))?;
    let payload_type = text(&envelope, "/payloadType")
        .ok_or(Malformed::NotAnEnvelope("the envelope has no payloadType"))?;
    let carried = decode(
        envelope
            .get("payload")
            .and_then(Value::as_str)
            .ok_or(Malformed::NotAnEnvelope("the envelope has no payload"))?,
        "the envelope's payload",
    )?;
    let entries = envelope
        .get("signatures")
        .and_then(Value::as_array)
        .map_or(&[][..], Vec::as_slice);
    // A signature that is not base64 is dropped; one bad entry beside a good
    // one does not spoil the envelope, but only bad ones are not "no
    // signature".
    let signatures: Vec<EnvelopeSignature> = entries
        .iter()
        .filter_map(|entry| {
            Some(EnvelopeSignature {
                key_id: text(entry, "/keyid"),
                bytes: STANDARD.decode(entry.get("sig")?.as_str()?).ok()?,
            })
        })
        .collect();
    if signatures.is_empty() {
        return Err(if entries.is_empty() {
            Malformed::NotAnEnvelope("the envelope has no signature")
        } else {
            Malformed::NotBase64("the envelope's signature")
        });
    }
    // The annotation only cross-checks: one that cannot be decoded
    // disagrees, and leaves the envelope readable.
    let payload_annotation_agrees =
        payload.map(|text| decode(text, "the payload").is_ok_and(|bytes| bytes == carried));
    if payload_type != IN_TOTO_PAYLOAD_TYPE {
        return Err(Malformed::NotAStatement(
            "the envelope is not an in-toto statement",
        ));
    }
    let statement: Value = serde_json::from_slice(&carried)
        .map_err(|_| Malformed::NotAStatement("the payload is not JSON"))?;
    Ok(Envelope {
        payload_type,
        payload: carried,
        signatures,
        payload_annotation_agrees,
        claims: claims_of(&statement),
    })
}

fn claims_of(statement: &Value) -> Claims {
    let list = statement
        .get("subject")
        .and_then(Value::as_array)
        .map_or(&[][..], Vec::as_slice);
    let subjects = list.iter().take(MOST_SUBJECTS).map(subject_of).collect();
    let subjects_omitted = list.len().saturating_sub(MOST_SUBJECTS);
    Claims {
        predicate_type: text(statement, "/predicateType"),
        subjects,
        subjects_omitted,
        commit: source_commit(statement),
    }
}

fn subject_of(subject: &Value) -> Subject {
    Subject {
        name: text(subject, "/name"),
        digest: text(subject, "/digest/sha256")
            .and_then(|hex| Digest::parse(&format!("sha256:{hex}"))),
    }
}

/// The commit a SLSA provenance names among its materials: v0.2's
/// `predicate.materials` (`digest.sha1`) and v1's
/// `predicate.buildDefinition.resolvedDependencies` (`digest.gitCommit`),
/// the entries whose `uri` is a `git+` one. None, or more than one different
/// commit, names no commit.
fn source_commit(statement: &Value) -> Option<String> {
    let lists = [
        "/predicate/materials",
        "/predicate/buildDefinition/resolvedDependencies",
    ];
    let commits: BTreeSet<String> = lists
        .iter()
        .filter_map(|pointer| statement.pointer(pointer).and_then(Value::as_array))
        .flatten()
        .filter(|material| text(material, "/uri").is_some_and(|uri| uri.starts_with("git+")))
        .filter_map(|material| {
            text(material, "/digest/sha1").or_else(|| text(material, "/digest/gitCommit"))
        })
        .filter(|commit| is_full_sha(commit))
        .map(|commit| commit.to_ascii_lowercase())
        .collect();
    match commits.len() {
        1 => commits.into_iter().next(),
        _ => None,
    }
}

#[cfg_attr(
    not(test),
    allow(dead_code, reason = "the join reads them in the verification step")
)]
impl Attestation {
    /// The annotation the signature was read from, as a JSON pointer into
    /// the run.
    pub(crate) fn signature_field(&self) -> String {
        format!(
            "/metadata/annotations/{}",
            pointer_segment(&format!("{SIGNATURE}{}", self.key))
        )
    }

    /// What was read, as a declared observation of the run it is an
    /// annotation of: an annotation is written by whoever can patch the run,
    /// so it is declared however it was signed.
    pub(crate) fn observation(&self, run: ObjectRef) -> Observation {
        let value = match &self.stored {
            Stored::Envelope(envelope) => envelope
                .claims
                .subjects
                .iter()
                .find_map(|subject| subject.digest.as_ref())
                .map(Digest::to_string),
            Stored::Unread | Stored::Malformed(_) => None,
        };
        Observation::new(
            role::TEKTON,
            run,
            &self.signature_field(),
            value.as_deref(),
            Fact::Declared,
        )
    }
}

mod config;

#[cfg(test)]
mod tests;
