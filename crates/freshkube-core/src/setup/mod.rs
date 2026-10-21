//! The setup file: what Freshkube reads, and from where, as one YAML file
//! beside the preferences (`docs/SETUP_FILE.md`, #706). It holds the
//! workspace ([`Workspace`], with the same keys and limits as
//! `workspace.json`), the metrics choice, the Coroot connection and the
//! supply chain's trust. Nothing reads or writes the file yet; this is its
//! types, [`parse`], [`to_text`] and [`commit`].
//!
//! **No key is ever in the file.** A `key` or a `token` is `keys.json`, the
//! file the app keeps keys in, or absent; Coroot's `api_key_file` names a file
//! that holds only the key. Any other value is refused without repeating it.
//!
//! **Unknown keys are kept at every level** and written back as they were
//! read; [`Setup::unknown_keys`] names them.
//!
//! **A file that doesn't parse says where, never what it read.** The read
//! runs in two passes, so the words come from the pass that failed:
//! the bytes into a YAML value (`invalid YAML at line L, column C`), then the
//! same bytes into the typed setup (`a value of the wrong type or shape at
//! line L, column C`). Then the workspace's own validation and this module's
//! rules run, and name an entry by id.
//!
//! **Settings writes the file whole**, in a fixed order, under a fixed header;
//! a person's own comments are not kept by a save.

use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::workspace::{self, Destination, Entry, MAX_BYTES, Pins, Seen, VERSION, Workspace};

mod migrate;
mod types;
mod validate;

pub use migrate::{Kept, Outcome, migrate};
pub use types::{
    Coroot, DEFAULT_LABEL, Delivery, EntryCoroot, EntryMetrics, Evidence, Forward, GitHub,
    KeysJson,
    Metrics, MetricsService, Mode, Results, SignIn, Source, StageNaming, Store, SupplyChain,
};

/// The comment every written file starts with.
const HEADER: &str = "\
# freshkube.yaml: what Freshkube reads, and from where.
# Settings writes this file whole and keeps only this header: comments you add
# are not kept by a save (the hand-edited file is kept as freshkube.yaml.bak).
# Keys never go here: they live in keys.json, or in a file api_key_file names.
";

type Extra = serde_json::Map<String, serde_json::Value>;

const EXAMPLE: &str = include_str!("../../../freshkube-prefs/src/examples/freshkube.yaml");

/// Reads and validates the bundled example with absolute paths on this platform.
pub fn example() -> Result<Setup, Invalid> {
    // Windows requires a drive for the example's Unix home to be absolute.
    #[cfg(windows)]
    let text = EXAMPLE.replace("/home/acme", "C:/home/acme");
    #[cfg(not(windows))]
    let text = EXAMPLE;
    parse(text.as_bytes())
}

/// Everything `freshkube.yaml` says.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Setup {
    pub workspace: Workspace,
    pub metrics: Metrics,
    /// The Coroot connection; `None` is not connected.
    pub coroot: Option<Coroot>,
    /// The supply chain's trust; `None` verifies no signature.
    pub supply_chain: Option<SupplyChain>,
    /// How delivery reads builds and ties Applications to Stages; `None` is the
    /// defaults.
    pub delivery: Option<Delivery>,
    /// Keys this version doesn't know, written back as they were read.
    extra: Extra,
}

/// Where in the file a parser found its problem. Both count from 1.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Place {
    pub line: usize,
    pub column: usize,
}

impl std::fmt::Display for Place {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "line {}, column {}", self.line, self.column)
    }
}

