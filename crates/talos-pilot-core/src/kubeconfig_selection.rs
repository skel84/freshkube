//! Session-local kubeconfig selection and credential-free identity validation.

use std::{
    fs,
    future::Future,
    io::Read,
    path::{Path, PathBuf},
    time::Duration,
};

use base64::{Engine, engine::general_purpose::STANDARD};
use kube::config::{Cluster, KubeConfigOptions, Kubeconfig};
use x509_parser::{parse_x509_certificate, pem::parse_x509_pem};

use crate::cluster_overview::K8sError;

const MAX_CONFIG_BYTES: u64 = 4 * 1024 * 1024;
const MAX_CA_BYTES: u64 = 1024 * 1024;

/// Session-local Kubernetes configuration choice for a cluster overview collector.
/// Explicit choices never consult `KUBECONFIG` or infer a default configuration.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum KubeconfigSelection {
    /// Preserve node-pinned roster discovery and ambient mismatch diagnostics.
    #[default]
    Automatic,
    /// Always use the kubeconfig served by the pinned Talos control plane.
    TalosControlPlane,
    /// Use a file only after its chosen context's CA matches the pinned node.
    File {
        /// Kubeconfig location; relative credential paths resolve against it.
        path: PathBuf,
        /// Named context, or the file's current context when omitted.
        context: Option<String>,
    },
}

/// Context metadata from a kubeconfig, without loading credentials or executing plugins.
#[derive(Debug, Clone)]
pub struct KubeconfigFileInfo {
    /// Sorted context names, including contexts that may not be usable yet.
    pub contexts: Vec<String>,
    /// The file's declared current context; it need not exist in `contexts`.
    pub current_context: Option<String>,
}

/// Reads a bounded kubeconfig file and lists its contexts without making a client.
///
/// This performs synchronous file I/O; GUI callers should run it on a worker.
/// Missing/invalid current contexts are returned unchanged so a named context can
/// still be selected. Parse diagnostics deliberately exclude file contents.
pub fn inspect_kubeconfig(path: &Path) -> Result<KubeconfigFileInfo, K8sError> {
    let config = read_selected_file(path)?;
    let mut contexts = config
        .contexts
        .into_iter()
        .map(|context| context.name)
        .collect::<Vec<_>>();
    contexts.sort();
    contexts.dedup();
    Ok(KubeconfigFileInfo {
        contexts,
        current_context: config.current_context,
    })
}

fn read_capped(reader: impl Read, limit: u64) -> Result<Vec<u8>, &'static str> {
    let mut bytes = Vec::new();
    reader
        .take(limit + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| "file could not be read")?;
    if bytes.len() as u64 > limit {
        return Err("file exceeds the supported size limit");
    }
    Ok(bytes)
}

fn read_bounded_regular_file(path: &Path, limit: u64) -> Result<Vec<u8>, &'static str> {
    let mut options = fs::OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        // A path replaced with a FIFO between selection and opening must not
        // block the worker before descriptor metadata can reject it.
        options.custom_flags(libc::O_NONBLOCK);
    }
    let file = options
        .open(path)
        .map_err(|_| "file is unreadable or does not exist")?;
    let metadata = file
        .metadata()
        .map_err(|_| "file metadata is unavailable")?;
    if !metadata.is_file() {
        return Err("path is not a regular file");
    }
    if metadata.len() > limit {
        return Err("file exceeds the supported size limit");
    }
    // Enforce the bound on the opened descriptor even if the file grows after
    // metadata inspection. The parsed snapshot never reopens the path.
    read_capped(file, limit)
}

fn decode_kubeconfig(bytes: Vec<u8>) -> Result<String, &'static str> {
    let (data, little_endian) = match bytes.as_slice() {
        [0xFF, 0xFE, rest @ ..] => (rest, true),
        [0xFE, 0xFF, rest @ ..] => (rest, false),
        [0xEF, 0xBB, 0xBF, rest @ ..] => {
            return String::from_utf8(rest.to_vec()).map_err(|_| "kubeconfig is not valid UTF-8");
        }
        _ => return String::from_utf8(bytes).map_err(|_| "kubeconfig is not valid UTF-8"),
    };
    if data.len() % 2 != 0 {
        return Err("kubeconfig is not valid UTF-16");
    }
    let words = data
        .chunks_exact(2)
        .map(|chunk| {
            let pair = [chunk[0], chunk[1]];
            if little_endian {
                u16::from_le_bytes(pair)
            } else {
                u16::from_be_bytes(pair)
            }
        })
        .collect::<Vec<_>>();
    String::from_utf16(&words).map_err(|_| "kubeconfig is not valid UTF-16")
}

fn resolve_relative(path: &mut Option<String>, directory: &Path) -> Result<(), K8sError> {
    if let Some(file) = path
        && Path::new(&*file).is_relative()
    {
        let joined = directory.join(&*file);
        *file = joined
            .to_str()
            .ok_or_else(|| {
                K8sError::KubeconfigParse(
                    "relative kubeconfig reference resolves to a non-UTF-8 path".into(),
                )
            })?
            .to_owned();
    }
    Ok(())
}

fn read_selected_file(path: &Path) -> Result<Kubeconfig, K8sError> {
    let bytes = read_bounded_regular_file(path, MAX_CONFIG_BYTES)
        .map_err(|reason| K8sError::KubeconfigParse(reason.to_string()))?;
    let text =
        decode_kubeconfig(bytes).map_err(|reason| K8sError::KubeconfigParse(reason.to_string()))?;
    let mut config = Kubeconfig::from_yaml(&text)
        .map_err(|_| K8sError::KubeconfigParse("selected file is not a valid kubeconfig".into()))?;
    // Match kube-rs read_from's path semantics without reopening the file. A
    // symlink's selected parent, not its canonical target, resolves references.
    if let Some(directory) = path.parent() {
        for named in &mut config.clusters {
            if let Some(cluster) = &mut named.cluster {
                resolve_relative(&mut cluster.certificate_authority, directory)?;
            }
        }
        for named in &mut config.auth_infos {
            if let Some(user) = &mut named.auth_info {
                resolve_relative(&mut user.client_certificate, directory)?;
                resolve_relative(&mut user.client_key, directory)?;
                resolve_relative(&mut user.token_file, directory)?;
            }
        }
    }
    Ok(config)
}

#[derive(Clone)]
pub(crate) struct PreparedKubeconfig {
    pub config: Option<Kubeconfig>,
    pub options: KubeConfigOptions,
    pub source: String,
    pub warning: Option<String>,
    pub selected_file: bool,
    pub explicit_selection: bool,
    pub pinned_fallback: Option<Kubeconfig>,
    pub fallback_source: String,
}

