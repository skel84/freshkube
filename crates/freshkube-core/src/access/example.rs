//! Invented answers for `--fixture` and `FRESHKUBE_ACCESS`: namespaces,
//! people and rules that name no real cluster.

use super::{Cells, RowKey, Rule, RulesAnswer, Who, columns::Column};

/// How the example answers (`FRESHKUBE_ACCESS`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Shape {
    /// Everything allowed everywhere.
    Full,
    /// A developer: writes in two namespaces, reads elsewhere, nothing in
    /// one.
    #[default]
    Limited,
    /// Every rules review is incomplete, so what is not shown granted reads
    /// "ask".
    Incomplete,
    /// Two namespaces' reviews and the Nodes row fail.
    Failed,
    /// The first read is one person, the next another.
    IdentityChanged,
}

impl Shape {
    /// An empty or unknown value is the default shape.
    pub fn parse(text: &str) -> Self {
        match text {
            "full" => Self::Full,
            "incomplete" => Self::Incomplete,
            "failed" => Self::Failed,
            "identity-changed" => Self::IdentityChanged,
            _ => Self::Limited,
        }
    }
}

pub const NAMESPACES: [&str; 8] = [
    "cert-manager",
    "default",
    "kube-system",
    "monitoring",
    "payments",
    "shop",
    "staging",
    "team-a",
];

/// Whether the shape flips who is asking after the first read.
pub fn flips(shape: Shape) -> bool {
    shape == Shape::IdentityChanged
}

/// Who is signed in; `epoch` counts the reads of who so far.
pub fn who(shape: Shape, epoch: usize) -> Who {
    if flips(shape) && epoch > 0 {
        Who {
            username: "bob@example.test".into(),
            groups: vec!["system:authenticated".into(), "viewers".into()],
        }
    } else {
        Who {
            username: "alice@example.test".into(),
            groups: vec![
                "system:authenticated".into(),
                "developers".into(),
                "team-shop".into(),
            ],
        }
    }
}

fn rules(rows: &[(&[&str], &[&str], &[&str])]) -> Vec<Rule> {
    rows.iter()
        .map(|(verbs, groups, resources)| Rule::new(verbs, groups, resources))
        .collect()
}

fn reader() -> Vec<Rule> {
    rules(&[
        (&["get", "list", "watch"], &[""], &["pods", "pods/log"]),
        (&["get", "list"], &["apps"], &["deployments"]),
    ])
}

fn writer() -> Vec<Rule> {
    let mut all = reader();
    all.extend(rules(&[
        (&["patch"], &["apps"], &["deployments", "deployments/scale"]),
        (&["patch"], &[""], &["configmaps", "services"]),
        (&["delete"], &[""], &["pods"]),
        (&["create"], &[""], &["pods/exec"]),
    ]));
    all
}

/// The example answer for a row, `epoch` being the count of who reads.
pub fn answer(shape: Shape, epoch: usize, key: &RowKey) -> Result<Cells, String> {
    let namespace = match key {
        RowKey::Nodes => return nodes(shape, epoch),
        RowKey::Namespace(namespace) => namespace.as_str(),
    };
    let rules = match shape {
        Shape::Full => {
            return Ok(RulesAnswer {
                rules: rules(&[(&["*"], &["*"], &["*"])]),
                ..Default::default()
            }
            .cells());
        }
        Shape::Incomplete => {
            return Ok(RulesAnswer {
                rules: reader(),
                incomplete: true,
                evaluation_error: Some(
                    "a webhook authorizer did not answer, so some rules may be missing".into(),
                ),
            }
            .cells());
        }
        Shape::Failed if matches!(namespace, "shop" | "payments") => {
            return Err("the review was refused: forbidden".into());
        }
        Shape::IdentityChanged if epoch > 0 => {
            if namespace == "kube-system" {
                Vec::new()
            } else {
                reader()
            }
        }
        _ => match namespace {
            "shop" | "payments" => writer(),
            "kube-system" => Vec::new(),
            _ => reader(),
        },
    };
    Ok(RulesAnswer {
        rules,
        ..Default::default()
    }
    .cells())
}

fn nodes(shape: Shape, _epoch: usize) -> Result<Cells, String> {
    use super::Cell::{No, NotApplicable, Yes};
    if shape == Shape::Failed {
        return Err("the review was refused: forbidden".into());
    }
    let edit = Column::Edit.node_needs().unwrap_or_default().to_vec();
    let editor = shape == Shape::Full;
    Ok(Cells {
        cells: super::COLUMNS
            .iter()
            .map(|column| match column {
                Column::Read => Yes,
                Column::Edit if editor => Yes,
                Column::Edit => No(edit.clone()),
                _ => NotApplicable,
            })
            .collect(),
        incomplete: false,
        note: None,
    })
}
