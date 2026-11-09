//! The answers that decide what is offered (#854, slice 2): before an action
//! is shown as available, one `SelfSubjectAccessReview` of what it declares
//! it needs ([`AccessQuestion`]), cached here.
//!
//! This only guides. The gate stays core's `write` preflight, which asks
//! afresh on the exact object: a yes here never skips it, and a no here is a
//! reason shown on a disabled control, not what stops a write. An answer that
//! never came, or failed, leaves the action on offer ("Couldn't check your
//! access"); it never blocks by itself.
//!
//! The cache is keyed by the access identity (`ClusterSource.id`), by who the
//! cluster says you are, and by the question (verb, resource with group and
//! subresource, namespace, name). A new identity or another person drops
//! every answer at once, and a [`OfferTicket`] carries the generation it was
//! issued in, so an answer read for a previous identity is ignored. Answers
//! expire after [`TTL`], and the cache holds at most [`CAP`] of them.

use std::{
    collections::{HashMap, HashSet},
    time::{Duration, Instant},
};

use super::PARALLEL;
use crate::write::AccessQuestion;

/// How long an answer is believed. After it the action reads as unasked
/// (on offer), and the next time the selection shows it is asked again.
pub const TTL: Duration = Duration::from_secs(120);

/// The most answers kept. A person reaches this only by selecting a great
/// many different objects; the oldest go first.
pub const CAP: usize = 256;

/// What a disabled-looking-but-open action says when its review didn't
/// answer.
pub const COULDNT_CHECK: &str = "Couldn't check your access";

/// What the cluster answered to one question.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Verdict {
    Allowed,
    Refused,
    /// The review failed; nothing is known.
    Failed(String),
}

/// What a control reads of the questions an action declares.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Offer {
    /// Not asked yet, still being asked, or the answer expired: on offer,
    /// with nothing said.
    Pending,
    Allowed,
    /// The cluster said no: disabled, with this ("Needs patch on deployments
    /// in shop").
    Refused(String),
    /// A review didn't answer: on offer, saying [`COULDNT_CHECK`].
    Unchecked,
}

impl Offer {
    /// The reason to show disabled, if any.
    pub fn refusal(&self) -> Option<&str> {
        match self {
            Offer::Refused(reason) => Some(reason),
            _ => None,
        }
    }

    /// What an enabled control still says.
    pub fn note(&self) -> Option<&'static str> {
        matches!(self, Offer::Unchecked).then_some(COULDNT_CHECK)
    }
}

/// Permission to ask one question, valid for the generation it was issued in.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OfferTicket {
    pub question: AccessQuestion,
    generation: u64,
}

#[derive(Clone, Debug)]
struct Entry {
    verdict: Verdict,
    at: Instant,
}

/// Who the cluster says you are, as far as it has been asked.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
enum Person {
    #[default]
    Unread,
    /// Asked: the user name, or none when the cluster can't say (a server
    /// older than 1.28 doesn't serve the review).
    Read(Option<String>),
}

#[derive(Debug, Default)]
pub struct Offers {
    identity: Option<String>,
    person: Person,
    generation: u64,
    answers: HashMap<AccessQuestion, Entry>,
    inflight: HashSet<AccessQuestion>,
}

impl Offers {
    fn drop_answers(&mut self) {
        self.answers.clear();
        self.inflight.clear();
        self.generation += 1;
    }

    /// Binds the cache to an access identity. Another one drops every answer
    /// and who you are at once; the same one changes nothing. Returns
    /// whether anything changed.
    pub fn reset(&mut self, identity: &str) -> bool {
        if self.identity.as_deref() == Some(identity) {
            return false;
        }
        self.drop_answers();
        self.identity = Some(identity.to_owned());
        self.person = Person::Unread;
        true
    }

    /// Whether who you are has been asked and answered (or failed). Nothing
    /// is asked before: an answer is kept for a person.
    pub fn is_ready(&self) -> bool {
        self.person != Person::Unread
    }

