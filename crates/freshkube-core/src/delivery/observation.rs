//! Where a link's evidence came from: which object, at which version, in
//! which cluster, and whether the cluster reported it, someone declared it,
//! or the join concluded it. An [`Observation`] is readable as text; it
//! needs no GUI and no cluster to be understood.

use serde_json::Value;

use super::digest::text;
use super::source::redact_message;

/// The role aliases of the clusters (and GitHub) a trail reads, as
/// [`Clusters`](super::Clusters) names them.
pub mod role {
    pub const KARGO: &str = "kargo";
    pub const ARGOCD: &str = "argocd";
    pub const TEKTON: &str = "tekton";
    pub const ENVIRONMENT: &str = "environment";
    pub const GITHUB: &str = "github";
    /// Where the change itself, which no cluster holds, is named.
    pub const CHANGE: &str = "change";
    /// The join's own conclusions, which no object holds.
    pub const JOIN: &str = "join";
}

/// The longest value an observation keeps, in characters. Values are SHAs,
/// digests and short statuses; anything longer is cut.
const MAX_VALUE: usize = 128;

/// `metadata.uid` and `metadata.resourceVersion`: which object, at which
/// version, an observation was made of. Either may be missing from what was
/// read.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Meta {
    pub uid: Option<String>,
    pub resource_version: Option<String>,
}

impl Meta {
    pub fn parse(value: &Value) -> Self {
        Self {
            uid: text(value, "/metadata/uid"),
            resource_version: text(value, "/metadata/resourceVersion"),
        }
    }
}

/// An object, as exactly as it was read.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ObjectRef {
    /// The API group; empty for the core group.
    pub group: String,
    pub kind: String,
    pub namespace: Option<String>,
    pub name: String,
    pub uid: Option<String>,
    pub resource_version: Option<String>,
}

impl ObjectRef {
    pub fn new(group: &str, kind: &str, namespace: Option<&str>, name: &str, meta: &Meta) -> Self {
        Self {
            group: group.to_owned(),
            kind: kind.to_owned(),
            namespace: namespace.map(str::to_owned),
            name: name.to_owned(),
            uid: meta.uid.clone(),
            resource_version: meta.resource_version.clone(),
        }
    }
}

impl std::fmt::Display for ObjectRef {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.kind)?;
        if !self.group.is_empty() {
            write!(f, ".{}", self.group)?;
        }
        f.write_str(" ")?;
        if let Some(namespace) = &self.namespace {
            write!(f, "{namespace}/")?;
        }
        f.write_str(&self.name)?;
        if let Some(uid) = &self.uid {
            write!(f, " uid {uid}")?;
        }
        if let Some(version) = &self.resource_version {
            write!(f, " rv {version}")?;
        }
        Ok(())
    }
}

/// What kind of statement an observation is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Fact {
    /// A controller's status: it says what it did or saw.
    Reported,
    /// A label, annotation or spec field: someone wrote it down.
    Declared,
    /// The join's own conclusion from other observations.
    Derived,
}

impl Fact {
    pub fn word(self) -> &'static str {
        match self {
            Self::Reported => "reported",
            Self::Declared => "declared",
            Self::Derived => "derived",
        }
    }
}

/// One thing read from one object.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Observation {
    /// The role alias of the cluster, or of GitHub, it was read from.
    pub cluster: String,
    pub object: ObjectRef,
    /// A JSON pointer (RFC 6901) into the object. Where the value sits in a
    /// list the pointer names the list.
    pub field: String,
    /// What was read there: short, and redacted. `None` when only the
    /// field's presence matters.
    pub value: Option<String>,
    pub fact: Fact,
}

impl Observation {
    pub fn new(
        cluster: &str,
        object: ObjectRef,
        field: &str,
        value: Option<&str>,
        fact: Fact,
    ) -> Self {
        Self {
            cluster: cluster.to_owned(),
            object,
            field: field.to_owned(),
            value: value.map(short_value),
            fact,
        }
    }

    pub fn reported(cluster: &str, object: ObjectRef, field: &str, value: Option<&str>) -> Self {
        Self::new(cluster, object, field, value, Fact::Reported)
    }

    pub fn declared(cluster: &str, object: ObjectRef, field: &str, value: Option<&str>) -> Self {
        Self::new(cluster, object, field, value, Fact::Declared)
    }

    pub fn derived(cluster: &str, object: ObjectRef, field: &str, value: Option<&str>) -> Self {
        Self::new(cluster, object, field, value, Fact::Derived)
    }

    /// One line, as [`render`](super::render) prints it after `from `.
    pub fn describe(&self) -> String {
        format!(
            "{}: {} {} {}{}",
            self.cluster,
            self.object,
            self.field,
            self.fact.word(),
            self.value
                .as_deref()
                .map(|value| format!(" = {value}"))
                .unwrap_or_default()
        )
    }
}

/// `value` redacted, on one line, and cut to [`MAX_VALUE`] characters.
fn short_value(value: &str) -> String {
    let redacted = redact_message(value);
    let line: String = redacted.split_whitespace().collect::<Vec<_>>().join(" ");
    if line.chars().count() > MAX_VALUE {
        let cut: String = line.chars().take(MAX_VALUE).collect();
        format!("{cut}…")
    } else {
        line
    }
}

/// A label or annotation key as a JSON pointer segment (RFC 6901).
pub fn pointer_segment(key: &str) -> String {
    key.replace('~', "~0").replace('/', "~1")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_value_is_one_short_redacted_line() {
        let long = "x".repeat(300);
        let object = ObjectRef::new("", "Pod", Some("acme"), "api", &Meta::default());
        let seen = Observation::reported("environment", object, "/status", Some(&long));
        assert!(seen.value.as_deref().unwrap().chars().count() <= MAX_VALUE + 1);
        assert_eq!(short_value("a\n  b"), "a b");
    }

    #[test]
    fn pointer_segments_escape_slashes() {
        assert_eq!(
            pointer_segment("pipelinesascode.tekton.dev/sha"),
            "pipelinesascode.tekton.dev~1sha"
        );
    }

    #[test]
    fn an_observation_reads_as_text() {
        let meta = Meta {
            uid: Some("u-1".into()),
            resource_version: Some("42".into()),
        };
        let object = ObjectRef::new("argoproj.io", "Rollout", Some("acme"), "api", &meta);
        let seen = Observation::declared("environment", object, "/spec", Some("sha256:abc"));
        assert_eq!(
            seen.describe(),
            "environment: Rollout.argoproj.io acme/api uid u-1 rv 42 /spec declared = sha256:abc"
        );
    }
}
