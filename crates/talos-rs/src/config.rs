//! Talos configuration parsing
//!
//! Parses the talosconfig file format used by talosctl.

use crate::error::TalosError;
use serde::Deserialize;
use std::collections::HashMap;
use std::ffi::OsStr;
use std::path::{Path, PathBuf};

/// Talos client configuration (matches talosconfig format)
#[derive(Debug, Clone, Deserialize)]
pub struct TalosConfig {
    /// Current context name
    pub context: String,
    /// Available contexts
    pub contexts: HashMap<String, Context>,
}

/// A single context in the talosconfig
#[derive(Debug, Clone, Deserialize)]
pub struct Context {
    /// API endpoints (e.g., "127.0.0.1:50000")
    pub endpoints: Vec<String>,
    /// Target nodes (optional, defaults to endpoints)
    #[serde(default)]
    pub nodes: Vec<String>,
    /// CA certificate (base64 encoded PEM)
    pub ca: String,
    /// Client certificate (base64 encoded PEM)
    pub crt: String,
    /// Client private key (base64 encoded PEM)
    pub key: String,
}

impl TalosConfig {
    /// Load configuration from the default location (~/.talos/config)
    pub fn load_default() -> Result<Self, TalosError> {
        let path = Self::default_path()?;
        tracing::debug!("Loading talosconfig from: {:?}", path);
        Self::load_from(&path)
    }

    /// Load configuration from a specific path
    pub fn load_from(path: &PathBuf) -> Result<Self, TalosError> {
        tracing::debug!("load_from called with path: {:?}", path);
        if !path.exists() {
            return Err(TalosError::ConfigNotFound(path.display().to_string()));
        }
        let content = std::fs::read_to_string(path)?;
        let config = Self::parse(&content)?;
        tracing::debug!(
            "Loaded config with {} contexts: {:?}",
            config.contexts.len(),
            config.contexts.keys().collect::<Vec<_>>()
        );
        Ok(config)
    }

    /// Parses a Talos configuration snapshot without reading files or connecting.
    ///
    /// Callers loading untrusted paths can enforce file type and size limits
    /// before handing the in-memory UTF-8 configuration to this parser.
    pub fn parse(content: &str) -> Result<Self, TalosError> {
        Ok(serde_yaml::from_str(content)?)
    }

    /// Get the default config path (from $TALOSCONFIG env var or ~/.talos/config)
    ///
    /// Reads the process environment; a caller that holds the values already
    /// uses [`Self::default_path_in`].
    pub fn default_path() -> Result<PathBuf, TalosError> {
        let talosconfig = std::env::var_os("TALOSCONFIG");
        Self::default_path_in(talosconfig.as_deref(), std::env::home_dir().as_deref())
    }

    /// The default config path from the values `default_path` reads: the
    /// `TALOSCONFIG` variable, and the home directory for `~/.talos/config`.
    ///
    /// A `TALOSCONFIG` that is set wins even when empty, which yields an empty
    /// path; one that is not valid UTF-8 counts as unset. With neither a usable
    /// variable nor a home directory the path is unknown.
    pub fn default_path_in(
        talosconfig: Option<&OsStr>,
        home: Option<&Path>,
    ) -> Result<PathBuf, TalosError> {
        if let Some(talosconfig) = talosconfig.and_then(OsStr::to_str) {
            tracing::debug!("TALOSCONFIG env var found: {}", talosconfig);
            return Ok(PathBuf::from(talosconfig));
        }

        tracing::debug!("TALOSCONFIG env var not set, using default path");
        // Fallback to default location
        let home = home.ok_or(TalosError::NoHomeDirectory)?;
        Ok(home.join(".talos").join("config"))
    }

    /// Get the current context
    pub fn current_context(&self) -> Option<&Context> {
        self.contexts.get(&self.context)
    }

    /// List all context names
    pub fn context_names(&self) -> Vec<&str> {
        self.contexts.keys().map(|s| s.as_str()).collect()
    }

    /// Get a specific context by name
    pub fn get_context(&self, name: &str) -> Result<&Context, TalosError> {
        self.contexts
            .get(name)
            .ok_or_else(|| TalosError::ContextNotFound(name.to_string()))
    }
}

