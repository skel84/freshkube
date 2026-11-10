//! What the Certificates page draws from one read: one entry per TLS Secret
//! a Gateway listener or an Ingress serves, plus the cert-manager
//! Certificates nothing serves yet. An entry's names and expiry come from
//! cert-manager's Certificate status where it manages the Secret (no Secret
//! is read), and otherwise from the parsed `tls.crt` once the person asks.
//! A fact that was not read is Unknown, never "right".
use super::chain::{Chain, issuance_chain};
use super::facts::{CertificateFacts, IngressFacts};
use super::names::uncovered;
use super::read::{CertsRead, Manager};
use super::secret::{SecretOutcome, SecretRequest, TlsFacts};
use crate::resources::traffic::routes::{GatewayFacts, RouteEntry, hosts_overlap};
use std::collections::BTreeMap;

/// Expiry under this many days reads Expiring.
pub const EXPIRING_DAYS: i64 = 14;
const DAY: i64 = 86_400;

/// How a Gateway listener or an Ingress serves a Secret.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Via {
    Listener {
        /// `namespace/name`.
        gateway: String,
        listener: String,
        port: i32,
    },
    Ingress {
        /// `namespace/name`.
        ingress: String,
    },
}

impl Via {
    pub fn words(&self) -> String {
        match self {
            Self::Listener {
                gateway,
                listener,
                port,
            } => format!("Gateway {gateway}, listener {listener} (:{port})"),
            Self::Ingress { ingress } => format!("Ingress {ingress}"),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Serving {
    pub via: Via,
    /// The hosts it serves under this certificate: a listener's hostname and
    /// its routes' hostnames, or an Ingress's TLS hosts. Empty when none are
    /// named, so nothing can be judged wrong.
    pub hosts: Vec<String>,
}

/// Worst first; the page groups by it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum CertState {
    /// Expired, or the issuance stopped, or the Secret is missing.
    Broken,
    /// The certificate doesn't name a host that is served under it.
    WrongName,
    /// Valid still, but its last renewal attempt failed.
    Failing,
    /// Being issued; nothing is wrong yet.
    Issuing,
    /// Expires in under 14 days.
    Expiring,
    /// The names and expiry were not read, or couldn't be.
    Unknown,
    Healthy,
}

impl CertState {
    /// Worst first, which is the order the page groups in.
    pub const ALL: [Self; 7] = [
        Self::Broken,
        Self::WrongName,
        Self::Failing,
        Self::Issuing,
        Self::Expiring,
        Self::Unknown,
        Self::Healthy,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::Broken => "Not valid",
            Self::WrongName => "Wrong name",
            Self::Failing => "Renewal failing",
            Self::Issuing => "Issuing",
            Self::Expiring => "Expiring",
            Self::Unknown => "Unknown",
            Self::Healthy => "Valid",
        }
    }
}

/// How long is left in words, whole days rounded down, and "under a day"
/// below one. `left` is seconds to `notAfter`; zero or less is expired.
pub fn expiry_words(left: i64) -> String {
    if left <= 0 {
        let ago = -left;
        return match ago / DAY {
            0 => "expired under a day ago".into(),
            1 => "expired 1 day ago".into(),
            days => format!("expired {days} days ago"),
        };
    }
    match left / DAY {
        0 => "in under a day".into(),
        1 => "in 1 day".into(),
        days => format!("in {days} days"),
    }
}

/// What the Secret read stands at, for a Secret no Certificate answers for.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SecretState {
    /// Not read: it is read only when the person asks.
    NotRead,
    Read(SecretOutcome),
}