    /// Takes who the cluster says you are (`None`: it could not say). Another
    /// person than the answers were read as drops them. Returns whether it
    /// did.
    pub fn set_person(&mut self, person: Option<String>) -> bool {
        let new = Person::Read(person);
        let dropped = matches!(&self.person, Person::Read(_) if self.person != new);
        if dropped {
            self.drop_answers();
        }
        self.person = new;
        dropped
    }

    pub fn in_flight(&self) -> usize {
        self.inflight.len()
    }

    pub fn answered(&self) -> usize {
        self.answers.len()
    }

    fn fresh(&self, question: &AccessQuestion, now: Instant) -> Option<&Verdict> {
        self.answers
            .get(question)
            .filter(|entry| now.saturating_duration_since(entry.at) < TTL)
            .map(|entry| &entry.verdict)
    }

    /// Tickets for the `questions` that have no fresh answer and are not
    /// being asked, while fewer than [`PARALLEL`] are in flight. Nothing
    /// until who you are is known.
    pub fn wants(&mut self, questions: &[AccessQuestion], now: Instant) -> Vec<OfferTicket> {
        let mut tickets = Vec::new();
        if !self.is_ready() {
            return tickets;
        }
        for question in questions {
            if self.inflight.len() >= PARALLEL {
                break;
            }
            if self.inflight.contains(question) || self.fresh(question, now).is_some() {
                continue;
            }
            self.inflight.insert(question.clone());
            tickets.push(OfferTicket {
                question: question.clone(),
                generation: self.generation,
            });
        }
        tickets
    }

    /// Takes an answer. One from an older generation is ignored; returns
    /// whether it was kept.
    pub fn land(&mut self, ticket: &OfferTicket, verdict: Verdict, now: Instant) -> bool {
        if ticket.generation != self.generation {
            return false;
        }
        self.inflight.remove(&ticket.question);
        if self.answers.len() >= CAP && !self.answers.contains_key(&ticket.question) {
            // Expired first, else the oldest.
            let gone = self
                .answers
                .iter()
                .min_by_key(|(_, entry)| entry.at)
                .map(|(question, _)| question.clone());
            if let Some(gone) = gone {
                self.answers.remove(&gone);
            }
        }
        self.answers.insert(
            ticket.question.clone(),
            Entry {
                verdict,
                at: now,
            },
        );
        true
    }

    /// Gives up a ticket whose job ended without an answer, so the question
    /// can be asked again.
    pub fn abandon(&mut self, ticket: &OfferTicket) {
        if ticket.generation == self.generation {
            self.inflight.remove(&ticket.question);
        }
    }

    /// Drops what is in flight, as a hidden page does. Answers stay.
    pub fn stop(&mut self) {
        self.inflight.clear();
        self.generation += 1;
    }

    /// Marks every answer to be asked again, at the next `wants`.
    pub fn forget(&mut self) {
        self.answers.clear();
    }

    /// What an action that needs all of `questions` reads: the first no wins
    /// (a refused action is shown disabled with its reason), then a review
    /// that failed, then one not answered yet; only all yes is allowed.
    pub fn offer(&self, questions: &[AccessQuestion], now: Instant) -> Offer {
        let mut pending = false;
        let mut failed = false;
        for question in questions {
            match self.fresh(question, now) {
                Some(Verdict::Refused) => return Offer::Refused(question.reason()),
                Some(Verdict::Failed(_)) => failed = true,
                Some(Verdict::Allowed) => {}
                None => pending = true,
            }
        }
        if failed {
            Offer::Unchecked
        } else if pending || questions.is_empty() {
            Offer::Pending
        } else {
            Offer::Allowed
        }
    }
}

impl From<crate::write::Access> for Verdict {
    fn from(access: crate::write::Access) -> Self {
        match access {
            crate::write::Access::Allowed => Verdict::Allowed,
            crate::write::Access::Denied { .. } => Verdict::Refused,
            crate::write::Access::Unknown { message } => Verdict::Failed(message),
        }
    }
}

impl Offers {
    /// Whether `question` is being asked now.
    pub fn is_in_flight(&self, question: &AccessQuestion) -> bool {
        self.inflight.contains(question)
    }
}
