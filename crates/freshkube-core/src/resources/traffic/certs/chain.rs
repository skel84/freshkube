//! The issuance chain Certificate → CertificateRequest → Order → Challenge,
//! read the way a route's trace reads: it stops at the first break and every
//! later step reads "not reached". The objects are chained through their
//! `ownerReferences` (a request is owned by its Certificate, an Order by the
//! request, a Challenge by the Order), never by guessing from names. What
//! cert-manager wrote is quoted apart from the reading of it. A list that was
//! refused, cut or not served is Unknown with its reason, never "no Order".
use super::facts::{CertificateFacts, ChallengeFacts, OrderFacts, RequestFacts};
use crate::failure::Failure;
use crate::resources::traffic::routes::{Cond, Listing, Tri};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StepKind {
    Certificate,
    Request,
    Order,
    Challenge,
}

impl StepKind {
    pub const ALL: [Self; 4] = [
        Self::Certificate,
        Self::Request,
        Self::Order,
        Self::Challenge,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::Certificate => "Certificate",
            Self::Request => "CertificateRequest",
            Self::Order => "Order",
            Self::Challenge => "Challenge",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Verdict {
    /// The object is there and nothing it says is wrong.
    Done,
    /// In progress, and cert-manager says nothing is wrong yet.
    Waiting,
    /// Working, with something to know: a renewal's last attempt failed.
    Warn,
    /// Failed, or stuck with a reason cert-manager gave.
    Broken,
    /// Not readable: refused, cut, not served.
    Unknown,
    /// An earlier step broke, so this one was not looked at.
    NotReached,
    /// Nothing to do here: the issuance is finished, or this issuer makes no
    /// such object.
    NotUsed,
}

impl Verdict {
    pub fn label(self) -> &'static str {
        match self {
            Self::Done => "Done",
            Self::Waiting => "In progress",
            Self::Warn => "Last attempt failed",
            Self::Broken => "Stopped",
            Self::Unknown => "Can't tell",
            Self::NotReached => "Not reached",
            Self::NotUsed => "Not used",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Step {
    pub kind: StepKind,
    pub verdict: Verdict,
    /// The object the step looked at, when there is one.
    pub object: Option<String>,
    /// What cert-manager wrote, or why the step reads as it does.
    pub says: String,
}

impl Step {
    fn new(kind: StepKind, verdict: Verdict, says: impl Into<String>) -> Self {
        Self {
            kind,
            verdict,
            object: None,
            says: says.into(),
        }
    }

    fn of(mut self, object: &str) -> Self {
        self.object = Some(object.to_owned());
        self
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Chain {
    pub steps: Vec<Step>,
}

impl Chain {
    /// Where it stopped: the first step that broke or can't be read, else
    /// the deepest step still in progress; none when it finished.
    pub fn stopped_at(&self) -> Option<&Step> {
        self.steps
            .iter()
            .find(|s| matches!(s.verdict, Verdict::Broken | Verdict::Unknown))
            .or_else(|| {
                self.steps
                    .iter()
                    .rev()
                    .find(|s| s.verdict == Verdict::Waiting)
            })
    }

    /// A step says something is wrong while the certificate still serves: a
    /// renewal whose last attempt failed.
    pub fn has_warning(&self) -> bool {
        self.steps.iter().any(|s| s.verdict == Verdict::Warn)
    }

    pub fn is_broken(&self) -> bool {
        self.steps.iter().any(|s| s.verdict == Verdict::Broken)
    }

    /// Nothing is in flight: no step is waiting, broken or unreadable.
    pub fn is_finished(&self) -> bool {
        self.stopped_at().is_none()
    }

    /// One line for a row: `stopped at Order — …`, or `in progress at …`.
    pub fn line(&self) -> String {
        match self.stopped_at() {
            Some(step) => {
                let at = match step.verdict {
                    Verdict::Broken => "stopped at",
                    Verdict::Unknown => "can't tell at",
                    _ => "in progress at",
                };
                format!("{at} {} — {}", step.kind.label(), step.says)
            }
            None => "issued".into(),
        }
    }
}

/// A list the chain reads, which may not have been needed or served.
#[derive(Clone, Debug)]
pub enum Lookup<T> {
    /// The kind is not served by this cluster.
    NotServed,
    /// Not read because every Certificate was issued and none is renewing.
    Skipped,
    Read(Result<Listing<T>, Failure>),
}

/// The lists the chain reads besides the Certificates.
#[derive(Clone, Debug)]
pub struct Issuance {
    pub requests: Lookup<RequestFacts>,
    pub orders: Lookup<OrderFacts>,
    pub challenges: Lookup<ChallengeFacts>,
}

impl Issuance {
    pub fn skipped() -> Self {
        Self {
            requests: Lookup::Skipped,
            orders: Lookup::Skipped,
            challenges: Lookup::Skipped,
        }
    }
}

/// A Certificate that is Ready, not being renewed and with no failed attempt
/// since has nothing in flight. cert-manager keeps `Ready=True` through a
/// failing renewal and writes `lastFailureTime`, so that is not issued.
pub fn is_issued(cert: &CertificateFacts) -> bool {
    cert.ready.as_ref().is_some_and(|c| c.status == Tri::True)
        && !cert.issuing.as_ref().is_some_and(|c| c.status == Tri::True)
        && cert.last_failure.is_none()
}

pub fn issuance_chain(cert: &CertificateFacts, reads: &Issuance) -> Chain {
    let mut steps = Vec::new();
    let first = certificate_step(cert);
    let go = matches!(first.verdict, Verdict::Done | Verdict::Warn) && !is_issued(cert);
    let issued = is_issued(cert);
    steps.push(first);
    if !go {
        let (verdict, says) = if issued {
            (Verdict::NotUsed, "issued; nothing in progress")
        } else {
            (Verdict::NotReached, "")
        };
        for kind in &StepKind::ALL[1..] {
            steps.push(Step::new(*kind, verdict, says));
        }
        return Chain { steps };
    }
    // The request.
    let (request, uid) = request_step(cert, &reads.requests);
    let proceed = matches!(request.verdict, Verdict::Waiting | Verdict::Done);
    let signed = request.verdict == Verdict::Done;
    steps.push(request);
    if !proceed {
        steps.push(Step::new(StepKind::Order, Verdict::NotReached, ""));
        steps.push(Step::new(StepKind::Challenge, Verdict::NotReached, ""));
        return Chain { steps };
    }
    if signed {
        let says = "the request is signed; nothing more to do here";
        steps.push(Step::new(StepKind::Order, Verdict::NotUsed, says));
        steps.push(Step::new(StepKind::Challenge, Verdict::NotUsed, says));
        return Chain { steps };
    }
    // The Order, owned by the request.
    let (order, order_uid) = order_step(&uid.unwrap_or_default(), &reads.orders);
    let proceed = matches!(order.verdict, Verdict::Waiting | Verdict::Done);
    let ordered = order.verdict == Verdict::Done;
    steps.push(order);
    if !proceed {
        let not_used = steps[2].verdict == Verdict::NotUsed;
        steps.push(Step::new(
            StepKind::Challenge,
            if not_used {
                Verdict::NotUsed
            } else {
                Verdict::NotReached
            },
            "",
        ));
        return Chain { steps };
    }
    steps.push(challenge_step(
        &order_uid.unwrap_or_default(),
        ordered,
        &reads.challenges,
    ));
    Chain { steps }
}

fn says(cond: &Cond) -> String {
    cond.says()
}

fn time(at: i64) -> String {
    chrono::DateTime::from_timestamp(at, 0).map_or_else(
        || "an unknown time".into(),
        |t| t.format("%Y-%m-%d %H:%M UTC").to_string(),
    )
}

fn certificate_step(cert: &CertificateFacts) -> Step {
    let step =
        |verdict, says: String| Step::new(StepKind::Certificate, verdict, says).of(&cert.name);
    let ready = cert.ready.as_ref();
    let issuing = cert.issuing.as_ref().filter(|c| c.status == Tri::True);
    if let Some(issuing) = issuing {
        let mut words = says(issuing);
        if cert.last_failure.is_some() {
            words.push_str(" (an earlier attempt failed and cert-manager is retrying)");
        }
        return step(Verdict::Done, words);
    }
    match ready {
        Some(ready) if ready.status == Tri::True && cert.last_failure.is_some() => step(
            Verdict::Warn,
            format!(
                "{}; the last attempt to renew it failed at {}",
                says(ready),
                time(cert.last_failure.unwrap_or_default())
            ),
        ),
        Some(ready) if ready.status == Tri::True => step(Verdict::Done, says(ready)),
        Some(ready) if ready.status == Tri::False => step(Verdict::Broken, says(ready)),
        Some(ready) => step(Verdict::Unknown, says(ready)),
        None => step(
            Verdict::Done,
            "cert-manager has not reported on it yet".into(),
        ),
    }
}

/// The request of the Certificate's current revision, and its UID.
fn request_step(cert: &CertificateFacts, lookup: &Lookup<RequestFacts>) -> (Step, Option<String>) {
    let kind = StepKind::Request;
    let unknown = |says: String| (Step::new(kind, Verdict::Unknown, says), None);
    let list = match lookup {
        Lookup::NotServed => return unknown("CertificateRequest is not served".into()),
        Lookup::Skipped => return unknown("CertificateRequests were not read".into()),
        Lookup::Read(Err(failure)) => {
            return unknown(format!(
                "CertificateRequests could not be read ({}: {})",
                failure.kind.label(),
                failure.message
            ));
        }
        Lookup::Read(Ok(list)) => list,
    };
    // The request of the revision being issued: the one after the last
    // issued. An older request, signed or not, is not this issuance's.
    let wanted = cert.revision.unwrap_or(0) + 1;
    let owned = list
        .items
        .iter()
        .filter(|r| r.owners.contains(&cert.uid) && r.revision == Some(wanted))
        .max_by(|a, b| a.name.cmp(&b.name));
    let Some(request) = owned else {
        return if list.cut {
            unknown("the CertificateRequest list was cut, so the request may be past it".into())
        } else {
            (
                Step::new(
                    kind,
                    Verdict::Waiting,
                    format!(
                        "cert-manager has not made the CertificateRequest for revision {wanted} yet"
                    ),
                ),
                None,
            )
        };
    };
    let cond = |name: &str| request.conditions.iter().find(|c| c.kind == name);
    let yes = |name: &str| cond(name).filter(|c| c.status == Tri::True);
    let (verdict, words) = if let Some(denied) = yes("Denied") {
        (Verdict::Broken, says(denied))
    } else if let Some(invalid) = yes("InvalidRequest") {
        (Verdict::Broken, says(invalid))
    } else if let Some(ready) = cond("Ready") {
        match (ready.status, ready.reason.as_str()) {
            (Tri::True, _) => (Verdict::Done, says(ready)),
            (Tri::False, "Failed") => (Verdict::Broken, says(ready)),
            (Tri::False, _) => (Verdict::Waiting, says(ready)),
            _ => (Verdict::Unknown, says(ready)),
        }
    } else if let Some(approved) = yes("Approved") {
        (
            Verdict::Waiting,
            format!("{}; waiting for the issuer to sign", says(approved)),
        )
    } else {
        (Verdict::Waiting, "waiting for approval".into())
    };
    (
        Step::new(kind, verdict, words).of(&request.name),
        Some(request.uid.clone()),
    )
}

fn order_step(request_uid: &str, lookup: &Lookup<OrderFacts>) -> (Step, Option<String>) {
    let kind = StepKind::Order;
    let unknown = |says: String| (Step::new(kind, Verdict::Unknown, says), None);
    let list = match lookup {
        Lookup::NotServed => {
            return (
                Step::new(
                    kind,
                    Verdict::NotUsed,
                    "Orders are not served here, so the issuer is not ACME",
                ),
                None,
            );
        }
        Lookup::Skipped => return unknown("Orders were not read".into()),
        Lookup::Read(Err(failure)) => {
            return unknown(format!(
                "Orders could not be read ({}: {})",
                failure.kind.label(),
                failure.message
            ));
        }
        Lookup::Read(Ok(list)) => list,
    };
    // The worst of the request's Orders decides, as a retry makes a new one.
    let rank = |o: &&OrderFacts| match o.state.as_str() {
        "invalid" | "expired" | "errored" => 0,
        "valid" => 3,
        _ => 1,
    };
    let order = list
        .items
        .iter()
        .filter(|o| o.owners.iter().any(|u| u == request_uid))
        .min_by_key(|o| (rank(o), std::cmp::Reverse(o.name.clone())));
    let Some(order) = order else {
        return if list.cut {
            unknown("the Order list was cut, so the Order may be past it".into())
        } else {
            (
                Step::new(
                    kind,
                    Verdict::NotUsed,
                    "no Order for this request: an issuer that is not ACME makes none",
                ),
                None,
            )
        };
    };
    let state = if order.state.is_empty() {
        "new"
    } else {
        order.state.as_str()
    };
    let detail = if order.reason.is_empty() {
        String::new()
    } else {
        format!(": {}", order.reason)
    };
    let (verdict, words) = match order.state.as_str() {
        "valid" => (Verdict::Done, "Order is valid".to_owned()),
        "invalid" | "expired" | "errored" => (Verdict::Broken, format!("Order is {state}{detail}")),
        _ => (Verdict::Waiting, format!("Order is {state}{detail}")),
    };
    (
        Step::new(kind, verdict, words).of(&order.name),
        Some(order.uid.clone()),
    )
}

fn challenge_step(order_uid: &str, ordered: bool, lookup: &Lookup<ChallengeFacts>) -> Step {
    let kind = StepKind::Challenge;
    let list = match lookup {
        Lookup::NotServed => {
            return Step::new(kind, Verdict::NotUsed, "Challenges are not served here");
        }
        Lookup::Skipped => return Step::new(kind, Verdict::Unknown, "Challenges were not read"),
        Lookup::Read(Err(failure)) => {
            return Step::new(
                kind,
                Verdict::Unknown,
                format!(
                    "Challenges could not be read ({}: {})",
                    failure.kind.label(),
                    failure.message
                ),
            );
        }
        Lookup::Read(Ok(list)) => list,
    };
    let owned: Vec<&ChallengeFacts> = list
        .items
        .iter()
        .filter(|c| c.owners.iter().any(|u| u == order_uid))
        .collect();
    if owned.is_empty() {
        return if list.cut {
            Step::new(
                kind,
                Verdict::Unknown,
                "the Challenge list was cut, so a Challenge may be past it",
            )
        } else if ordered {
            Step::new(
                kind,
                Verdict::NotUsed,
                "the Order is valid; no Challenge is left",
            )
        } else {
            Step::new(kind, Verdict::Waiting, "no Challenge yet")
        };
    }
    let describe = |c: &ChallengeFacts| {
        let kind = if c.kind.is_empty() { "ACME" } else { &c.kind };
        let state = if c.state.is_empty() { "new" } else { &c.state };
        let reason = if c.reason.is_empty() {
            String::new()
        } else {
            format!(": {}", c.reason)
        };
        format!("{kind} challenge for {} is {state}{reason}", c.dns_name)
    };
    let broken = |c: &ChallengeFacts| {
        matches!(c.state.as_str(), "invalid" | "expired" | "errored")
            || (matches!(c.state.as_str(), "pending" | "processing" | "") && !c.reason.is_empty())
    };
    let bad: Vec<&ChallengeFacts> = owned.iter().copied().filter(|c| broken(c)).collect();
    if let Some(first) = bad.first() {
        let more = bad.len() - 1;
        let words = if more == 0 {
            describe(first)
        } else {
            format!("{}; and {more} more", describe(first))
        };
        return Step::new(kind, Verdict::Broken, words).of(&first.name);
    }
    if owned.iter().all(|c| c.state == "valid") {
        return Step::new(
            kind,
            Verdict::Done,
            format!("{} challenge(s) valid", owned.len()),
        );
    }
    let waiting = owned.iter().find(|c| c.state != "valid").unwrap();
    let presented = if waiting.presented {
        ""
    } else {
        " (not presented yet)"
    };
    Step::new(
        kind,
        Verdict::Waiting,
        format!("{}{presented}", describe(waiting)),
    )
    .of(&waiting.name)
}
