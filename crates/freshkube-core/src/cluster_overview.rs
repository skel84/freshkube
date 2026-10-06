//! Framework-free cluster overview collection.
//!
//! The collector keeps connection failures, partial data, and non-fatal warnings
//! separate so callers can render useful cluster state without treating an
//! unavailable optional source (such as Talos discovery) as a cluster failure.

use std::{
    collections::HashMap,
    future::Future,
    path::PathBuf,
    sync::{Arc, Mutex},
    time::Duration,
};

use k8s_openapi::api::core::v1::Node;
use kube::{
    Client, Config,
    api::{Api, ListParams},
};
use talos_rs::{
    DiscoveryMember, EtcdMemberInfo, EtcdMemberStatus, NodeCpuInfo, NodeLoadAvg, NodeMemory,
    NodeServices, TalosClient, TalosConfig, TalosError, VersionInfo,
    get_discovery_members_with_retry,
};

pub use crate::client_cache::ConfigIdentity;
use crate::client_cache::{self, KubernetesEntry, KubernetesKey, TalosKey};

pub use crate::kubeconfig_selection::{
    KubeconfigFileInfo, KubeconfigSelection, inspect_kubeconfig,
};
use crate::kubeconfig_selection::{
    PreparedKubeconfig, attempt_with_pinned_retry, parse_pinned_kubeconfig, prepare_kubeconfig,
};

/// Connection state for one configured Talos context.
///
/// Warnings on [`ClusterOverview`] are intentionally independent of this state:
/// a context can be connected and contain only a partial node roster when Talos
/// discovery is unavailable.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum ClusterConnectionStatus {
    /// No connection attempt has completed yet.
    #[default]
    Disconnected,
    /// At least one Talos endpoint has provided usable cluster data.
    Connected,
    /// No configured endpoint could be reached and no prior data remains.
    Unreachable(String),
}

impl ClusterConnectionStatus {
    /// Whether the context has usable Talos connectivity.
    pub fn is_connected(&self) -> bool {
        matches!(self, Self::Connected)
    }

    /// The connection failure suitable for display, if one exists.
    pub fn error(&self) -> Option<&str> {
        match self {
            Self::Unreachable(message) => Some(message),
            Self::Disconnected | Self::Connected => None,
        }
    }
}

/// Compact etcd state for a cluster overview header.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct EtcdSummary {
    /// Number of voting members responding to the status request.
    pub healthy: usize,
    /// Number of configured voting members; learners do not contribute to quorum.
    pub total: usize,
    /// Whether the responding members meet the etcd quorum requirement.
    pub has_quorum: bool,
}

impl EtcdSummary {
    fn from_statuses(members: &[EtcdMemberInfo], statuses: &[EtcdMemberStatus]) -> Self {
        let voters = members.iter().filter(|member| !member.is_learner);
        let total = voters.clone().count();
        let healthy = voters
            .filter(|member| {
                statuses
                    .iter()
                    .any(|status| status.member_id == member.id && !status.is_learner)
            })
            .count();
        Self {
            healthy,
            total,
            has_quorum: crate::indicators::quorum(healthy, total).state.has_quorum(),
        }
    }
}

/// A UI-framework-free snapshot of one Talos context.
///
/// `client` remains available to a frontend for node-targeted detail views.
/// Collection warnings never change [`connection`](Self::connection) to an
/// error: they describe incomplete optional data while preserving all data that
/// was collected successfully.
#[derive(Clone, Default)]
pub struct ClusterOverview {
    /// Context / cluster name from talosconfig.
    pub name: String,
    /// Base client for this context. Consumers may derive node clients from it.
    pub client: Option<TalosClient>,
    /// Reachability of the Talos context.
    pub connection: ClusterConnectionStatus,
    /// Configured endpoints, retained for error display and endpoint fallback.
    pub endpoints: Vec<String>,
    /// Version information from queried nodes.
    pub versions: Vec<VersionInfo>,
    /// Service information from queried nodes.
    pub services: Vec<NodeServices>,
    /// Memory information from queried nodes.
    pub memory: Vec<NodeMemory>,
    /// Load average information from queried nodes.
    pub load_avg: Vec<NodeLoadAvg>,
    /// CPU information from queried nodes.
    pub cpu_info: Vec<NodeCpuInfo>,
    /// etcd members, which are necessarily control plane nodes.
    pub etcd_members: Vec<EtcdMemberInfo>,
    /// Talos discovery roster, including workers when available.
    pub discovery_members: Vec<DiscoveryMember>,
    /// etcd quorum state for the cluster header.
    pub etcd_summary: Option<EtcdSummary>,
    /// Alarms from the overview cycle; an unavailable answer stays unknown.
    pub etcd_alarms: Option<Vec<talos_rs::EtcdAlarm>>,
    /// Hostname-to-IP map used for node-targeted detail operations.
    pub node_ips: HashMap<String, String>,
    /// Non-fatal warning when the worker roster could not be discovered.
    pub discovery_warning: Option<String>,
    /// Non-fatal warning when a requested kubeconfig could not safely be used.
    pub kubeconfig_warning: Option<String>,
    /// Effective Kubernetes roster source and context, refreshed with discovery.
    pub kubeconfig_source: Option<String>,
}

impl ClusterOverview {
    /// Best control plane IP for cluster-correct Kubernetes operations.
    ///
    /// An etcd address is unambiguously a control plane node, so it wins over a
    /// discovery role. Configured endpoint fallback preserves the old behavior
    /// when no roster source is available.
    pub fn control_plane_ip(&self) -> Option<String> {
        self.etcd_members
            .iter()
            .find_map(|member| member.ip_address())
            .or_else(|| {
                self.discovery_members
                    .iter()
                    .find(|member| member.machine_type == "controlplane")
                    .and_then(|member| member.addresses.first().cloned())
            })
            .or_else(|| {
                self.endpoints
                    .first()
                    .map(|endpoint| talos_rs::target_host(endpoint).to_string())
            })
    }

    /// Whether a node is a control plane node.
    ///
    /// Talos discovery's machine type is authoritative when it describes the
    /// node. Without discovery, etcd service presence is the safe fallback;
    /// every remaining node is therefore a worker and remains visible.
    pub fn node_is_controlplane(&self, node: &str) -> bool {
        let key = talos_rs::target_host(node);
        if let Some(member) = self.discovery_members.iter().find(|member| {
            member.hostname.eq_ignore_ascii_case(node)
                || member.hostname.eq_ignore_ascii_case(key)
                || member
                    .addresses
                    .iter()
                    .any(|address| address == node || address == key)
        }) {
            return member.machine_type.eq_ignore_ascii_case("controlplane");
        }

        self.services
            .iter()
            .find(|services| services.node == node || (services.node.is_empty() && node.is_empty()))
            .is_some_and(|services| services.services.iter().any(|service| service.id == "etcd"))
    }
}

/// Errors that prevent loading the configured contexts altogether.
///
/// Per-context connection failures are represented inside [`ClusterOverview`]
/// so the remaining contexts continue to be collected and displayed.
#[derive(Debug, thiserror::Error)]
pub enum ClusterOverviewError {
    /// Talos configuration could not be loaded.
    #[error("failed to load talosconfig{path}: {message}")]
    ConfigLoad { path: String, message: String },
    /// A requested context is not present in the loaded configuration.
    #[error("context '{name}' not found in talosconfig; available contexts: {available:?}")]
    ContextNotFound {
        name: String,
        available: Vec<String>,
    },
}

/// Most nodes queried at once during an overview refresh.
const NODE_CONCURRENCY: usize = 8;
/// Longest any single node request may take before that request is abandoned.
const NODE_CALL_TIMEOUT: Duration = Duration::from_secs(20);

/// Talos connection keys by context for the collector's cached connections.
/// Keys hold no credentials, but the type stays opaque on principle.
#[derive(Clone, Default)]
struct ConnectionKeys(Arc<Mutex<HashMap<String, TalosKey>>>);

