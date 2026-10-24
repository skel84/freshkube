//! Edit YAML (#858): the detail pane's YAML made editable, as a spec-change
//! write through the write module (`docs/WRITE_MODEL.md`). The page reads one
//! [`EditSession`], hands its text back at Cmd-S and gets an
//! [`ActionDescription`]; preflight, the dialog, the send and the audit are the
//! framework's. What is applied is the fields `freshkube` already owns plus the
//! changes ([`body`]), by server-side apply with field manager `freshkube`.

mod body;
mod locked;

use serde_json::Value;

use super::everyday::Row;
use super::send::{Refusal, get};
use super::{
    ActionDescription, Change, FIELD_MANAGER, GitOps, ObjectChange, Observation, Preview, Target,
    Tier,
};

pub use locked::{NOT_READ, locked_fields};

/// Why an edit is not described.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EditError {
    /// The text is not YAML.
    Yaml(String),
    /// The text is not one object.
    NotAnObject,
    /// The apiVersion, kind, name or namespace was changed: that is another
    /// object.
    Identity(&'static str),
    /// There is no change to send.
    NothingChanged,
}

impl EditError {
    pub fn text(&self) -> String {
        match self {
            Self::Yaml(why) => format!("The text is not valid YAML: {why}"),
            Self::NotAnObject => "The text must be one object.".to_owned(),
            Self::Identity(field) => {
                format!("{field} is the object's identity and can't be changed here.")
            }
            Self::NothingChanged => "Nothing was changed.".to_owned(),
        }
    }
}

/// One object read for editing: what the person edits and what the edit is
/// based on.
#[derive(Clone, Debug)]
pub struct EditSession {
    target: Target,
    /// The object without the cluster's own fields; the text starts from it.
    document: Value,
    /// What `freshkube` owns of it, as the server's `fieldsV1`.
    owned: Option<Value>,
    gitops_warning: Option<String>,
}

impl EditSession {
    /// Reads the object a row names, as it is now. A Secret is read as
    /// metadata only. An object that is another one now (a different UID) is
    /// not edited.
    pub async fn read(client: &kube::Client, row: &Row) -> Result<Self, String> {
        let target = Target {
            group: row.group.clone(),
            version: row.version.clone(),
            kind: row.kind.clone(),
            resource: row.resource.clone(),
            namespace: row.namespace.clone(),
            name: row.name.clone(),
            uid: row.uid.clone(),
            resource_version: String::new(),
            subresource: None,
        };
        let object = get(client, &target)
            .await
            .map_err(|refusal| match refusal {
                Refusal::Denied(why) => {
                    format!("You are not allowed to read {}: {why}", target.label())
                }
                _ => format!("{} could not be read.", target.label()),
            })?;
        if object["metadata"]["uid"].as_str() != Some(row.uid.as_str()) {
            return Err(format!(
                "{} is a different object now. Refresh the list.",
                target.label()
            ));
        }
        Self::from_object(target.clone(), &object)
            .ok_or_else(|| format!("{} has no identity to edit.", target.label()))
    }

    /// From the object as read (`read`'s result, or an example's).
    pub fn from_object(mut target: Target, object: &Value) -> Option<Self> {
        let text = |pointer: &str| object.pointer(pointer).and_then(Value::as_str);
        target.uid = text("/metadata/uid")?.to_owned();
        target.resource_version = text("/metadata/resourceVersion")?.to_owned();
        let owned = object["metadata"]["managedFields"]
            .as_array()
            .into_iter()
            .flatten()
            .find(|entry| {
                entry["manager"] == FIELD_MANAGER
                    && entry["operation"] == "Apply"
                    && entry.get("subresource").is_none()
            })
            .map(|entry| entry["fieldsV1"].clone());
        let gitops_warning = GitOps::of(object).map(|tool| tool.edit_warning(object));
        let mut document = object.clone();
        body::hide(&mut document, target.is_secret());
        Some(Self {
            target,
            document,
            owned,
            gitops_warning,
        })
    }

    pub fn target(&self) -> &Target {
        &self.target
    }

    /// The text the editor starts from.
    pub fn text(&self) -> String {
        serde_yaml::to_string(&self.document).unwrap_or_default()
    }

    pub fn locked(&self) -> Vec<&'static str> {
        locked_fields(self.target.is_secret())
    }

    /// The write the edited text asks for: server-side apply of what
    /// `freshkube` owns plus the changes, a spec change at every level.
    pub fn describe(&self, text: &str) -> Result<ActionDescription, EditError> {
        let edited: Value =
            serde_yaml::from_str(text).map_err(|why| EditError::Yaml(why.to_string()))?;
        if !edited.is_object() {
            return Err(EditError::NotAnObject);
        }
        for field in ["apiVersion", "kind", "metadata.name", "metadata.namespace"] {
            let pointer = format!("/{}", field.replace('.', "/"));
            if edited.pointer(&pointer) != self.document.pointer(&pointer) {
                return Err(EditError::Identity(match field {
                    "apiVersion" => "apiVersion",
                    "kind" => "kind",
                    "metadata.name" => "metadata.name",
                    _ => "metadata.namespace",
                }));
            }
        }
        let built = body::build(
            &self.document,
            &edited,
            self.owned.as_ref(),
            self.target.is_secret(),
        );
        if built.changed.is_empty() {
            return Err(EditError::NothingChanged);
        }
        let label = self.target.label();
        // ponytail: the first changed field stands for the edit; a value the
        // server normalises is told as not observed, never as a failure.
        let observe = built.changed.iter().find_map(|path| {
            let pointer: String = path
                .iter()
                .map(|key| format!("/{}", key.replace('~', "~0").replace('/', "~1")))
                .collect();
            let value = edited.pointer(&pointer)?.clone();
            Some(Observation::FieldEquals { pointer, value })
        });
        let sentence = format!(
            "Apply your edit to {label}: {} changed, sent as server-side apply with field manager {FIELD_MANAGER}, never forced.{}",
            freshkube_plural(built.changed.len()),
            self.gitops_warning.as_deref().unwrap_or_default()
        );
        let mut preview = vec![];
        if !built.ignored.is_empty() {
            preview.push(Preview {
                title: "Not applied: an edit can't change these".into(),
                lines: built.ignored,
            });
        }
        if !built.kept.is_empty() {
            preview.push(Preview {
                title: "Not removed: another manager owns these".into(),
                lines: built.kept,
            });
        }
        Ok(ActionDescription {
            kind: "edit-yaml".into(),
            tier: Tier::SpecChange,
            verb: "Apply".into(),
            sentence,
            changes: vec![ObjectChange {
                target: self.target.clone(),
                change: Change::Apply(built.body),
                observe: observe.unwrap_or(Observation::Unobservable),
                inverse: None,
                depends: vec![],
            }],
            destinations: vec![],
            no_confirmation: false,
            title: Some(format!("Edit {label}")),
            subjects: vec![],
            preview,
        })
    }
}

fn freshkube_plural(count: usize) -> String {
    format!("{count} field{}", if count == 1 { "" } else { "s" })
}
