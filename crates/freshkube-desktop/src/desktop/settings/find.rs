//! What the setup finders share (#821): what a person has ignored in this
//! session.

/// A suggestion the person ignored. It lasts for the session, across
/// checks and section changes, and is gone at the next launch; nothing is
/// saved.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(super) enum Ignored {
    /// A metrics value suggested for an entry.
    MetricsValue { entry: String, value: String },
}