impl std::fmt::Debug for ConnectionKeys {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ConnectionKeys")
            .finish_non_exhaustive()
    }
}

impl ConnectionKeys {
    fn set(&self, context: &str, key: TalosKey) {
        self.0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .insert(context.to_owned(), key);
    }

    fn get(&self, context: &str) -> Option<TalosKey> {
        self.0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .get(context)
            .cloned()
    }
}

/// Loads and refreshes framework-free cluster overview snapshots.
#[derive(Debug, Clone, Default)]
pub struct ClusterOverviewCollector {
    config_path: Option<PathBuf>,
    context_filter: Option<String>,
    kubeconfig_selection: KubeconfigSelection,
    connection_keys: ConnectionKeys,
    observed_nodes:
        Option<crate::kubernetes_summary::Part<Vec<crate::kubernetes_summary::NodeSummary>>>,
}

impl ClusterOverviewCollector {
    /// Creates a collector using an optional talosconfig path and context name.
    pub fn new(config_path: Option<PathBuf>, context_filter: Option<String>) -> Self {
        Self {
            config_path,
            context_filter,
            kubeconfig_selection: KubeconfigSelection::Automatic,
            connection_keys: ConnectionKeys::default(),
            observed_nodes: None,
        }
    }

    /// Once a desktop observation session exists, roster fallback consumes its
    /// evidence, including failures, instead of making a hidden Node list.
    pub fn set_observed_nodes(
        &mut self,
        nodes: Option<crate::kubernetes_summary::Part<Vec<crate::kubernetes_summary::NodeSummary>>>,
    ) {
        self.observed_nodes = nodes;
    }

    /// Replaces the talosconfig path used by subsequent connections.
    pub fn set_config_path(&mut self, config_path: Option<PathBuf>) {
        self.config_path = config_path;
    }

    /// Selects the session-local kubeconfig source for subsequent refreshes.
    ///
    /// File contexts are validated against the pinned Talos control plane before
    /// any selected credentials or authentication plugins can be used.
    pub fn set_kubeconfig_selection(&mut self, selection: KubeconfigSelection) {
        self.kubeconfig_selection = selection;
    }

    /// Creates an identity-validated client for the selected cluster and session
    /// kubeconfig selection. No ambient client is ever used as a fallback.
    pub async fn kubernetes_client(&self, cluster: &ClusterOverview) -> Result<Client, K8sError> {
        self.kubernetes_client_with_source(cluster)
            .await
            .map(|(client, _, _)| client)
    }

    /// As [`Self::kubernetes_client`], including the effective source after a
    /// selected-file rejection or credential-setup failure.
    pub async fn kubernetes_client_with_source(
        &self,
        cluster: &ClusterOverview,
    ) -> Result<(Client, String, Option<String>), K8sError> {
        let client = cluster.client.as_ref().ok_or_else(|| {
            K8sError::KubeconfigFetch("The selected Talos context is disconnected".into())
        })?;
        // A hit is only for the identical context, connection, pinned node,
        // selection and selected-file identity; creation below always validates.
        let key = self.kubernetes_key(cluster, client);
        if let Some(entry) = client_cache::kubernetes_entry(&key) {
            return Ok((entry.client, entry.source, entry.warning));
        }
        let mut prepared = tokio::time::timeout(
            std::time::Duration::from_secs(12),
            self.prepare_roster_kubeconfig(cluster, client),
        )
        .await
        .map_err(|_| K8sError::KubeconfigFetch("Pinned kubeconfig preparation timed out".into()))?;
        let created = tokio::time::timeout(
            std::time::Duration::from_secs(22),
            attempt_with_pinned_retry(
                &mut prepared,
                std::time::Duration::from_secs(10),
                client_from_roster_config,
            ),
        )
        .await
        .map_err(|_| K8sError::ClientCreate("Validated Kubernetes setup timed out".into()))??;
        client_cache::store_kubernetes_entry(
            key,
            KubernetesEntry {
                client: created.clone(),
                source: prepared.source.clone(),
                warning: prepared.warning.clone(),
            },
        );
        Ok((created, prepared.source, prepared.warning))
    }

    /// Forgets the reused Kubernetes client and prepared kubeconfig for this
    /// cluster. Call it when a Kubernetes request made with that client fails,
    /// so the next request revalidates and rebuilds.
    pub fn forget_kubernetes_client(&self, cluster: &ClusterOverview) {
        if let Some(client) = &cluster.client {
            client_cache::forget_kubernetes(client, &cluster.name);
        }
    }

    /// Forgets every reused Kubernetes client, e.g. after a settings change.
    pub fn forget_all_kubernetes_clients() {
        client_cache::forget_all_kubernetes();
    }

    fn kubernetes_key(&self, cluster: &ClusterOverview, client: &TalosClient) -> KubernetesKey {
        KubernetesKey::new(
            client,
            &cluster.name,
            cluster.control_plane_ip().as_deref(),
            &self.kubeconfig_selection,
        )
    }
    /// Loads configured contexts and establishes an independent client for each.
    ///
    /// A failed context becomes an [`ClusterConnectionStatus::Unreachable`]
    /// snapshot rather than preventing the other contexts from being shown.
    pub async fn connect(&self) -> Result<Vec<ClusterOverview>, ClusterOverviewError> {
        let config = self.load_config()?;
        self.connect_from_config(&config).await
    }

    /// Connects using an already-loaded configuration without reopening its path.
    ///
    /// Frontends that perform bounded file reads can keep the parsed configuration
    /// and connection identity pinned to the same inspected descriptor.
    pub async fn connect_from_config(
        &self,
        config: &TalosConfig,
    ) -> Result<Vec<ClusterOverview>, ClusterOverviewError> {
        self.connect_inner(config, None).await
    }

    /// As [`Self::connect_from_config`], reusing the Talos connection from an
    /// earlier call when nothing that determines it has changed.
    ///
    /// `identity` fingerprints the exact configuration bytes `config` was
    /// parsed from. The connection is keyed on that, the config path, the
    /// context, and its endpoints and target nodes; it is dropped when a
    /// refresh sees a transport-level failure. Endpoint selection is unchanged.
    pub async fn connect_from_config_reusing(
        &self,
        config: &TalosConfig,
        identity: ConfigIdentity,
    ) -> Result<Vec<ClusterOverview>, ClusterOverviewError> {
        self.connect_inner(config, Some(identity)).await
    }

    async fn connect_inner(
        &self,
        config: &TalosConfig,
        identity: Option<ConfigIdentity>,
    ) -> Result<Vec<ClusterOverview>, ClusterOverviewError> {
        let _ = rustls::crypto::ring::default_provider().install_default();
        let context_names = self.context_names(config)?;
        let mut clusters = Vec::with_capacity(context_names.len());

        for name in context_names {
            let mut cluster = ClusterOverview {
                name: name.clone(),
                ..Default::default()
            };

            match config.get_context(&name) {
                Ok(context) => {
                    cluster.endpoints = context.endpoints.clone();
                    let key = identity.map(|identity| TalosKey {
                        config_path: self.config_path.clone(),
                        identity,
                        context: name.clone(),
                        endpoints: context.endpoints.clone(),
                        nodes: context.target_nodes().to_vec(),
                    });
                    let reused = key.as_ref().and_then(client_cache::talos_client);
                    let connected = match reused {
                        Some(client) => Ok(client),
                        None => TalosClient::from_context(context).await,
                    };
                    match connected {
                        Ok(client) => {
                            if let Some(key) = key {
                                client_cache::store_talos_client(key.clone(), client.clone());
                                self.connection_keys.set(&name, key);
                            }
                            cluster.client = Some(client);
                            cluster.connection = ClusterConnectionStatus::Connected;
                        }
                        Err(error) => {
                            cluster.connection = ClusterConnectionStatus::Unreachable(format!(
                                "Could not connect — {}",
                                root_cause(&error)
                            ));
                        }
                    }
                }
                Err(error) => {
                    cluster.connection = ClusterConnectionStatus::Unreachable(error.to_string());
                }
            }

            clusters.push(cluster);
        }

        Ok(clusters)
    }

