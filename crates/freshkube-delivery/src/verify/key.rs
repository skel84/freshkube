//! The key the person names: a public key file, never read from the cluster.
//! ECDSA over P-256 is what `cosign generate-key-pair` writes and what
//! Chains signs with by default; any other key type is named and refused,
//! never accepted without a check.
//!
//! The check is `ring`'s, already in the build for TLS. A P-256 public key in
//! `SubjectPublicKeyInfo` form is a fixed 27-byte prefix, which names the
//! algorithm, pins the curve and starts the uncompressed point, then that
//! point's other 64 bytes; the prefix is compared whole, so a key of any other
//! shape cannot pass for one.

use std::fmt;
use std::io::Read;
use std::path::Path;

use base64::Engine;
use base64::engine::general_purpose::{STANDARD, STANDARD_NO_PAD};
use ring::digest;
use ring::signature::{ECDSA_P256_SHA256_ASN1, UnparsedPublicKey};

/// The most a key file may hold. A P-256 public key in PEM is about 180
/// bytes; this leaves room for a comment and nothing for a bundle of keys.
pub(crate) const MOST_KEY_FILE_BYTES: u64 = 8 * 1024;

/// DER of `SEQUENCE { SEQUENCE { id-ecPublicKey, prime256v1 }, BIT STRING`
/// up to the point: 27 bytes (the last is the `0x04` of an uncompressed point),
/// followed by that point's other 64 bytes.
const P256_PREFIX: [u8; 27] = [
    0x30, 0x59, 0x30, 0x13, 0x06, 0x07, 0x2a, 0x86, 0x48, 0xce, 0x3d, 0x02, 0x01, 0x06, 0x08, 0x2a,
    0x86, 0x48, 0xce, 0x3d, 0x03, 0x01, 0x07, 0x03, 0x42, 0x00, 0x04,
];
const P256_DER_LEN: usize = 91;

// The content of the algorithm OIDs, to name a key we refuse.
const EC_PUBLIC_KEY: &[u8] = &[0x2a, 0x86, 0x48, 0xce, 0x3d, 0x02, 0x01];
const PRIME256V1: &[u8] = &[0x2a, 0x86, 0x48, 0xce, 0x3d, 0x03, 0x01, 0x07];
const SECP384R1: &[u8] = &[0x2b, 0x81, 0x04, 0x00, 0x22];
const ED25519: &[u8] = &[0x2b, 0x65, 0x70];
const RSA_ENCRYPTION: &[u8] = &[0x2a, 0x86, 0x48, 0x86, 0xf7, 0x0d, 0x01, 0x01, 0x01];

/// A public key that can check a signature.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct TrustedKey {
    /// The uncompressed point, `04 || X || Y`.
    point: [u8; 65],
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
        const BEGIN: &str = "-----BEGIN PUBLIC KEY-----";
        const END: &str = "-----END PUBLIC KEY-----";
        if text.matches("-----BEGIN").count() != 1 || text.matches(BEGIN).count() != 1 {
            return Err(KeyProblem::NotOnePublicKey);
        }
        let (_, rest) = text.split_once(BEGIN).ok_or(KeyProblem::NotOnePublicKey)?;
        let (body, after) = rest.split_once(END).ok_or(KeyProblem::NotOnePublicKey)?;
        if !after.trim().is_empty() {
            return Err(KeyProblem::NotOnePublicKey);
        }
        let encoded: String = body.split_whitespace().collect();
        let der = STANDARD
            .decode(encoded)
            .map_err(|_| KeyProblem::Malformed)?;
        Self::from_der(&der)
    }

    fn from_der(der: &[u8]) -> Result<Self, KeyProblem> {
        if der.len() == P256_DER_LEN && der.starts_with(&P256_PREFIX) {
            let mut point = [0u8; 65];
            point.copy_from_slice(&der[P256_DER_LEN - 65..]);
            let fingerprint = digest::digest(&digest::SHA256, der);
            return Ok(Self {
                point,
                fingerprint: format!("SHA256:{}", STANDARD_NO_PAD.encode(fingerprint)),
            });
        }
        Err(match other_algorithm(der) {
            Some(kind) => KeyProblem::Unsupported(kind),
            // A P-256 key of another shape (a compressed point, extra
            // fields), or not a key at all, is not one we can check.
            None => KeyProblem::Malformed,
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
    /// this key's signature over `message`. A point that is not on the curve
    /// verifies nothing.
    pub(crate) fn verifies(&self, message: &[u8], signature: &[u8]) -> bool {
        UnparsedPublicKey::new(&ECDSA_P256_SHA256_ASN1, &self.point)
            .verify(message, signature)
            .is_ok()
    }

    /// The `SubjectPublicKeyInfo` PEM of a P-256 point: what a key file holds.
    #[cfg(test)]
    pub(crate) fn pem_of_point(point: &[u8]) -> String {
        let mut der = P256_PREFIX[..P256_PREFIX.len() - 1].to_vec();
        der.extend_from_slice(point);
        format!(
            "-----BEGIN PUBLIC KEY-----\n{}\n-----END PUBLIC KEY-----\n",
            STANDARD.encode(der)
        )
    }
}

/// The name of a well-formed key of a type that is not ECDSA P-256, from a
/// minimal read of `SEQUENCE { SEQUENCE { OID, … }, … }`. `None` for P-256
/// itself and for anything that does not read as a key.
fn other_algorithm(der: &[u8]) -> Option<&'static str> {
    let (outer, _) = element(der, 0x30)?;
    let (algorithm, _) = element(outer, 0x30)?;
    let (oid, parameters) = element(algorithm, 0x06)?;
    let curve = element(parameters, 0x06).map(|(curve, _)| curve);
    match (oid, curve) {
        (EC_PUBLIC_KEY, Some(PRIME256V1)) => None,
        (EC_PUBLIC_KEY, Some(SECP384R1)) => Some("ECDSA P-384"),
        (EC_PUBLIC_KEY, _) => Some("ECDSA (another curve)"),
        (ED25519, _) => Some("Ed25519"),
        (RSA_ENCRYPTION, _) => Some("RSA"),
        _ => Some("an unknown type of"),
    }
}

/// One DER element with `tag`: its content and what follows it. Lengths
/// beyond two bytes are refused; a key is a few hundred bytes.
fn element(bytes: &[u8], tag: u8) -> Option<(&[u8], &[u8])> {
    let (&first, rest) = bytes.split_first()?;
    if first != tag {
        return None;
    }
    let (&length, rest) = rest.split_first()?;
    let (length, rest) = match length {
        0..=0x7f => (usize::from(length), rest),
        0x81 => {
            let (&length, rest) = rest.split_first()?;
            (usize::from(length), rest)
        }
        0x82 => {
            let (high, rest) = rest.split_at_checked(2)?;
            (usize::from(u16::from_be_bytes([high[0], high[1]])), rest)
        }
        _ => return None,
    };
    rest.split_at_checked(length)
}