impl Context {
    /// Decode the CA certificate from base64
    pub fn ca_pem(&self) -> Result<Vec<u8>, TalosError> {
        use base64::Engine;
        Ok(base64::engine::general_purpose::STANDARD.decode(&self.ca)?)
    }

    /// Decode the client certificate from base64
    pub fn client_cert_pem(&self) -> Result<Vec<u8>, TalosError> {
        use base64::Engine;
        Ok(base64::engine::general_purpose::STANDARD.decode(&self.crt)?)
    }

    /// Decode the client key from base64
    pub fn client_key_pem(&self) -> Result<Vec<u8>, TalosError> {
        use base64::Engine;
        Ok(base64::engine::general_purpose::STANDARD.decode(&self.key)?)
    }

    /// Get all endpoint URLs (normalized).
    ///
    /// Used to build a load-balanced channel that fails over across every
    /// configured endpoint, mirroring `talosctl`, which load balances and fails
    /// over between the endpoints in a context.
    pub fn endpoint_urls(&self) -> Vec<String> {
        self.endpoints
            .iter()
            .map(|e| normalize_endpoint(e))
            .collect()
    }

    /// Get target nodes, falling back to endpoints if not specified
    pub fn target_nodes(&self) -> &[String] {
        if self.nodes.is_empty() {
            &self.endpoints
        } else {
            &self.nodes
        }
    }
}

