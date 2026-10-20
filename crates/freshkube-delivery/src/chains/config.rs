//! Chains' configuration, `chains-config`: what it is set to store, in which
//! backends, and which transparency log it uploads to.
//!
//! Nothing reads it yet: the join does, once a missing attestation has to
//! say whether every place Chains stores one was looked at.

#![cfg_attr(
    not(test),
    allow(
        dead_code,
        reason = "the join reads the configuration in the verification step"
    )
)]

use serde_json::Value;

use crate::observation::pointer_segment;
use crate::read::{Reader, Resource};
use freshkube_core::resources::Failure;

/// The ConfigMap Chains reads its configuration from.
pub(crate) const CONFIG_MAP: &str = "chains-config";

/// What Chains is set to store, and where, from `chains-config`. A key the
/// ConfigMap leaves out has Chains' documented default, and says so.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ChainsConfig {
    pub task_runs: Artifact,
    pub pipeline_runs: Artifact,
    pub transparency: Transparency,
}

/// How Chains handles one kind of run.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Artifact {
    pub backends: Vec<Backend>,
    /// `x509`, `kms` or `none`, as written.
    pub signer: String,
    /// `in-toto`, `slsa/v1`, `slsa/v2alpha3`, …, as written.
    pub format: String,
    /// Whether the ConfigMap set the storage, as against Chains' default.
    pub storage_set: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Backend {
    /// Annotations on the run: what this step reads.
    Tekton,
    /// The registry: named, and read in the registry step.
    Oci,
    /// Another backend (`gcs`, `docdb`, `grafeas`, `archivista`, …).
    Other(String),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Transparency {
    pub enabled: Enabled,
    /// `transparency.url`, or Chains' default log when the ConfigMap sets none.
    pub url: String,
    pub url_set: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Enabled {
    Off,
    On,
    /// Only runs annotated `chains.tekton.dev/transparency-upload: "true"`.
    Manual,
    /// A value Chains does not document, as written.
    Other(String),
}

const DEFAULT_LOG: &str = "https://rekor.sigstore.dev";

impl ChainsConfig {
    /// From the `chains-config` ConfigMap.
    pub(crate) fn parse(config_map: &Value) -> Self {
        let raw = |key: &str| {
            config_map
                .pointer(&format!("/data/{}", pointer_segment(key)))
                .and_then(Value::as_str)
                .map(str::to_owned)
        };
        let data = |key: &str| raw(key).filter(|value| !value.is_empty());
        let artifact = |kind: &str| {
            // An empty storage is a setting: it turns storage off.
            let storage = raw(&format!("artifacts.{kind}.storage"));
            Artifact {
                backends: storage
                    .as_deref()
                    .map_or_else(|| vec![Backend::Tekton], backends_of),
                signer: data(&format!("artifacts.{kind}.signer")).unwrap_or_else(|| "x509".into()),
                format: data(&format!("artifacts.{kind}.format"))
                    .unwrap_or_else(|| "in-toto".into()),
                storage_set: storage.is_some(),
            }
        };
        let url = data("transparency.url");
        Self {
            task_runs: artifact("taskrun"),
            pipeline_runs: artifact("pipelinerun"),
            transparency: Transparency {
                enabled: match data("transparency.enabled").as_deref() {
                    None | Some("false") => Enabled::Off,
                    Some("true") => Enabled::On,
                    Some("manual") => Enabled::Manual,
                    Some(other) => Enabled::Other(other.to_owned()),
                },
                url_set: url.is_some(),
                url: url.unwrap_or_else(|| DEFAULT_LOG.to_owned()),
            },
        }
    }
}

/// Chains' `artifacts.*.storage`: a comma-separated list; the empty string
/// disables storage.
fn backends_of(storage: &str) -> Vec<Backend> {
    storage
        .split(',')
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .map(|name| match name {
            "tekton" => Backend::Tekton,
            "oci" => Backend::Oci,
            other => Backend::Other(other.to_owned()),
        })
        .collect()
}

impl Artifact {
    /// Whether every place Chains is set to store this kind of attestation
    /// is one this step reads. Only then is "no attestation found" more than
    /// "none found here".
    pub fn all_stored_here(&self) -> bool {
        self.backends
            .iter()
            .all(|backend| *backend == Backend::Tekton)
    }
}

/// `chains-config` in `namespace`: one GET of a ConfigMap, whose failure
/// (refused, not found, unreachable) the caller names.
pub(crate) async fn read_config<R: Reader>(
    reader: &R,
    namespace: &str,
) -> Result<ChainsConfig, Failure> {
    let config_maps = Resource::new("", "v1", "configmaps", true);
    let config_map = reader
        .get(&config_maps, Some(namespace), CONFIG_MAP)
        .await?;
    Ok(ChainsConfig::parse(&config_map))
}
