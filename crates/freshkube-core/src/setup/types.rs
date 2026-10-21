//! The parts of `freshkube.yaml` that are not the workspace's own: Tekton
//! Results, Coroot and the metrics source, per entry and for the whole
//! workspace, and the supply chain's trust.
//!
//! Every struct keeps the keys this version doesn't know and writes them back
//! as they were read. No type here holds a key: a key or a token is
//! [`KeysJson`] or nothing.

use std::path::PathBuf;

use serde::{Deserialize, Deserializer, Serialize, Serializer, de::Visitor};

type Extra = serde_json::Map<String, serde_json::Value>;

/// The only value a `key` or `token` takes: the key [#707]'s store keeps.
///
/// [#707]: https://github.com/skel84/freshkube/issues/707
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct KeysJson;

/// What a refused key value says in the deserializer's error, so `parse`
/// can tell it from a wrong shape. The refused value is never part of it.
pub(super) const REFUSED: &str = "freshkube-setup: key value refused";

const KEYS_JSON: &str = "keys.json";

impl Serialize for KeysJson {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(KEYS_JSON)
    }
}

impl<'de> Deserialize<'de> for KeysJson {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct Only;
        impl Visitor<'_> for Only {
            type Value = KeysJson;

            fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str("keys.json")
            }

            // Whatever the value is, the refusal does not quote it.
            fn visit_str<E: serde::de::Error>(self, text: &str) -> Result<KeysJson, E> {
                if text == KEYS_JSON {
                    Ok(KeysJson)
                } else {
                    Err(E::custom(REFUSED))
                }
            }
        }
        deserializer.deserialize_str(Only)
    }
}

/// Where Tekton Results is read for an entry. Exactly one of `address` and
/// `forward`; no `results` at all is off.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Results {
    /// An `https://` or `http://` URL the person enters.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub address: Option<String>,
    /// The address's or configured forward's bearer token, kept in `keys.json`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub token: Option<KeysJson>,
    /// Additional certificate authorities, PEM, for Results' TLS on either route.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ca_file: Option<PathBuf>,
    /// A Service and port in the entry's cluster, forwarded only when the
    /// person presses Forward.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub forward: Option<Forward>,
    #[serde(flatten)]
    pub(super) extra: Extra,
}

/// A Service and port to forward to.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Forward {
    pub namespace: String,
    pub service: String,
    pub port: u16,
    #[serde(flatten)]
    pub(super) extra: Extra,
}

impl Forward {
    pub fn new(namespace: impl Into<String>, service: impl Into<String>, port: u16) -> Self {
        Self {
            namespace: namespace.into(),
            service: service.into(),
            port,
            extra: Extra::default(),
        }
    }
}

/// An entry's link to Coroot.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct EntryCoroot {
    /// Overrides `coroot.project` for this entry.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project: Option<String>,
    /// The Coroot cluster that is this entry.
    pub cluster: String,
    #[serde(flatten)]
    pub(super) extra: Extra,
}

impl EntryCoroot {
    pub fn new(cluster: impl Into<String>) -> Self {
        Self {
            project: None,
            cluster: cluster.into(),
            extra: Extra::default(),
        }
    }
}

/// An entry's part of the metrics setup.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct EntryMetrics {
    /// The entry's value of the central label; its id when unset.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value: Option<String>,
    /// The entry's own store, which wins under any mode.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<Source>,
    #[serde(flatten)]
    pub(super) extra: Extra,
}

/// A store as an entry names it: a Service in its own cluster, or a URL.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Source {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub service: Option<MetricsService>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    /// The URL's bearer token, kept in `keys.json`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub token: Option<KeysJson>,
    #[serde(flatten)]
    pub(super) extra: Extra,
}

/// A Service reached through its cluster's API server proxy.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct MetricsService {
    pub namespace: String,
    pub name: String,
    pub port: u16,
    /// The port speaks TLS.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub https: bool,
    /// A path prefix, normalised as `PrometheusService::with_path` does:
    /// no leading or trailing slash. Written with a leading one.
    #[serde(
        default,
        deserialize_with = "read_path",
        serialize_with = "write_path",
        skip_serializing_if = "String::is_empty"
    )]
    pub path: String,
    #[serde(flatten)]
    pub(super) extra: Extra,
}

impl MetricsService {
    pub fn new(namespace: impl Into<String>, name: impl Into<String>, port: u16) -> Self {
        Self {
            namespace: namespace.into(),
            name: name.into(),
            port,
            https: false,
            path: String::new(),
            extra: Extra::default(),
        }
    }
}

fn read_path<'de, D: Deserializer<'de>>(deserializer: D) -> Result<String, D::Error> {
    let text = String::deserialize(deserializer)?;
    Ok(crate::monitoring::normalise_path(&text))
}

fn write_path<S: Serializer>(path: &str, serializer: S) -> Result<S::Ok, S::Error> {
    serializer.collect_str(&format_args!("/{path}"))
}

/// How the workspace finds its metrics.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Mode {
    /// Discovery in the open cluster, as before the file.
    #[default]
    Automatic,
    /// One store for every entry, its series told apart by a label.
    Central,
    /// Each entry names its own source.
    PerEntry,
}

/// The `metrics` block. `store` and `label` are kept, unread, under another
/// mode than `central`.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Metrics {
    #[serde(default)]
    pub mode: Mode,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub store: Option<Store>,
    /// The label that tells clusters apart; [`DEFAULT_LABEL`] when unset.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    #[serde(flatten)]
    pub(super) extra: Extra,
}

