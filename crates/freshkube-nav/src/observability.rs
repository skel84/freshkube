//! A link into Observability: one Coroot application of a workspace entry.

use freshkube_core::coroot::AppId;

/// Which report of the application's page the link opens on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ObservabilityTab {
    /// The application's page on the report it was last on.
    Overview,
    /// The PostgreSQL report.
    Postgres,
    /// The MySQL (and MariaDB) report.
    Mysql,
}

impl ObservabilityTab {
    /// The report's name as Coroot's application page labels it; none leaves
    /// the page on its current report.
    pub fn report(self) -> Option<&'static str> {
        match self {
            Self::Overview => None,
            Self::Postgres => Some("Postgres"),
            Self::Mysql => Some("MySQL"),
        }
    }
}

/// Navigation to an application in Observability. It names the workspace
/// entry it was made in, so a link made in one cluster is never opened on
/// another's Coroot.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ObservabilityLink {
    /// The workspace entry's id.
    pub entry: String,
    pub app: AppId,
    pub tab: ObservabilityTab,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tabs_name_coroots_reports() {
        assert_eq!(ObservabilityTab::Overview.report(), None);
        assert_eq!(ObservabilityTab::Postgres.report(), Some("Postgres"));
        assert_eq!(ObservabilityTab::Mysql.report(), Some("MySQL"));
    }

    #[test]
    fn links_compare_by_entry_application_and_tab() {
        let link = |entry: &str| ObservabilityLink {
            entry: entry.into(),
            app: AppId::new("core-fra:shop:DatabaseCluster:orders-db"),
            tab: ObservabilityTab::Postgres,
        };
        assert_eq!(link("core-fra"), link("core-fra"));
        assert_ne!(link("core-fra"), link("core-lon"));
    }
}
