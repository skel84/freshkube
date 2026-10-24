//! What a link opens, in words, for the toast.
use freshkube_nav::{LinkPage, NodeTab, ViewState, ViewTab};

/// `deployments.apps payments/checkout (YAML)`, `pods in payments`,
/// `node cp-1 (Diagnostics)`, `Etcd`.
pub(in crate::desktop) fn describe(view: &ViewState) -> String {
    match view.page {
        LinkPage::Resources => match (&view.kind, &view.object, &view.ns) {
            (Some(kind), Some(object), ns) => {
                let place = match ns {
                    Some(ns) => format!("{}/{}", ns.as_str(), object.name.as_str()),
                    None => object.name.as_str().to_owned(),
                };
                let tab = match view.tab {
                    ViewTab::Details => "",
                    ViewTab::Yaml => " (YAML)",
                    ViewTab::Events => " (Events)",
                    ViewTab::Ports => " (Ports)",
                    ViewTab::Logs => " (Logs)",
                };
                format!("{} {place}{tab}", kind.as_str())
            }
            (Some(kind), None, Some(ns)) => format!("{} in {}", kind.as_str(), ns.as_str()),
            (Some(kind), None, None) => kind.as_str().to_owned(),
            (None, ..) => "Resources".to_owned(),
        },
        LinkPage::Nodes => match &view.node {
            Some(node) if view.node_tab != NodeTab::Overview => {
                format!("node {} ({})", node.as_str(), view.node_tab.label())
            }
            Some(node) => format!("node {}", node.as_str()),
            None => "Nodes".to_owned(),
        },
        page => title(page).to_owned(),
    }
}

fn title(page: LinkPage) -> &'static str {
    match page {
        LinkPage::Overview => "Overview",
        LinkPage::Nodes => "Nodes",
        LinkPage::Health => "Health",
        LinkPage::Databases => "Databases",
        LinkPage::Access => "Your access",
        LinkPage::Resources => "Resources",
        LinkPage::Etcd => "etcd",
        LinkPage::SystemServices => "System services",
        LinkPage::Security => "Security",
        LinkPage::Lifecycle => "Lifecycle",
        LinkPage::Monitoring => "Monitoring",
        LinkPage::Observability => "Observability",
        LinkPage::Applications => "Applications",
    }
}