/// A rule of the setup that a file broke, apart from the workspace's. `at`
/// is a field (`metrics.store.url`) or a field of an entry
/// (`the cluster “cluster-b” results.address`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Fault {
    Empty(String),
    NotUrl(String),
    /// Not a Kubernetes namespace name; the value is never repeated.
    NotNamespace(String),
    NonLoopbackHttp(String),
    RelativePath(String),
    ZeroPort(String),
    /// Exactly one of the keys `of` is set.
    OneOf {
        at: String,
        of: &'static str,
    },
    /// `at` goes only with `with`.
    Only {
        at: String,
        with: &'static str,
    },
    LabelName(String),
    Pointer(String),
    /// A name template that has no `{stage}` in it, so it names no Stage.
    TemplateWithoutStage(String),
    CentralWithoutStore,
    /// `coroot` sets both `key` and `api_key_file`.
    KeyTwice,
    /// An entry names a Coroot cluster, and neither it nor `coroot` names a
    /// project, so no application of that cluster could be found.
    CorootWithoutProject(String),
    /// Two entries link the same Coroot cluster of the same project, so a
    /// Coroot application would belong to both. Ids, in file order.
    CorootTwice {
        first: String,
        second: String,
        project: String,
        cluster: String,
    },
}

impl std::fmt::Display for Fault {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Fault::Empty(at) => write!(f, "{at} is empty"),
            Fault::NotNamespace(at) => write!(f, "{at} isn’t a Kubernetes namespace name"),
            Fault::NotUrl(at) => write!(f, "{at} isn’t an http:// or https:// URL"),
            Fault::NonLoopbackHttp(at) => write!(f, "{at}: plain HTTP is allowed only on loopback"),
            Fault::RelativePath(at) => write!(f, "{at} isn’t an absolute path"),
            Fault::ZeroPort(at) => write!(f, "{at} isn’t a port"),
            Fault::OneOf { at, of } => write!(f, "{at} must set exactly one of {of}"),
            Fault::Only { at, with } => write!(f, "{at} only goes with {with}"),
            Fault::LabelName(at) => write!(f, "{at} isn’t a Prometheus label name"),
            Fault::Pointer(at) => write!(f, "{at} isn’t a JSON pointer, which starts with /"),
            Fault::TemplateWithoutStage(at) => {
                write!(f, "{at} has no {{stage}} in it, so it names no Stage")
            }
            Fault::CentralWithoutStore => {
                f.write_str("metrics.mode is central, and metrics.store is missing")
            }
            Fault::KeyTwice => f.write_str("coroot sets both key and api_key_file"),
            Fault::CorootWithoutProject(id) => write!(
                f,
                "the cluster “{id}” names a Coroot cluster, and neither it nor coroot names a project"
            ),
            Fault::CorootTwice {
                first,
                second,
                project,
                cluster,
            } => write!(
                f,
                "the Coroot cluster “{cluster}” of project “{project}” is linked to “{first}” and “{second}”"
            ),
        }
    }
}

/// Why a file or a setup is refused. No variant holds a value from the file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Invalid {
    TooLarge,
    /// The bytes are not YAML (pass one).
    NotYaml(Option<Place>),
    /// The YAML is not a setup: a value of the wrong type or shape (pass two).
    WrongShape(Option<Place>),
    /// A `key` or `token` that is not `keys.json`.
    KeyValue(Option<Place>),
    MissingVersion,
    UnreadableVersion,
    UnknownVersion(u64),
    /// The workspace's own refusal, naming an entry by id.
    Workspace(workspace::Invalid),
    Rule(Fault),
}

impl std::fmt::Display for Invalid {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let at = |f: &mut std::fmt::Formatter<'_>, place: &Option<Place>| match place {
            Some(place) => write!(f, " at {place}"),
            None => Ok(()),
        };
        match self {
            Invalid::TooLarge => f.write_str("it is larger than 256 KiB"),
            Invalid::NotYaml(place) => {
                f.write_str("invalid YAML")?;
                at(f, place)
            }
            Invalid::WrongShape(place) => {
                f.write_str("a value of the wrong type or shape")?;
                at(f, place)
            }
            Invalid::KeyValue(place) => {
                f.write_str("a key or token other than keys.json")?;
                at(f, place)?;
                f.write_str(": keys never go in this file")
            }
            Invalid::MissingVersion => f.write_str("it has no version"),
            Invalid::UnreadableVersion => f.write_str("its version isn’t a whole number"),
            Invalid::UnknownVersion(version) => {
                write!(f, "it is version {version}, and this app reads {VERSION}")
            }
            Invalid::Workspace(invalid) => invalid.fmt(f),
            Invalid::Rule(fault) => fault.fmt(f),
        }
    }
}

