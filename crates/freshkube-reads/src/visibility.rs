//! What an owner's read does when its page hides and shows again.

use std::time::{Duration, Instant};

/// Whether a read hidden and shown again reads again. Hiding always drops
/// the work and keeps what was read as last known; the owner picks what
/// showing does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OnShow {
    /// Read again once what was read is at least this old, or was never
    /// timed.
    ReadIfOlder(Duration),
    /// Never read again on showing; the owner reads when it chooses, as the
    /// dock's tabs do.
    Never,
}

impl OnShow {
    /// Whether data last known current at `since` should be read again at
    /// `now`. `None` is data never timed, which counts as old.
    pub(crate) fn reads_again(self, since: Option<Instant>, now: Instant) -> bool {
        match self {
            OnShow::ReadIfOlder(threshold) => {
                since.is_none_or(|since| now.saturating_duration_since(since) >= threshold)
            }
            OnShow::Never => false,
        }
    }
}
