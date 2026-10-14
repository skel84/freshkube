//! The objects a link names.

use freshkube_core::resources::ResourceKind;

/// Identifies one observed incarnation of any resource: the connection, the
/// kubectl resource key (`pods`, `deployments.apps`), the address and the UID
/// the API server assigned at creation. A session epoch is deliberately not
/// part of identity, so a relist keeps the selection while the object still
/// exists; epochs tag batches instead (see `ResourceStore::apply`).
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ResourceIdentity {
    pub connection: String,
    pub resource: String,
    /// Empty for cluster-scoped resources.
    pub namespace: String,
    pub name: String,
    pub uid: String,
}

impl ResourceIdentity {
    /// `namespace/name`, or `name` for a cluster-scoped resource.
    pub fn address(&self) -> String {
        if self.namespace.is_empty() {
            self.name.clone()
        } else {
            format!("{}/{}", self.namespace, self.name)
        }
    }
}

/// The object the pane shows: an identity from the list, with its kind.
#[derive(Clone, Debug, PartialEq)]
pub struct DetailTarget {
    pub identity: ResourceIdentity,
    pub kind: ResourceKind,
}

/// An object link may carry a UID from metadata or a summary.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ObjectRef {
    pub namespace: String,
    pub name: String,
    pub uid: String,
    /// The cluster the link was made in: a `ResourceIdentity::connection`.
    /// `None` means the one that is open. A link naming another cluster is
    /// never opened against this one.
    pub connection: Option<String>,
}
impl From<ResourceIdentity> for ObjectRef {
    fn from(identity: ResourceIdentity) -> Self {
        Self {
            namespace: identity.namespace,
            name: identity.name,
            uid: identity.uid,
            connection: Some(identity.connection),
        }
    }
}