    /// Refreshes all overview data for one context, retaining useful data when
    /// an optional source is transiently unavailable.
    pub async fn refresh(&self, cluster: &mut ClusterOverview) {
        let Some(client) = cluster.client.clone() else {
            return;
        };

        let mut endpoint_failed = false;
        match client.etcd_members().await {
            Ok(members) => {
                replace_node_ips_from_etcd(cluster, &members);
                cluster.etcd_members = members;
            }
            Err(error) => {
                endpoint_failed = error.is_transport_failure();
                tracing::warn!(
                    "Failed to fetch etcd members for {}: {}",
                    cluster.name,
                    error
                );
                cluster.etcd_members.clear();
            }
        }

        let mut kubeconfig = None;
        let fallback_ips = cluster
            .etcd_members
            .iter()
            .filter_map(|member| member.ip_address())
            .collect::<Vec<_>>();
        let discovery_config_path = self.config_path.as_deref().and_then(|path| path.to_str());
        if self.config_path.is_some() && discovery_config_path.is_none() {
            cluster.discovery_warning = Some(
                "Discovery roster unavailable because talosctl does not support the selected non-UTF-8 talosconfig path."
                    .into(),
            );
            self.refresh_roster_from_kubernetes(cluster, &client, &mut kubeconfig)
                .await;
        } else {
            match get_discovery_members_with_retry(
                &cluster.name,
                discovery_config_path,
                &fallback_ips,
            )
            .await
            {
                Ok(members) if !members.is_empty() => {
                    replace_node_ips_from_discovery(cluster, &members);
                    cluster.discovery_members = members;
                    cluster.discovery_warning = None;
                }
                outcome => {
                    match &outcome {
                        Ok(_) => tracing::warn!(
                            "Discovery returned no members for {} (discovery service likely disabled)",
                            cluster.name
                        ),
                        Err(error) => tracing::warn!(
                            "Failed to fetch discovery members for {} after retries: {}",
                            cluster.name,
                            error
                        ),
                    }

                    // Preserve a known roster through transient discovery failures.
                    if cluster.discovery_members.is_empty()
                        || matches!(self.kubeconfig_selection, KubeconfigSelection::File { .. })
                    {
                        self.refresh_roster_from_kubernetes(cluster, &client, &mut kubeconfig)
                            .await;
                    }
                }
            }
        }

        // Resolve source/validation even when Talos discovery succeeded. Keep
        // the prepared config for roster fallback so Talos serves it only once.
        if kubeconfig.is_none() {
            let prepared = self.prepare_roster_kubeconfig(cluster, &client).await;
            cluster.kubeconfig_warning = prepared.warning.clone();
            cluster.kubeconfig_source = Some(prepared.source.clone());
        }

        let mut node_transport_failures = Vec::new();
        let nodes_to_query = nodes_to_query(cluster);
        if !nodes_to_query.is_empty() {
            let base = client.clone();
            let per_node = collect_bounded(nodes_to_query, NODE_CONCURRENCY, move |name, ip| {
                let node_client = base.with_node(&ip);
                async move { collect_node(node_client, name, NODE_CALL_TIMEOUT).await }
            })
            .await;

            let mut versions = Vec::new();
            let mut services = Vec::new();
            let mut memory = Vec::new();
            let mut load_avg = Vec::new();
            let mut cpu_info = Vec::new();
            for node in per_node {
                node_transport_failures.push(node.version_transport_failure);
                versions.extend(node.versions);
                services.extend(node.services);
                memory.extend(node.memory);
                load_avg.extend(node.load_avg);
                cpu_info.extend(node.cpu_info);
            }

            cluster.versions = versions;
            cluster.services = services;
            cluster.memory = memory;
            cluster.load_avg = load_avg;
            cluster.cpu_info = cpu_info;

            // Target IPs rather than hostnames: hostnames are not guaranteed to
            // resolve from the frontend host.
            let control_plane_ips = cluster
                .etcd_members
                .iter()
                .filter_map(|member| member.ip_address())
                .collect::<Vec<_>>();
            let (statuses, alarms) = tokio::join!(
                client.etcd_status_for_nodes(&control_plane_ips),
                tokio::time::timeout(NODE_CALL_TIMEOUT, client.etcd_alarms()),
            );
            cluster.etcd_alarms = alarms.ok().and_then(Result::ok);
            if let Ok(statuses) = statuses {
                cluster.etcd_summary =
                    Some(EtcdSummary::from_statuses(&cluster.etcd_members, &statuses));
            }
        }

        // Reconnect next time rather than keep reusing a dead channel.
        if client_cache::connection_unusable(endpoint_failed, &node_transport_failures)
            && let Some(key) = self.connection_keys.get(&cluster.name)
        {
            client_cache::forget_talos_client(&key);
        }

        let has_any_data = !cluster.versions.is_empty()
            || !cluster.etcd_members.is_empty()
            || !cluster.discovery_members.is_empty();
        if has_any_data {
            cluster.connection = ClusterConnectionStatus::Connected;
        } else {
            cluster.connection = ClusterConnectionStatus::Unreachable(
                "Unable to reach any configured Talos endpoint. Check network connectivity and the endpoints in your talosconfig."
                    .to_string(),
            );
        }
    }

    /// Refreshes the live metrics for one already-known node.
    ///
    /// This is intentionally narrower than [`Self::refresh`]: it avoids roster,
    /// etcd, and Kubernetes discovery calls during the five-second foreground
    /// refresh loop while preserving the latest data for every other node.
    pub async fn refresh_node(&self, cluster: &mut ClusterOverview, node_name: &str) {
        let Some(client) = cluster.client.clone() else {
            return;
        };
        let Some(node_ip) = cluster.node_ips.get(node_name).cloned() else {
            return;
        };
        let node_name = node_name.to_owned();
        let node_client = client.with_node(&node_ip);

        if let Ok(mut values) = node_client.services().await {
            for value in &mut values {
                value.node.clone_from(&node_name);
            }
            cluster.services.retain(|value| value.node != node_name);
            cluster.services.extend(values);
        }

        if let Ok(mut values) = node_client.memory().await {
            for value in &mut values {
                value.node.clone_from(&node_name);
            }
            cluster.memory.retain(|value| value.node != node_name);
            cluster.memory.extend(values);
        }

        if let Ok(mut values) = node_client.load_avg().await {
            for value in &mut values {
                value.node.clone_from(&node_name);
            }
            cluster.load_avg.retain(|value| value.node != node_name);
            cluster.load_avg.extend(values);
        }
    }

    fn load_config(&self) -> Result<TalosConfig, ClusterOverviewError> {
        match &self.config_path {
            Some(path) => {
                TalosConfig::load_from(path).map_err(|error| ClusterOverviewError::ConfigLoad {
                    path: format!(" from {}", path.display()),
                    message: error.to_string(),
                })
            }
            None => TalosConfig::load_default().map_err(|error| ClusterOverviewError::ConfigLoad {
                path: String::new(),
                message: error.to_string(),
            }),
        }
    }

    fn context_names(&self, config: &TalosConfig) -> Result<Vec<String>, ClusterOverviewError> {
        match &self.context_filter {
            Some(name) if config.contexts.contains_key(name) => Ok(vec![name.clone()]),
            Some(name) => Err(ClusterOverviewError::ContextNotFound {
                name: name.clone(),
                available: config.contexts.keys().cloned().collect(),
            }),
            None => Ok(config.contexts.keys().cloned().collect()),
        }
    }