/// The central label unless `metrics.label` sets another.
pub const DEFAULT_LABEL: &str = "cluster";

impl Metrics {
    /// The label name central mode scopes by.
    pub fn label_name(&self) -> &str {
        self.label.as_deref().unwrap_or(DEFAULT_LABEL)
    }

    /// Whether the block says nothing a file without it wouldn't.
    pub(super) fn is_default(&self) -> bool {
        *self == Self::default()
    }
}

/// The central store: a Service in a hosting entry, or a URL.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Store {
    /// The entry that hosts the Service.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub entry: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub service: Option<MetricsService>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub token: Option<KeysJson>,
    #[serde(flatten)]
    pub(super) extra: Extra,
}

/// How a person signs in to Coroot.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum SignIn {
    ApiKey,
    Session,
    Anonymous,
}

/// The Coroot connection. It never holds a key.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Coroot {
    pub server: String,
    pub sign_in: SignIn,
    /// The key the app keeps for this server.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub key: Option<KeysJson>,
    /// A file that holds only the key, read when connecting.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub api_key_file: Option<PathBuf>,
    /// For every entry that names none.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project: Option<String>,
    #[serde(flatten)]
    pub(super) extra: Extra,
}

impl Coroot {
    pub fn new(server: impl Into<String>, sign_in: SignIn) -> Self {
        Self {
            server: server.into(),
            sign_in,
            key: None,
            api_key_file: None,
            project: None,
            extra: Extra::default(),
        }
    }
}

/// How delivery reads the setup's builds, Applications and Kargo Stages.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Delivery {
    /// How an Application with no authorized-stage annotation names its
    /// Stage; off when absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stage_naming: Option<StageNaming>,
    /// The namespace the builds run in, optionally containing `{project}`
    /// for the change's Kargo project. A build is searched for by its image
    /// digest only in a namespace named here; without it only a commit finds
    /// a build.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub build_namespace: Option<String>,
    /// How pull requests are read from GitHub; anonymous when absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub github: Option<GitHub>,
    #[serde(flatten)]
    pub(super) extra: Extra,
}

/// Where GitHub's token comes from: the `gh` command line's, or the one
/// kept in `keys.json`. Never a value of the token itself.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GitHubToken {
    /// `gh auth token`, run when GitHub is read and kept in memory only.
    Gh,
    /// The account `GitHub https://api.github.com/` of `keys.json`.
    KeysJson,
}

impl Serialize for GitHubToken {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(match self {
            Self::Gh => "gh",
            Self::KeysJson => KEYS_JSON,
        })
    }
}

impl<'de> Deserialize<'de> for GitHubToken {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct Only;
        impl Visitor<'_> for Only {
            type Value = GitHubToken;

            fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str("gh or keys.json")
            }

            // Whatever the value is, the refusal does not quote it.
            fn visit_str<E: serde::de::Error>(self, text: &str) -> Result<GitHubToken, E> {
                match text {
                    "gh" => Ok(GitHubToken::Gh),
                    KEYS_JSON => Ok(GitHubToken::KeysJson),
                    _ => Err(E::custom(REFUSED)),
                }
            }
        }
        deserializer.deserialize_str(Only)
    }
}

/// GitHub's REST API at `api.github.com`, the only host a pull request is
/// read from. It never holds a token.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct GitHub {
    /// Where the token comes from. Without it, public repositories are
    /// read anonymously, at a lower rate limit, and `gh` is never run.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub token: Option<GitHubToken>,
    #[serde(flatten)]
    pub(super) extra: Extra,
}

impl Delivery {
    pub fn naming(stage_naming: StageNaming) -> Self {
        Self {
            stage_naming: Some(stage_naming),
            ..Self::default()
        }
    }
}

/// A name template tying an Application to a Kargo Stage.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct StageNaming {
    /// The Application's name, with `{stage}` and optionally `{project}`
    /// standing for the Stage's names.
    pub name_template: String,
    /// The annotation or label of an Application that holds its Kargo
    /// project. Without it the name alone makes the claim, a weaker one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project_key: Option<String>,
    #[serde(flatten)]
    pub(super) extra: Extra,
}

impl StageNaming {
    pub fn new(name_template: impl Into<String>) -> Self {
        Self {
            name_template: name_template.into(),
            project_key: None,
            extra: Extra::default(),
        }
    }
}

/// The supply chain's trust.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SupplyChain {
    /// The public key signatures are verified against.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub public_key_file: Option<PathBuf>,
    /// Read the transparency log's entries.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub transparency_log: bool,
    /// The log's HTTPS address; absent means Chains' configured address.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub transparency_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub evidence: Option<Evidence>,
    #[serde(flatten)]
    pub(super) extra: Extra,
}

/// The pipeline's own evidence record.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Evidence {
    /// The PipelineRun result that holds it.
    pub result: String,
    /// JSON pointer to the commit in that result.
    pub commit: String,
    /// JSON pointer to the image digest in that result.
    pub digest: String,
    #[serde(flatten)]
    pub(super) extra: Extra,
}

impl Evidence {
    pub fn new(
        result: impl Into<String>,
        commit: impl Into<String>,
        digest: impl Into<String>,
    ) -> Self {
        Self {
            result: result.into(),
            commit: commit.into(),
            digest: digest.into(),
            extra: Extra::default(),
        }
    }
}
