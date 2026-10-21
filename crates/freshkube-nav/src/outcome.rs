//! Why a link could not be followed (#873): what the pane's slot says in
//! place of the object, each with its Why and Next. The words are drafts for
//! review.

/// The reader's own outcome of following a link, closed: no free text from
/// the link reaches it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum LinkOutcome {
    /// The server refused the read (401 or 403) with the reader's access.
    CantRead,
    /// No such object here: 404, or its namespace is gone.
    NotFound,
    /// The kind is not in this cluster's discovery.
    NotServed,
    /// The same name, a different UID: the link was made from an object that
    /// was deleted and created again.
    Replaced,
}

impl LinkOutcome {
    pub fn title(self) -> &'static str {
        match self {
            Self::CantRead => "Can’t read this",
            Self::NotFound => "Not found",
            Self::NotServed => "Not served here",
            Self::Replaced => "Replaced",
        }
    }

    /// Why the link did not open what it names.
    pub fn why(self) -> &'static str {
        match self {
            Self::CantRead => {
                "Your own access to this cluster doesn’t allow reading it: the server refused the read."
            }
            Self::NotFound => {
                "This cluster has no object by that name. It may have been deleted, or the link was made in another cluster."
            }
            Self::NotServed => {
                "This cluster doesn’t serve that kind of resource. Its API may not be installed here."
            }
            Self::Replaced => {
                "An object with that name exists, but it isn’t the one the link was made from: it was deleted and created again."
            }
        }
    }

    /// What to do next.
    pub fn next(self) -> &'static str {
        match self {
            Self::CantRead => "Ask for read access, or check that this is the right cluster.",
            Self::NotFound => "Look for it in the list, or close this.",
            Self::NotServed => "Pick a kind this cluster serves from the sidebar.",
            Self::Replaced => "Open the current one, or close this.",
        }
    }

    /// A stable word for ids and tests.
    pub fn slug(self) -> &'static str {
        match self {
            Self::CantRead => "cant-read",
            Self::NotFound => "not-found",
            Self::NotServed => "not-served",
            Self::Replaced => "replaced",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_outcome_says_why_and_what_next() {
        for outcome in [
            LinkOutcome::CantRead,
            LinkOutcome::NotFound,
            LinkOutcome::NotServed,
            LinkOutcome::Replaced,
        ] {
            assert!(!outcome.title().is_empty());
            assert!(outcome.why().ends_with('.'));
            assert!(outcome.next().ends_with('.'));
            assert!(!outcome.slug().contains(' '));
        }
    }
}