#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct CertEntry {
    pub namespace: String,
    /// The Secret's name.
    pub secret: String,
    pub servings: Vec<Serving>,
    /// The cert-manager Certificate that keeps the Secret, when there is one.
    pub managed: Option<CertificateFacts>,
    pub chain: Option<Chain>,
    pub secret_state: SecretState,
    /// What the certificate names, from the Certificate or the Secret; none
    /// when it is not known.
    pub names: Option<Vec<String>>,
    pub subject: Option<String>,
    pub issuer: Option<String>,
    pub not_before: Option<i64>,
    pub not_after: Option<i64>,
    /// Hosts served under it that no name covers.
    pub wrong: Vec<String>,
    pub state: CertState,
    pub reason: String,
    /// Whole days left, negative once expired.
    pub days_left: Option<i64>,
    /// Seconds to `notAfter`, negative once expired.
    pub left_secs: Option<i64>,
    /// Why the Secret may not be read: Certificates that may keep it could
    /// not all be read.
    pub withheld: Option<String>,
}

impl CertEntry {
    pub fn key(&self) -> String {
        format!("{}/{}", self.namespace, self.secret)
    }

    /// Where the facts come from, in words.
    pub fn source(&self) -> String {
        match (&self.managed, &self.secret_state) {
            (Some(cert), _) => format!("cert-manager Certificate {}", cert.key()),
            (None, SecretState::Read(SecretOutcome::Read(_))) => {
                "the Secret's certificate, read on request".into()
            }
            (None, _) => "not read".into(),
        }
    }

    /// Every host served under it.
    pub fn hosts(&self) -> Vec<String> {
        let mut hosts: Vec<String> = Vec::new();
        for host in self.servings.iter().flat_map(|s| &s.hosts) {
            if !hosts.contains(host) {
                hosts.push(host.clone());
            }
        }
        hosts
    }

    /// The Secret to read when the person asks. None where a cert-manager
    /// Certificate's status answers (the Secret is never read then), and
    /// none once it has been read.
    pub fn secret_request(&self) -> Option<SecretRequest> {
        self.can_read_secret().then(|| SecretRequest {
            namespace: self.namespace.clone(),
            name: self.secret.clone(),
        })
    }

    pub fn can_read_secret(&self) -> bool {
        // A refused or failed read may be tried again; a read one may not.
        self.managed.is_none()
            && self.withheld.is_none()
            && matches!(
                self.secret_state,
                SecretState::NotRead | SecretState::Read(SecretOutcome::Refused(_))
            )
    }

    fn facts(&mut self, now: i64) {
        self.names = None;
        self.subject = None;
        self.issuer = None;
        self.not_before = None;
        self.not_after = None;
        if let Some(cert) = &self.managed {
            self.names = Some(cert.names.clone());
            self.issuer = cert
                .issuer
                .as_ref()
                .map(|i| format!("{} {}", i.kind, i.name));
            self.not_before = cert.not_before;
            self.not_after = cert.not_after;
        } else if let SecretState::Read(SecretOutcome::Read(TlsFacts {
            subject,
            names,
            issuer,
            not_before,
            not_after,
        })) = &self.secret_state
        {
            self.names = Some(names.clone());
            self.subject = Some(subject.clone());
            self.issuer = Some(issuer.clone());
            self.not_before = Some(*not_before);
            self.not_after = Some(*not_after);
        }
        self.left_secs = self.not_after.map(|end| end - now);
        self.days_left = self.left_secs.map(|left| left.div_euclid(DAY));
    }

    /// Works out the state from what is known. Pure: the page calls it again
    /// when a Secret has been read.
    pub fn assess(&mut self, now: i64) {
        self.facts(now);
        self.wrong = match &self.names {
            Some(names) => uncovered(names, &self.hosts()),
            None => Vec::new(),
        };
        let (state, reason) = self.judge(now);
        self.state = state;
        self.reason = reason;
    }

