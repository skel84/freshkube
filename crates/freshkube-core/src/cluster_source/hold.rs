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
    pub routes: bool,
}

impl Hold {
    /// Example data that answers everything.
    pub const NONE: Self = Self {
        talos: false,
        lists: false,
        coroot: false,
        monitoring: false,
        applications: false,
        routes: false,
    };

    /// Every word `FRESHKUBE_FIXTURE_HOLD` takes, with what it holds: the one
    /// list `parse` looks values up in and `docs/DEBUG_KNOBS.md`'s test
    /// compares with the doc.
    pub const SHAPES: &'static [(&'static str, Self)] = &[
        (
            "talos",
            Self {
                talos: true,
                ..Self::NONE
            },
        ),
        (
            "lists",
            Self {
                lists: true,
                ..Self::NONE
            },
        ),
        (
            "coroot",
            Self {
                coroot: true,
                ..Self::NONE
            },
        ),
        (
            "monitoring",
            Self {
                monitoring: true,
                ..Self::NONE
            },
        ),
        (
            "applications",
            Self {
                applications: true,
                ..Self::NONE
            },
        ),
        (
            "routes",
            Self {
                routes: true,
                ..Self::NONE
            },
        ),
        (
            "all",
            Self {
                talos: true,
                lists: true,
                coroot: true,
                monitoring: true,
                applications: true,
                routes: true,
            },
        ),
    ];

    /// The holds a `FRESHKUBE_FIXTURE_HOLD` value names; an unknown or
    /// missing one holds nothing.
    pub fn parse(value: &str) -> Self {
        Self::SHAPES
            .iter()
            .find_map(|(word, hold)| (*word == value).then_some(*hold))
            .unwrap_or(Self::NONE)
    }
}
