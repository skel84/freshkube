//! The cache of answers, per access identity.
//!
//! The page owns the jobs; the board owns what is known and what may land.
//! A [`Ticket`] carries the generation it was issued in, and an answer from
//! an older generation is ignored, so nothing read for a previous identity
//! (another access, or the same access as another person) is ever shown.

use std::collections::{BTreeMap, BTreeSet};

use super::{Cells, Who};

/// Reviews in flight at once. A frame shows a screenful of rows; this keeps
/// the burst a polite size.
pub const PARALLEL: usize = 4;

/// A row of the table.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum RowKey {
    /// The cluster-wide row, answered by exact reviews.
    Nodes,
    Namespace(String),
}

/// What one review answered.
pub type Answer = Result<Cells, String>;

/// A permission to ask one row, valid for the generation it was issued in.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Ticket {
    pub key: RowKey,
    generation: u64,
}

/// Why the earlier answers went.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Changed {
    /// The earlier user, when it was one the board knew.
    pub from: Option<String>,
    pub to: Option<String>,
}

/// What the table draws for a row.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RowView<'a> {
    /// Not asked yet, or being asked again; `earlier` is what was known.
    Checking {
        earlier: Option<&'a Cells>,
    },
    Known(&'a Cells),
    Failed(&'a str),
}

#[derive(Debug, Default)]
pub struct Board {
    identity: Option<String>,
    generation: u64,
    who: Option<Who>,
    answers: BTreeMap<RowKey, Answer>,
    stale: BTreeSet<RowKey>,
    inflight: BTreeSet<RowKey>,
    changed: Option<Changed>,
}

impl Board {
    pub fn who(&self) -> Option<&Who> {
        self.who.as_ref()
    }

    pub fn changed(&self) -> Option<&Changed> {
        self.changed.as_ref()
    }

    pub fn dismiss_changed(&mut self) {
        self.changed = None;
    }

    pub fn is_in_flight(&self, key: &RowKey) -> bool {
        self.inflight.contains(key)
    }

    pub fn in_flight(&self) -> usize {
        self.inflight.len()
    }

    fn drop_answers(&mut self) {
        self.answers.clear();
        self.stale.clear();
        self.inflight.clear();
        self.generation += 1;
    }

    /// Binds the board to an access identity. A different one drops every
    /// answer at once and the table says the access changed; the same one
    /// changes nothing. Returns whether anything changed.
    pub fn reset(&mut self, identity: &str) -> bool {
        if self.identity.as_deref() == Some(identity) {
            return false;
        }
        let had = self.identity.is_some() && (!self.answers.is_empty() || self.who.is_some());
        let from = self.who.as_ref().map(|who| who.username.clone());
        self.drop_answers();
        self.identity = Some(identity.to_owned());
        self.who = None;
        self.changed = had.then_some(Changed { from, to: None });
        true
    }

    /// Takes who the cluster says you are. A different person (or groups)
    /// than the answers were read as drops them at once. Returns whether it
    /// did.
    pub fn set_who(&mut self, who: Who) -> bool {
        let dropped = match &self.who {
            Some(old) if *old != who => {
                self.changed = Some(Changed {
                    from: Some(old.username.clone()),
                    to: Some(who.username.clone()),
                });
                self.drop_answers();
                true
            }
            _ => false,
        };
        self.who = Some(who);
        dropped
    }

    /// Marks every answer to be asked again; the old ones stay on show until
    /// the new ones land.
    pub fn recheck_all(&mut self) {
        self.stale.extend(self.answers.keys().cloned());
    }

    /// Marks one row to be asked again.
    pub fn recheck(&mut self, key: &RowKey) {
        if self.answers.contains_key(key) {
            self.stale.insert(key.clone());
        }
    }

    /// Tickets for those of `shown` that have no answer, or an answer to be
    /// asked again, and are not being asked, while fewer than [`PARALLEL`]
    /// are in flight. A row that failed is asked again only after a
    /// [`recheck`](Self::recheck).
    pub fn wants(&mut self, shown: &[RowKey]) -> Vec<Ticket> {
        let mut tickets = Vec::new();
        for key in shown {
            if self.inflight.len() >= PARALLEL {
                break;
            }
            if self.inflight.contains(key) {
                continue;
            }
            if self.answers.contains_key(key) && !self.stale.contains(key) {
                continue;
            }
            self.stale.remove(key);
            self.inflight.insert(key.clone());
            tickets.push(Ticket {
                key: key.clone(),
                generation: self.generation,
            });
        }
        tickets
    }

    /// Takes an answer. One from an older generation is ignored; returns
    /// whether it was kept.
    pub fn land(&mut self, ticket: &Ticket, answer: Answer) -> bool {
        if ticket.generation != self.generation {
            return false;
        }
        self.inflight.remove(&ticket.key);
        self.answers.insert(ticket.key.clone(), answer);
        true
    }

    /// Gives up a ticket whose job ended without an answer, so the row can
    /// be asked again.
    pub fn abandon(&mut self, ticket: &Ticket) {
        if ticket.generation == self.generation {
            self.inflight.remove(&ticket.key);
        }
    }

    /// Drops what is in flight, as a hidden page does. Answers stay.
    pub fn stop(&mut self) {
        self.inflight.clear();
        self.generation += 1;
    }

    pub fn view(&self, key: &RowKey) -> RowView<'_> {
        match self.answers.get(key) {
            None => RowView::Checking { earlier: None },
            Some(_) if self.inflight.contains(key) => RowView::Checking {
                earlier: self.answers.get(key).and_then(|a| a.as_ref().ok()),
            },
            Some(Ok(cells)) => RowView::Known(cells),
            Some(Err(message)) => RowView::Failed(message),
        }
    }

    pub fn answered(&self) -> usize {
        self.answers.len()
    }
}
