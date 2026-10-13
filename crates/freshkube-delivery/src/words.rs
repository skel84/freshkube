//! The words for a part or entry that could not be read, from the
//! failure's kind alone: the Sources view, the Applications banner and the
//! change page's hops all say them here.

use freshkube_core::resources::FailureKind;

/// Whether `kind` says the entry itself was not reached, as opposed to a
/// refusal, a kind the server doesn't serve, or no entry at all.
pub fn is_unreached(kind: FailureKind) -> bool {
    match kind {
        FailureKind::Unreachable | FailureKind::Timeout | FailureKind::Other => true,
        FailureKind::Config
        | FailureKind::Unauthorized
        | FailureKind::Forbidden
        | FailureKind::NotFound => false,
    }
}

/// `Couldn't reach ⟨entry⟩: ⟨why⟩`.
pub fn unreachable(entry: &str, why: &str) -> String {
    format!("Couldn't reach {entry}: {why}")
}

/// What reading `what` on `entry` came to, in a sentence, for a read that
/// failed with `kind` and the words `why`. A [`FailureKind::Config`] is a
/// part no entry holds, and says `why` alone.
pub fn failed(kind: FailureKind, what: &str, entry: &str, why: &str) -> String {
    match kind {
        FailureKind::Config => why.to_owned(),
        FailureKind::Unreachable | FailureKind::Timeout | FailureKind::Other => {
            unreachable(entry, why)
        }
        FailureKind::Unauthorized | FailureKind::Forbidden => {
            format!("{what} refused on {entry}: {why}")
        }
        FailureKind::NotFound => format!("{what} isn't served on {entry}"),
    }
}