impl PreparedKubeconfig {
    pub(crate) fn unavailable(reason: &str) -> Self {
        Self {
            config: None,
            options: KubeConfigOptions::default(),
            source: "Unavailable: no verified Talos control-plane kubeconfig".into(),
            warning: Some(reason.into()),
            selected_file: false,
            explicit_selection: false,
            pinned_fallback: None,
            fallback_source: "Unavailable".into(),
        }
    }

    pub(crate) fn fallback_after_selected_failure(&mut self) -> Option<Kubeconfig> {
        if !self.selected_file {
            return None;
        }
        let pinned = self.pinned_fallback.clone()?;
        let selected_source = self.source.clone();
        self.source = format!("{} — selected file unavailable", self.fallback_source);
        self.warning = Some(format!(
            "{selected_source} could not be used for Kubernetes roster requests. Using {} instead; ambient configuration is not used.",
            self.fallback_source,
        ));
        self.config = Some(pinned.clone());
        self.options = KubeConfigOptions::default();
        self.selected_file = false;
        Some(pinned)
    }
}

/// Parse pinned node data on the same worker as file loading and CA validation.
pub(crate) fn parse_pinned_kubeconfig(data: &str) -> Option<Kubeconfig> {
    if data.len() as u64 > MAX_CONFIG_BYTES {
        return None;
    }
    Kubeconfig::from_yaml(data).ok()
}

/// Keep synchronous credential loading/auth plugins off the async executor so
/// the caller's deadline can expire. A timed-out blocking setup may finish later;
/// its result is discarded and never used for Kubernetes requests.
pub(crate) async fn run_roster_setup<T, F>(setup: F) -> Result<T, K8sError>
where
    T: Send + 'static,
    F: FnOnce() -> Result<T, K8sError> + Send + 'static,
{
    tokio::task::spawn_blocking(setup)
        .await
        .map_err(|_| K8sError::ClientCreate("Kubernetes credential setup worker failed".into()))?
}

/// Bound the selected attempt and its pinned retry independently. Ambient/default
/// roster behavior is unchanged; source state switches before a retry begins.
pub(crate) async fn attempt_with_pinned_retry<T, F, Fut>(
    prepared: &mut PreparedKubeconfig,
    limit: Duration,
    attempt: F,
) -> Result<T, K8sError>
where
    F: Fn(Kubeconfig, KubeConfigOptions) -> Fut,
    Fut: Future<Output = Result<T, K8sError>>,
{
    let config = prepared.config.clone().ok_or_else(|| {
        K8sError::KubeconfigFetch("No verified pinned control-plane kubeconfig is available".into())
    })?;
    if !prepared.explicit_selection {
        return attempt(config, prepared.options.clone()).await;
    }
    if !prepared.selected_file {
        return tokio::time::timeout(limit, attempt(config, prepared.options.clone()))
            .await
            .unwrap_or_else(|_| {
                Err(K8sError::ApiError(
                    "Pinned Kubernetes roster attempt timed out".into(),
                ))
            });
    }
    let selected = tokio::time::timeout(limit, attempt(config, prepared.options.clone())).await;
    let timed_out = selected.is_err();
    let outcome = selected.unwrap_or_else(|_| {
        Err(K8sError::ApiError(
            "Selected Kubernetes roster attempt timed out".into(),
        ))
    });
    if outcome.is_err()
        && let Some(pinned) = prepared.fallback_after_selected_failure()
    {
        if timed_out && let Some(warning) = &mut prepared.warning {
            warning.push_str(" The selected attempt timed out.");
        }
        return tokio::time::timeout(limit, attempt(pinned, KubeConfigOptions::default()))
            .await
            .unwrap_or_else(|_| {
                Err(K8sError::ApiError(
                    "Pinned Kubernetes roster retry timed out".into(),
                ))
            });
    }
    outcome
}

/// Pure source resolution apart from file reads. The injectable ambient reader
/// makes the explicit-mode boundary testable without changing process globals.
pub(crate) fn prepare_kubeconfig(
    selection: &KubeconfigSelection,
    pinned: Option<Kubeconfig>,
    node_ip: Option<&str>,
    ambient: impl FnOnce() -> Option<Kubeconfig>,
) -> PreparedKubeconfig {
    let (Some(pinned), Some(node_ip)) = (pinned, node_ip) else {
        let mut unavailable = PreparedKubeconfig::unavailable(match selection {
            KubeconfigSelection::File { .. } => {
                "Selected kubeconfig was not used: the pinned Talos control-plane kubeconfig is unavailable, so cluster identity cannot be verified. Kubernetes roster requests are disabled; ambient configuration is not used."
            }
            _ => {
                "Kubernetes roster requests are unavailable: no pinned Talos control-plane kubeconfig could be obtained."
            }
        });
        if matches!(selection, KubeconfigSelection::Automatic) {
            // Legacy automatic warnings only report a proven ambient mismatch.
            unavailable.warning = None;
        }
        if let KubeconfigSelection::File { path, context } = selection {
            unavailable.source = format!(
                "Unavailable: file {} (context: {}) is unverified",
                path.display(),
                context.as_deref().unwrap_or("file current-context")
            );
        }
        return unavailable;
    };
    let pinned_context = pinned.current_context.as_deref().unwrap_or("unspecified");
    let fallback_source = format!("Talos control plane {node_ip} (context: {pinned_context})");
    let mut prepared = PreparedKubeconfig {
        config: Some(pinned.clone()),
        options: KubeConfigOptions::default(),
        source: fallback_source.clone(),
        warning: None,
        selected_file: false,
        explicit_selection: !matches!(selection, KubeconfigSelection::Automatic),
        pinned_fallback: Some(pinned.clone()),
        fallback_source: fallback_source.clone(),
    };
    match selection {
        KubeconfigSelection::Automatic => {
            // Keep legacy automatic behavior: ambient config is diagnostic only,
            // and the roster always uses the pinned node's config.
            if let Some(ambient) = ambient()
                && let (Some(ambient_ca), Some(pinned_ca)) =
                    (inline_current_ca(&ambient), inline_current_ca(&pinned))
                && ambient_ca != pinned_ca
            {
                prepared.warning = Some(format!(
                    "KUBECONFIG points at a different cluster than this Talos context — ignoring it. Kubernetes roster requests use the kubeconfig from control plane node {node_ip} instead."
                ));
            }
        }
        KubeconfigSelection::TalosControlPlane => {}
        KubeconfigSelection::File { path, context } => {
            let validation = read_selected_file(path)
                .map_err(|_| "selected file is unreadable, oversized, or invalid")
                .and_then(|config| validate_selected(config, context.as_deref(), &pinned));
            match validation {
                Ok((config, context)) => {
                    prepared.source = format!(
                        "File {} (context: {context}; verified against Talos {node_ip})",
                        path.display()
                    );
                    prepared.options.context = Some(context);
                    prepared.config = Some(config);
                    prepared.selected_file = true;
                }
                Err(reason) => {
                    let requested_context = context.as_deref().unwrap_or("file current-context");
                    prepared.source = format!("{fallback_source} — selected file rejected");
                    prepared.warning = Some(format!(
                        "Selected kubeconfig {} (context: {requested_context}) was not used: {reason}. Kubernetes roster requests use the pinned Talos control plane {node_ip} instead; ambient configuration is not used.",
                        path.display()
                    ));
                }
            }
        }
    }
    prepared
}