impl Setup {
    /// A setup of this workspace, with the default metrics and no Coroot or
    /// supply chain.
    pub fn new(mut workspace: Workspace) -> Self {
        let extra = workspace.take_unknown();
        Self {
            workspace,
            extra,
            ..Self::default()
        }
    }

    /// The workspace's validation, then the rules of the parts around it.
    pub fn validate(&self) -> Result<(), Invalid> {
        self.workspace.validate().map_err(Invalid::Workspace)?;
        let rule = |result: Result<(), Fault>| result.map_err(Invalid::Rule);
        for entry in &self.workspace.clusters {
            rule(validate::entry(entry))?;
        }
        rule(validate::coroot_links(
            &self.workspace.clusters,
            self.coroot.as_ref().and_then(|c| c.project.as_deref()),
        ))?;
        rule(validate::metrics(&self.metrics))?;
        if let Some(coroot) = &self.coroot {
            rule(validate::coroot(coroot))?;
        }
        if let Some(chain) = &self.supply_chain {
            rule(validate::supply_chain(chain))?;
        }
        if let Some(delivery) = &self.delivery {
            rule(validate::delivery(delivery))?;
        }
        Ok(())
    }

    /// What [`validate`](Self::validate) checks, and that the written file
    /// stays within what the next launch reads.
    pub fn check_saveable(&self) -> Result<(), Invalid> {
        self.validate()?;
        let text = to_text(self);
        if text.len() as u64 > MAX_BYTES {
            return Err(Invalid::TooLarge);
        }
        // A legacy workspace can hold a key now reserved by setup. Refuse
        // that collision rather than write duplicate YAML keys or drop it.
        if self.extra.keys().any(|key| {
            matches!(
                key.as_str(),
                "version"
                    | "kubeconfig"
                    | "clusters"
                    | "destinations"
                    | "sources"
                    | "metrics"
                    | "coroot"
                    | "supply_chain"
                    | "delivery"
            )
        }) {
            return Err(Invalid::WrongShape(None));
        }
        Ok(())
    }

    /// Keys this version doesn't know, as `key`, `metrics.key` or
    /// `cluster-id.results.key`, sorted. They are kept when the file is
    /// saved; Settings names them so a misspelt key doesn't go unnoticed.
    pub fn unknown_keys(&self) -> Vec<String> {
        let mut keys = self.workspace.unknown_keys();
        named(&mut keys, "", &self.extra);
        let metrics = &self.metrics;
        named(&mut keys, "metrics", &metrics.extra);
        if let Some(store) = &metrics.store {
            named(&mut keys, "metrics.store", &store.extra);
            if let Some(service) = &store.service {
                named(&mut keys, "metrics.store.service", &service.extra);
            }
        }
        if let Some(coroot) = &self.coroot {
            named(&mut keys, "coroot", &coroot.extra);
        }
        if let Some(chain) = &self.supply_chain {
            named(&mut keys, "supply_chain", &chain.extra);
            if let Some(evidence) = &chain.evidence {
                named(&mut keys, "supply_chain.evidence", &evidence.extra);
            }
        }
        if let Some(delivery) = &self.delivery {
            named(&mut keys, "delivery", &delivery.extra);
            if let Some(naming) = &delivery.stage_naming {
                named(&mut keys, "delivery.stage_naming", &naming.extra);
            }
            if let Some(github) = &delivery.github {
                named(&mut keys, "delivery.github", &github.extra);
            }
        }
        for entry in &self.workspace.clusters {
            entry_keys(&mut keys, entry);
        }
        keys.sort();
        keys
    }
}

