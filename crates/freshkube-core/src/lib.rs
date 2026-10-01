//! freshkube-core: Core business logic for Freshkube
//!
//! This crate contains domain types, shared utilities, and business logic
//! for Freshkube. It is intentionally kept independent of any UI framework
//! to enable:
//!
//! - Unit testing without UI dependencies
//! - Reuse in CLI tools or other consumers
//! - Clear separation between business logic and presentation
//!
//! # Modules
//!
//! - [`types`] - Core domain types (Cluster, Node, Service, etc.)
//! - [`indicators`] - Health and status indicators for consistent UI representation
//! - [`formatting`] - Utilities for formatting bytes, durations, percentages, etc.
//! - [`selection`] - Generic selection logic for list-based UI components
//! - [`async_state`] - Async component state management (loading, error, refresh)
//! - [`errors`] - Error formatting utilities for user-friendly messages
//! - [`network`] - Network analysis utilities (port mapping, connection classification)
//! - [`diagnostics`] - Diagnostic types for health checks and CNI detection
//! - [`constants`] - Shared constants (thresholds, CRD names, refresh intervals)

pub mod async_state;
mod client_cache;
pub mod cluster_overview;

pub mod constants;
pub mod diagnostic_runner;
pub mod diagnostics;
pub mod errors;
pub mod formatting;
pub mod indicators;
pub mod inspection;
mod kubeconfig_selection;
pub mod logs;
pub mod maintenance;
pub mod network;
pub mod operations;
pub mod pcap;
pub mod security_lifecycle;
pub mod selection;
pub mod types;
pub mod workloads;

// Re-export commonly used items at crate root
pub use async_state::*;
pub use cluster_overview::*;
pub use diagnostics::*;
pub use errors::*;
pub use formatting::*;
pub use indicators::*;
pub use selection::*;
pub use types::*;

// Network is not re-exported at root to avoid name conflicts
// Use freshkube_core::network::* explicitly