fn inline_current_ca(config: &Kubeconfig) -> Option<&str> {
    let (_, cluster) = context_cluster(config, None).ok()?;
    cluster.certificate_authority_data.as_deref()
}

fn context_cluster<'a>(
    config: &'a Kubeconfig,
    requested: Option<&str>,
) -> Result<(&'a str, &'a Cluster), &'static str> {
    let name = requested
        .or(config.current_context.as_deref())
        .filter(|name| !name.is_empty())
        .ok_or("no current context is set; choose a named context")?;
    let mut contexts = config
        .contexts
        .iter()
        .filter(|context| context.name == name);
    let context = contexts.next().ok_or("chosen context does not exist")?;
    if contexts.next().is_some() {
        return Err("chosen context is ambiguous");
    }
    let context_data = context
        .context
        .as_ref()
        .ok_or("chosen context has no configuration")?;
    let mut clusters = config
        .clusters
        .iter()
        .filter(|cluster| cluster.name == context_data.cluster);
    let cluster = clusters
        .next()
        .ok_or("chosen context's cluster does not exist")?;
    if clusters.next().is_some() {
        return Err("chosen cluster is ambiguous");
    }
    let cluster = cluster
        .cluster
        .as_ref()
        .ok_or("chosen cluster has no configuration")?;
    Ok((context.name.as_str(), cluster))
}

fn validated_ca(cluster: &Cluster) -> Result<(Vec<Vec<u8>>, Vec<u8>), &'static str> {
    if cluster.insecure_skip_tls_verify == Some(true) {
        return Err("TLS certificate verification is disabled");
    }
    let server = cluster
        .server
        .as_deref()
        .ok_or("cluster server is missing")?;
    let authority = server
        .strip_prefix("https://")
        .ok_or("cluster server does not use HTTPS")?
        .split('/')
        .next()
        .unwrap_or_default();
    if authority.is_empty() || authority.contains('@') || authority.chars().any(char::is_whitespace)
    {
        return Err("cluster server is invalid");
    }
    let bytes = if let Some(data) = cluster.certificate_authority_data.as_deref() {
        if data.len() as u64 > MAX_CA_BYTES * 2 {
            return Err("certificate authority exceeds the supported size limit");
        }
        let compact = data
            .bytes()
            .filter(|byte| !byte.is_ascii_whitespace())
            .collect::<Vec<_>>();
        STANDARD
            .decode(compact)
            .map_err(|_| "certificate authority data is invalid base64")?
    } else if let Some(path) = cluster.certificate_authority.as_deref() {
        read_bounded_regular_file(Path::new(path), MAX_CA_BYTES)
            .map_err(|_| "certificate authority file is unreadable, non-regular, or oversized")?
    } else {
        return Err("certificate authority is missing");
    };
    if bytes.len() as u64 > MAX_CA_BYTES {
        return Err("certificate authority exceeds the supported size limit");
    }
    let mut remaining = bytes.as_slice();
    let mut certificates = Vec::new();
    loop {
        remaining = remaining.trim_ascii();
        if remaining.is_empty() {
            break;
        }
        if !remaining.starts_with(b"-----BEGIN CERTIFICATE-----") {
            return Err("certificate authority is not a valid PEM certificate bundle");
        }
        let (rest, pem) =
            parse_x509_pem(remaining).map_err(|_| "certificate authority PEM is invalid")?;
        if pem.label != "CERTIFICATE" {
            return Err("certificate authority contains a non-certificate block");
        }
        let (trailing, _) = parse_x509_certificate(&pem.contents)
            .map_err(|_| "certificate authority certificate is invalid")?;
        if !trailing.is_empty() {
            return Err("certificate authority certificate has trailing data");
        }
        certificates.push(pem.contents);
        remaining = rest;
    }
    if certificates.is_empty() {
        return Err("certificate authority is empty");
    }
    certificates.sort();
    certificates.dedup();
    Ok((certificates, bytes))
}

fn validate_selected(
    mut selected: Kubeconfig,
    requested: Option<&str>,
    pinned: &Kubeconfig,
) -> Result<(Kubeconfig, String), &'static str> {
    let (name, cluster) = context_cluster(&selected, requested)?;
    let name = name.to_owned();
    let (selected_ca, selected_pem) = validated_ca(cluster)?;
    let (_, pinned_cluster) = context_cluster(pinned, None)
        .map_err(|_| "pinned Talos kubeconfig has no usable current context")?;
    let (pinned_ca, _) = validated_ca(pinned_cluster)
        .map_err(|_| "pinned Talos kubeconfig has no verifiable TLS certificate authority")?;
    if selected_ca != pinned_ca {
        return Err("certificate authority identifies a different cluster");
    }
    // Freeze the exact CA bytes validated above so a referenced CA file cannot
    // change between identity validation and kube-rs client construction.
    let cluster_name = selected
        .contexts
        .iter()
        .find(|context| context.name == name)
        .and_then(|context| context.context.as_ref())
        .expect("validated context")
        .cluster
        .clone();
    let cluster = selected
        .clusters
        .iter_mut()
        .find(|cluster| cluster.name == cluster_name)
        .and_then(|cluster| cluster.cluster.as_mut())
        .expect("validated cluster");
    cluster.certificate_authority_data = Some(STANDARD.encode(selected_pem));
    cluster.certificate_authority = None;
    selected.current_context = Some(name.clone());
    Ok((selected, name))
}

#[cfg(test)]
mod tests {
    use super::*;
    use kube::config::{AuthInfo, Context, ExecConfig, NamedAuthInfo, NamedCluster, NamedContext};
    use std::sync::atomic::{AtomicU64, Ordering};

