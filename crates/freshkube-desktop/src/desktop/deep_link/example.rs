//! How the example clusters identify themselves, so a link made in one
//! fixture window opens in another. Invented: nothing here is read from a
//! cluster. Each example cluster has a CA of its own, so a link made in a
//! context this window never opened matches it on the CA alone ("CA only")
//! until it is checked.
use super::identity::Local;
use crate::debug_options::LinkShape;
use freshkube_core::fingerprint::{StrongRead, strong_from_uid};

/// What the example kubeconfig says of `context`.
pub(super) fn local(context: &str, shape: Option<LinkShape>) -> Local {
    let ca = match shape {
        Some(LinkShape::Unmatched) => format!("example/other/ca/{context}"),
        // Two entries that are one cluster have one CA.
        Some(LinkShape::Several) if context == "stage-fra" => "example/ca/dev-fra".to_owned(),
        _ => format!("example/ca/{context}"),
    };
    Local {
        server: strong_from_uid(&format!("example/server/{context}")),
        ca: Some(strong_from_uid(&ca)),
    }
}

/// What a `kube-system` read of `context` answers: its own UID, or a refusal
/// under `ca-only`.
pub(super) fn read(context: &str, shape: Option<LinkShape>) -> StrongRead {
    match shape {
        Some(LinkShape::CaOnly) => StrongRead::Refused,
        Some(LinkShape::Unmatched) => StrongRead::Uid(format!("example/other/{context}")),
        // Two entries that are one cluster: the same namespace UID.
        Some(LinkShape::Several) if context == "stage-fra" => {
            StrongRead::Uid("example/kube-system/dev-fra".into())
        }
        _ => StrongRead::Uid(format!("example/kube-system/{context}")),
    }
}
