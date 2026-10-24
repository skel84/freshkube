//! The tabs a link opens on.

/// Where the pane opens: a section of Details, YAML, or the dock's logs.
/// The strip has two tabs, Details and YAML (an HTTPRoute also has Trace); Overview, Ports and Events are
/// sections of Details, one scrolling page.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tab {
    /// Details, from the top.
    Overview,
    Yaml,
    /// Details, scrolled to its events.
    Events,
    /// Not a tab: asking for it opens the object's logs in the dock, for
    /// pods and the workloads that run them.
    Logs,
    /// Details, scrolled to its ports: pods, Services and the workloads
    /// that run pods.
    Ports,
    /// An HTTPRoute's trace, on its own tab: the strip of any other object
    /// has no such tab, and asking for it opens Details.
    Trace,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum NodeTab {
    #[default]
    Overview,
    Pods,
    Services,
    Processes,
    Storage,
    Network,
    Diagnostics,
    Logs,
    Events,
    Yaml,
}

impl NodeTab {
    pub fn label(self) -> &'static str {
        match self {
            Self::Overview => "Overview",
            Self::Pods => "Pods",
            Self::Services => "System services",
            Self::Processes => "Processes",
            Self::Storage => "Storage",
            Self::Network => "Network",
            Self::Diagnostics => "Diagnostics",
            Self::Logs => "Logs",
            Self::Events => "Events",
            Self::Yaml => "YAML",
        }
    }
}

impl NodeTab {
    /// The word a link carries for this tab.
    pub fn slug(self) -> &'static str {
        match self {
            Self::Overview => "overview",
            Self::Pods => "pods",
            Self::Services => "services",
            Self::Processes => "processes",
            Self::Storage => "storage",
            Self::Network => "network",
            Self::Diagnostics => "diagnostics",
            Self::Logs => "logs",
            Self::Events => "events",
            Self::Yaml => "yaml",
        }
    }

    pub fn from_slug(slug: &str) -> Option<Self> {
        [
            Self::Overview,
            Self::Pods,
            Self::Services,
            Self::Processes,
            Self::Storage,
            Self::Network,
            Self::Diagnostics,
            Self::Logs,
            Self::Events,
            Self::Yaml,
        ]
        .into_iter()
        .find(|tab| tab.slug() == slug)
    }
}
