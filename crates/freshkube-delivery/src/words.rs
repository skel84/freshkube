//! The words for a part or entry that could not be read, from the
//! failure's kind alone: the Sources view, the Applications banner and the
//! change page's hops all say them here.

use freshkube_core::resources::FailureKind;

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

/// `⟨entry⟩ ⟨why⟩`: an entry that answered but didn't finish, with `why`
/// the reason (`didn't finish reading within 30 s`).
pub(crate) fn unfinished(entry: &str, why: &str) -> String {
    format!("{entry} {why}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_kind_has_its_words() {
        let say = |kind| failed(kind, "Kargo", "cluster-a", "the reason");
        assert_eq!(say(FailureKind::Config), "the reason");
        for kind in [
            FailureKind::Unreachable,
            FailureKind::Timeout,
            FailureKind::Other,
        ] {
            assert_eq!(say(kind), "Couldn't reach cluster-a: the reason");
        }
        for kind in [FailureKind::Unauthorized, FailureKind::Forbidden] {
            assert_eq!(say(kind), "Kargo refused on cluster-a: the reason");
        }
        assert_eq!(
            say(FailureKind::NotFound),
            "Kargo isn't served on cluster-a"
        );
    }

    #[test]
    fn an_entry_that_did_not_finish_is_named_first() {
        assert_eq!(
            unfinished("cluster-a", "didn't finish reading within 30 s"),
            "cluster-a didn't finish reading within 30 s"
        );
    }
}
