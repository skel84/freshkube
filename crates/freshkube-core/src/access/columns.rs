//! The table's columns and how a rules review fills them.

use k8s_openapi::api::authorization::v1::ResourceRule;

/// One thing a column needs: a verb on a resource, maybe a subresource.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Need {
    pub verb: &'static str,
    /// The API group; empty for the core group.
    pub group: &'static str,
    pub resource: &'static str,
    pub subresource: Option<&'static str>,
}

impl Need {
    const fn on(verb: &'static str, group: &'static str, resource: &'static str) -> Self {
        Self {
            verb,
            group,
            resource,
            subresource: None,
        }
    }

    const fn sub(
        verb: &'static str,
        group: &'static str,
        resource: &'static str,
        subresource: &'static str,
    ) -> Self {
        Self {
            verb,
            group,
            resource,
            subresource: Some(subresource),
        }
    }

    /// The resource as people say it: `pods`, `pods/log`.
    pub fn resource_words(&self) -> String {
        match self.subresource {
            Some(sub) => format!("{}/{sub}", self.resource),
            None => self.resource.to_owned(),
        }
    }

    /// The resource with its group, as an access request names it:
    /// `deployments.apps/scale`.
    pub fn qualified(&self) -> String {
        let base = if self.group.is_empty() {
            self.resource.to_owned()
        } else {
            format!("{}.{}", self.resource, self.group)
        };
        match self.subresource {
            Some(sub) => format!("{base}/{sub}"),
            None => base,
        }
    }

    /// The short form of a no: "needs patch on deployments in shop". Other
    /// surfaces that say why an action is off use the same words.
    pub fn phrase(&self, namespace: Option<&str>) -> String {
        match namespace {
            Some(namespace) => format!(
                "needs {} on {} in {namespace}",
                self.verb,
                self.resource_words()
            ),
            None => format!(
                "needs {} on {} cluster-wide",
                self.verb,
                self.resource_words()
            ),
        }
    }
}

/// A column of the table.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Column {
    Read,
    Logs,
    Shell,
    Forward,
    ScaleRestart,
    Edit,
    DeletePods,
    Secrets,
}

/// The columns in order.
pub const COLUMNS: [Column; 8] = [
    Column::Read,
    Column::Logs,
    Column::Shell,
    Column::Forward,
    Column::ScaleRestart,
    Column::Edit,
    Column::DeletePods,
    Column::Secrets,
];

impl Column {
    pub fn label(self) -> &'static str {
        match self {
            Self::Read => "Read",
            Self::Logs => "Logs",
            Self::Shell => "Shell",
            Self::Forward => "Forward",
            Self::ScaleRestart => "Scale·restart",
            Self::Edit => "Edit",
            Self::DeletePods => "Delete pods",
            Self::Secrets => "Secrets",
        }
    }

    pub fn slug(self) -> &'static str {
        match self {
            Self::Read => "read",
            Self::Logs => "logs",
            Self::Shell => "shell",
            Self::Forward => "forward",
            Self::ScaleRestart => "scale-restart",
            Self::Edit => "edit",
            Self::DeletePods => "delete-pods",
            Self::Secrets => "secrets",
        }
    }

    /// Everything the column needs in a namespace: it is a yes only when
    /// every one is granted.
    pub fn needs(self) -> &'static [Need] {
        const READ: [Need; 3] = [
            Need::on("get", "", "pods"),
            Need::on("list", "", "pods"),
            Need::on("watch", "", "pods"),
        ];
        const LOGS: [Need; 1] = [Need::sub("get", "", "pods", "log")];
        const SHELL: [Need; 1] = [Need::sub("create", "", "pods", "exec")];
        const FORWARD: [Need; 1] = [Need::sub("create", "", "pods", "portforward")];
        const SCALE: [Need; 2] = [
            Need::on("patch", "apps", "deployments"),
            Need::sub("patch", "apps", "deployments", "scale"),
        ];
        const EDIT: [Need; 2] = [
            Need::on("patch", "", "configmaps"),
            Need::on("patch", "", "services"),
        ];
        const DELETE: [Need; 1] = [Need::on("delete", "", "pods")];
        const SECRETS: [Need; 1] = [Need::on("get", "", "secrets")];
        match self {
            Self::Read => &READ,
            Self::Logs => &LOGS,
            Self::Shell => &SHELL,
            Self::Forward => &FORWARD,
            Self::ScaleRestart => &SCALE,
            Self::Edit => &EDIT,
            Self::DeletePods => &DELETE,
            Self::Secrets => &SECRETS,
        }
    }

    /// What the cluster-wide Nodes row asks for this column; `None` where
    /// the column means nothing for nodes.
    pub fn node_needs(self) -> Option<&'static [Need]> {
        const READ: [Need; 2] = [Need::on("get", "", "nodes"), Need::on("list", "", "nodes")];
        // Cordon and uncordon are a patch.
        const EDIT: [Need; 1] = [Need::on("patch", "", "nodes")];
        match self {
            Self::Read => Some(&READ),
            Self::Edit => Some(&EDIT),
            _ => None,
        }
    }
}

