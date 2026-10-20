//! The key the person names: a public key file, never read from the cluster.
//! ECDSA over P-256 is what `cosign generate-key-pair` writes and what
//! Chains signs with by default; any other key type is named and refused,
//! never accepted without a check.

use std::fmt;
use std::io::Read;
use std::path::Path;

use base64::Engine;
use base64::engine::general_purpose::STANDARD_NO_PAD;
use p256::ecdsa::VerifyingKey;
use p256::pkcs8::spki::SubjectPublicKeyInfoRef;
use p256::pkcs8::{DecodePublicKey, Document};
use sha2::{Digest as _, Sha256};

/// The most a key file may hold. A P-256 public key in PEM is about 180
/// bytes; this leaves room for a comment and nothing for a bundle of keys.
pub(crate) const MOST_KEY_FILE_BYTES: u64 = 8 * 1024;

const EC_PUBLIC_KEY: &str = "1.2.840.10045.2.1";
const PRIME256V1: &str = "1.2.840.10045.3.1.7";
const SECP384R1: &str = "1.3.132.0.34";
const ED25519: &str = "1.3.101.112";
const RSA_ENCRYPTION: &str = "1.2.840.113549.1.1.1";

/// A public key that can check a signature.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct TrustedKey {
    key: VerifyingKey,
    fingerprint: String,
}

/// Why a key file cannot be used.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum KeyProblem {
    /// The file could not be read, or is not a regular file or too large.
    Unreadable(String),
    /// A private key block. The app reads a public key and nothing more.
    PrivateKey,
    /// Not exactly one `PUBLIC KEY` block (a certificate, or several keys).
    NotOnePublicKey,
    /// A `PUBLIC KEY` block that does not decode.
    Malformed,
    /// A key type this step does not check, by name.
    Unsupported(&'static str),
}

impl fmt::Display for KeyProblem {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unreadable(why) => write!(f, "the key file can't be read: {why}"),
            Self::PrivateKey => {
                f.write_str("the key file holds a private key; name the public key")
            }
            Self::NotOnePublicKey => {
                f.write_str("the key file does not hold exactly one PUBLIC KEY block")
            }
            Self::Malformed => f.write_str("the key file's public key can't be decoded"),
            Self::Unsupported(kind) => {
                write!(f, "{kind} keys are not supported yet; use ECDSA P-256")
            }
        }
    }
}

impl TrustedKey {
    /// From the text of a key file.
    pub(crate) fn from_pem(text: &str) -> Result<Self, KeyProblem> {
        if text.contains("PRIVATE KEY") {
            return Err(KeyProblem::PrivateKey);
        }
        if text.matches("-----BEGIN").count() != 1 || !text.contains("-----BEGIN PUBLIC KEY-----") {
            return Err(KeyProblem::NotOnePublicKey);
        }
        let der = Document::from_public_key_pem(text).map_err(|_| KeyProblem::Malformed)?;
        if let Some(kind) = unsupported(der.as_bytes()) {
            return Err(KeyProblem::Unsupported(kind));
        }
        let key =
            VerifyingKey::from_public_key_der(der.as_bytes()).map_err(|_| KeyProblem::Malformed)?;
        let digest = Sha256::digest(der.as_bytes());
        Ok(Self {
            key,
            fingerprint: format!("SHA256:{}", STANDARD_NO_PAD.encode(digest)),
        })
    }

    /// From a file: a regular file of at most [`MOST_KEY_FILE_BYTES`].
    pub(crate) fn read(path: &Path) -> Result<Self, KeyProblem> {
        let unreadable = |error: std::io::Error| KeyProblem::Unreadable(error.kind().to_string());
        let file = std::fs::File::open(path).map_err(unreadable)?;
        let meta = file.metadata().map_err(unreadable)?;
        if !meta.is_file() {
            return Err(KeyProblem::Unreadable("not a regular file".into()));
        }
        let mut text = String::new();
        file.take(MOST_KEY_FILE_BYTES + 1)
            .read_to_string(&mut text)
            .map_err(unreadable)?;
        if text.len() as u64 > MOST_KEY_FILE_BYTES {
            return Err(KeyProblem::Unreadable("larger than 8 KiB".into()));
        }
        Self::from_pem(&text)
    }

    /// `SHA256:` and the base64 of the SHA-256 of the key's DER
    /// `SubjectPublicKeyInfo`: what the screens show, never the key itself.
    pub(crate) fn fingerprint(&self) -> &str {
        &self.fingerprint
    }

    /// Whether `signature` (ASN.1 DER, as cosign and Chains write it) is
    /// this key's signature over `message`.
    pub(crate) fn verifies(&self, message: &[u8], signature: &[u8]) -> bool {
        use p256::ecdsa::signature::Verifier;
        p256::ecdsa::Signature::from_der(signature)
            .is_ok_and(|signature| self.key.verify(message, &signature).is_ok())
    }
}

/// The name of a well-formed key of a type that is not ECDSA P-256.
fn unsupported(der: &[u8]) -> Option<&'static str> {
    let info = SubjectPublicKeyInfoRef::try_from(der).ok()?;
    let algorithm = info.algorithm.oid.to_string();
    let curve = info
        .algorithm
        .parameters_oid()
        .map(|oid| oid.to_string())
        .unwrap_or_default();
    match (algorithm.as_str(), curve.as_str()) {
        (EC_PUBLIC_KEY, PRIME256V1) => None,
        (EC_PUBLIC_KEY, SECP384R1) => Some("ECDSA P-384"),
        (EC_PUBLIC_KEY, _) => Some("ECDSA (another curve)"),
        (ED25519, _) => Some("Ed25519"),
        (RSA_ENCRYPTION, _) => Some("RSA"),
        _ => Some("this type of"),
    }
}
