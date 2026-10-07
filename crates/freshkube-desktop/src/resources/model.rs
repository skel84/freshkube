use std::cmp::Ordering;
use std::sync::Arc;

use super::rows::{PodRow, RowOwner};

/// Identifies one observed incarnation of any resource: the connection, the
/// kubectl resource key (`pods`, `deployments.apps`), the address and the UID
/// the API server assigned at creation. A session epoch is deliberately not
/// part of identity, so a relist keeps the selection while the object still
/// exists; epochs tag batches instead (see `ResourceBatch`).
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub(crate) struct ResourceIdentity {
    pub(crate) connection: String,
    pub(crate) resource: String,
    /// Empty for cluster-scoped resources.
    pub(crate) namespace: String,
    pub(crate) name: String,
    pub(crate) uid: String,
}

impl ResourceIdentity {
    /// `namespace/name`, or `name` for a cluster-scoped resource.
    pub(crate) fn address(&self) -> String {
        if self.namespace.is_empty() {
            self.name.clone()
        } else {
            format!("{}/{}", self.namespace, self.name)
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ColumnKind {
    Text,
    Number,
    /// Rendered from the object's creation time, so it stays current.
    Age,
}

/// One column the server (or the example data) prints for a kind.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ResourceColumn {
    pub(crate) name: String,
    pub(crate) kind: ColumnKind,
    /// Shown only in the wide view, like `kubectl get -o wide`.
    pub(crate) wide: bool,
}

impl ResourceColumn {
    pub(crate) fn new(name: &str, kind: ColumnKind, wide: bool) -> Self {
        Self {
            name: name.into(),
            kind,
            wide,
        }
    }
}

/// A row as printed for its kind. Cells line up with the store's columns.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct ResourceRow {
    pub(crate) identity: ResourceIdentity,
    pub(crate) cells: Vec<String>,
    /// Creation time in Unix seconds, when known.
    pub(crate) created: Option<i64>,
    pub(crate) terminating: bool,
    /// Changes whenever the object does, so an open detail pane knows to
    /// read it again. Not part of identity.
    pub(crate) resource_version: String,
    /// The object that manages this one, as the Owner column shows it.
    pub(crate) owner: Option<RowOwner>,
    /// Where the part of the name its owner generated starts.
    pub(crate) generated: Option<usize>,
    /// A pod's state, readiness, node and resources.
    pub(crate) pod: Option<Arc<PodRow>>,
}

impl ResourceRow {
    pub(crate) fn age(&self, now: i64) -> Option<String> {
        self.created
            .map(|created| format_age(now.saturating_sub(created).max(0) as u64))
    }
}

/// Human-readable age with up to two units, as in `kubectl get`.
pub(crate) fn format_age(seconds: u64) -> String {
    let (days, hours, minutes) = (seconds / 86_400, seconds / 3_600 % 24, seconds / 60 % 60);
    if days >= 365 {
        format!("{}y{}d", days / 365, days % 365)
    } else if days >= 8 {
        format!("{days}d")
    } else if days >= 1 {
        if hours > 0 {
            format!("{days}d{hours}h")
        } else {
            format!("{days}d")
        }
    } else if seconds >= 3_600 {
        if minutes > 0 && seconds < 8 * 3_600 {
            format!("{}h{minutes}m", seconds / 3_600)
        } else {
            format!("{}h", seconds / 3_600)
        }
    } else if seconds >= 60 {
        format!("{}m", seconds / 60)
    } else {
        format!("{seconds}s")
    }
}

/// Orders text with embedded numbers by value: `pod-2` before `pod-10`,
/// `2 (5m ago)` before `10`. Ties fall back to plain ordering.
pub(crate) fn natural_cmp(left: &str, right: &str) -> Ordering {
    let (mut a, mut b) = (left.chars().peekable(), right.chars().peekable());
    loop {
        match (a.peek().copied(), b.peek().copied()) {
            (None, None) => return left.cmp(right),
            (None, Some(_)) => return Ordering::Less,
            (Some(_), None) => return Ordering::Greater,
            (Some(x), Some(y)) if x.is_ascii_digit() && y.is_ascii_digit() => {
                let take = |chars: &mut std::iter::Peekable<std::str::Chars>| {
                    let mut digits = String::new();
                    while let Some(c) = chars.peek().copied().filter(char::is_ascii_digit) {
                        digits.push(c);
                        chars.next();
                    }
                    digits
                };
                let (x, y) = (take(&mut a), take(&mut b));
                let (xs, ys) = (x.trim_start_matches('0'), y.trim_start_matches('0'));
                let order = xs.len().cmp(&ys.len()).then_with(|| xs.cmp(ys));
                if order != Ordering::Equal {
                    return order;
                }
            }
            (Some(x), Some(y)) => {
                let order = x.cmp(&y);
                if order != Ordering::Equal {
                    return order;
                }
                a.next();
                b.next();
            }
        }
    }
}

/// The meaning of a printed status, for colouring; unknown values are neutral.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum StatusTone {
    Neutral,
    Success,
    Warning,
    Danger,
}

