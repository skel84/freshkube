use super::{AppId, Source};
use crate::resources::{ResourceKind, builtin_by_gvk};

/// An address to resolve through metadata on the explicitly associated access.
/// A Coroot AppId is never used as a Kubernetes UID.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ObjectSubject {
    access: String,
    kind: ResourceKind,
    namespace: String,
    name: String,
}

impl ObjectSubject {
    pub fn for_app(source: &Source, app: &AppId) -> Option<Self> {
        let association = source.association()?;
        if app.cluster_id() != association.cluster() {
            return None;
        }
        let api = match app.kind() {
            "Pod" | "Service" => "v1",
            "Deployment" | "StatefulSet" | "DaemonSet" | "ReplicaSet" => "apps/v1",
            "Job" | "CronJob" => "batch/v1",
            _ => return None,
        };
        let namespace = app.namespace()?;
        if association.access().is_empty()
            || !dns_name(namespace, 63, false)
            || !dns_name(app.name(), 253, true)
        {
            return None;
        }
        Some(Self {
            access: association.access().into(),
            kind: builtin_by_gvk(api, app.kind())?,
            namespace: namespace.into(),
            name: app.name().into(),
        })
    }
    pub fn access(&self) -> &str {
        &self.access
    }
    pub fn kind(&self) -> &ResourceKind {
        &self.kind
    }
    pub fn namespace(&self) -> &str {
        &self.namespace
    }
    pub fn name(&self) -> &str {
        &self.name
    }
}

fn dns_name(value: &str, limit: usize, dots: bool) -> bool {
    let alphanumeric = |byte: u8| byte.is_ascii_lowercase() || byte.is_ascii_digit();
    !value.is_empty()
        && value.len() <= limit
        && alphanumeric(value.as_bytes()[0])
        && alphanumeric(value.as_bytes()[value.len() - 1])
        && value
            .bytes()
            .all(|byte| alphanumeric(byte) || byte == b'-' || (dots && byte == b'.'))
}
