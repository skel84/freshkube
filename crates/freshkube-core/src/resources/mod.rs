//! Read-only Kubernetes resource browsing for any kind.
//!
//! Collections are listed and watched through the API server's table format
//! (`meta.k8s.io/v1` `Table`), which carries the columns `kubectl get` prints
//! for every kind, custom resources included. Clients come either from a
//! kubeconfig context ([`connect`]) or from the Talos side of the app. One
//! object is read in full on demand ([`get_object`]), with its events
//! ([`watch_object_events`]). Custom kinds come from discovery
//! ([`list_custom_groups`], [`list_group_kinds`]). A pod's containers come with its
//! overview, and their logs from [`follow_pod_log`].
//! Nothing here changes the cluster, except [`start_exec`], which runs a
//! shell in a container only when the user starts one, and
//! [`start_forward`], which forwards a pod's port only when the user asks.

mod connection;
mod contexts;
mod discovery;
mod events;
mod exec;
mod failure;
mod forward;
mod kinds;
mod object;
mod pod_logs;
mod table;
mod watch;

pub use connection::{Connection, connect};
pub use contexts::{KubeContext, KubeconfigReport, discover_contexts, kubeconfig_sources};
pub use discovery::{ApiGroup, GroupKinds, list_custom_groups, list_group_kinds};
pub use events::{EventScope, EventUpdate, ObjectEvent, watch_object_events};
pub use exec::{
    BATCH_BYTES, BATCH_TIME, ExecEnd, ExecFailure, ExecFailureKind, ExecGuard, ExecInput,
    ExecOutput, ExecRequest, ExecSession, ExecSize, SHELL, start_exec,
};
pub use failure::{Failure, FailureKind};
pub use forward::{
    DeclaredPort, Forward, ForwardEnd, ForwardFailure, ForwardFailureKind, ForwardGuard,
    ForwardRequest, ForwardState, ForwardStatus, ForwardTarget, Listeners, POD_WAIT, PodWatches,
    WorkloadKind, candidates, declared_ports, listen_local, preferred_port, start_forward,
};
pub use kinds::{ResourceKind, builtin};
pub use object::{
    Condition, ObjectDocument, Overview, Owner, SecretKey, SecretSummary, SecretValue, get_object,
    hidden_value, object_from_yaml, reveal_secret_value,
};
pub use pod_logs::{
    Container, ContainerRole, ContainerState, LogPosition, LogRequest, MAX_ATTEMPTS,
    MAX_LINE_BYTES, PodContainers, PodLogUpdate, Termination, follow_pod_log, pod_containers,
};
pub use table::{RowMetadata, Table, TableColumn, TableRow, list_table};
pub use watch::{WatchBatch, WatchEvent, watch_collection};
