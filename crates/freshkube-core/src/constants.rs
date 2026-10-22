//! Shared constants for Talos and Kubernetes
//!
//! This module contains domain-specific constants like thresholds,
//! CRD names for addon detection, and other shared values.

/// Threshold for considering a pod to have "high" restarts
pub const HIGH_RESTART_THRESHOLD: i32 = 5;

/// Upper bound for one screen request, including Kubernetes client setup.
pub const SCREEN_DEADLINE: std::time::Duration = std::time::Duration::from_secs(60);

/// The salt of a deep link's cluster fingerprint (#873). Public on purpose:
/// it only separates this use of a hash from any other, so a link is never a
/// raw namespace UID. A per-install salt would stop two people matching.
pub const FINGERPRINT_SALT: &str = "freshkube/deep-link/v1";

/// Maximum log entries to retain in memory
pub const MAX_LOG_ENTRIES: usize = 5000;

/// Maximum packet capture size (40 MB)
pub const MAX_CAPTURE_SIZE: usize = 40 * 1024 * 1024;

/// Node memory use (percent of total) at which memory is shown as high and a
/// node's health warns
pub const MEMORY_WARNING_PERCENT: f64 = 85.0;

/// Node memory use (percent of total) at which memory is shown as critical and
/// a node's health fails
pub const MEMORY_CRITICAL_PERCENT: f64 = 95.0;

// =============================================================================
// Addon Detection CRDs
// =============================================================================

/// cert-manager CRDs for detection
pub const CERT_MANAGER_CRDS: &[&str] = &[
    "certificates.cert-manager.io",
    "issuers.cert-manager.io",
    "clusterissuers.cert-manager.io",
];

/// external-secrets CRDs for detection
pub const EXTERNAL_SECRETS_CRDS: &[&str] = &[
    "externalsecrets.external-secrets.io",
    "secretstores.external-secrets.io",
];

/// Kyverno CRDs for detection
pub const KYVERNO_CRDS: &[&str] = &["clusterpolicies.kyverno.io", "policies.kyverno.io"];

/// ArgoCD CRDs for detection
pub const ARGOCD_CRDS: &[&str] = &["applications.argoproj.io", "appprojects.argoproj.io"];

/// Flux CRDs for detection
pub const FLUX_CRDS: &[&str] = &[
    "kustomizations.kustomize.toolkit.fluxcd.io",
    "gitrepositories.source.toolkit.fluxcd.io",
];

// =============================================================================
// Default Refresh Intervals
// =============================================================================

/// Default refresh intervals for components (in seconds)
pub mod refresh_intervals {
    /// Normal refresh for moderately changing data (processes, etcd)
    pub const NORMAL: u64 = 5;
}

/// What an agent's reads may ask for and be answered with
/// (`agent_reads`, docs/features/agent.md). Asked of the server where the API
/// allows, and checked again in core on every answer.
pub mod agent {
    use std::time::Duration;

    /// Items in one list, or events in one answer.
    pub const MAX_ITEMS: usize = 200;
    /// Items when the agent names no limit.
    pub const DEFAULT_ITEMS: usize = 100;
    /// Log lines in one answer.
    pub const MAX_LOG_LINES: usize = 500;
    /// Log lines when the agent names no count.
    pub const DEFAULT_LOG_LINES: usize = 200;
    /// Log bytes in one answer, and the `limitBytes` asked of the server.
    pub const MAX_LOG_BYTES: usize = 64 * 1024;
    /// The farthest back a log or an event may reach, in seconds.
    pub const MAX_SINCE_SECONDS: u64 = 86_400;
    /// How far back events reach when the agent names no age.
    pub const DEFAULT_EVENT_SINCE_SECONDS: u64 = 3_600;
    /// A log line is cut here.
    pub const MAX_LINE_BYTES: usize = 2 * 1024;
    /// An event message is cut here.
    pub const MAX_EVENT_MESSAGE_BYTES: usize = 1024;
    /// One object, as JSON, once its noise is removed.
    pub const MAX_OBJECT_BYTES: usize = 64 * 1024;
    /// Any answer, as JSON.
    pub const MAX_ANSWER_BYTES: usize = 128 * 1024;
    /// One read, from the gate to the answer.
    pub const READ_DEADLINE: Duration = Duration::from_secs(20);
    /// Reads one session may make.
    pub const MAX_READS_PER_SESSION: u32 = 500;
    /// A cluster entry id, as the workspace bounds it.
    pub const MAX_ENTRY_ID_BYTES: usize = 64;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_thresholds() {
        const { assert!(HIGH_RESTART_THRESHOLD > 0) };
        const { assert!(MAX_LOG_ENTRIES > 0) };
        const { assert!(MAX_CAPTURE_SIZE > 0) };
    }

    #[test]
    fn test_crd_lists_not_empty() {
        assert!(!CERT_MANAGER_CRDS.is_empty());
        assert!(!EXTERNAL_SECRETS_CRDS.is_empty());
        assert!(!KYVERNO_CRDS.is_empty());
        assert!(!ARGOCD_CRDS.is_empty());
        assert!(!FLUX_CRDS.is_empty());
    }
}
