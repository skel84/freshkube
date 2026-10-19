//! The words for a part or entry that could not be read, from the
//! failure's kind alone: the Sources view, the Applications banner and the
//! change page's hops all say them here.

use freshkube_core::resources::FailureKind;

/// `Couldn't reach ⟨entry⟩: ⟨why⟩`.
pub fn unreachable(entry: &str, why: &str) -> String {
    format!("Couldn't reach {entry}: {why}")
}

/// `refused on ⟨entry⟩: ⟨why⟩`: what a read the entry refused (401 or 403)
/// came to, for a sentence that names its subject itself.
pub(crate) fn refused_on(entry: &str, why: &str) -> String {
    format!("refused on {entry}: {why}")
}

/// `⟨what⟩ refused on ⟨entry⟩: ⟨why⟩`: a read the entry refused (401 or 403).
fn refused(what: &str, entry: &str, why: &str) -> String {
    format!("{what} {}", refused_on(entry, why))
}

/// `it wasn't found on ⟨entry⟩`: the answer for one object the entry doesn't
/// have (404).
pub(crate) fn wasnt_found_on(entry: &str) -> String {
    format!("it wasn't found on {entry}")
}

/// `⟨what⟩ isn't served on ⟨entry⟩`: a kind the entry doesn't serve (404).
fn not_served(what: &str, entry: &str) -> String {
    format!("{what} isn't served on {entry}")
}

/// `⟨what⟩ were refused on ⟨entry⟩: ⟨why⟩`, for a plural `what` that was
/// asked for and refused.
pub(crate) fn were_refused(what: &str, entry: &str, why: &str) -> String {
    format!("{what} were refused on {entry}: {why}")
}

/// `⟨what⟩ aren't served on ⟨entry⟩`, for a plural `what`.
pub(crate) fn arent_served(what: &str, entry: &str) -> String {
    format!("{what} aren't served on {entry}")
}

/// `⟨what⟩ couldn't be read on ⟨entry⟩: ⟨why⟩`, for a plural `what` that
/// was asked for and failed in some other way.
pub(crate) fn unreadable(what: &str, entry: &str, why: &str) -> String {
    format!("{what} couldn't be read on {entry}: {why}")
}

/// What reading `what` on `entry` came to, in a sentence, for a read that
/// failed with `kind` and the words `why`. A [`FailureKind::Config`] is a
/// part no entry holds, and says `why` alone.
/// Surface: delivery entry sentences in Sources, Applications banners and change hops.
pub fn failed(kind: FailureKind, what: &str, entry: &str, why: &str) -> String {
    match kind {
        FailureKind::Config => why.to_owned(),
        FailureKind::Unreachable | FailureKind::Timeout | FailureKind::Other => {
            unreachable(entry, why)
        }
        FailureKind::Unauthorized | FailureKind::Forbidden => refused(what, entry, why),
        FailureKind::NotFound => not_served(what, entry),
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
    fn singular_sentences_are_worded_here() {
        assert_eq!(
            refused_on("cluster-a", "forbidden"),
            "refused on cluster-a: forbidden"
        );
        assert_eq!(wasnt_found_on("cluster-a"), "it wasn't found on cluster-a");
        assert_eq!(
            refused("Kargo", "cluster-a", "forbidden"),
            "Kargo refused on cluster-a: forbidden"
        );
        assert_eq!(
            not_served("Kargo", "cluster-a"),
            "Kargo isn't served on cluster-a"
        );
    }

    #[test]
    fn plural_sentences_are_worded_here() {
        assert_eq!(
            were_refused("Kargo Projects", "cluster-a", "forbidden"),
            "Kargo Projects were refused on cluster-a: forbidden"
        );
        assert_eq!(
            arent_served("Kargo Projects", "cluster-a"),
            "Kargo Projects aren't served on cluster-a"
        );
        assert_eq!(
            unreadable("Kargo Projects", "cluster-a", "slow"),
            "Kargo Projects couldn't be read on cluster-a: slow"
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