fn entry_keys(keys: &mut Vec<String>, entry: &Entry) {
    let id = &entry.id;
    if let Some(results) = &entry.results {
        named(keys, &format!("{id}.results"), &results.extra);
        if let Some(forward) = &results.forward {
            named(keys, &format!("{id}.results.forward"), &forward.extra);
        }
    }
    if let Some(coroot) = &entry.coroot {
        named(keys, &format!("{id}.coroot"), &coroot.extra);
    }
    if let Some(metrics) = &entry.metrics {
        named(keys, &format!("{id}.metrics"), &metrics.extra);
        if let Some(source) = &metrics.source {
            named(keys, &format!("{id}.metrics.source"), &source.extra);
            if let Some(service) = &source.service {
                named(
                    keys,
                    &format!("{id}.metrics.source.service"),
                    &service.extra,
                );
            }
        }
    }
}

fn named(keys: &mut Vec<String>, prefix: &str, extra: &Extra) {
    keys.extend(extra.keys().map(|key| {
        if prefix.is_empty() {
            key.clone()
        } else {
            format!("{prefix}.{key}")
        }
    }));
}

/// The file as pass two reads it. Each key is read where it stands, so a
/// parser's place is the value's own; only what no field names goes to
/// `extra`.
#[derive(Deserialize)]
struct Read {
    #[allow(dead_code)] // checked by pass one, and written fresh
    version: u64,
    #[serde(default)]
    kubeconfig: Option<std::path::PathBuf>,
    #[serde(default)]
    clusters: Vec<Entry>,
    #[serde(default)]
    destinations: Vec<Destination>,
    #[serde(default)]
    sources: Pins,
    #[serde(default)]
    metrics: Metrics,
    #[serde(default)]
    coroot: Option<Coroot>,
    #[serde(default)]
    supply_chain: Option<SupplyChain>,
    #[serde(default)]
    delivery: Option<Delivery>,
    #[serde(flatten)]
    extra: Extra,
}

/// The file as it is written, in the order of the design's example.
#[derive(Serialize)]
struct Write<'a> {
    version: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    kubeconfig: &'a Option<std::path::PathBuf>,
    clusters: &'a [Entry],
    #[serde(skip_serializing_if = "<[_]>::is_empty")]
    destinations: &'a [Destination],
    #[serde(skip_serializing_if = "Pins::is_empty")]
    sources: &'a Pins,
    #[serde(skip_serializing_if = "Metrics::is_default")]
    metrics: &'a Metrics,
    #[serde(skip_serializing_if = "Option::is_none")]
    coroot: &'a Option<Coroot>,
    #[serde(skip_serializing_if = "Option::is_none")]
    supply_chain: &'a Option<SupplyChain>,
    #[serde(skip_serializing_if = "Option::is_none")]
    delivery: &'a Option<Delivery>,
    #[serde(flatten)]
    extra: &'a Extra,
}

fn place(error: &serde_yaml::Error) -> Option<Place> {
    error.location().map(|at| Place {
        line: at.line(),
        column: at.column(),
    })
}

