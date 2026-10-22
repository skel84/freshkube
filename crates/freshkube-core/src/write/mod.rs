//! How the app decides what a person may change in a cluster, and the one
//! path every change to a Kubernetes object goes through
//! (`docs/WRITE_MODEL.md`, #852 and #853): the protection level a cluster
//! entry sets, the tiers, preflight, the send, accepted versus observed, Undo
//! and the audit log. No UI types: a page describes an action and draws what
//! this answers.

mod access;
mod action;
mod audit;
mod diff;
mod outcome;
mod preflight;
mod protection;
mod requirement;
mod run;
mod send;
mod undo;

#[cfg(any(test, feature = "testing"))]
pub mod testing;

pub use access::{Access, review as review_access};
pub use action::{ActionDescription, Change, Inverse, ObjectChange, Observation, Target};
pub use audit::{
    AuditObject, AuditRequest, AuditStore, KEEP as AUDIT_KEEP, Phase, RESULT_LIMIT, ROTATE_AT,
    Read as AuditRead, Record as AuditRecord, RunId, SCHEMA as AUDIT_SCHEMA, scrub,
};
pub use diff::{ChangeKind, FieldChange};
pub use outcome::{Accepted, DEFAULT_WAIT, MAX_WAIT, Observed, Watching};
pub use preflight::{Block, ObjectDiff, Prepared, Verdict, Who};
pub use protection::{Protection, READ_ONLY_REASON};
pub use requirement::{
    CANCEL_WINDOW, Confirmation, Requirement, Tier, UNDO_WINDOW, expected_name, requirement,
};
pub use run::{Event, Finished, Headline, NotSent, ObjectResult, Runner, Status, Step};
pub use send::{ChangedWhat, Conflict, FIELD_MANAGER, Refusal};
pub use undo::{UndoOffer, UndoRefused};

#[cfg(test)]
mod tests;
