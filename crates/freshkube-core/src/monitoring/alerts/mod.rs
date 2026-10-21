//! Alerts, rules and silences, read-only: Alertmanager's API v2, vmalert's
//! rules and alerts, and PrometheusRule objects. Everything goes through the
//! cluster's `services/proxy` with GET, like Monitoring's Prometheus, so it
//! needs the same access and changes nothing.
//!
//! - [`model`]: the shapes, whichever server they came from.
//! - [`parse`]: pure parsers for each server's JSON.
//! - [`discover`]: which Services are the Alertmanager and the vmalert.
//! - [`read`]: the clients, the bounded reads and the central label's scope.
//! - [`rules`]: PrometheusRule objects and the Rules page's read.
//! - [`state`]: what the session alert read knows, and what it may say of
//!   health: unread, stale or refused is Unknown, never "no alerts".
//! - [`drive`]: the session's loop, every 60 s once a source was found.
//! - [`example`]: made-up data and its shapes, for `--fixture`.
mod discover;
mod drive;
pub mod example;
mod model;
mod parse;
mod read;
mod rules;
mod state;

#[cfg(test)]
mod tests;

pub use discover::{Candidate, Found, LOOKED_FOR, Rank, Remembered, find, rank, rank_all};
pub use drive::{connect, drive};
pub use model::{
    Alert, AlertState, Kind, Rule, RuleGroup, RuleHealth, RuleKind, Severity, Silence,
    SilenceMatcher, SilenceState,
};
pub use parse::dedupe;
pub use read::{
    Active, AlertsData, Client, MAX_ACTIVE, MAX_ANSWER, MAX_PAGE_ALERTS, ReadFrom, RulesData,
    Scope, SilencesData, Sources, active, alerts_page, scoped, silences_page,
};
pub use rules::{any_objects, objects, page as rules_page};
pub use state::{Counts, Health, POLL, State, Unknown};