/// Parses the file's bytes in two passes, then validates what they say.
pub fn parse(bytes: &[u8]) -> Result<Setup, Invalid> {
    if bytes.len() as u64 > MAX_BYTES {
        return Err(Invalid::TooLarge);
    }
    let value: serde_yaml::Value =
        serde_yaml::from_slice(bytes).map_err(|error| Invalid::NotYaml(place(&error)))?;
    match value.get("version") {
        None => return Err(Invalid::MissingVersion),
        Some(version) => match version.as_u64() {
            None => return Err(Invalid::UnreadableVersion),
            Some(version) if version != u64::from(VERSION) => {
                return Err(Invalid::UnknownVersion(version));
            }
            Some(_) => {}
        },
    }
    let read: Read = serde_yaml::from_slice(bytes).map_err(|error| {
        if error.to_string().contains(types::REFUSED) {
            Invalid::KeyValue(place(&error))
        } else {
            Invalid::WrongShape(place(&error))
        }
    })?;
    let mut workspace = Workspace::new(read.clusters);
    workspace.kubeconfig = read.kubeconfig;
    workspace.destinations = read.destinations;
    workspace.sources = read.sources;
    let setup = Setup {
        workspace,
        metrics: read.metrics,
        coroot: read.coroot,
        supply_chain: read.supply_chain,
        delivery: read.delivery,
        extra: read.extra,
    };
    setup.validate()?;
    Ok(setup)
}

/// The file's text: the header, the version, then each part in the design's
/// order, unknown keys last. Comments of a person's own are not kept.
pub fn to_text(setup: &Setup) -> String {
    let write = Write {
        version: VERSION,
        kubeconfig: &setup.workspace.kubeconfig,
        clusters: &setup.workspace.clusters,
        destinations: &setup.workspace.destinations,
        sources: &setup.workspace.sources,
        metrics: &setup.metrics,
        coroot: &setup.coroot,
        supply_chain: &setup.supply_chain,
        delivery: &setup.delivery,
        extra: &setup.extra,
    };
    let body = serde_yaml::to_string(&write).expect("a setup serializes");
    format!("{HEADER}{body}")
}

/// Why [`commit`] wrote nothing, or failed writing.
#[derive(Debug)]
pub enum SaveError {
    /// The file is not what it was when it was read: someone changed,
    /// created or removed it. Nothing was written.
    Changed,
    /// The setup is invalid, or too large for the next launch to read.
    Invalid(Invalid),
    Write(std::io::Error),
}

/// Saves `setup` if the file is still as it was when it was read
/// (`expected`), through a temporary file renamed into place, created `0600`.
/// Returns what the file now holds, for the next save. Re-reading just before
/// the write narrows the window in which a hand edit can be lost; it cannot
/// close it, as no portable compare-and-rename exists. A file that doesn't
/// parse is never set aside here: it is the person's edit in progress, and
/// the caller refuses to save while it holds.
pub fn commit(file: &Path, expected: &Seen, setup: &Setup) -> Result<Seen, SaveError> {
    setup.check_saveable().map_err(SaveError::Invalid)?;
    if &workspace::read_seen(file).0 != expected {
        return Err(SaveError::Changed);
    }
    let text = to_text(setup);
    if let Some(parent) = file.parent() {
        std::fs::create_dir_all(parent).map_err(SaveError::Write)?;
    }
    workspace::write_whole(file, &text).map_err(SaveError::Write)?;
    Ok(Seen::Bytes(text.into_bytes()))
}

/// Preserves a hand-edited file as `freshkube.yaml.bak` before committing.
/// The backup is replaced atomically and only after the expected bytes and
/// the new setup have been checked. A backup failure leaves the setup alone.
pub fn commit_with_backup(
    file: &Path,
    expected: &Seen,
    setup: &Setup,
    hand_edited: bool,
) -> Result<Seen, SaveError> {
    setup.check_saveable().map_err(SaveError::Invalid)?;
    if &workspace::read_seen(file).0 != expected {
        return Err(SaveError::Changed);
    }
    if hand_edited && let Seen::Bytes(bytes) = expected {
        let text = std::str::from_utf8(bytes).map_err(|_| {
            SaveError::Write(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "the setup file is not UTF-8",
            ))
        })?;
        workspace::write_whole(&file.with_file_name("freshkube.yaml.bak"), text)
            .map_err(SaveError::Write)?;
    }
    // Check again: an editor may have saved while the backup was written.
    commit(file, expected, setup)
}

#[cfg(test)]
mod tests;
