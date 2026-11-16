//! Silences and Expire on an Alertmanager (#884, slice 2, W1). A silence is
//! `POST api/v2/silences` through the Kubernetes API's service proxy, not a
//! Kubernetes object, so the framework's object steps don't apply as they
//! are; this module holds what is specific to it and nothing of the runner:
//!
//! - [`silence`]: the draft a person edits, the rules that class it as narrow
//!   or broad ([`silence::classify`]) and what it refuses outright. Pure.
//! - [`writer`]: [`AlertWriter`], the only thing that sends a `POST` or a
//!   `DELETE` to an Alertmanager, plus the reads a silence needs first (the
//!   preview of what it would mute) and after (the read-back). It goes
//!   through the person's own `kube::Client`: no stored credential.
//!
//! - [`change`]: the two [`Change`](super::Change) payloads, the scope the
//!   runner resolved and the descriptions a page hands over.
//! - `runner`: the runner's preflight, the check again before the send and
//!   the read-back (W2).
//!
//! The runner sends them (W2), but no page, dialog or example runner offers
//! one before W3, so no silence or Expire can be sent from the app yet. The
//! design is the plan in #884; `docs/WRITE_MODEL.md` has it.
mod change;
mod runner;
pub mod silence;
pub mod writer;

#[cfg(any(test, feature = "testing"))]
pub mod testing;

#[cfg(test)]
mod tests;

pub use change::{
    ExpireChange, Scope, ServiceSeen, SilenceChange, describe_expire, describe_silence,
};
pub(super) use change::{depends_on, questions, send};
pub(super) use runner::{AlertSeen, is_alert_action};
pub use silence::{
    Class, Draft, MAX_COMMENT, MAX_DURATION, MAX_MATCHERS, MAX_MUTED_NARROW, MAX_VALUE,
    MIN_DURATION, Mode, PROTECTED_CAP, Reason, Refusal, classify, validate,
};
pub use writer::{
    AlertWriter, CREATED_BY_PREFIX, CreateRequest, MAX_PREVIEW_ALERTS, MAX_PREVIEW_BYTES,
    MAX_PREVIEW_LISTED, Muted, Preview, PreviewedAlert, REQUEST_TIMEOUT, ReadBack, WriteError,
    access_question, find_landed,
};