    // Public certificate only, no private key. Expiration is irrelevant to
    // offline certificate identity comparison (TLS validates it at request time).
    const CERTIFICATE: &str = "-----BEGIN CERTIFICATE-----\n\
MIIBUTCB2KADAgECAgkAtXGSKEzrTR4wCgYIKoZIzj0EAwIwEDEOMAwGA1UEAwwF\n\
YmVubm8wHhcNMTgxMTEzMDI1NDQwWhcNMTkxMTEzMDI1NDQwWjAQMQ4wDAYDVQQD\n\
DAViZW5ubzB2MBAGByqGSM49AgEGBSuBBAAiA2IABDhrHLVTMHC7GTyB/MNztToW\n\
ss2zlmvR62X1pQaBN6fYhBJE1XYa0V2C1fGGXj92MencOtXyfYVxn+DY07gyT/71\n\
HQ12TJOe90wwjy2/6N1W1jOv5HjphVT8JQlVNqAC+DAKBggqhkjOPQQDAgNoADBl\n\
AjAfzS1tmZ+GSAaXrsPcfAd1A9yfWVtB8tWxFNNo2j7/cL3puf2vQnlwV/0BoZZ1\n\
K4ICMQDaE0HemgYp6RPQzeb96a2gjlaEbwNy1B8O74fE24WRXiOUm4eln9wGQ3I1\n\
iV6NtZU=\n-----END CERTIFICATE-----\n";

    struct TemporaryDirectory(PathBuf);

    impl TemporaryDirectory {
        fn new() -> Self {
            static NEXT: AtomicU64 = AtomicU64::new(0);
            let path = std::env::temp_dir().join(format!(
                "talos-pilot-kubeconfig-{}-{}-{}",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_nanos(),
                NEXT.fetch_add(1, Ordering::Relaxed),
            ));
            fs::create_dir(&path).unwrap();
            Self(path)
        }

        fn save(&self, name: &str, config: &Kubeconfig) -> PathBuf {
            let path = self.0.join(name);
            fs::write(&path, serde_yaml::to_string(config).unwrap()).unwrap();
            path
        }
    }

    impl Drop for TemporaryDirectory {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn config(name: &str, certificate: &str) -> Kubeconfig {
        Kubeconfig {
            current_context: Some(name.into()),
            contexts: vec![NamedContext {
                name: name.into(),
                context: Some(Context {
                    cluster: name.into(),
                    ..Default::default()
                }),
            }],
            clusters: vec![NamedCluster {
                name: name.into(),
                cluster: Some(Cluster {
                    server: Some(format!("https://{name}.invalid:6443")),
                    certificate_authority_data: Some(STANDARD.encode(certificate)),
                    ..Default::default()
                }),
            }],
            ..Default::default()
        }
    }

    fn changed_certificate() -> String {
        let (_, pem) = parse_x509_pem(CERTIFICATE.as_bytes()).unwrap();
        let mut der = pem.contents;
        // Different certificate identity, without changing its ASN.1 structure.
        *der.last_mut().unwrap() ^= 1;
        format!(
            "-----BEGIN CERTIFICATE-----\n{}\n-----END CERTIFICATE-----\n",
            STANDARD.encode(der)
        )
    }

    fn cluster_mut(config: &mut Kubeconfig) -> &mut Cluster {
        config.clusters[0].cluster.as_mut().unwrap()
    }

    fn prepare_file(
        path: PathBuf,
        context: Option<&str>,
        pinned: Option<Kubeconfig>,
    ) -> PreparedKubeconfig {
        prepare_kubeconfig(
            &KubeconfigSelection::File {
                path,
                context: context.map(str::to_owned),
            },
            pinned,
            Some("10.0.0.1"),
            || panic!("explicit file selection must never consult ambient configuration"),
        )
    }

    #[test]
    fn matching_ca_uses_selected_config_context_and_endpoint() {
        let directory = TemporaryDirectory::new();
        let selected = config("selected", CERTIFICATE);
        let path = directory.save("config", &selected);
        let prepared = prepare_file(path.clone(), None, Some(config("pinned", CERTIFICATE)));
        assert!(prepared.selected_file);
        assert!(prepared.warning.is_none());
        assert_eq!(prepared.options.context.as_deref(), Some("selected"));
        assert_eq!(
            prepared.config.unwrap().clusters[0]
                .cluster
                .as_ref()
                .unwrap()
                .server,
            selected.clusters[0].cluster.as_ref().unwrap().server
        );
        assert!(prepared.source.contains(&path.display().to_string()));
        assert!(prepared.source.contains("context: selected"));
        assert!(prepared.source.contains("verified against Talos 10.0.0.1"));
    }

    #[test]
    fn certificate_comparison_ignores_pem_wrapping_crlf_and_bundle_order() {
        let other = changed_certificate();
        let pinned = config("pinned", &format!("{CERTIFICATE}{other}"));
        let selected = config(
            "selected",
            &format!("{other}{CERTIFICATE}{CERTIFICATE}").replace('\n', "\r\n"),
        );
        assert!(validate_selected(selected, None, &pinned).is_ok());
    }

    #[test]
    fn different_ca_falls_back_to_pinned_without_ambient() {
        let directory = TemporaryDirectory::new();
        let path = directory.save("config", &config("foreign", &changed_certificate()));
        let prepared = prepare_file(path, None, Some(config("pinned", CERTIFICATE)));
        assert!(!prepared.selected_file);
        assert_eq!(
            prepared.config.unwrap().current_context.as_deref(),
            Some("pinned")
        );
        assert!(prepared.source.contains("Talos control plane 10.0.0.1"));
        assert!(prepared.source.contains("selected file rejected"));
        let warning = prepared.warning.unwrap();
        assert!(warning.contains("different cluster"));
        assert!(warning.contains("ambient configuration is not used"));
        assert!(!warning.contains("BEGIN CERTIFICATE"));
        assert!(!warning.contains("workloads"));
    }

    #[test]
    fn named_context_overrides_foreign_file_current_context() {
        let directory = TemporaryDirectory::new();
        let mut selected = config("foreign", &changed_certificate());
        let matching = config("matching", CERTIFICATE);
        selected.contexts.extend(matching.contexts);
        selected.clusters.extend(matching.clusters);
        let path = directory.save("config", &selected);
        let prepared = prepare_file(
            path.clone(),
            Some("matching"),
            Some(config("pinned", CERTIFICATE)),
        );
        assert!(prepared.selected_file);
        assert_eq!(prepared.options.context.as_deref(), Some("matching"));
        assert_eq!(
            prepared.config.unwrap().current_context.as_deref(),
            Some("matching")
        );
        assert!(!prepare_file(path, None, Some(config("pinned", CERTIFICATE))).selected_file);
    }

    #[test]
    fn named_context_works_without_current_context_and_inspection_does_not_reject() {
        let directory = TemporaryDirectory::new();
        let mut selected = config("chosen", CERTIFICATE);
        selected.current_context = None;
        let path = directory.save("config", &selected);
        let info = inspect_kubeconfig(&path).unwrap();
        assert_eq!(info.contexts, vec!["chosen"]);
        assert!(info.current_context.is_none());
        assert!(
            prepare_file(
                path.clone(),
                Some("chosen"),
                Some(config("pinned", CERTIFICATE))
            )
            .selected_file
        );
        let prepared = prepare_file(path, None, Some(config("pinned", CERTIFICATE)));
        assert!(!prepared.selected_file);
        assert!(prepared.warning.unwrap().contains("choose a named context"));
    }

