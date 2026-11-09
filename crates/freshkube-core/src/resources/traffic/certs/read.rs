//! The reads behind the Certificates page: the Gateway API read the Routes
//! page makes (for listeners and their routes' hosts), the Ingresses, and
//! cert-manager's Certificates and, only when some Certificate is not issued,
//! its CertificateRequests, Orders and Challenges. All are bounded lists. No
//! Secret is read here: a Secret is read only by `read_tls_secret`, on
//! request.
use super::chain::{Issuance, Lookup, is_issued};
use super::facts::{CertificateFacts, ChallengeFacts, IngressFacts, OrderFacts, RequestFacts};
use crate::failure::Failure;
use crate::resources::traffic::routes::{Listing, RoutesRead, list_json, read_routes};
use crate::resources::{ResourceKind, list_custom_groups, list_group_kinds};
use serde_json::Value;

pub const CERTIFICATE_LIMIT: u32 = 500;
pub const INGRESS_LIMIT: u32 = 500;
pub const ISSUANCE_LIMIT: u32 = 500;

pub const CERT_MANAGER_GROUP: &str = "cert-manager.io";
pub const ACME_GROUP: &str = "acme.cert-manager.io";

/// Whether cert-manager is there to ask.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Manager {
    /// No `cert-manager.io` group: its issuance is not shown, and the page
    /// says so rather than "no certificates".
    NotInstalled,
    /// The group is there without the kinds the page needs.
    Incomplete { missing: Vec<&'static str> },
    /// Usable. `acme` says whether Orders and Challenges are served.
    Ready { acme: bool },
    /// Discovery failed, so whether it is installed is not known.
    Unknown(Failure),
}

impl Manager {
    /// The plain reading of cert-manager not being usable, for a banner.
    pub fn problem(&self) -> Option<String> {
        match self {
            Self::NotInstalled => Some(
                "cert-manager is not installed in this cluster, so Certificates and their issuance are not shown. Certificates a Gateway listener or Ingress serves are read from their Secrets on request."
                    .into(),
            ),
            Self::Incomplete { missing } => Some(format!(
                "cert-manager is only partly installed: {} not served.",
                missing.join(", ")
            )),
            Self::Unknown(failure) => Some(format!(
                "Whether cert-manager is installed could not be read ({}: {}).",
                failure.kind.label(),
                failure.message
            )),
            Self::Ready { .. } => None,
        }
    }
}

/// Everything one read of the page found.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub struct CertsRead {
    /// The Gateways and routes, read as the Routes page reads them.
    pub gateway: Result<RoutesRead, Failure>,
    pub ingresses: Result<Listing<IngressFacts>, Failure>,
    pub manager: Manager,
    /// Empty and Ok when cert-manager is not usable.
    pub certificates: Result<Listing<CertificateFacts>, Failure>,
    pub issuance: Issuance,
    /// Certificate at the version the cluster serves it, for a link to one.
    pub certificate_kind: Option<ResourceKind>,
    pub notes: Vec<String>,
}

impl CertsRead {
    pub fn new(manager: Manager) -> Self {
        Self {
            gateway: Ok(RoutesRead::new(
                crate::resources::traffic::routes::ApiState::NotInstalled,
            )),
            ingresses: Ok(Listing::new(Vec::new(), false)),
            manager,
            certificates: Ok(Listing::new(Vec::new(), false)),
            issuance: Issuance::skipped(),
            certificate_kind: None,
            notes: Vec::new(),
        }
    }

    pub fn with_gateway(mut self, gateway: Result<RoutesRead, Failure>) -> Self {
        self.gateway = gateway;
        self
    }

    pub fn with_ingresses(mut self, ingresses: Result<Listing<IngressFacts>, Failure>) -> Self {
        self.ingresses = ingresses;
        self
    }

    pub fn with_certificates(
        mut self,
        certificates: Result<Listing<CertificateFacts>, Failure>,
        issuance: Issuance,
    ) -> Self {
        self.certificates = certificates;
        self.issuance = issuance;
        self
    }

    pub fn with_certificate_kind(mut self, kind: ResourceKind) -> Self {
        self.certificate_kind = Some(kind);
        self
    }

    pub fn with_notes(mut self, notes: Vec<String>) -> Self {
        self.notes = notes;
        self
    }
}

/// The cert-manager kinds this page lists, each at the version served.
struct Kinds {
    certificate: ResourceKind,
    request: ResourceKind,
    order: Option<ResourceKind>,
    challenge: Option<ResourceKind>,
}