    fn judge(&self, now: i64) -> (CertState, String) {
        // A renewal that failed while the certificate still serves is
        // Failing, whatever step of the retry broke: it is not yet not valid.
        // It only keeps the chain's break from reading Not valid; a wrong
        // name is the worse state and is checked first.
        let failing = self.chain.as_ref().and_then(|chain| {
            let warn = chain
                .steps
                .iter()
                .find(|s| s.verdict == super::chain::Verdict::Warn)?;
            self.not_after
                .is_none_or(|end| end > now)
                .then(|| match chain.stopped_at() {
                    Some(step) if step.verdict != super::chain::Verdict::Warn => {
                        format!("{}; {}", warn.says, chain.line())
                    }
                    _ => warn.says.clone(),
                })
        });
        if failing.is_none()
            && let Some(chain) = &self.chain
            && chain.is_broken()
        {
            return (CertState::Broken, chain.line());
        }
        if let SecretState::Read(outcome) = &self.secret_state {
            match outcome {
                SecretOutcome::NotFound => {
                    return (
                        CertState::Broken,
                        "the Secret does not exist, so nothing is served".into(),
                    );
                }
                SecretOutcome::NoCertificate => {
                    return (CertState::Broken, "the Secret holds no tls.crt".into());
                }
                SecretOutcome::NotTls { kind } => {
                    return (
                        CertState::Broken,
                        format!("the Secret is of type {kind} and holds no tls.crt"),
                    );
                }
                SecretOutcome::Unparseable(why) => {
                    return (CertState::Broken, format!("not a certificate: {why}"));
                }
                SecretOutcome::Refused(failure) => {
                    return (
                        CertState::Unknown,
                        format!(
                            "the Secret could not be read ({}: {})",
                            failure.kind.label(),
                            failure.message
                        ),
                    );
                }
                SecretOutcome::Read(_) => {}
            }
        }
        if let Some(end) = self.not_after
            && end <= now
        {
            return (CertState::Broken, "the certificate has expired".into());
        }
        if !self.wrong.is_empty() {
            return (
                CertState::WrongName,
                format!("does not name {}", self.wrong.join(", ")),
            );
        }
        if let Some(reason) = failing {
            return (CertState::Failing, reason);
        }
        if let Some(chain) = &self.chain
            && !chain.is_finished()
        {
            return (CertState::Issuing, chain.line());
        }
        if let (Some(days), Some(_)) = (self.days_left, self.not_after)
            && days < EXPIRING_DAYS
        {
            return (
                CertState::Expiring,
                format!(
                    "expires {}",
                    expiry_words(self.left_secs.unwrap_or_default())
                ),
            );
        }
        if self.not_after.is_some() && self.names.is_some() {
            return (CertState::Healthy, "valid".into());
        }
        if self.managed.is_some() {
            return (
                CertState::Unknown,
                "cert-manager has not issued it yet, so it has no expiry".into(),
            );
        }
        match &self.withheld {
            Some(why) => (CertState::Unknown, why.clone()),
            None => (
                CertState::Unknown,
                "Secret not read; Read Secret shows its names and expiry".into(),
            ),
        }
    }
}

/// What the page draws, from one read.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub struct CertsView {
    pub manager: Manager,
    pub entries: Vec<CertEntry>,
    /// Reads that were refused or cut, in words: what the rows may lack.
    pub notes: Vec<String>,
    pub certificates_cut: bool,
}

type SecretKey = (String, String);

