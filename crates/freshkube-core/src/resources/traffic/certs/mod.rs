//! Certificates and cert-manager issuance (#949, slice N4). Read-only, and no
//! UI types: the Certificates page draws what this answers.
//!
//! Each TLS Secret a Gateway listener or an Ingress serves is one entry, with
//! the hosts served under it, its names and expiry, and any host it does not
//! name ("wrong name"). Where a cert-manager Certificate keeps the Secret, the
//! facts come from the Certificate's status and **no Secret is read**. Any
//! other Secret is read only when the person asks (`read_tls_secret`), parsed
//! here, and only the facts of `tls.crt` leave: `tls.key` is never part of any
//! type in this module (safety guarantee 21). cert-manager's issuance
//! (Certificate → CertificateRequest → Order → Challenge) reads like a route's
//! trace: it stops at the first break and says what cert-manager wrote.
mod chain;
pub mod example;
mod facts;
mod names;
mod read;
mod secret;
mod view;

pub use chain::{Chain, Issuance, Lookup, Step, StepKind, Verdict, is_issued, issuance_chain};
pub use facts::{
    CertificateFacts, ChallengeFacts, IngressFacts, IngressTls, IssuerRef, OrderFacts, RequestFacts,
};
pub use names::{covers, uncovered};
pub use read::{
    ACME_GROUP, CERT_MANAGER_GROUP, CERTIFICATE_LIMIT, CertsRead, INGRESS_LIMIT, ISSUANCE_LIMIT,
    Manager, read_certificates,
};
pub use secret::{SecretOutcome, SecretRequest, TlsFacts, facts_from_secret, read_tls_secret};
pub use view::{CertEntry, CertState, CertsView, EXPIRING_DAYS, SecretState, Serving, Via};

#[cfg(test)]
mod tests;
