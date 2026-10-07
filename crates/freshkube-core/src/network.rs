//! Network analysis utilities
//!
//! Provides port-to-service mapping and network connection analysis
//! for Talos Linux and Kubernetes clusters.

/// Get service name for a port
///
/// Returns the service name if the port is a well-known Talos/K8s port.
///
/// # Examples
///
/// ```
/// use freshkube_core::network::port_to_service;
///
/// assert_eq!(port_to_service(6443), Some("kube-apiserver"));
/// assert_eq!(port_to_service(50000), Some("apid"));
/// assert_eq!(port_to_service(12345), None);
/// ```
pub fn port_to_service(port: u16) -> Option<&'static str> {
    Some(match port {
        // Talos services
        50000 => "apid",
        50001 => "trustd",
        51821 => "kubernetesd",
        // etcd
        2379 => "etcd-client",
        2380 => "etcd-peer",
        // Kubernetes control plane
        6443 => "kube-apiserver",
        10250 => "kubelet",
        10259 => "kube-scheduler",
        10257 => "kube-controller-manager",
        // Kubernetes networking
        10256 => "kube-proxy",
        8472 => "flannel-vxlan",
        4240 => "cilium-health",
        4244 => "cilium-hubble",
        // Common services
        53 => "dns",
        443 => "https",
        80 => "http",
        _ => return None,
    })
}

/// Get service name for a u32 port (for compatibility)
pub fn port_to_service_u32(port: u32) -> Option<&'static str> {
    if port > u16::MAX as u32 {
        return None;
    }
    port_to_service(port as u16)
}

/// Connection direction
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConnectionDirection {
    /// Incoming connection (we're listening)
    Inbound,
    /// Outgoing connection (we initiated)
    Outbound,
    /// Unknown direction
    Unknown,
}

/// Classify a connection by its ports
pub fn classify_connection(local_port: u16, remote_port: u16) -> ConnectionDirection {
    let local_known = port_to_service(local_port).is_some();
    let remote_known = port_to_service(remote_port).is_some();

    if local_known && !remote_known {
        ConnectionDirection::Inbound
    } else if !local_known && remote_known {
        ConnectionDirection::Outbound
    } else {
        ConnectionDirection::Unknown
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_port_to_service() {
        assert_eq!(port_to_service(6443), Some("kube-apiserver"));
        assert_eq!(port_to_service(50000), Some("apid"));
        assert_eq!(port_to_service(2379), Some("etcd-client"));
        assert_eq!(port_to_service(12345), None);
    }

    #[test]
    fn test_port_to_service_u32() {
        assert_eq!(port_to_service_u32(6443), Some("kube-apiserver"));
        assert_eq!(port_to_service_u32(70000), None); // Out of u16 range
    }

    #[test]
    fn test_classify_connection() {
        // Local is kube-apiserver, remote is ephemeral -> inbound
        assert_eq!(
            classify_connection(6443, 54321),
            ConnectionDirection::Inbound
        );

        // Local is ephemeral, remote is kube-apiserver -> outbound
        assert_eq!(
            classify_connection(54321, 6443),
            ConnectionDirection::Outbound
        );

        // Both unknown -> unknown
        assert_eq!(
            classify_connection(54321, 54322),
            ConnectionDirection::Unknown
        );
    }
}