impl CertsView {
    pub fn new(read: &CertsRead, now: i64) -> Self {
        let mut serving: BTreeMap<SecretKey, Vec<Serving>> = BTreeMap::new();
        let mut notes = read.notes.clone();
        match &read.gateway {
            Ok(gateway) => {
                match &gateway.gateways {
                    Ok(gateways) => {
                        listener_servings(&gateways.items, gateway.routes.iter(), &mut serving);
                        if gateways.cut {
                            notes.push(
                                "Gateways were cut at their limit; more listener certificates may exist"
                                    .into(),
                            );
                        }
                    }
                    Err(failure) => notes.push(format!(
                        "Gateways could not be read ({}: {}); listener certificates are not shown",
                        failure.kind.label(),
                        failure.message
                    )),
                }
                if gateway.routes_cut {
                    notes.push(
                        "Routes were cut at their limit; more hosts may be served under a listener's certificate than were judged"
                            .into(),
                    );
                }
            }
            Err(failure) => notes.push(format!(
                "Gateways could not be read ({}: {}); listener certificates are not shown",
                failure.kind.label(),
                failure.message
            )),
        }
        match &read.ingresses {
            Ok(list) => ingress_servings(&list.items, &mut serving),
            Err(failure) => notes.push(format!(
                "Ingresses could not be read ({}: {}); their certificates are not shown",
                failure.kind.label(),
                failure.message
            )),
        }
        let managed: BTreeMap<SecretKey, &CertificateFacts> = match &read.certificates {
            Ok(list) => list
                .items
                .iter()
                .filter(|c| !c.secret_name.is_empty())
                .map(|c| ((c.namespace.clone(), c.secret_name.clone()), c))
                .collect(),
            Err(_) => BTreeMap::new(),
        };
        let mut keys: Vec<SecretKey> = serving.keys().cloned().collect();
        for key in managed.keys() {
            if !serving.contains_key(key) {
                keys.push(key.clone());
            }
        }
        keys.sort();
        let issuance = &read.issuance;
        // Without every Certificate, a Secret that has none here may still be
        // one cert-manager keeps: it is not offered for reading.
        let withheld: Option<String> = match (&read.manager, &read.certificates) {
            (Manager::Unknown(_) | Manager::Incomplete { .. }, _) => Some(
                "Whether cert-manager keeps this Secret is not known, so it is not offered for reading"
                    .into(),
            ),
            (_, Err(_)) => Some(
                "The Certificates could not be read, so whether cert-manager keeps this Secret is not known and it is not offered for reading"
                    .into(),
            ),
            (_, Ok(list)) if list.cut => Some(
                "The Certificates were cut at their limit, so whether cert-manager keeps this Secret is not known and it is not offered for reading"
                    .into(),
            ),
            _ => None,
        };
        let entries = keys
            .into_iter()
            .map(|key| {
                let cert = managed.get(&key).copied();
                let mut entry = CertEntry {
                    namespace: key.0.clone(),
                    secret: key.1.clone(),
                    servings: serving.remove(&key).unwrap_or_default(),
                    managed: cert.cloned(),
                    chain: cert.map(|c| issuance_chain(c, issuance)),
                    secret_state: SecretState::NotRead,
                    names: None,
                    subject: None,
                    issuer: None,
                    not_before: None,
                    not_after: None,
                    wrong: Vec::new(),
                    state: CertState::Unknown,
                    reason: String::new(),
                    days_left: None,
                    left_secs: None,
                    withheld: withheld.clone().filter(|_| cert.is_none()),
                };
                entry.assess(now);
                entry
            })
            .collect();
        Self {
            manager: read.manager.clone(),
            entries,
            notes,
            certificates_cut: read.certificates.as_ref().is_ok_and(|l| l.cut),
        }
    }

    /// Records the outcome of a Secret the person asked to read, and works
    /// the entry out again. A Secret a Certificate answers for is ignored.
    pub fn apply_secret(&mut self, key: &str, outcome: SecretOutcome, now: i64) {
        if let Some(entry) = self
            .entries
            .iter_mut()
            .find(|e| e.key() == key && e.managed.is_none())
        {
            entry.secret_state = SecretState::Read(outcome);
            entry.assess(now);
        }
    }

    /// Forgets every Secret read, so each entry reads as not yet read.
    pub fn forget_secrets(&mut self, now: i64) {
        for entry in &mut self.entries {
            if entry.secret_state != SecretState::NotRead {
                entry.secret_state = SecretState::NotRead;
                entry.assess(now);
            }
        }
    }

    pub fn entry(&self, key: &str) -> Option<&CertEntry> {
        self.entries.iter().find(|e| e.key() == key)
    }