    #[test]
    fn invalid_current_context_can_be_overridden_and_contexts_are_sorted() {
        let directory = TemporaryDirectory::new();
        let mut selected = config("zeta", CERTIFICATE);
        selected
            .contexts
            .extend(config("alpha", CERTIFICATE).contexts);
        selected
            .clusters
            .extend(config("alpha", CERTIFICATE).clusters);
        selected.current_context = Some("missing".into());
        let path = directory.save("config", &selected);
        let info = inspect_kubeconfig(&path).unwrap();
        assert_eq!(info.contexts, vec!["alpha", "zeta"]);
        assert_eq!(info.current_context.as_deref(), Some("missing"));
        assert!(
            prepare_file(
                path.clone(),
                Some("alpha"),
                Some(config("pinned", CERTIFICATE))
            )
            .selected_file
        );
        assert!(
            !prepare_file(path.clone(), None, Some(config("pinned", CERTIFICATE))).selected_file
        );
        assert!(
            !prepare_file(path, Some("absent"), Some(config("pinned", CERTIFICATE))).selected_file
        );
    }

    #[test]
    fn missing_empty_or_ambiguous_contexts_fail_safely() {
        let pinned = config("pinned", CERTIFICATE);
        let mut selected = config("selected", CERTIFICATE);
        selected.contexts.clear();
        assert!(validate_selected(selected, Some("selected"), &pinned).is_err());
        let mut selected = config("selected", CERTIFICATE);
        selected.contexts.push(selected.contexts[0].clone());
        assert!(validate_selected(selected, None, &pinned).is_err());
        let mut selected = config("selected", CERTIFICATE);
        selected.current_context = Some(String::new());
        assert!(validate_selected(selected, None, &pinned).is_err());
        let mut selected = config("selected", CERTIFICATE);
        selected.clusters.clear();
        assert!(validate_selected(selected, None, &pinned).is_err());
        let mut selected = config("selected", CERTIFICATE);
        selected.clusters.push(selected.clusters[0].clone());
        assert!(validate_selected(selected, None, &pinned).is_err());
    }

    #[test]
    fn relative_ca_certificate_and_key_paths_are_resolved_and_ca_is_frozen() {
        let directory = TemporaryDirectory::new();
        fs::write(directory.0.join("ca.pem"), CERTIFICATE).unwrap();
        let mut selected = config("selected", CERTIFICATE);
        let cluster = cluster_mut(&mut selected);
        cluster.certificate_authority_data = None;
        cluster.certificate_authority = Some("ca.pem".into());
        selected.auth_infos.push(NamedAuthInfo {
            name: "user".into(),
            auth_info: Some(AuthInfo {
                client_certificate: Some("client.pem".into()),
                client_key: Some("client.key".into()),
                ..Default::default()
            }),
        });
        let path = directory.save("config", &selected);
        let loaded = read_selected_file(&path).unwrap();
        assert_eq!(
            loaded.clusters[0]
                .cluster
                .as_ref()
                .unwrap()
                .certificate_authority
                .as_deref(),
            directory.0.join("ca.pem").to_str()
        );
        let user = loaded.auth_infos[0].auth_info.as_ref().unwrap();
        assert_eq!(
            user.client_certificate.as_deref(),
            directory.0.join("client.pem").to_str()
        );
        assert_eq!(
            user.client_key.as_deref(),
            directory.0.join("client.key").to_str()
        );
        let prepared = prepare_file(path, None, Some(config("pinned", CERTIFICATE)));
        assert!(prepared.selected_file);
        let selected = prepared.config.unwrap();
        let cluster = selected.clusters[0].cluster.as_ref().unwrap();
        assert!(cluster.certificate_authority.is_none());
        assert_eq!(
            STANDARD
                .decode(cluster.certificate_authority_data.as_ref().unwrap())
                .unwrap(),
            CERTIFICATE.as_bytes()
        );
    }

    #[test]
    fn insecure_tls_or_non_https_server_is_rejected_before_credentials() {
        let pinned = config("pinned", CERTIFICATE);
        let mut selected = config("selected", CERTIFICATE);
        cluster_mut(&mut selected).insecure_skip_tls_verify = Some(true);
        assert!(validate_selected(selected, None, &pinned).is_err());
        let mut selected = config("selected", CERTIFICATE);
        cluster_mut(&mut selected).server = Some("http://selected.invalid".into());
        assert!(validate_selected(selected, None, &pinned).is_err());
        let mut selected = config("selected", CERTIFICATE);
        cluster_mut(&mut selected).server = Some("https://".into());
        assert!(validate_selected(selected, None, &pinned).is_err());
    }

    #[test]
    fn empty_missing_invalid_and_unreadable_ca_are_rejected() {
        let pinned = config("pinned", CERTIFICATE);
        for data in [
            Some(""),
            Some("not base64!"),
            Some("bm90IGEgY2VydGlmaWNhdGU="),
            None,
        ] {
            let mut selected = config("selected", CERTIFICATE);
            cluster_mut(&mut selected).certificate_authority_data = data.map(str::to_owned);
            assert!(validate_selected(selected, None, &pinned).is_err());
        }
        let directory = TemporaryDirectory::new();
        let mut selected = config("selected", CERTIFICATE);
        let cluster = cluster_mut(&mut selected);
        cluster.certificate_authority_data = None;
        cluster.certificate_authority =
            Some(directory.0.join("missing-ca.pem").display().to_string());
        assert!(validate_selected(selected, None, &pinned).is_err());
        let mut selected = config("selected", CERTIFICATE);
        cluster_mut(&mut selected).certificate_authority_data =
            Some(STANDARD.encode("-----BEGIN CERTIFICATE-----\nYWJj\n-----END CERTIFICATE-----\n"));
        assert!(validate_selected(selected, None, &pinned).is_err());
    }

    #[test]
    fn invalid_unreadable_empty_and_oversized_files_fall_back_safely() {
        let directory = TemporaryDirectory::new();
        let invalid = directory.0.join("invalid");
        fs::write(&invalid, "users: [secret-token\n").unwrap();
        let error = inspect_kubeconfig(&invalid).unwrap_err().to_string();
        assert!(!error.contains("secret-token"));
        let empty = directory.0.join("empty");
        fs::write(&empty, "").unwrap();
        let oversized = directory.0.join("oversized");
        fs::File::create(&oversized)
            .unwrap()
            .set_len(MAX_CONFIG_BYTES + 1)
            .unwrap();
        for path in [
            invalid,
            empty,
            oversized,
            directory.0.join("missing"),
            directory.0.clone(),
        ] {
            let prepared = prepare_file(path, None, Some(config("pinned", CERTIFICATE)));
            assert!(!prepared.selected_file);
            assert!(prepared.warning.is_some());
            assert_eq!(
                prepared.config.unwrap().current_context.as_deref(),
                Some("pinned")
            );
        }
    }

