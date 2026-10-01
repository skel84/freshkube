//! Read-only Kubernetes resource browsing for any kind.
//!
//! Collections are listed and watched through the API server's table format
//! (`meta.k8s.io/v1` `Table`), which carries the columns `kubectl get` prints
//! for every kind, custom resources included. Clients come either from a
//! kubeconfig context ([`connect`]) or from the Talos side of the app. One
//! object is read in full on demand ([`get_object`]), with its events
//! ([`watch_object_events`]). Custom kinds come from discovery
//! ([`list_custom_groups`], [`list_group_kinds`]).
//! Nothing here changes the cluster.

mod connection;
mod contexts;
mod discovery;
mod events;
mod failure;
mod kinds;
mod object;
mod table;
mod watch;

pub use connection::{Connection, connect};
pub use contexts::{KubeContext, KubeconfigReport, discover_contexts, kubeconfig_sources};
pub use discovery::{ApiGroup, GroupKinds, list_custom_groups, list_group_kinds};
pub use events::{EventScope, EventUpdate, ObjectEvent, watch_object_events};
pub use failure::{Failure, FailureKind};
pub use kinds::{ResourceKind, builtin};
pub use object::{
    Condition, ObjectDocument, Overview, Owner, SecretKey, SecretSummary, SecretValue, get_object,
    hidden_value, object_from_yaml, reveal_secret_value,
};
pub use table::{RowMetadata, Table, TableColumn, TableRow, list_table};
pub use watch::{WatchBatch, WatchEvent, watch_collection};