async fn discover(client: &kube::Client) -> (Manager, Option<Kinds>) {
    let groups = match list_custom_groups(client).await {
        Ok(groups) => groups,
        Err(failure) => return (Manager::Unknown(failure), None),
    };
    let Some(group) = groups.iter().find(|g| g.name == CERT_MANAGER_GROUP) else {
        return (Manager::NotInstalled, None);
    };
    let served = match list_group_kinds(client, group).await {
        Ok(served) => served,
        Err(failure) => return (Manager::Unknown(failure), None),
    };
    let find = |kinds: &[ResourceKind], kind: &str| kinds.iter().find(|k| k.kind == kind).cloned();
    let (certificate, request) = (
        find(&served.kinds, "Certificate"),
        find(&served.kinds, "CertificateRequest"),
    );
    let missing: Vec<&'static str> = [
        ("Certificate", certificate.is_none()),
        ("CertificateRequest", request.is_none()),
    ]
    .into_iter()
    .filter_map(|(name, absent)| absent.then_some(name))
    .collect();
    let (Some(certificate), Some(request)) = (certificate, request) else {
        return (Manager::Incomplete { missing }, None);
    };
    let (mut order, mut challenge) = (None, None);
    if let Some(acme) = groups.iter().find(|g| g.name == ACME_GROUP)
        && let Ok(kinds) = list_group_kinds(client, acme).await
    {
        order = find(&kinds.kinds, "Order");
        challenge = find(&kinds.kinds, "Challenge");
    }
    (
        Manager::Ready {
            acme: order.is_some() && challenge.is_some(),
        },
        Some(Kinds {
            certificate,
            request,
            order,
            challenge,
        }),
    )
}

async fn list_as<T>(
    client: &kube::Client,
    kind: &ResourceKind,
    limit: u32,
    parse: fn(&Value) -> Option<T>,
) -> Result<Listing<T>, Failure> {
    let list = list_json(client, kind, limit).await?;
    Ok(Listing::new(
        list.items.iter().filter_map(parse).collect(),
        list.cut,
    ))
}

fn ingress_kind() -> ResourceKind {
    ResourceKind::new("networking.k8s.io", "v1", "Ingress", "ingresses", true)
}

fn note<T>(what: &str, result: &Result<Listing<T>, Failure>, notes: &mut Vec<String>) {
    match result {
        Err(failure) => notes.push(format!(
            "{what} could not be read ({}: {})",
            failure.kind.label(),
            failure.message
        )),
        Ok(list) if list.cut => notes.push(format!("{what} were cut at their limit")),
        Ok(_) => {}
    }
}

/// Reads what the Certificates page shows: lists and gets only, and no Secret.
pub async fn read_certificates(client: &kube::Client) -> CertsRead {
    let ingress = ingress_kind();
    let (gateway, ingresses, (manager, kinds)) = futures::join!(
        read_routes(client, None),
        list_as(client, &ingress, INGRESS_LIMIT, IngressFacts::parse),
        discover(client),
    );
    let mut notes = Vec::new();
    note("Ingresses", &ingresses, &mut notes);
    let mut read = CertsRead::new(manager)
        .with_gateway(gateway)
        .with_ingresses(ingresses);
    if let Some(kinds) = kinds {
        let certificates = list_as(
            client,
            &kinds.certificate,
            CERTIFICATE_LIMIT,
            CertificateFacts::parse,
        )
        .await;
        note("Certificates", &certificates, &mut notes);
        let busy = certificates
            .as_ref()
            .map(|list| !list.items.iter().all(is_issued))
            .unwrap_or(false);
        let issuance = if busy {
            let requests = Lookup::Read(
                list_as(client, &kinds.request, ISSUANCE_LIMIT, RequestFacts::parse).await,
            );
            let orders = match &kinds.order {
                Some(kind) => {
                    Lookup::Read(list_as(client, kind, ISSUANCE_LIMIT, OrderFacts::parse).await)
                }
                None => Lookup::NotServed,
            };
            let challenges = match &kinds.challenge {
                Some(kind) => {
                    Lookup::Read(list_as(client, kind, ISSUANCE_LIMIT, ChallengeFacts::parse).await)
                }
                None => Lookup::NotServed,
            };
            Issuance {
                requests,
                orders,
                challenges,
            }
        } else {
            Issuance::skipped()
        };
        read = read
            .with_certificates(certificates, issuance)
            .with_certificate_kind(kinds.certificate);
    }
    read.with_notes(notes)
}