    async fn prepare_roster_kubeconfig(
        &self,
        cluster: &ClusterOverview,
        client: &TalosClient,
    ) -> PreparedKubeconfig {
        let node_ip = cluster.control_plane_ip();
        let key = self.kubernetes_key(cluster, client);
        if let Some(prepared) = client_cache::prepared_kubeconfig(&key) {
            return prepared;
        }
        let pinned = match node_ip.as_deref() {
            Some(ip) => client.with_node(ip).kubeconfig().await.ok(),
            None => None,
        };
        let selection = self.kubeconfig_selection.clone();
        let prepared = tokio::task::spawn_blocking(move || {
            let pinned = pinned.as_deref().and_then(parse_pinned_kubeconfig);
            prepare_kubeconfig(&selection, pinned, node_ip.as_deref(), || {
                kube::config::Kubeconfig::read().ok()
            })
        })
        .await
        .unwrap_or_else(|_| PreparedKubeconfig::unavailable(
            "Kubeconfig validation worker failed; unverified Kubernetes credentials were not used.",
        ));
        client_cache::store_prepared_kubeconfig(key, &prepared);
        prepared
    }

    async fn refresh_roster_from_kubernetes(
        &self,
        cluster: &mut ClusterOverview,
        client: &TalosClient,
        prepared: &mut Option<PreparedKubeconfig>,
    ) {
        if let Some(observed) = &self.observed_nodes {
            if let Some(nodes) = observed.loaded() {
                let members = k8s_nodes_to_discovery_members(
                    nodes
                        .iter()
                        .map(|node| K8sNodeInfo {
                            name: node.name.clone(),
                            internal_ip: node
                                .addresses
                                .iter()
                                .find(|(kind, _)| kind == "InternalIP")
                                .map(|(_, address)| address.clone()),
                            is_control_plane: node
                                .roles
                                .iter()
                                .any(|role| matches!(role.as_str(), "control-plane" | "master")),
                        })
                        .collect(),
                );
                replace_node_ips_from_discovery(cluster, &members);
                cluster.discovery_members = members;
            }
            cluster.discovery_warning = observed
                .error()
                .map(|reason| format!("Shared Kubernetes roster: {reason}"));
            return;
        }
        let control_plane_ip = cluster.control_plane_ip();
        match control_plane_ip {
            Some(_) => {
                if prepared.is_none() {
                    *prepared = Some(self.prepare_roster_kubeconfig(cluster, client).await);
                }
                let prepared = prepared.as_mut().expect("prepared roster config");
                let members = self.discovery_members_with_selection(prepared).await;
                cluster.kubeconfig_warning = prepared.warning.clone();
                cluster.kubeconfig_source = Some(prepared.source.clone());
                match members {
                    Ok(members) if !members.is_empty() => {
                        replace_node_ips_from_discovery(cluster, &members);
                        let count = members.len();
                        cluster.discovery_members = members;
                        cluster.discovery_warning = None;
                        tracing::info!(
                            "Enumerated {} node(s) via the Kubernetes API for {} (Talos discovery unavailable)",
                            count,
                            cluster.name
                        );
                    }
                    Ok(_) => {
                        cluster.discovery_warning = Some(
                        "Worker nodes unavailable: Talos discovery returned no members and the Kubernetes API reported no nodes."
                            .to_string(),
                    );
                    }
                    Err(error) => {
                        tracing::warn!(
                            "Kubernetes node enumeration failed for {}: {}",
                            cluster.name,
                            error
                        );
                        cluster.discovery_warning = Some(format!(
                            "Worker nodes may be missing: Talos discovery is unavailable and the Kubernetes API could not be reached ({error})."
                        ));
                    }
                }
            }
            None => {
                let prepared = PreparedKubeconfig::unavailable(
                    "Kubernetes roster requests are unavailable: no pinned Talos control plane is known.",
                );
                cluster.kubeconfig_warning = prepared.warning;
                cluster.kubeconfig_source = Some(prepared.source);
                cluster.discovery_warning = Some(
                    "Worker nodes unavailable: cluster discovery returned no members (the discovery service may be disabled)."
                        .to_string(),
                );
            }
        }
    }

    async fn discovery_members_with_selection(
        &self,
        prepared: &mut PreparedKubeconfig,
    ) -> Result<Vec<DiscoveryMember>, K8sError> {
        attempt_with_pinned_retry(
            prepared,
            std::time::Duration::from_secs(10),
            |config, options| async move { discovery_members_from_config(config, &options).await },
        )
        .await
    }
}

async fn discovery_members_from_config(
    kubeconfig: kube::config::Kubeconfig,
    options: &kube::config::KubeConfigOptions,
) -> Result<Vec<DiscoveryMember>, K8sError> {
    let client = client_from_roster_config(kubeconfig, options.clone()).await?;
    Ok(k8s_nodes_to_discovery_members(
        list_cluster_nodes(&client).await?,
    ))
}

async fn client_from_roster_config(
    kubeconfig: kube::config::Kubeconfig,
    options: kube::config::KubeConfigOptions,
) -> Result<Client, K8sError> {
    let runtime = tokio::runtime::Handle::current();
    // kube-rs may read credential files or execute authentication plugins.
    // Keep setup isolated so an enclosing deadline can discard late results.
    crate::kubeconfig_selection::run_roster_setup(move || {
        runtime.block_on(async {
            let config = Config::from_custom_kubeconfig(kubeconfig, &options)
                .await
                .map_err(|_| {
                    K8sError::ClientCreate(
                        "Could not load credentials for the validated Kubernetes source".into(),
                    )
                })?;
            Client::try_from(config).map_err(|_| {
                K8sError::ClientCreate(
                    "Could not construct a client for the validated Kubernetes source".into(),
                )
            })
        })
    })
    .await
}

/// Everything one node returned, tagged with the node's name.
#[derive(Default)]
struct NodeMetrics {
    versions: Vec<VersionInfo>,
    services: Vec<NodeServices>,
    memory: Vec<NodeMemory>,
    load_avg: Vec<NodeLoadAvg>,
    cpu_info: Vec<NodeCpuInfo>,
    /// The version request failed in a way that suggests a dead connection.
    version_transport_failure: bool,
}

/// One request's outcome; a failure keeps the other requests' data usable.
enum CallOutcome<T> {
    Data(Vec<T>),
    Failed { transport: bool },
}

impl<T> CallOutcome<T> {
    fn transport_failure(&self) -> bool {
        matches!(self, Self::Failed { transport: true })
    }

    fn into_data(self) -> Vec<T> {
        match self {
            Self::Data(values) => values,
            Self::Failed { .. } => Vec::new(),
        }
    }
}

async fn bounded_call<T>(
    call: impl Future<Output = Result<Vec<T>, TalosError>>,
    limit: Duration,
) -> CallOutcome<T> {
    match tokio::time::timeout(limit, call).await {
        Ok(Ok(values)) => CallOutcome::Data(values),
        Ok(Err(error)) => CallOutcome::Failed {
            transport: error.is_transport_failure(),
        },
        Err(_) => CallOutcome::Failed { transport: true },
    }
}

/// Runs a node's five requests concurrently, each under its own timeout.
async fn collect_node(client: TalosClient, name: String, limit: Duration) -> NodeMetrics {
    let (versions, services, memory, load_avg, cpu_info) = tokio::join!(
        bounded_call(client.version(), limit),
        bounded_call(client.services(), limit),
        bounded_call(client.memory(), limit),
        bounded_call(client.load_avg(), limit),
        bounded_call(client.cpu_info(), limit),
    );
    let mut metrics = NodeMetrics {
        version_transport_failure: versions.transport_failure(),
        versions: versions.into_data(),
        services: services.into_data(),
        memory: memory.into_data(),
        load_avg: load_avg.into_data(),
        cpu_info: cpu_info.into_data(),
    };
    for value in &mut metrics.versions {
        value.node.clone_from(&name);
    }
    for value in &mut metrics.services {
        value.node.clone_from(&name);
    }
    for value in &mut metrics.memory {
        value.node.clone_from(&name);
    }
    for value in &mut metrics.load_avg {
        value.node.clone_from(&name);
    }
    for value in &mut metrics.cpu_info {
        value.node.clone_from(&name);
    }
    metrics
}