    #[test]
    fn missing_or_unverifiable_pinned_config_never_uses_selected_credentials() {
        let directory = TemporaryDirectory::new();
        let path = directory.save("config", &config("selected", CERTIFICATE));
        let prepared = prepare_file(path.clone(), None, None);
        assert!(prepared.config.is_none());
        assert!(prepared.source.starts_with("Unavailable"));
        assert!(
            prepared
                .warning
                .unwrap()
                .contains("cluster identity cannot be verified")
        );
        let mut pinned = config("pinned", CERTIFICATE);
        cluster_mut(&mut pinned).certificate_authority_data = None;
        let prepared = prepare_file(path, None, Some(pinned));
        assert!(!prepared.selected_file);
        assert!(
            prepared
                .warning
                .unwrap()
                .contains("no verifiable TLS certificate authority")
        );
    }

    #[test]
    fn explicit_talos_mode_never_consults_ambient() {
        let prepared = prepare_kubeconfig(
            &KubeconfigSelection::TalosControlPlane,
            Some(config("pinned", CERTIFICATE)),
            Some("10.0.0.1"),
            || panic!("explicit Talos mode must not read ambient configuration"),
        );
        assert!(!prepared.selected_file);
        assert!(prepared.warning.is_none());
        assert_eq!(
            prepared.config.unwrap().current_context.as_deref(),
            Some("pinned")
        );
        assert!(prepared.source.contains("context: pinned"));
    }

    #[test]
    fn automatic_mode_preserves_pinned_roster_and_ambient_mismatch_warning() {
        let pinned = config("pinned", CERTIFICATE);
        let foreign = config("foreign", &changed_certificate());
        let prepared = prepare_kubeconfig(
            &KubeconfigSelection::Automatic,
            Some(pinned.clone()),
            Some("10.0.0.1"),
            || Some(foreign),
        );
        assert_eq!(
            prepared.config.unwrap().current_context.as_deref(),
            Some("pinned")
        );
        assert!(
            prepared
                .warning
                .unwrap()
                .contains("KUBECONFIG points at a different cluster")
        );
        let prepared = prepare_kubeconfig(
            &KubeconfigSelection::Automatic,
            Some(pinned),
            Some("10.0.0.1"),
            || Some(config("same", CERTIFICATE)),
        );
        assert!(prepared.warning.is_none());
        assert!(!prepared.selected_file);
    }

    #[test]
    fn inspection_and_rejected_selection_never_execute_authentication_plugins() {
        let directory = TemporaryDirectory::new();
        let mut selected = config("foreign", &changed_certificate());
        selected.contexts[0].context.as_mut().unwrap().user = Some("plugin-user".into());
        selected.auth_infos.push(NamedAuthInfo {
            name: "plugin-user".into(),
            auth_info: Some(AuthInfo {
                exec: Some(ExecConfig {
                    command: Some(directory.0.join("must-not-run").display().to_string()),
                    api_version: None,
                    args: None,
                    env: None,
                    drop_env: None,
                    interactive_mode: None,
                    provide_cluster_info: false,
                    cluster: None,
                }),
                ..Default::default()
            }),
        });
        let path = directory.save("config", &selected);
        assert_eq!(inspect_kubeconfig(&path).unwrap().contexts, vec!["foreign"]);
        let prepared = prepare_file(path, None, Some(config("pinned", CERTIFICATE)));
        assert!(!prepared.selected_file);
        assert!(prepared.warning.unwrap().contains("different cluster"));
    }

    #[tokio::test]
    async fn verified_file_builds_kube_config_for_selected_endpoint_and_tls_roots() {
        let directory = TemporaryDirectory::new();
        let path = directory.save("config", &config("selected", CERTIFICATE));
        let prepared = prepare_file(path, None, Some(config("pinned", CERTIFICATE)));
        let config =
            kube::Config::from_custom_kubeconfig(prepared.config.unwrap(), &prepared.options)
                .await
                .unwrap();
        assert_eq!(config.cluster_url.host(), Some("selected.invalid"));
        assert_eq!(config.cluster_url.port_u16(), Some(6443));
        assert!(!config.accept_invalid_certs);
        let (_, certificate) = parse_x509_pem(CERTIFICATE.as_bytes()).unwrap();
        assert_eq!(config.root_cert, Some(vec![certificate.contents]));
    }

    #[test]
    fn pinned_parsing_is_bounded_and_does_not_report_document_contents() {
        assert!(parse_pinned_kubeconfig("users: [secret-token").is_none());
        assert!(parse_pinned_kubeconfig(&" ".repeat(MAX_CONFIG_BYTES as usize + 1)).is_none());
        let yaml = serde_yaml::to_string(&config("pinned", CERTIFICATE)).unwrap();
        assert_eq!(
            parse_pinned_kubeconfig(&yaml)
                .unwrap()
                .current_context
                .as_deref(),
            Some("pinned")
        );
    }

    #[test]
    fn automatic_unavailable_source_does_not_add_new_legacy_warnings() {
        let prepared = prepare_kubeconfig(
            &KubeconfigSelection::Automatic,
            None,
            Some("10.0.0.1"),
            || panic!("no ambient read is needed without pinned identity"),
        );
        assert!(prepared.config.is_none());
        assert!(prepared.warning.is_none());
    }

    #[cfg(unix)]
    #[test]
    fn explicit_paths_reject_devices_and_accept_symlinks_to_regular_files() {
        assert!(inspect_kubeconfig(Path::new("/dev/null")).is_err());
        let directory = TemporaryDirectory::new();
        let path = directory.save("config", &config("selected", CERTIFICATE));
        let link = directory.0.join("linked-config");
        std::os::unix::fs::symlink(path, &link).unwrap();
        assert_eq!(
            inspect_kubeconfig(&link).unwrap().contexts,
            vec!["selected"]
        );
        assert!(prepare_file(link, None, Some(config("pinned", CERTIFICATE))).selected_file);
    }

