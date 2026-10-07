//! Domain types for freshkube
//!
//! These types represent the core domain model for Talos clusters.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// Node role in the cluster
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum NodeRole {
    ControlPlane,
    Worker,
    Unknown,
}

impl std::fmt::Display for NodeRole {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            NodeRole::ControlPlane => write!(f, "CP"),
            NodeRole::Worker => write!(f, "Worker"),
            NodeRole::Unknown => write!(f, "?"),
        }
    }
}

/// Service running on a node
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Service {
    pub id: String,
    pub state: ServiceState,
    pub health: ServiceHealth,
}

/// Service running state
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum ServiceState {
    Running,
    Starting,
    Stopping,
    Stopped,
    Failed,
    Unknown,
}

impl std::fmt::Display for ServiceState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ServiceState::Running => write!(f, "Running"),
            ServiceState::Starting => write!(f, "Starting"),
            ServiceState::Stopping => write!(f, "Stopping"),
            ServiceState::Stopped => write!(f, "Stopped"),
            ServiceState::Failed => write!(f, "Failed"),
            ServiceState::Unknown => write!(f, "Unknown"),
        }
    }
}

/// Service health information
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServiceHealth {
    pub healthy: bool,
    pub last_check: Option<DateTime<Utc>>,
    pub message: Option<String>,
}

/// Overall cluster health
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ClusterHealth {
    Healthy,
    Degraded {
        unhealthy_nodes: usize,
        total_nodes: usize,
    },
    Critical {
        reason: String,
    },
    Unknown,
}

impl ClusterHealth {
    pub fn symbol(&self) -> &'static str {
        match self {
            ClusterHealth::Healthy => "●",
            ClusterHealth::Degraded { .. } => "◐",
            ClusterHealth::Critical { .. } => "○",
            ClusterHealth::Unknown => "?",
        }
    }

    pub fn label(&self) -> String {
        match self {
            ClusterHealth::Healthy => "Healthy".to_string(),
            ClusterHealth::Degraded {
                unhealthy_nodes,
                total_nodes,
            } => format!("Degraded ({}/{})", unhealthy_nodes, total_nodes),
            ClusterHealth::Critical { reason } => format!("Critical: {}", reason),
            ClusterHealth::Unknown => "Unknown".to_string(),
        }
    }
}

/// Log line from Talos
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LogLine {
    pub timestamp: DateTime<Utc>,
    pub node: String,
    pub service: String,
    pub level: LogLevel,
    pub message: String,
}

/// Log level
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum LogLevel {
    Debug,
    Info,
    Warning,
    Error,
    Unknown,
}

impl std::fmt::Display for LogLevel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LogLevel::Debug => write!(f, "DEBUG"),
            LogLevel::Info => write!(f, "INFO"),
            LogLevel::Warning => write!(f, "WARN"),
            LogLevel::Error => write!(f, "ERROR"),
            LogLevel::Unknown => write!(f, "???"),
        }
    }
}