/// Normalize an endpoint string into a full `https://host:port` URL.
///
/// Handles bare IPv4/hostname (adds the default Talos API port 50000), IPv6
/// with or without brackets, an existing `http(s)://` scheme, and explicit ports.
fn normalize_endpoint(e: &str) -> String {
    if e.starts_with("https://") || e.starts_with("http://") {
        e.to_string()
    } else if e.starts_with('[') {
        // IPv6 address with brackets - check if port is specified
        if e.contains("]:") {
            // Has port specified: [::1]:50000
            format!("https://{}", e)
        } else {
            // No port: [::1] -> add default port
            format!("https://{}:50000", e)
        }
    } else if e.contains("::") || e.matches(':').count() > 1 {
        // Raw IPv6 address without brackets - add brackets and default port
        format!("https://[{}]:50000", e)
    } else if e.contains(':') {
        // IPv4 or hostname with port specified
        format!("https://{}", e)
    } else {
        // IPv4 or hostname without port - add default Talos API port
        format!("https://{}:50000", e)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_config() {
        let yaml = r#"
context: default
contexts:
  default:
    endpoints:
      - 192.0.2.100:50000
    ca: Y2EtY2VydA==
    crt: Y2xpZW50LWNlcnQ=
    key: Y2xpZW50LWtleQ==
"#;
        let config = TalosConfig::parse(yaml).unwrap();
        assert_eq!(config.context, "default");
        assert_eq!(config.contexts.len(), 1);

        let ctx = config.current_context().unwrap();
        assert_eq!(ctx.endpoints, vec!["192.0.2.100:50000"]);
    }

    #[test]
    fn in_memory_parser_rejects_invalid_schema_without_reading_a_path() {
        assert!(TalosConfig::parse("context: [invalid]\ncontexts: {}").is_err());
        assert!(TalosConfig::parse("context: missing-required-contexts").is_err());
    }

    #[test]
    fn test_context_names() {
        let yaml = r#"
context: prod
contexts:
  dev:
    endpoints: ["dev.example.com:50000"]
    ca: YQ==
    crt: Yg==
    key: Yw==
  prod:
    endpoints: ["prod.example.com:50000"]
    ca: ZA==
    crt: ZQ==
    key: Zg==
"#;
        let config: TalosConfig = serde_yaml::from_str(yaml).unwrap();
        let names = config.context_names();
        assert!(names.contains(&"dev"));
        assert!(names.contains(&"prod"));
    }

    #[test]
    fn test_endpoint_url() {
        // Test endpoint with explicit port
        let ctx = Context {
            endpoints: vec!["192.0.2.100:50000".to_string()],
            nodes: vec![],
            ca: "YQ==".to_string(),
            crt: "Yg==".to_string(),
            key: "Yw==".to_string(),
        };
        assert_eq!(
            ctx.endpoint_urls().into_iter().next(),
            Some("https://192.0.2.100:50000".to_string())
        );

        let ctx2 = Context {
            endpoints: vec!["node1.example.com".to_string()],
            nodes: vec![],
            ca: "YQ==".to_string(),
            crt: "Yg==".to_string(),
            key: "Yw==".to_string(),
        };
        assert_eq!(
            ctx2.endpoint_urls().into_iter().next(),
            Some("https://node1.example.com:50000".to_string())
        );

        let ctx3 = Context {
            endpoints: vec!["https://192.0.2.100:50000".to_string()],
            nodes: vec![],
            ca: "YQ==".to_string(),
            crt: "Yg==".to_string(),
            key: "Yw==".to_string(),
        };
        assert_eq!(
            ctx3.endpoint_urls().into_iter().next(),
            Some("https://192.0.2.100:50000".to_string())
        );
    }

    #[test]
    fn test_endpoint_urls_multiple() {
        // Multiple endpoints in mixed formats should all be normalized and returned.
        let ctx = Context {
            endpoints: vec![
                "192.0.2.9".to_string(),
                "192.0.2.216:50000".to_string(),
                "https://talos.example.com:6443".to_string(),
            ],
            nodes: vec![],
            ca: "YQ==".to_string(),
            crt: "Yg==".to_string(),
            key: "Yw==".to_string(),
        };
        assert_eq!(
            ctx.endpoint_urls(),
            vec![
                "https://192.0.2.9:50000".to_string(),
                "https://192.0.2.216:50000".to_string(),
                "https://talos.example.com:6443".to_string(),
            ]
        );
    }

    #[test]
    fn test_endpoint_urls_empty() {
        let ctx = Context {
            endpoints: vec![],
            nodes: vec![],
            ca: "YQ==".to_string(),
            crt: "Yg==".to_string(),
            key: "Yw==".to_string(),
        };
        assert!(ctx.endpoint_urls().is_empty());
    }

    #[test]
    fn test_endpoint_url_with_ipv6() {
        // IPv6 with brackets and port
        let ctx = Context {
            endpoints: vec!["[2001:db8::1]:50000".to_string()],
            nodes: vec![],
            ca: "YQ==".to_string(),
            crt: "Yg==".to_string(),
            key: "Yw==".to_string(),
        };
        assert_eq!(
            ctx.endpoint_urls().into_iter().next(),
            Some("https://[2001:db8::1]:50000".to_string())
        );

        // Raw IPv6 without brackets - should add brackets and default port
        let ctx2 = Context {
            endpoints: vec!["2001:db8::1".to_string()],
            nodes: vec![],
            ca: "YQ==".to_string(),
            crt: "Yg==".to_string(),
            key: "Yw==".to_string(),
        };
        assert_eq!(
            ctx2.endpoint_urls().into_iter().next(),
            Some("https://[2001:db8::1]:50000".to_string())
        );

        // IPv6 with brackets and port
        let ctx3 = Context {
            endpoints: vec!["[::1]:50000".to_string()],
            nodes: vec![],
            ca: "YQ==".to_string(),
            crt: "Yg==".to_string(),
            key: "Yw==".to_string(),
        };
        assert_eq!(
            ctx3.endpoint_urls().into_iter().next(),
            Some("https://[::1]:50000".to_string())
        );

        // IPv6 with brackets but no port - should add default port
        let ctx4 = Context {
            endpoints: vec!["[::1]".to_string()],
            nodes: vec![],
            ca: "YQ==".to_string(),
            crt: "Yg==".to_string(),
            key: "Yw==".to_string(),
        };
        assert_eq!(
            ctx4.endpoint_urls().into_iter().next(),
            Some("https://[::1]:50000".to_string())
        );
    }

    #[test]
    fn test_endpoint_url_with_custom_port() {
        let ctx = Context {
            endpoints: vec!["192.0.2.100:443".to_string()],
            nodes: vec![],
            ca: "YQ==".to_string(),
            crt: "Yg==".to_string(),
            key: "Yw==".to_string(),
        };
        assert_eq!(
            ctx.endpoint_urls().into_iter().next(),
            Some("https://192.0.2.100:443".to_string())
        );
    }

    #[test]
    fn test_endpoint_url_with_hostname() {
        let ctx = Context {
            endpoints: vec!["talos.example.com".to_string()],
            nodes: vec![],
            ca: "YQ==".to_string(),
            crt: "Yg==".to_string(),
            key: "Yw==".to_string(),
        };
        assert_eq!(
            ctx.endpoint_urls().into_iter().next(),
            Some("https://talos.example.com:50000".to_string())
        );

        let ctx2 = Context {
            endpoints: vec!["talos.example.com:8443".to_string()],
            nodes: vec![],
            ca: "YQ==".to_string(),
            crt: "Yg==".to_string(),
            key: "Yw==".to_string(),
        };
        assert_eq!(
            ctx2.endpoint_urls().into_iter().next(),
            Some("https://talos.example.com:8443".to_string())
        );
    }

    #[test]
    fn test_target_nodes_fallback() {
        let ctx = Context {
            endpoints: vec!["192.0.2.100:50000".to_string()],
            nodes: vec![],
            ca: "YQ==".to_string(),
            crt: "Yg==".to_string(),
            key: "Yw==".to_string(),
        };
        assert_eq!(ctx.target_nodes(), &["192.0.2.100:50000"]);

        let ctx2 = Context {
            endpoints: vec!["127.0.0.1:50000".to_string()],
            nodes: vec!["192.0.2.101".to_string(), "192.0.2.102".to_string()],
            ca: "YQ==".to_string(),
            crt: "Yg==".to_string(),
            key: "Yw==".to_string(),
        };
        assert_eq!(ctx2.target_nodes(), &["192.0.2.101", "192.0.2.102"]);
    }

    #[test]
    fn test_empty_endpoints() {
        let ctx = Context {
            endpoints: vec![],
            nodes: vec![],
            ca: "YQ==".to_string(),
            crt: "Yg==".to_string(),
            key: "Yw==".to_string(),
        };
        assert_eq!(ctx.endpoint_urls().into_iter().next(), None);
    }

    #[test]
    fn default_path_in_uses_talosconfig_when_set() {
        let path = TalosConfig::default_path_in(
            Some(OsStr::new("/custom/path/to/talosconfig")),
            Some(Path::new("/home/ana")),
        )
        .unwrap();
        assert_eq!(path, PathBuf::from("/custom/path/to/talosconfig"));
    }

    #[test]
    fn default_path_in_falls_back_to_the_home_directory() {
        let path = TalosConfig::default_path_in(None, Some(Path::new("/home/ana"))).unwrap();
        assert_eq!(path, PathBuf::from("/home/ana/.talos/config"));
    }

    #[test]
    fn default_path_in_keeps_an_empty_talosconfig() {
        let path = TalosConfig::default_path_in(Some(OsStr::new("")), Some(Path::new("/home/ana")))
            .unwrap();
        assert_eq!(path, PathBuf::new());
    }

    #[test]
    fn default_path_in_without_home_or_variable_is_an_error() {
        assert!(matches!(
            TalosConfig::default_path_in(None, None),
            Err(TalosError::NoHomeDirectory)
        ));
        // A variable still answers when there is no home.
        assert_eq!(
            TalosConfig::default_path_in(Some(OsStr::new("/t")), None).unwrap(),
            PathBuf::from("/t")
        );
    }

    #[cfg(unix)]
    #[test]
    fn default_path_in_treats_a_talosconfig_that_is_not_utf8_as_unset() {
        use std::os::unix::ffi::OsStrExt;
        let path = TalosConfig::default_path_in(
            Some(OsStr::from_bytes(&[0xff, 0xfe])),
            Some(Path::new("/home/ana")),
        )
        .unwrap();
        assert_eq!(path, PathBuf::from("/home/ana/.talos/config"));
    }

    /// The wrapper reads the real environment; it never sets any.
    #[test]
    fn default_path_reads_the_process_environment() {
        let expected = TalosConfig::default_path_in(
            std::env::var_os("TALOSCONFIG").as_deref(),
            std::env::home_dir().as_deref(),
        )
        .ok();
        assert_eq!(TalosConfig::default_path().ok(), expected);
    }
}