/// Runs `collect_one` for every `(name, address)` with at most `limit` in
/// flight. Results are in input order regardless of completion order, and a
/// panicking node yields empty metrics instead of hiding the others.
async fn collect_bounded<F, Fut>(
    nodes: Vec<(String, String)>,
    limit: usize,
    collect_one: F,
) -> Vec<NodeMetrics>
where
    F: Fn(String, String) -> Fut + Send + Sync + 'static,
    Fut: Future<Output = NodeMetrics> + Send + 'static,
{
    let collect_one = Arc::new(collect_one);
    let permits = Arc::new(tokio::sync::Semaphore::new(limit.max(1)));
    let mut results: Vec<NodeMetrics> = nodes.iter().map(|_| NodeMetrics::default()).collect();
    let mut tasks = tokio::task::JoinSet::new();
    for (index, (name, ip)) in nodes.into_iter().enumerate() {
        let collect_one = Arc::clone(&collect_one);
        let permits = Arc::clone(&permits);
        tasks.spawn(async move {
            let _permit = permits.acquire_owned().await;
            (index, collect_one(name, ip).await)
        });
    }
    while let Some(joined) = tasks.join_next().await {
        if let Ok((index, metrics)) = joined {
            results[index] = metrics;
        }
    }
    results
}

fn root_cause(error: &dyn std::error::Error) -> String {
    let mut deepest = error;
    while let Some(source) = deepest.source() {
        deepest = source;
    }
    deepest.to_string()
}

fn replace_node_ips_from_etcd(cluster: &mut ClusterOverview, members: &[EtcdMemberInfo]) {
    cluster.node_ips.clear();
    for member in members {
        if let Some(ip) = member.ip_address() {
            cluster.node_ips.insert(member.hostname.clone(), ip);
        }
    }
}

fn replace_node_ips_from_discovery(cluster: &mut ClusterOverview, members: &[DiscoveryMember]) {
    cluster.node_ips.clear();
    for member in members {
        if let Some(ip) = member.addresses.first() {
            cluster.node_ips.insert(member.hostname.clone(), ip.clone());
        }
    }
}

fn nodes_to_query(cluster: &ClusterOverview) -> Vec<(String, String)> {
    if !cluster.discovery_members.is_empty() {
        cluster
            .discovery_members
            .iter()
            .filter_map(|member| {
                member.addresses.first().map(|ip| {
                    let name = if member.hostname.is_empty() {
                        ip.clone()
                    } else {
                        member.hostname.clone()
                    };
                    (name, ip.clone())
                })
            })
            .collect()
    } else if !cluster.etcd_members.is_empty() {
        cluster
            .etcd_members
            .iter()
            .filter_map(|member| {
                member.ip_address().map(|ip| {
                    let name = if member.hostname.is_empty() {
                        ip.clone()
                    } else {
                        member.hostname.clone()
                    };
                    (name, ip)
                })
            })
            .collect()
    } else if !cluster.versions.is_empty() {
        tracing::debug!(
            "Using version info as fallback node source for {}",
            cluster.name
        );
        cluster
            .versions
            .iter()
            .map(|version| {
                (
                    version.node.clone(),
                    talos_rs::target_host(&version.node).to_string(),
                )
            })
            .collect()
    } else {
        tracing::warn!(
            "No node sources available for {}, skipping queries",
            cluster.name
        );
        Vec::new()
    }
}

/// Error type for Kubernetes helpers used by the overview collector.
#[derive(Debug, thiserror::Error)]
pub enum K8sError {
    /// Talos could not serve a kubeconfig.
    #[error("Failed to get kubeconfig from Talos: {0}")]
    KubeconfigFetch(String),
    /// The served kubeconfig could not be parsed.
    #[error("Failed to parse kubeconfig: {0}")]
    KubeconfigParse(String),
    /// Kube could not construct a client.
    #[error("Failed to create K8s client: {0}")]
    ClientCreate(String),
    /// The Kubernetes API request failed.
    #[error("K8s API error: {0}")]
    ApiError(String),
}

/// Source of the kubeconfig used to create a Kubernetes client.
#[derive(Debug, Clone)]
pub enum KubeconfigSource {
    /// `KUBECONFIG` or the default kubeconfig path.
    Environment,
    /// A specific Talos control plane node.
    TalosNode(String),
    /// Source is unknown or unavailable.
    Unavailable(String),
}

/// Create a Kubernetes client from Talos-provided kubeconfig.
pub async fn create_k8s_client(talos_client: &TalosClient) -> Result<Client, K8sError> {
    let (client, _) = create_k8s_client_with_source(talos_client, None, None).await?;
    Ok(client)
}

/// Create a Kubernetes client while preserving cluster identity when possible.
///
/// A pinned control plane's kubeconfig is the identity source. Ambient
/// `KUBECONFIG` is used only when its current context has the same inline CA;
/// otherwise it is ignored so a frontend never reads or mutates another cluster.
pub async fn create_k8s_client_with_source(
    talos_client: &TalosClient,
    control_plane_ip: Option<&str>,
    kubeconfig_client: Option<&TalosClient>,
) -> Result<(Client, KubeconfigSource), K8sError> {
    if let Some(node_ip) = control_plane_ip {
        tracing::debug!("Targeting control plane node {} for kubeconfig", node_ip);
        match fetch_node_kubeconfig(&talos_client.with_node(node_ip)).await {
            Ok(node_kubeconfig) => {
                let node_ca = current_context_cluster_ca(&node_kubeconfig);
                match (node_ca.as_deref(), ambient_kubeconfig_ca()) {
                    (Some(node_ca), Some(ambient_ca)) if node_ca == ambient_ca => {
                        if let Ok(config) = Config::infer().await
                            && let Ok(client) = Client::try_from(config)
                        {
                            tracing::debug!("KUBECONFIG matches the Talos cluster; using it");
                            return Ok((client, KubeconfigSource::Environment));
                        }
                        return Ok((
                            client_from_kubeconfig(node_kubeconfig).await?,
                            KubeconfigSource::TalosNode(node_ip.to_string()),
                        ));
                    }
                    (Some(_), Some(_)) => {
                        tracing::warn!(
                            "KUBECONFIG points at a different cluster than control plane node {}; ignoring it and using the node's kubeconfig",
                            node_ip
                        );
                        return Ok((
                            client_from_kubeconfig(node_kubeconfig).await?,
                            KubeconfigSource::TalosNode(node_ip.to_string()),
                        ));
                    }
                    _ => {
                        return Ok((
                            client_from_kubeconfig(node_kubeconfig).await?,
                            KubeconfigSource::TalosNode(node_ip.to_string()),
                        ));
                    }
                }
            }
            Err(error) => {
                tracing::warn!(
                    "Failed to fetch kubeconfig from node {}: {} (falling back)",
                    node_ip,
                    error
                );
            }
        }
    }

    if let Some(kubeconfig_client) = kubeconfig_client {
        tracing::debug!("Trying provided kubeconfig client");
        match fetch_kubeconfig_from_client(kubeconfig_client).await {
            Ok(client) => {
                return Ok((
                    client,
                    KubeconfigSource::TalosNode("control-plane".to_string()),
                ));
            }
            Err(error) => {
                tracing::warn!("Failed to fetch kubeconfig from provided client: {}", error);
            }
        }
    }

    if let Ok(config) = Config::infer().await
        && let Ok(client) = Client::try_from(config)
    {
        tracing::debug!("Using kubeconfig from environment (KUBECONFIG or default path)");
        return Ok((client, KubeconfigSource::Environment));
    }

    tracing::debug!("Trying main Talos client for kubeconfig (may fail with multiple nodes)");
    let client = fetch_kubeconfig_from_client(talos_client).await?;
    Ok((client, KubeconfigSource::TalosNode("vip".to_string())))
}