/// One cell of the table.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Cell {
    Yes,
    /// The cluster's rules do not grant these.
    No(Vec<Need>),
    /// The rules review was incomplete and does not show these granted, so
    /// the gate's own review decides when the action is tried.
    Ask(Vec<Need>),
    /// The column means nothing for this row.
    NotApplicable,
}

/// A rule of a rules review, in the words the table matches on.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct Rule {
    pub verbs: Vec<String>,
    pub groups: Vec<String>,
    pub resources: Vec<String>,
    /// A rule naming objects grants those objects alone, so it is never
    /// read as a grant on the whole resource.
    pub names: Vec<String>,
}

impl Rule {
    pub fn new(verbs: &[&str], groups: &[&str], resources: &[&str]) -> Self {
        let own = |words: &[&str]| words.iter().map(|word| (*word).to_owned()).collect();
        Self {
            verbs: own(verbs),
            groups: own(groups),
            resources: own(resources),
            names: Vec::new(),
        }
    }

    pub fn from_resource_rule(rule: &ResourceRule) -> Self {
        Self {
            verbs: rule.verbs.clone(),
            groups: rule.api_groups.clone().unwrap_or_default(),
            resources: rule.resources.clone().unwrap_or_default(),
            names: rule.resource_names.clone().unwrap_or_default(),
        }
    }

    fn grants(&self, need: &Need) -> bool {
        self.names.is_empty()
            && self.verbs.iter().any(|v| v == "*" || v == need.verb)
            && self.groups.iter().any(|g| g == "*" || g == need.group)
            && self
                .resources
                .iter()
                .any(|rule| resource_matches(rule, need))
    }
}

/// How RBAC matches a rule's resource: `*`, the resource (or
/// `resource/subresource`), `*/subresource`, and `resource/*`.
fn resource_matches(rule: &str, need: &Need) -> bool {
    let combined = need.resource_words();
    rule == "*"
        || rule == combined
        || need
            .subresource
            .is_some_and(|sub| rule.strip_prefix("*/") == Some(sub))
        || rule.strip_suffix("/*") == Some(need.resource)
}

/// What one rules review answered, reduced to what the table matches on.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct RulesAnswer {
    pub rules: Vec<Rule>,
    /// The server could not list every rule (a webhook authorizer, say), so
    /// a missing grant may exist.
    pub incomplete: bool,
    /// The server's own words for why, when it gave any.
    pub evaluation_error: Option<String>,
}

impl RulesAnswer {
    pub fn grants(&self, need: &Need) -> bool {
        self.rules.iter().any(|rule| rule.grants(need))
    }

    /// The cell of every column, in [`COLUMNS`] order.
    pub fn cells(&self) -> Cells {
        let cells = COLUMNS
            .iter()
            .map(|column| {
                let missing: Vec<Need> = column
                    .needs()
                    .iter()
                    .filter(|need| !self.grants(need))
                    .copied()
                    .collect();
                if missing.is_empty() {
                    Cell::Yes
                } else if self.incomplete {
                    Cell::Ask(missing)
                } else {
                    Cell::No(missing)
                }
            })
            .collect();
        Cells {
            cells,
            incomplete: self.incomplete,
            note: self.evaluation_error.clone(),
        }
    }
}

/// A row's cells, in [`COLUMNS`] order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Cells {
    pub cells: Vec<Cell>,
    /// The rules review was incomplete: "ask" cells are not a no.
    pub incomplete: bool,
    /// The server's note on an incomplete review.
    pub note: Option<String>,
}

impl Cells {
    pub fn cell(&self, column: Column) -> &Cell {
        let ix = COLUMNS.iter().position(|c| *c == column).unwrap_or(0);
        &self.cells[ix]
    }

    /// Every need the row lacks (a no) or cannot confirm (an ask), in
    /// column order.
    pub fn missing(&self) -> Vec<(Column, &[Need])> {
        COLUMNS
            .iter()
            .zip(&self.cells)
            .filter_map(|(column, cell)| match cell {
                Cell::No(needs) | Cell::Ask(needs) => Some((*column, needs.as_slice())),
                Cell::Yes | Cell::NotApplicable => None,
            })
            .collect()
    }

    /// The needs of cells that are a plain no, for an access request.
    pub fn denied(&self) -> Vec<Need> {
        let mut needs: Vec<Need> = COLUMNS
            .iter()
            .zip(&self.cells)
            .filter_map(|(_, cell)| match cell {
                Cell::No(needs) => Some(needs.iter().copied()),
                _ => None,
            })
            .flatten()
            .collect();
        needs.sort();
        needs.dedup();
        needs
    }
}
