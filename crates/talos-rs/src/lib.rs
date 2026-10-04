//! talos-rs: Rust SDK for Talos Linux API
//!
//! This crate provides a high-level client for interacting with Talos Linux
//! clusters via their gRPC API.
//!
//! # Example
//!
//! ```no_run
//! use talos_rs::TalosClient;
//!
//! #[tokio::main]
//! async fn main() -> Result<(), Box<dyn std::error::Error>> {
//!     // Connect using default talosconfig
//!     let client = TalosClient::from_default_config().await?;
//!
//!     // Get version info
//!     let versions = client.version().await?;
//!     for v in versions {
//!         println!("{}: {}", v.node, v.version);
//!     }
//!
//!     Ok(())
//! }
//! ```

// TalosError contains tonic types which are large, but this is fine for an API client
// where errors are on the cold path
#![allow(clippy::result_large_err)]

pub mod auth;
pub mod client;
pub mod config;
pub mod error;
mod log_stream;
pub mod talosctl;

/// Generated protobuf types and gRPC clients
pub mod proto {
    pub mod google {
        pub mod rpc {
            tonic::include_proto!("google.rpc");
        }
    }

    pub mod common {
        tonic::include_proto!("common");
    }

    pub mod machine {
        tonic::include_proto!("machine");
    }

    pub mod storage {
        tonic::include_proto!("storage");
    }

    pub mod time {
        tonic::include_proto!("time");
    }

    pub mod inspect {
        tonic::include_proto!("inspect");
    }
}

pub use client::{
    // Configuration types
    ApplyConfigResult,
    ApplyMode,
    // Connection types
    ConnectionCounts,
    ConnectionInfo,
    ConnectionState,
    // Node info types
    CpuStat,
    // Etcd types
    EtcdAlarm,
    EtcdAlarmType,
    EtcdMemberInfo,
    EtcdMemberStatus,
    MemInfo,
    // Network types
    NetDevRate,
    NetDevStats,
    NetstatFilter,
    NodeConnections,
    NodeCpuInfo,
    NodeLoadAvg,
    NodeMemory,
    NodeNetworkStats,
    // Process types
    NodeProcesses,
    NodeServices,
    NodeSystemStat,
    // Time types
    NodeTimeInfo,
    ProcessInfo,
    ProcessState,
    // Operation types
    RebootMode,
    RebootResult,
    ServiceHealth,
    ServiceInfo,
    ServiceRestartResult,
    ShutdownResult,
    TalosClient,
    VersionInfo,
};
pub use config::{Context, TalosConfig};
pub use error::TalosError;
pub use talosctl::{
    AddressStatus, DiscoveryMember, DiskInfo, GenConfigResult, InsecureApplyResult,
    InsecureVersionInfo, KubeSpanPeerStatus, MachineConfigInfo, VolumeStatus,
    apply_config_insecure, check_insecure_connection, gen_config, gen_config_with_install_disk,
    get_address_status, get_discovery_members, get_discovery_members_for_context,
    get_discovery_members_with_retry, get_disks, get_disks_for_context, get_disks_for_node,
    get_disks_insecure, get_kubespan_peers, get_kubespan_peers_for_node, get_machine_config,
    get_version_insecure, get_volume_status, get_volume_status_for_node,
    get_volume_status_insecure, is_kubespan_enabled, is_kubespan_enabled_for_node, reboot_insecure,
    shutdown_insecure,
};