    #[test]
    fn selected_request_failure_only_retries_pinned_and_updates_effective_identity() {
        let directory = TemporaryDirectory::new();
        let path = directory.save("config", &config("selected", CERTIFICATE));
        let mut prepared = prepare_file(path.clone(), None, Some(config("pinned", CERTIFICATE)));
        let fallback = prepared.fallback_after_selected_failure().unwrap();
        assert_eq!(fallback.current_context.as_deref(), Some("pinned"));
        assert_eq!(
            prepared.config.as_ref().unwrap().current_context.as_deref(),
            Some("pinned")
        );
        assert!(!prepared.selected_file);
        assert!(prepared.options.context.is_none());
        assert!(prepared.source.contains("Talos control plane 10.0.0.1"));
        assert!(prepared.source.contains("selected file unavailable"));
        let warning = prepared.warning.as_deref().unwrap();
        assert!(warning.contains(&path.display().to_string()));
        assert!(warning.contains("context: selected"));
        assert!(warning.contains("ambient configuration is not used"));
        assert!(prepared.fallback_after_selected_failure().is_none());
        assert!(
            prepare_file(path, None, None)
                .fallback_after_selected_failure()
                .is_none()
        );
    }

    #[tokio::test]
    async fn rejected_file_and_explicit_talos_attempts_are_bounded_without_losing_diagnostics() {
        let directory = TemporaryDirectory::new();
        let mut rejected = prepare_file(
            directory.0.join("missing"),
            None,
            Some(config("pinned", CERTIFICATE)),
        );
        let original_warning = rejected.warning.clone();
        let error = attempt_with_pinned_retry::<(), _, _>(
            &mut rejected,
            Duration::from_millis(1),
            |_, _| std::future::pending(),
        )
        .await
        .unwrap_err();
        assert!(
            error
                .to_string()
                .contains("Pinned Kubernetes roster attempt timed out")
        );
        assert_eq!(rejected.warning, original_warning);
        assert!(rejected.source.contains("selected file rejected"));
        let mut talos = prepare_kubeconfig(
            &KubeconfigSelection::TalosControlPlane,
            Some(config("pinned", CERTIFICATE)),
            Some("10.0.0.1"),
            || panic!("must not use ambient"),
        );
        assert!(
            attempt_with_pinned_retry::<(), _, _>(&mut talos, Duration::from_millis(1), |_, _| {
                std::future::pending()
            },)
            .await
            .is_err()
        );
    }

    #[tokio::test]
    async fn blocking_credential_setup_does_not_prevent_deadline_or_pinned_retry() {
        let directory = TemporaryDirectory::new();
        let path = directory.save("config", &config("selected", CERTIFICATE));
        let mut prepared = prepare_file(path, None, Some(config("pinned", CERTIFICATE)));
        use std::sync::atomic::{AtomicBool, Ordering};
        let release = std::sync::Arc::new(AtomicBool::new(false));
        let result =
            attempt_with_pinned_retry(&mut prepared, Duration::from_millis(50), |config, _| {
                let release = release.clone();
                async move {
                    run_roster_setup(move || {
                        if config.current_context.as_deref() == Some("selected") {
                            while !release.load(Ordering::Acquire) {
                                std::thread::park_timeout(Duration::from_millis(1));
                            }
                        }
                        Ok(config.current_context.unwrap())
                    })
                    .await
                }
            })
            .await;
        // Release even if assertions fail, so the deliberately blocked setup
        // cannot prevent the test runtime from shutting down.
        release.store(true, Ordering::Release);
        assert_eq!(result.unwrap(), "pinned");
        assert!(prepared.warning.unwrap().contains("timed out"));
        assert!(prepared.source.contains("selected file unavailable"));
    }

    #[tokio::test]
    async fn pending_selected_attempt_times_out_then_retries_pinned_with_new_identity() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        let directory = TemporaryDirectory::new();
        let path = directory.save("config", &config("selected", CERTIFICATE));
        let mut prepared = prepare_file(path, None, Some(config("pinned", CERTIFICATE)));
        let calls = AtomicUsize::new(0);
        let outcome = attempt_with_pinned_retry(
            &mut prepared,
            Duration::from_millis(1),
            |config, options| {
                calls.fetch_add(1, Ordering::SeqCst);
                async move {
                    if config.current_context.as_deref() == Some("selected") {
                        assert_eq!(options.context.as_deref(), Some("selected"));
                        std::future::pending::<Result<(), K8sError>>().await
                    } else {
                        assert_eq!(config.current_context.as_deref(), Some("pinned"));
                        assert!(options.context.is_none());
                        Ok(())
                    }
                }
            },
        )
        .await;
        assert!(outcome.is_ok());
        assert_eq!(calls.load(Ordering::SeqCst), 2);
        assert!(prepared.source.contains("Talos control plane 10.0.0.1"));
        assert!(prepared.source.contains("selected file unavailable"));
        assert!(
            prepared
                .warning
                .as_deref()
                .unwrap()
                .contains("selected attempt timed out")
        );
        assert_eq!(
            prepared.config.unwrap().current_context.as_deref(),
            Some("pinned")
        );
    }

