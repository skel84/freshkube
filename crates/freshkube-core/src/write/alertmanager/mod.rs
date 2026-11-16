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
//! Nothing outside `freshkube-core` names this yet: the runner reaches it in
//! W2 and no page offers it before W3, so no silence or Expire can be sent
//! from the app. `docs/WRITE_MODEL.md` has the design.
pub mod silence;
pub mod writer;

#[cfg(test)]
mod tests;

pub use silence::{
    Class, Draft, MAX_COMMENT, MAX_DURATION, MAX_MATCHERS, MAX_MUTED_NARROW, MAX_VALUE,
    MIN_DURATION, Mode, PROTECTED_CAP, Reason, Refusal, classify, validate,
};
pub use writer::{
    AlertWriter, CreateRequest, MAX_PREVIEW_ALERTS, MAX_PREVIEW_BYTES, Muted, Preview,
    PreviewedAlert, ReadBack, WriteError,
};