    /// The entries serving a host, for a route's trace to link to.
    pub fn serving_host(&self, host: &str) -> Vec<&CertEntry> {
        self.entries
            .iter()
            .filter(|e| e.hosts().iter().any(|h| h == host))
            .collect()
    }

    /// The entries a Gateway's listener serves (`gateway` is
    /// `namespace/name`), which is what a route's trace reaches.
    pub fn for_listener(&self, gateway: &str, listener: &str) -> Vec<&CertEntry> {
        self.entries
            .iter()
            .filter(|e| {
                e.servings.iter().any(|s| {
                    matches!(&s.via, Via::Listener { gateway: g, listener: l, .. }
                        if g == gateway && l == listener)
                })
            })
            .collect()
    }
}

fn listener_servings<'a>(
    gateways: &[GatewayFacts],
    routes: impl Iterator<Item = &'a RouteEntry> + Clone,
    out: &mut BTreeMap<SecretKey, Vec<Serving>>,
) {
    for gateway in gateways {
        for listener in &gateway.listeners {
            if listener.certificate_refs.is_empty() {
                continue;
            }
            // The hosts the listener serves: of each attached route, the
            // hostnames that overlap the listener's own, narrowed to the
            // listener's where the route's is wider; a route naming no host
            // serves the listener's.
            let mut hosts: Vec<String> = Vec::new();
            let mut add = |host: &str| {
                if !hosts.iter().any(|h| h == host) {
                    hosts.push(host.to_owned());
                }
            };
            let mut attached = false;
            for entry in routes.clone() {
                let route = &entry.facts;
                for parent in route.parents.iter().filter(|p| p.is_gateway()) {
                    if parent.namespace_or(&route.namespace) != gateway.namespace
                        || parent.name != gateway.name
                    {
                        continue;
                    }
                    // The listeners the trace found the route may attach to:
                    // those that fit it and do not refuse its namespace.
                    let admitted = entry
                        .trace
                        .parents
                        .iter()
                        .any(|t| &t.parent == parent && t.admitted.contains(&listener.name));
                    if !admitted {
                        continue;
                    }
                    attached = true;
                    match (&listener.hostname, route.hostnames.is_empty()) {
                        (Some(own), true) => add(own),
                        (None, true) => {}
                        (own, false) => {
                            for host in &route.hostnames {
                                match own {
                                    None => add(host),
                                    // The Gateway API reads a wildcard as a suffix of
                                    // any depth; serve whichever host is narrower.
                                    Some(own) if hosts_overlap(own, host) => {
                                        let route_wider = host.starts_with('*')
                                            && (!own.starts_with('*') || host.len() <= own.len());
                                        add(if route_wider { own } else { host })
                                    }
                                    Some(_) => {}
                                }
                            }
                        }
                    }
                }
            }
            if !attached && let Some(host) = &listener.hostname {
                add(host);
            }
            for (namespace, name) in &listener.certificate_refs {
                let key = (
                    namespace
                        .clone()
                        .unwrap_or_else(|| gateway.namespace.clone()),
                    name.clone(),
                );
                out.entry(key).or_default().push(Serving {
                    via: Via::Listener {
                        gateway: format!("{}/{}", gateway.namespace, gateway.name),
                        listener: listener.name.clone(),
                        port: listener.port,
                    },
                    hosts: hosts.clone(),
                });
            }
        }
    }
}

fn ingress_servings(ingresses: &[IngressFacts], out: &mut BTreeMap<SecretKey, Vec<Serving>>) {
    for ingress in ingresses {
        for tls in &ingress.tls {
            let Some(secret) = &tls.secret else {
                continue;
            };
            let hosts = if tls.hosts.is_empty() {
                ingress.hosts.clone()
            } else {
                tls.hosts.clone()
            };
            out.entry((ingress.namespace.clone(), secret.clone()))
                .or_default()
                .push(Serving {
                    via: Via::Ingress {
                        ingress: ingress.key(),
                    },
                    hosts,
                });
        }
    }
}
