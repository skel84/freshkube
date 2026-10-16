//! A kubeconfig context reached directly, for Kubernetes-only mode. The
//! type lives in core's `cluster_source`; this keeps its old path.

pub use freshkube_core::cluster_source::DirectAccess;
