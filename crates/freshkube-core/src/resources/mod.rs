//! Read-only Kubernetes resource browsing for any kind.
//!
//! Collections are listed and watched through the API server's table format
//! (`meta.k8s.io/v1` `Table`), which carries the columns `kubectl get` prints
//! for every kind, custom resources included. Clients come either from a
//! kubeconfig context ([`connect`]) or from the Talos side of the app.
//! Nothing here changes the cluster.

mod connection;
mod contexts;
mod failure;
mod kinds;
mod table;
mod watch;

pub use connection::{Connection, connect};
pub use contexts::{KubeContext, KubeconfigReport, discover_contexts, kubeconfig_sources};
pub use failure::{Failure, FailureKind};
pub use kinds::{ResourceKind, builtin};
pub use table::{RowMetadata, Table, TableColumn, TableRow, get_object_yaml, list_table};
pub use watch::{WatchBatch, WatchEvent, watch_collection};
