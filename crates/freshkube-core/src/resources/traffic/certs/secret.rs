//! The one place a TLS Secret is read (#949, N4). It runs only on request,
//! for a Secret no cert-manager Certificate answers for, and it is a reveal
//! path with a narrow mouth: the Secret is read by name, `tls.crt` is parsed
//! here, and what leaves is [`TlsFacts`], which has no field for `tls.key`,
//! for the raw bytes or for any other key. The decoded values of the Secret
//! are wiped before this returns. Safety guarantee 21.
use crate::failure::{Failure, FailureKind};
use k8s_openapi::api::core::v1::Secret;
use kube::{Api, Client};
use x509_parser::extensions::GeneralName;
use x509_parser::pem::Pem;
use zeroize::Zeroize;

/// What a certificate says about itself: the parsed `tls.crt` and nothing
/// else. Times are Unix seconds.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TlsFacts {
    pub subject: String,
    /// The subject alternative names (DNS names and addresses).
    pub names: Vec<String>,
    pub issuer: String,
    pub not_before: i64,
    pub not_after: i64,
}

/// How reading one Secret ended.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SecretOutcome {
    Read(TlsFacts),
    /// A Secret of another type with no `tls.crt`.
    NotTls {
        kind: String,
    },
    /// A TLS Secret without a `tls.crt`.
    NoCertificate,
    /// `tls.crt` is not a certificate; the words say how, and never quote it.
    Unparseable(String),
    /// The Secret does not exist (any more).
    NotFound,
    /// The read was refused or failed: unknown, not "wrong".
    Refused(Failure),
}

/// A Secret to read, which only [`super::CertEntry::secret_request`] makes,
/// and it makes none for a Secret a Certificate's status answers for.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SecretRequest {
    pub(super) namespace: String,
    pub(super) name: String,
}

impl SecretRequest {
    pub fn namespace(&self) -> &str {
        &self.namespace
    }

    pub fn name(&self) -> &str {
        &self.name
    }
}

/// Reads one Secret by name (a single GET) and keeps only the certificate's
/// facts.
pub async fn read_tls_secret(client: &Client, request: &SecretRequest) -> SecretOutcome {
    match Api::<Secret>::namespaced(client.clone(), &request.namespace)
        .get(&request.name)
        .await
    {
        Ok(secret) => facts_from_secret(secret),
        Err(error) => {
            let failure = Failure::from_kube(error);
            if failure.kind == FailureKind::NotFound {
                SecretOutcome::NotFound
            } else {
                SecretOutcome::Refused(failure)
            }
        }
    }
}

/// The facts of a Secret's `tls.crt`. Takes the Secret by value and wipes
/// every value in it, `tls.key` included, before it returns.
pub fn facts_from_secret(mut secret: Secret) -> SecretOutcome {
    let outcome = {
        let data = secret.data.as_ref();
        match data.and_then(|data| data.get("tls.crt")) {
            Some(crt) => parse_certificate(&crt.0),
            None if secret.type_.as_deref() == Some("kubernetes.io/tls") => {
                SecretOutcome::NoCertificate
            }
            None => SecretOutcome::NotTls {
                kind: secret.type_.clone().unwrap_or_else(|| "Opaque".into()),
            },
        }
    };
    for value in secret.data.iter_mut().flat_map(|data| data.values_mut()) {
        value.0.zeroize();
    }
    for value in secret
        .string_data
        .iter_mut()
        .flat_map(|data| data.values_mut())
    {
        value.zeroize();
    }
    outcome
}

fn parse_certificate(pem: &[u8]) -> SecretOutcome {
    let Some(Ok(block)) = Pem::iter_from_buffer(pem).next() else {
        return SecretOutcome::Unparseable("tls.crt holds no PEM block".into());
    };
    if block.label != "CERTIFICATE" {
        return SecretOutcome::Unparseable("the first PEM block is not a certificate".into());
    }
    let Ok(certificate) = block.parse_x509() else {
        return SecretOutcome::Unparseable("the certificate could not be parsed".into());
    };
    let names = certificate
        .subject_alternative_name()
        .ok()
        .flatten()
        .map(|extension| {
            extension
                .value
                .general_names
                .iter()
                .filter_map(|name| match name {
                    GeneralName::DNSName(dns) => Some((*dns).to_owned()),
                    GeneralName::IPAddress(bytes) => ip(bytes),
                    _ => None,
                })
                .collect()
        })
        .unwrap_or_default();
    let validity = certificate.validity();
    SecretOutcome::Read(TlsFacts {
        subject: certificate.subject().to_string(),
        names,
        issuer: certificate.issuer().to_string(),
        not_before: validity.not_before.timestamp(),
        not_after: validity.not_after.timestamp(),
    })
}

fn ip(bytes: &[u8]) -> Option<String> {
    match bytes.len() {
        4 => Some(std::net::Ipv4Addr::new(bytes[0], bytes[1], bytes[2], bytes[3]).to_string()),
        16 => Some(std::net::Ipv6Addr::from(<[u8; 16]>::try_from(bytes).ok()?).to_string()),
        _ => None,
    }
}