pub(crate) fn status_tone(value: &str) -> StatusTone {
    let value = value.trim();
    match value {
        "Running" | "Ready" | "Active" | "Bound" | "Available" | "Complete" | "Completed"
        | "Succeeded" | "True" | "Healthy" | "Synced" | "Established" => StatusTone::Success,
        "Pending"
        | "ContainerCreating"
        | "PodInitializing"
        | "Terminating"
        | "Unknown"
        | "Released"
        | "Progressing"
        | "Suspended"
        | "NotReady"
        | "Ready,SchedulingDisabled"
        | "SchedulingDisabled"
        | "Waiting" => StatusTone::Warning,
        _ if value.starts_with("Init:")
            && !value.contains("Error")
            && !value.contains("CrashLoop") =>
        {
            StatusTone::Warning
        }
        _ if value.contains("BackOff")
            || value.contains("Err")
            || value.contains("Fail")
            || value.contains("OOMKilled")
            || value == "Lost"
            || value == "Evicted"
            || value == "False" =>
        {
            StatusTone::Danger
        }
        _ => StatusTone::Neutral,
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum ReadState {
    Loading,
    Loaded,
    /// The identity may not read this collection.
    Refused(String),
    /// The read failed and nothing was ever loaded.
    Failed(String),
    /// The API server doesn't serve the kind: its definition was removed, or
    /// the version read is no longer served. Retrying won't help until that
    /// changes.
    Missing(String),
    /// Rows from an earlier read are shown, but the watch has failed and is
    /// retrying, so they may be out of date.
    Stale(String),
}

impl ReadState {
    /// Whether rows are being shown (possibly stale).
    pub(crate) fn shows_rows(&self) -> bool {
        matches!(self, Self::Loaded | Self::Stale(_))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SortKey {
    /// A printed column, by index into the store's columns.
    Column(usize),
    /// The namespace, then the name: the Name column when listing all
    /// namespaces, which shows both.
    Namespace,
    /// The owner's short kind and name.
    Owner,
    /// A pod's ready share of its containers, least ready first, then
    /// how many it has.
    Ready,
    /// A pod's restarts, then how many containers are ready.
    Restarts,
    /// A pod's use, as metrics-server reports it.
    Cpu,
    Memory,
    /// The node a pod runs on.
    Node,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SortDirection {
    /// Observation order: as listed, then as objects appeared.
    Default,
    Ascending,
    Descending,
}

impl SortDirection {
    /// A header click: ascending, then descending, then back to as listed.
    pub(crate) fn next(self) -> Self {
        match self {
            Self::Default => Self::Ascending,
            Self::Ascending => Self::Descending,
            Self::Descending => Self::Default,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn natural_order_compares_numbers_by_value() {
        let mut values = ["pod-10", "pod-2", "pod-1", "10 (1d ago)", "2 (5m ago)", "0"];
        values.sort_by(|a, b| natural_cmp(a, b));
        assert_eq!(
            values,
            ["0", "2 (5m ago)", "10 (1d ago)", "pod-1", "pod-2", "pod-10"]
        );
        assert_eq!(natural_cmp("1/1", "0/1"), Ordering::Greater);
        assert_eq!(natural_cmp("a", "a"), Ordering::Equal);
    }

    #[test]
    fn age_matches_kubectl_style() {
        assert_eq!(format_age(59), "59s");
        assert_eq!(format_age(60), "1m");
        assert_eq!(format_age(2 * 3_600 + 5 * 60), "2h5m");
        assert_eq!(format_age(9 * 3_600), "9h");
        assert_eq!(format_age(86_400 + 3 * 3_600), "1d3h");
        assert_eq!(format_age(21 * 86_400), "21d");
        assert_eq!(format_age(400 * 86_400), "1y35d");
    }

    #[test]
    fn statuses_map_to_tones() {
        assert_eq!(status_tone("Running"), StatusTone::Success);
        assert_eq!(status_tone("ContainerCreating"), StatusTone::Warning);
        assert_eq!(status_tone("Init:0/2"), StatusTone::Warning);
        assert_eq!(status_tone("Init:CrashLoopBackOff"), StatusTone::Danger);
        assert_eq!(status_tone("CrashLoopBackOff"), StatusTone::Danger);
        assert_eq!(status_tone("Error (exit 1)"), StatusTone::Danger);
        assert_eq!(status_tone("ClusterIP"), StatusTone::Neutral);
    }

    #[test]
    fn header_clicks_cycle_through_both_orders_and_back() {
        let mut direction = SortDirection::Default;
        let mut seen = Vec::new();
        for _ in 0..3 {
            direction = direction.next();
            seen.push(direction);
        }
        assert_eq!(
            seen,
            [
                SortDirection::Ascending,
                SortDirection::Descending,
                SortDirection::Default
            ]
        );
    }
}

/// An object link may carry a UID from metadata or a summary.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ObjectRef {
    pub(crate) namespace: String,
    pub(crate) name: String,
    pub(crate) uid: String,
}
impl From<ResourceIdentity> for ObjectRef {
    fn from(identity: ResourceIdentity) -> Self {
        Self {
            namespace: identity.namespace,
            name: identity.name,
            uid: identity.uid,
        }
    }
}