    #[tokio::test]
    async fn pending_pinned_retry_has_its_own_deadline_and_sanitized_error() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        let directory = TemporaryDirectory::new();
        let path = directory.save("config", &config("selected", CERTIFICATE));
        let mut prepared = prepare_file(path, None, Some(config("pinned", CERTIFICATE)));
        let calls = AtomicUsize::new(0);
        let outcome = attempt_with_pinned_retry(&mut prepared, Duration::from_millis(1), |_, _| {
            calls.fetch_add(1, Ordering::SeqCst);
            std::future::pending::<Result<(), K8sError>>()
        })
        .await;
        assert_eq!(calls.load(Ordering::SeqCst), 2);
        assert_eq!(
            outcome.unwrap_err().to_string(),
            "K8s API error: Pinned Kubernetes roster retry timed out"
        );
        assert!(!prepared.selected_file);
        assert!(
            prepared
                .warning
                .as_deref()
                .unwrap()
                .contains("selected attempt timed out")
        );
    }

    #[test]
    fn capped_reader_stops_at_limit_plus_one_even_without_a_known_file_length() {
        struct EndlessReader(usize);
        impl Read for EndlessReader {
            fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
                buffer.fill(0);
                self.0 += buffer.len();
                Ok(buffer.len())
            }
        }
        let mut reader = EndlessReader(0);
        assert!(read_capped(&mut reader, 32).is_err());
        assert_eq!(reader.0, 33);
        assert_eq!(
            read_capped(std::io::Cursor::new(vec![0; 32]), 32)
                .unwrap()
                .len(),
            32
        );
    }

    #[test]
    fn opened_file_growth_cannot_escape_the_read_cap() {
        let directory = TemporaryDirectory::new();
        let path = directory.0.join("growing");
        fs::write(&path, b"abcd").unwrap();
        let opened = fs::File::open(&path).unwrap();
        assert_eq!(opened.metadata().unwrap().len(), 4);
        fs::OpenOptions::new()
            .write(true)
            .open(&path)
            .unwrap()
            .set_len(128)
            .unwrap();
        assert!(read_capped(opened, 8).is_err());
        assert!(read_bounded_regular_file(&path, 8).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn fifo_replacing_a_regular_config_or_ca_is_rejected_without_a_writer() {
        use std::os::unix::ffi::OsStrExt;
        let directory = TemporaryDirectory::new();
        let path = directory.0.join("replaced-file");
        fs::write(&path, "previous regular file").unwrap();
        assert!(fs::metadata(&path).unwrap().is_file());
        fs::remove_file(&path).unwrap();
        let c_path = std::ffi::CString::new(path.as_os_str().as_bytes()).unwrap();
        // SAFETY: c_path is NUL terminated and remains alive for the syscall.
        assert_eq!(unsafe { libc::mkfifo(c_path.as_ptr(), 0o600) }, 0);
        assert!(inspect_kubeconfig(&path).is_err());
        let mut selected = config("selected", CERTIFICATE);
        let cluster = cluster_mut(&mut selected);
        cluster.certificate_authority_data = None;
        cluster.certificate_authority = Some(path.display().to_string());
        assert!(validated_ca(cluster).is_err());
    }

    #[test]
    fn frozen_file_parser_supports_utf8_bom_utf16_and_multidocument_merging() {
        let directory = TemporaryDirectory::new();
        let mut first = config("first", CERTIFICATE);
        first.auth_infos.push(NamedAuthInfo {
            name: "relative-user".into(),
            auth_info: Some(AuthInfo {
                client_certificate: Some("client.pem".into()),
                client_key: Some("client.key".into()),
                token_file: Some("token".into()),
                ..Default::default()
            }),
        });
        let mut second = config("second", CERTIFICATE);
        second.current_context = None;
        cluster_mut(&mut second).certificate_authority_data = None;
        cluster_mut(&mut second).certificate_authority = Some("ca.pem".into());
        let yaml = format!(
            "{}\n---\n{}",
            serde_yaml::to_string(&first).unwrap(),
            serde_yaml::to_string(&second).unwrap()
        );
        let mut utf8_bom = vec![0xEF, 0xBB, 0xBF];
        utf8_bom.extend_from_slice(yaml.as_bytes());
        let mut utf16_le = vec![0xFF, 0xFE];
        let mut utf16_be = vec![0xFE, 0xFF];
        for word in yaml.encode_utf16() {
            utf16_le.extend_from_slice(&word.to_le_bytes());
            utf16_be.extend_from_slice(&word.to_be_bytes());
        }
        for (index, bytes) in [yaml.into_bytes(), utf8_bom, utf16_le, utf16_be]
            .into_iter()
            .enumerate()
        {
            let path = directory.0.join(format!("config-{index}"));
            fs::write(&path, bytes).unwrap();
            let loaded = read_selected_file(&path).unwrap();
            assert_eq!(loaded.current_context.as_deref(), Some("first"));
            assert_eq!(
                inspect_kubeconfig(&path).unwrap().contexts,
                vec!["first", "second"]
            );
            assert_eq!(
                loaded.clusters[1]
                    .cluster
                    .as_ref()
                    .unwrap()
                    .certificate_authority
                    .as_deref(),
                directory.0.join("ca.pem").to_str()
            );
            let user = loaded.auth_infos[0].auth_info.as_ref().unwrap();
            assert_eq!(
                user.client_certificate.as_deref(),
                directory.0.join("client.pem").to_str()
            );
            assert_eq!(
                user.client_key.as_deref(),
                directory.0.join("client.key").to_str()
            );
            assert_eq!(
                user.token_file.as_deref(),
                directory.0.join("token").to_str()
            );
        }
        assert!(decode_kubeconfig(vec![0xFF, 0xFE, 0]).is_err());
        assert!(decode_kubeconfig(vec![0xFE, 0xFF, 0xD8, 0]).is_err());
    }

    #[test]
    fn ca_file_read_enforces_the_same_descriptor_bound() {
        let directory = TemporaryDirectory::new();
        let path = directory.0.join("oversized-ca.pem");
        fs::File::create(&path)
            .unwrap()
            .set_len(MAX_CA_BYTES + 1)
            .unwrap();
        let mut selected = config("selected", CERTIFICATE);
        let cluster = cluster_mut(&mut selected);
        cluster.certificate_authority_data = None;
        cluster.certificate_authority = Some(path.display().to_string());
        assert!(validated_ca(cluster).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn non_utf8_paths_preserve_basename_and_reject_unrepresentable_references() {
        use std::os::unix::ffi::OsStringExt;
        let directory = TemporaryDirectory::new();
        let basename = std::ffi::OsString::from_vec(b"config-\xFF".to_vec());
        let path = directory.0.join(&basename);
        assert_eq!(path.file_name(), Some(basename.as_os_str()));
        // APFS rejects invalid UTF-8 names at creation; Linux filesystems allow
        // exercising the actual read in addition to portable PathBuf semantics.
        #[cfg(not(target_os = "macos"))]
        {
            fs::write(
                &path,
                serde_yaml::to_string(&config("selected", CERTIFICATE)).unwrap(),
            )
            .unwrap();
            assert_eq!(
                inspect_kubeconfig(&path).unwrap().contexts,
                vec!["selected"]
            );
        }
        let bad_parent = directory
            .0
            .join(std::ffi::OsString::from_vec(b"directory-\xFF".to_vec()));
        let mut reference = Some("client.key".into());
        assert!(resolve_relative(&mut reference, &bad_parent).is_err());
        assert_eq!(reference.as_deref(), Some("client.key"));
    }

    #[cfg(unix)]
    #[test]
    fn symlink_uses_selected_parent_for_relative_references_not_target_parent() {
        let directory = TemporaryDirectory::new();
        let target_parent = directory.0.join("target");
        fs::create_dir(&target_parent).unwrap();
        let mut selected = config("selected", CERTIFICATE);
        cluster_mut(&mut selected).certificate_authority_data = None;
        cluster_mut(&mut selected).certificate_authority = Some("ca.pem".into());
        let target = target_parent.join("config");
        fs::write(&target, serde_yaml::to_string(&selected).unwrap()).unwrap();
        fs::write(directory.0.join("ca.pem"), CERTIFICATE).unwrap();
        let link = directory.0.join("linked-config");
        std::os::unix::fs::symlink(target, &link).unwrap();
        let loaded = read_selected_file(&link).unwrap();
        assert_eq!(
            loaded.clusters[0]
                .cluster
                .as_ref()
                .unwrap()
                .certificate_authority
                .as_deref(),
            directory.0.join("ca.pem").to_str()
        );
        assert!(prepare_file(link, None, Some(config("pinned", CERTIFICATE))).selected_file);
    }
}