/// Fetch a Kubernetes client directly from a control plane node without ever
/// consulting ambient `KUBECONFIG`. Use this for node enumeration.
pub async fn create_k8s_client_from_node(
    talos_client: &TalosClient,
    control_plane_ip: &str,
) -> Result<Client, K8sError> {
    fetch_kubeconfig_from_client(&talos_client.with_node(control_plane_ip)).await
}

/// As [`create_k8s_client_from_node`], reusing the client for the same Talos
/// connection and control plane for a couple of minutes.
///
/// The kubeconfig is still served by `control_plane_ip` and ambient
/// `KUBECONFIG` is still never consulted; only the repeat fetch is skipped.
/// Call [`forget_cached_k8s_client_from_node`] when a request fails.
pub async fn create_k8s_client_from_node_cached(
    talos_client: &TalosClient,
    control_plane_ip: &str,
) -> Result<Client, K8sError> {
    if let Some(client) = client_cache::node_client(talos_client, control_plane_ip) {
        return Ok(client);
    }
    let client = create_k8s_client_from_node(talos_client, control_plane_ip).await?;
    client_cache::store_node_client(talos_client, control_plane_ip, client.clone());
    Ok(client)
}

/// Drops the client cached by [`create_k8s_client_from_node_cached`].
pub fn forget_cached_k8s_client_from_node(talos_client: &TalosClient, control_plane_ip: &str) {
    client_cache::forget_node_client(talos_client, control_plane_ip);
}

/// Creates a Kubernetes client with a supplied control-plane kubeconfig source.
///
/// Kept for diagnostic callers that do not have a concrete control plane IP.
pub async fn create_k8s_client_with_kubeconfig_source(
    talos_client: &TalosClient,
    kubeconfig_client: Option<&TalosClient>,
) -> Result<Client, K8sError> {
    let (client, _) = create_k8s_client_with_source(talos_client, None, kubeconfig_client).await?;
    Ok(client)
}

async fn fetch_node_kubeconfig(client: &TalosClient) -> Result<kube::config::Kubeconfig, K8sError> {
    let kubeconfig = client
        .kubeconfig()
        .await
        .map_err(|error| K8sError::KubeconfigFetch(error.to_string()))?;
    serde_yaml::from_str(&kubeconfig).map_err(|error| K8sError::KubeconfigParse(error.to_string()))
}

async fn client_from_kubeconfig(kubeconfig: kube::config::Kubeconfig) -> Result<Client, K8sError> {
    let config = Config::from_custom_kubeconfig(kubeconfig, &Default::default())
        .await
        .map_err(|error| K8sError::ClientCreate(error.to_string()))?;
    Client::try_from(config).map_err(|error| K8sError::ClientCreate(error.to_string()))
}

async fn fetch_kubeconfig_from_client(client: &TalosClient) -> Result<Client, K8sError> {
    client_from_kubeconfig(fetch_node_kubeconfig(client).await?).await
}

fn current_context_cluster_ca(kubeconfig: &kube::config::Kubeconfig) -> Option<String> {
    let context_name = kubeconfig.current_context.as_deref()?;
    let context = kubeconfig
        .contexts
        .iter()
        .find(|context| context.name == context_name)?;
    let cluster_name = &context.context.as_ref()?.cluster;
    let cluster = kubeconfig
        .clusters
        .iter()
        .find(|cluster| &cluster.name == cluster_name)?;
    cluster.cluster.as_ref()?.certificate_authority_data.clone()
}

fn ambient_kubeconfig_ca() -> Option<String> {
    let kubeconfig = kube::config::Kubeconfig::read().ok()?;
    current_context_cluster_ca(&kubeconfig)
}

/// Returns a warning only when ambient KUBECONFIG can be proven to identify a
/// different cluster than the pinned control plane node.
pub async fn kubeconfig_mismatch_warning(
    talos_client: &TalosClient,
    control_plane_ip: &str,
) -> Option<String> {
    let ambient_ca = ambient_kubeconfig_ca()?;
    let node_kubeconfig = fetch_node_kubeconfig(&talos_client.with_node(control_plane_ip))
        .await
        .ok()?;
    let node_ca = current_context_cluster_ca(&node_kubeconfig)?;
    (ambient_ca != node_ca).then(|| format!(
        "KUBECONFIG points at a different cluster than this Talos context — ignoring it. Kubernetes views (workloads, drain/cordon) use the kubeconfig from control plane node {control_plane_ip} instead."
    ))
}

/// A Kubernetes node relevant to the Talos node roster.
#[derive(Debug, Clone, PartialEq)]
pub struct K8sNodeInfo {
    /// Kubernetes node name, normally matching the Talos hostname.
    pub name: String,
    /// Internal node IP, used to reach Talos.
    pub internal_ip: Option<String>,
    /// Whether the node has a control-plane role label.
    pub is_control_plane: bool,
}

fn node_info_from(node: &Node) -> K8sNodeInfo {
    let name = node.metadata.name.clone().unwrap_or_default();
    let internal_ip = node
        .status
        .as_ref()
        .and_then(|status| status.addresses.as_ref())
        .and_then(|addresses| {
            addresses
                .iter()
                .find(|address| address.type_ == "InternalIP")
                .map(|address| address.address.clone())
        });
    let is_control_plane = node.metadata.labels.as_ref().is_some_and(|labels| {
        labels.contains_key("node-role.kubernetes.io/control-plane")
            || labels.contains_key("node-role.kubernetes.io/master")
    });

    K8sNodeInfo {
        name,
        internal_ip,
        is_control_plane,
    }
}

/// Lists all nodes known by the Kubernetes API.
pub async fn list_cluster_nodes(client: &Client) -> Result<Vec<K8sNodeInfo>, K8sError> {
    let nodes: Api<Node> = Api::all(client.clone());
    let list = nodes
        .list(&ListParams::default())
        .await
        .map_err(|error| K8sError::ApiError(format!("Failed to list nodes: {error}")))?;
    Ok(list.items.iter().map(node_info_from).collect())
}

/// Enumerates this cluster's complete node roster through a kubeconfig served
/// by `control_plane_ip`; an ambient KUBECONFIG is never used for this path.
pub async fn discovery_members_via_k8s(
    talos_client: &TalosClient,
    control_plane_ip: &str,
) -> Result<Vec<DiscoveryMember>, K8sError> {
    let k8s_client = create_k8s_client_from_node(talos_client, control_plane_ip).await?;
    let nodes = list_cluster_nodes(&k8s_client).await?;
    Ok(k8s_nodes_to_discovery_members(nodes))
}

