//! What example data never answers.

/// What example data never answers, so the pages that wait for it stay on
/// their loading rows, for captures and the stress binary:
/// `FRESHKUBE_FIXTURE_HOLD=talos` holds the Talos overview and the
/// Kubernetes summary (Nodes, System services, Operations), `lists` the
/// Resources lists, `coroot` Observability's applications, incidents and
/// traces, `monitoring` the dashboard's panels, `applications` the
/// Applications list, `all` every one. Parsed into the desktop app's
/// `DebugOptions`, which debug and stress builds fill and a release build
/// leaves empty.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Hold {
    pub talos: bool,
    pub lists: bool,
    pub coroot: bool,
    pub monitoring: bool,
    pub applications: bool,
}

impl Hold {
    /// Example data that answers everything.
    pub const NONE: Self = Self {
        talos: false,
        lists: false,
        coroot: false,
        monitoring: false,
        applications: false,
    };

    /// The holds a `FRESHKUBE_FIXTURE_HOLD` value names; an unknown or
    /// missing one holds nothing.
    pub fn parse(value: &str) -> Self {
        Self {
            talos: matches!(value, "talos" | "all"),
            lists: matches!(value, "lists" | "all"),
            coroot: matches!(value, "coroot" | "all"),
            monitoring: matches!(value, "monitoring" | "all"),
            applications: matches!(value, "applications" | "all"),
        }
    }
}