/// Maps a Kubernetes roster to Talos discovery members, omitting nodes without
/// an InternalIP because they cannot be reached through the Talos API.
pub fn k8s_nodes_to_discovery_members(nodes: Vec<K8sNodeInfo>) -> Vec<DiscoveryMember> {
    nodes
        .into_iter()
        .filter_map(|node| {
            let ip = node.internal_ip?;
            Some(DiscoveryMember {
                id: node.name.clone(),
                addresses: vec![ip],
                hostname: node.name,
                machine_type: if node.is_control_plane {
                    "controlplane".to_string()
                } else {
                    "worker".to_string()
                },
                operating_system: String::new(),
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use talos_rs::ServiceInfo;

    #[tokio::test]
    async fn parsed_connection_does_not_reopen_the_configured_path() {
        let parsed = TalosConfig::parse("context: unused\ncontexts: {}").unwrap();
        let collector = ClusterOverviewCollector::new(
            Some(PathBuf::from("/nonexistent/never-reopen-talosconfig")),
            None,
        );
        assert!(
            collector
                .connect_from_config(&parsed)
                .await
                .unwrap()
                .is_empty()
        );

        let filtered = ClusterOverviewCollector::new(
            Some(PathBuf::from("/nonexistent/never-reopen-talosconfig")),
            Some("missing".into()),
        );
        assert!(matches!(
            filtered.connect_from_config(&parsed).await,
            Err(ClusterOverviewError::ContextNotFound { name, .. }) if name == "missing"
        ));
    }

    #[tokio::test]
    async fn identity_client_requires_connected_selected_talos_cluster() {
        let mut collector = ClusterOverviewCollector::default();
        collector.set_kubeconfig_selection(KubeconfigSelection::File {
            path: PathBuf::from("/nonexistent/explicit-kubeconfig"),
            context: Some("selected".into()),
        });
        let cluster = ClusterOverview {
            name: "selected-talos".into(),
            ..Default::default()
        };
        let error = match collector.kubernetes_client(&cluster).await {
            Ok(_) => panic!("Disconnected identity must not create an ambient client"),
            Err(error) => error,
        };
        assert!(
            error
                .to_string()
                .contains("selected Talos context is disconnected")
        );
    }

    #[test]
    fn explicit_file_rejection_retains_only_pinned_source_with_warning() {
        let pinned = kube::config::Kubeconfig::from_yaml(
            "apiVersion: v1\nkind: Config\ncurrent-context: pinned\ncontexts:\n- name: pinned\n  context:\n    cluster: selected\nclusters:\n- name: selected\n  cluster:\n    server: https://10.0.0.1:6443\n",
        ).unwrap();
        let prepared = prepare_kubeconfig(
            &KubeconfigSelection::File {
                path: PathBuf::from("/nonexistent/explicit-kubeconfig"),
                context: Some("requested".into()),
            },
            Some(pinned),
            Some("10.0.0.1"),
            || panic!("Explicit source must not consult ambient credentials"),
        );
        assert!(prepared.source.contains("selected file rejected"));
        assert!(
            prepared
                .warning
                .as_deref()
                .unwrap()
                .contains("ambient configuration is not used")
        );
        assert_eq!(
            prepared.config.unwrap().current_context.as_deref(),
            Some("pinned")
        );
    }

    fn version(node: &str) -> VersionInfo {
        VersionInfo {
            node: node.to_string(),
            version: String::new(),
            sha: String::new(),
            built: String::new(),
            go_version: String::new(),
            os: String::new(),
            arch: String::new(),
            platform: String::new(),
        }
    }

    fn member(hostname: &str, ip: &str, machine_type: &str) -> DiscoveryMember {
        DiscoveryMember {
            id: hostname.to_string(),
            addresses: vec![ip.to_string()],
            hostname: hostname.to_string(),
            machine_type: machine_type.to_string(),
            operating_system: String::new(),
        }
    }

    fn services(node: &str, ids: &[&str]) -> NodeServices {
        NodeServices {
            node: node.to_string(),
            services: ids
                .iter()
                .map(|id| ServiceInfo {
                    id: id.to_string(),
                    state: "Running".to_string(),
                    health: None,
                })
                .collect(),
        }
    }

    #[test]
    fn discovery_machine_type_is_authoritative_with_etcd_fallback() {
        let cluster = ClusterOverview {
            versions: vec![version("cp1"), version("worker1"), version("legacy-cp")],
            discovery_members: vec![
                member("cp1", "10.0.0.1", "controlplane"),
                member("worker1", "10.0.0.2", "worker"),
            ],
            services: vec![services("legacy-cp", &["etcd", "kubelet"])],
            ..Default::default()
        };

        assert!(cluster.node_is_controlplane("cp1"));
        assert!(!cluster.node_is_controlplane("worker1"));
        assert!(cluster.node_is_controlplane("legacy-cp"));
        assert!(!cluster.node_is_controlplane("unknown"));
    }

    #[test]
    fn ipv6_node_addresses_match_whole() {
        let cluster = ClusterOverview {
            discovery_members: vec![member("cp6", "2001:db8::5", "controlplane")],
            ..Default::default()
        };
        assert!(cluster.node_is_controlplane("2001:db8::5"));
        assert!(cluster.node_is_controlplane("[2001:db8::5]:50000"));
        assert!(!cluster.node_is_controlplane("2001:db8::6"));

        assert_eq!(
            ClusterOverview {
                endpoints: vec!["[2001:db8::5]:50000".into()],
                ..Default::default()
            }
            .control_plane_ip()
            .as_deref(),
            Some("2001:db8::5")
        );

        let cluster = ClusterOverview {
            versions: vec![version("2001:db8::5"), version("10.0.0.5:50000")],
            ..Default::default()
        };
        assert_eq!(
            nodes_to_query(&cluster),
            vec![
                ("2001:db8::5".to_string(), "2001:db8::5".to_string()),
                ("10.0.0.5:50000".to_string(), "10.0.0.5".to_string()),
            ]
        );
    }

    #[test]
    fn k8s_roster_maps_roles_and_drops_ipless_nodes() {
        let members = k8s_nodes_to_discovery_members(vec![
            K8sNodeInfo {
                name: "cp1".into(),
                internal_ip: Some("10.0.0.1".into()),
                is_control_plane: true,
            },
            K8sNodeInfo {
                name: "w1".into(),
                internal_ip: Some("10.0.0.2".into()),
                is_control_plane: false,
            },
            K8sNodeInfo {
                name: "ghost".into(),
                internal_ip: None,
                is_control_plane: false,
            },
        ]);

        assert_eq!(members.len(), 2);
        assert_eq!(members[0].machine_type, "controlplane");
        assert_eq!(members[1].machine_type, "worker");
        assert_eq!(members[1].addresses, vec!["10.0.0.2"]);
    }

    #[test]
    fn control_plane_ip_prefers_etcd_then_roster_then_endpoint() {
        let etcd_member = EtcdMemberInfo {
            id: 1,
            hostname: "cp1".into(),
            peer_urls: vec!["https://10.0.0.1:2380".into()],
            client_urls: vec!["https://10.0.0.1:2379".into()],
            is_learner: false,
        };
        let roster_member = member("cp2", "10.0.0.9", "controlplane");

        assert_eq!(
            ClusterOverview {
                etcd_members: vec![etcd_member],
                discovery_members: vec![roster_member.clone()],
                endpoints: vec!["10.0.0.5:50000".into()],
                ..Default::default()
            }
            .control_plane_ip()
            .as_deref(),
            Some("10.0.0.1")
        );
        assert_eq!(
            ClusterOverview {
                discovery_members: vec![roster_member],
                endpoints: vec!["10.0.0.5:50000".into()],
                ..Default::default()
            }
            .control_plane_ip()
            .as_deref(),
            Some("10.0.0.9")
        );
        assert_eq!(
            ClusterOverview {
                endpoints: vec!["10.0.0.5:50000".into()],
                ..Default::default()
            }
            .control_plane_ip()
            .as_deref(),
            Some("10.0.0.5")
        );
    }

    #[test]
    fn collector_uses_replaced_config_path() {
        let mut collector =
            ClusterOverviewCollector::new(Some(PathBuf::from("/initial/talosconfig")), None);

        collector.set_config_path(Some(PathBuf::from("/selected/talosconfig")));

        assert_eq!(
            collector.config_path.as_ref(),
            Some(&PathBuf::from("/selected/talosconfig"))
        );
    }

    #[cfg(unix)]
    #[test]
    fn collector_accepts_non_utf8_config_path() {
        use std::os::unix::ffi::OsStringExt;

        let path = PathBuf::from(std::ffi::OsString::from_vec(vec![
            b'/', b't', b'm', b'p', b'/', 0xFF,
        ]));
        let collector = ClusterOverviewCollector::new(Some(path.clone()), None);

        assert_eq!(collector.config_path, Some(path));
    }

    #[test]
    fn kubeconfig_selection_is_per_collector_and_defaults_to_automatic() {
        let mut selected = ClusterOverviewCollector::new(None, None);
        let unchanged = selected.clone();
        let choice = KubeconfigSelection::File {
            path: PathBuf::from("/session/kubeconfig"),
            context: Some("chosen".into()),
        };
        selected.set_kubeconfig_selection(choice.clone());
        assert_eq!(selected.kubeconfig_selection, choice);
        assert_eq!(
            unchanged.kubeconfig_selection,
            KubeconfigSelection::Automatic
        );
        assert_eq!(
            ClusterOverviewCollector::default().kubeconfig_selection,
            KubeconfigSelection::Automatic
        );
    }

    fn services_for(node: &str) -> NodeServices {
        NodeServices {
            node: node.into(),
            services: vec![],
        }
    }

    fn nodes(names: &[&str]) -> Vec<(String, String)> {
        names
            .iter()
            .enumerate()
            .map(|(index, name)| ((*name).to_string(), format!("10.0.0.{index}")))
            .collect()
    }

    #[tokio::test]
    async fn nodes_are_queried_concurrently() {
        // Every node waits for all the others; sequential collection would
        // never fill the barrier and each wait would time out.
        let barrier = Arc::new(tokio::sync::Barrier::new(4));
        let results = collect_bounded(nodes(&["a", "b", "c", "d"]), 8, move |name, _| {
            let barrier = Arc::clone(&barrier);
            async move {
                let met = tokio::time::timeout(Duration::from_secs(5), barrier.wait())
                    .await
                    .is_ok();
                NodeMetrics {
                    services: vec![services_for(&name)],
                    version_transport_failure: !met,
                    ..Default::default()
                }
            }
        })
        .await;
        assert_eq!(results.len(), 4);
        assert!(results.iter().all(|node| !node.version_transport_failure));
    }

    #[tokio::test]
    async fn concurrency_is_bounded() {
        let running = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let peak = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let (running_in, peak_in) = (Arc::clone(&running), Arc::clone(&peak));
        let results = collect_bounded(nodes(&["a", "b", "c", "d", "e", "f"]), 2, move |_, _| {
            let (running, peak) = (Arc::clone(&running_in), Arc::clone(&peak_in));
            async move {
                use std::sync::atomic::Ordering::SeqCst;
                let now = running.fetch_add(1, SeqCst) + 1;
                peak.fetch_max(now, SeqCst);
                tokio::time::sleep(Duration::from_millis(30)).await;
                running.fetch_sub(1, SeqCst);
                NodeMetrics::default()
            }
        })
        .await;
        assert_eq!(results.len(), 6);
        assert_eq!(peak.load(std::sync::atomic::Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn slow_node_times_out_without_delaying_the_others() {
        let started = std::time::Instant::now();
        let results = collect_bounded(nodes(&["down", "up-1", "up-2"]), 8, |name, _| async move {
            let limit = Duration::from_millis(150);
            let call = async {
                if name == "down" {
                    tokio::time::sleep(Duration::from_secs(30)).await;
                }
                Ok::<_, TalosError>(vec![services_for(&name)])
            };
            let outcome = bounded_call(call, limit).await;
            NodeMetrics {
                version_transport_failure: outcome.transport_failure(),
                services: outcome.into_data(),
                ..Default::default()
            }
        })
        .await;
        assert!(started.elapsed() < Duration::from_secs(5));
        assert!(results[0].version_transport_failure);
        assert!(results[0].services.is_empty());
        assert_eq!(results[1].services[0].node, "up-1");
        assert_eq!(results[2].services[0].node, "up-2");
    }

    #[tokio::test]
    async fn output_order_follows_input_not_completion() {
        let results = collect_bounded(nodes(&["slow", "mid", "fast"]), 8, |name, _| async move {
            let delay = match name.as_str() {
                "slow" => 90,
                "mid" => 45,
                _ => 1,
            };
            tokio::time::sleep(Duration::from_millis(delay)).await;
            NodeMetrics {
                services: vec![services_for(&name)],
                ..Default::default()
            }
        })
        .await;
        let order: Vec<_> = results
            .iter()
            .map(|node| node.services[0].node.as_str())
            .collect();
        assert_eq!(order, ["slow", "mid", "fast"]);
    }

    #[tokio::test]
    async fn a_panicking_node_does_not_hide_the_others() {
        let results = collect_bounded(nodes(&["bad", "good"]), 8, |name, _| async move {
            if name == "bad" {
                panic!("collector bug");
            }
            NodeMetrics {
                services: vec![services_for(&name)],
                ..Default::default()
            }
        })
        .await;
        assert!(results[0].services.is_empty());
        assert_eq!(results[1].services[0].node, "good");
    }

    #[test]
    fn rejected_file_selection_keeps_fallback_and_warning_when_cached() {
        use crate::client_cache::TtlCache;
        let pinned = kube::config::Kubeconfig::from_yaml(
            "apiVersion: v1\nkind: Config\ncurrent-context: pinned\ncontexts:\n- name: pinned\n  context:\n    cluster: selected\nclusters:\n- name: selected\n  cluster:\n    server: https://10.0.0.1:6443\n",
        ).unwrap();
        let prepared = prepare_kubeconfig(
            &KubeconfigSelection::File {
                path: PathBuf::from("/nonexistent/explicit-kubeconfig"),
                context: Some("requested".into()),
            },
            Some(pinned),
            Some("10.0.0.1"),
            || panic!("Explicit source must not consult ambient credentials"),
        );
        let now = std::time::Instant::now();
        let mut cache = TtlCache::new(Some(Duration::from_secs(120)), 4);
        cache.insert("key", prepared, now);
        let served = cache.get(&"key", now + Duration::from_secs(60)).unwrap();
        assert!(served.source.contains("selected file rejected"));
        assert!(
            served
                .warning
                .as_deref()
                .unwrap()
                .contains("ambient configuration is not used")
        );
        assert_eq!(
            served.config.unwrap().current_context.as_deref(),
            Some("pinned")
        );
        assert!(cache.get(&"key", now + Duration::from_secs(121)).is_none());
    }

    #[test]
    fn unavailable_prepared_kubeconfig_is_not_cacheable() {
        assert!(
            PreparedKubeconfig::unavailable("transient")
                .config
                .is_none()
        );
    }

    #[test]
    fn etcd_summary_counts_each_known_voter_once_and_excludes_learners() {
        let members: Vec<_> = (1..=3)
            .map(|id| EtcdMemberInfo {
                id,
                hostname: format!("cp-{id}"),
                peer_urls: Vec::new(),
                client_urls: Vec::new(),
                is_learner: id == 3,
            })
            .collect();
        let status = |id| EtcdMemberStatus {
            node: format!("cp-{id}"),
            member_id: id,
            protocol_version: String::new(),
            db_size: 0,
            db_size_in_use: 0,
            leader_id: 1,
            raft_index: 0,
            raft_term: 1,
            raft_applied_index: 0,
            errors: Vec::new(),
            is_learner: id == 3,
        };
        let summary = EtcdSummary::from_statuses(
            &members,
            &[status(1), status(1), status(2), status(3), status(99)],
        );
        assert_eq!(
            (summary.healthy, summary.total, summary.has_quorum),
            (2, 2, true)
        );
        assert_eq!(
            crate::indicators::quorum(summary.healthy, summary.total).remaining_tolerance,
            0
        );
        let summary =
            EtcdSummary::from_statuses(&members, &[status(1), status(1), status(3), status(99)]);
        assert_eq!(
            (summary.healthy, summary.total, summary.has_quorum),
            (1, 2, false)
        );
    }
}
