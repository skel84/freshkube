//! The one-time move into `freshkube.yaml` (`docs/SETUP_FILE.md`, Migration).
//!
//! At a launch with no `freshkube.yaml`, [`migrate`] makes it from
//! `workspace.json`, `coroot.json` and the sources `monitoring.json` chose.
//! Each step looks at the disk first, so a launch that stops anywhere is
//! finished by the next, and a launch with nothing to do does nothing.
//!
//! **It never deletes and never replaces a file.** The setup is written to a
//! temporary file and linked into place, which fails if another launch got
//! there first; that launch's file is then the one read. The old files are
//! renamed by [`workspace::set_aside`], which keeps every backup. Whatever is
//! unsure leaves the old files exactly where they were, in use as before: an
//! unusable `workspace.json`, a setup that would not read back as written, a
//! `preferences.json` that can't take the reconnect flag, a file that can't
//! be linked.
//!
//! What moves: the workspace with every key it holds, unknown ones too;
//! `coroot.json`'s server, sign-in and project, with `key: keys.json` when it
//! was set to remember a key (keys themselves are not migrated); the
//! `reconnect` flag, which is session state and goes to `preferences.json` as
//! `coroot_reconnect`; and a source `monitoring.json` chose for an entry's
//! context, which becomes that entry's `metrics.source`. The mode stays
//! `automatic` and `monitoring.json` is left as it is. A URL source whose
//! token is in the credential store is not copied, as its token can't go with
//! it; it stays in `monitoring.json`.

use std::collections::BTreeMap;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use chrono::{DateTime, Utc};
use serde_json::Value;

use super::{Coroot, KeysJson, MetricsService, Setup, SignIn, Source, parse, to_text, validate};
use crate::workspace::{self, Loaded, MAX_BYTES, Workspace};
use crate::read_bounded_regular_file;

const SETUP: &str = "freshkube.yaml";
const WORKSPACE: &str = "workspace.json";
const COROOT: &str = "coroot.json";
const MONITORING: &str = "monitoring.json";
const PREFERENCES: &str = "preferences.json";
/// The key in `preferences.json` that `coroot.json`'s `reconnect` becomes.
const RECONNECT: &str = "coroot_reconnect";
const COROOT_MAX: u64 = 64 * 1024;
const MONITORING_MAX: u64 = 256 * 1024;
const PREFERENCES_MAX: u64 = 256 * 1024;
/// The top-level keys the setup writes itself, so an unknown key can't be one.
const RESERVED: [&str; 8] = [
    "version",
    "kubeconfig",
    "clusters",
    "destinations",
    "sources",
    "metrics",
    "coroot",
    "supply_chain",
];

/// What a launch's migration did.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Outcome {
    /// Nothing to do: `freshkube.yaml` is absent, and so are the old files.
    Nothing,
    /// `freshkube.yaml` is the setup. `moved` names the old files this launch
    /// set aside; it is empty when another launch had already done so.
    Done { moved: Vec<&'static str> },
    /// The old files stay in use, as before.
    Kept(Kept),
}

/// Why the old files stay in use.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kept {
    /// `workspace.json` can't be used; Settings shows its notice, as today.
    WorkspaceUnusable,
    /// `freshkube.yaml` exists and can't be used; it is the person's, so the
    /// old files stay where they are.
    SetupUnusable,
    /// The setup would not read back as built, or can't be written.
    NotSafe,
}

impl Outcome {
    /// The status line to show once, when this launch set files aside.
    pub fn notice(&self) -> Option<String> {
        let Outcome::Done { moved } = self else {
            return None;
        };
        match moved.as_slice() {
            [] => None,
            [one] => Some(format!(
                "Moved {one} into freshkube.yaml; the old file is .bak"
            )),
            [first, second] => Some(format!(
                "Moved {first} and {second} into freshkube.yaml; the old files are .bak"
            )),
            many => Some(format!(
                "Moved {} into freshkube.yaml; the old files are .bak",
                many.join(", ")
            )),
        }
    }
}

/// Runs the migration for the files beside `preferences` (the path of
/// `preferences.json`). Call it once at startup, before anything reads the
/// workspace, and never in example mode.
pub fn migrate(preferences: &Path, now: DateTime<Utc>) -> Outcome {
    run(preferences, now, || {})
}

struct Files {
    setup: PathBuf,
    workspace: PathBuf,
    coroot: PathBuf,
    monitoring: PathBuf,
    preferences: PathBuf,
}

impl Files {
    fn beside(preferences: &Path) -> Self {
        let at = |name: &str| preferences.with_file_name(name);
        Self {
            setup: at(SETUP),
            workspace: at(WORKSPACE),
            coroot: at(COROOT),
            monitoring: at(MONITORING),
            preferences: preferences.with_file_name(PREFERENCES),
        }
    }
}

/// Whether anything is at `path`, a broken link or a directory included.
fn exists(path: &Path) -> bool {
    std::fs::symlink_metadata(path).is_ok()
}

/// `before_link` runs after the setup is written to its temporary file and
/// before it is linked into place: the moment another launch can win.
fn run(preferences: &Path, now: DateTime<Utc>, before_link: impl FnOnce()) -> Outcome {
    let files = Files::beside(preferences);
    // 1. The setup exists: it wins, and the old files are set aside.
    if exists(&files.setup) {
        return adopt(&files, now);
    }
    // 2. Neither old file exists: nothing to write.
    let (has_workspace, has_coroot) = (exists(&files.workspace), exists(&files.coroot));
    if !has_workspace && !has_coroot {
        return Outcome::Nothing;
    }
    // 3. A workspace.json that can't be used migrates nothing.
    let workspace = if has_workspace {
        match workspace::load(&files.workspace) {
            Loaded::Workspace(workspace) => workspace,
            Loaded::Missing => Workspace::default(),
            Loaded::Refused(why) => {
                tracing::warn!("workspace.json not migrated, it isn't used: {why}");
                return Outcome::Kept(Kept::WorkspaceUnusable);
            }
        }
    } else {
        Workspace::default()
    };
    // 4. Build, check, write.
    let coroot = has_coroot.then(|| read_coroot(&files.coroot)).flatten();
    let Some((setup, reconnect)) = build(workspace, coroot, read_sources(&files.monitoring)) else {
        return Outcome::Kept(Kept::NotSafe);
    };
    let text = to_text(&setup);
    if !parse(text.as_bytes()).is_ok_and(|back| back == setup) {
        tracing::warn!("workspace not migrated: the setup would not read back as built");
        return Outcome::Kept(Kept::NotSafe);
    }
    if reconnect && let Err(why) = keep_reconnect(&files.preferences) {
        tracing::warn!("workspace not migrated, preferences.json can't take a flag: {why}");
        return Outcome::Kept(Kept::NotSafe);
    }
    match link_into_place(&files.setup, &text, before_link) {
        Ok(_) => {}
        Err(why) => {
            tracing::warn!("workspace not migrated, freshkube.yaml can't be written: {why}");
            return Outcome::Kept(Kept::NotSafe);
        }
    }
    // Whoever's file is in place is the one read, and the old files go aside.
    adopt(&files, now)
}

/// Step 1 and the end of step 4: `freshkube.yaml` is there. When it can be
/// used, the old files are set aside; when it can't, they are left alone.
fn adopt(files: &Files, now: DateTime<Utc>) -> Outcome {
    if !exists(&files.workspace) && !exists(&files.coroot) {
        return Outcome::Nothing;
    }
    let usable = read_bounded_regular_file(&files.setup, MAX_BYTES)
        .ok()
        .is_some_and(|bytes| parse(&bytes).is_ok());
    if !usable {
        return Outcome::Kept(Kept::SetupUnusable);
    }
    let mut moved = Vec::new();
    for (name, path) in [(WORKSPACE, &files.workspace), (COROOT, &files.coroot)] {
        if !exists(path) {
            continue;
        }
        match workspace::set_aside(path, now) {
            Ok(_) => moved.push(name),
            // Another launch moved it first.
            Err(why) if why.kind() == std::io::ErrorKind::NotFound => {}
            Err(why) => tracing::warn!("{name} not set aside, the next launch tries again: {why}"),
        }
    }
    Outcome::Done { moved }
}

/// The setup from the old files, and whether `coroot.json` asked to
/// reconnect. A Coroot connection or a source that doesn't validate is left
/// out; a workspace that doesn't is a reason to migrate nothing.
fn build(
    mut workspace: Workspace,
    coroot: Option<SavedCoroot>,
    sources: BTreeMap<String, Source>,
) -> Option<(Setup, bool)> {
    let extra = workspace.take_unknown();
    if RESERVED.iter().any(|key| extra.contains_key(*key)) {
        return None;
    }
    for entry in &mut workspace.clusters {
        let Some(source) = sources.get(&entry.context) else {
            continue;
        };
        if entry.metrics.as_ref().is_some_and(|m| m.source.is_some()) {
            continue;
        }
        let before = entry.metrics.clone();
        entry.metrics.get_or_insert_with(Default::default).source = Some(source.clone());
        if validate::entry(entry).is_err() {
            entry.metrics = before;
        }
    }
    let mut setup = Setup::new(workspace);
    setup.extra = extra;
    let mut reconnect = false;
    if let Some(saved) = coroot
        && validate::coroot(&saved.coroot).is_ok()
    {
        reconnect = saved.reconnect;
        setup.coroot = Some(saved.coroot);
    }
    setup.validate().ok()?;
    Some((setup, reconnect))
}

struct SavedCoroot {
    coroot: Coroot,
    reconnect: bool,
}

/// `coroot.json` as Observability reads it: a server and a known sign-in are
/// required, the rest is lenient. Unusable counts as no Coroot.
fn read_coroot(file: &Path) -> Option<SavedCoroot> {
    let bytes = read_bounded_regular_file(file, COROOT_MAX).ok()?;
    let value: Value = serde_json::from_slice(&bytes).ok()?;
    let object = value.as_object()?;
    let server = object.get("url")?.as_str()?;
    if server.is_empty() {
        return None;
    }
    let sign_in = match object.get("auth")?.as_str()? {
        "api-key" => SignIn::ApiKey,
        "session" => SignIn::Session,
        "anonymous" => SignIn::Anonymous,
        _ => return None,
    };
    let flag = |key: &str| object.get(key).and_then(Value::as_bool) == Some(true);
    let mut coroot = Coroot::new(server, sign_in);
    if flag("remember") && sign_in != SignIn::Anonymous {
        coroot.key = Some(KeysJson);
    }
    coroot.project = object
        .get("project")
        .and_then(Value::as_str)
        .filter(|project| !project.is_empty())
        .map(str::to_owned);
    coroot.extra = object
        .iter()
        .filter(|(key, _)| !["url", "auth", "project", "reconnect", "remember"].contains(&key.as_str()))
        .map(|(key, value)| (key.clone(), value.clone()))
        .collect();
    Some(SavedCoroot {
        coroot,
        reconnect: flag("reconnect"),
    })
}

/// The sources `monitoring.json` chose, by context. A file that can't be
/// read gives none.
fn read_sources(file: &Path) -> BTreeMap<String, Source> {
    let Ok(bytes) = read_bounded_regular_file(file, MONITORING_MAX) else {
        return BTreeMap::new();
    };
    let Ok(value) = serde_json::from_slice::<Value>(&bytes) else {
        return BTreeMap::new();
    };
    let chosen = value.get("sources").and_then(Value::as_object);
    chosen
        .into_iter()
        .flatten()
        .filter_map(|(context, choice)| Some((context.clone(), read_choice(choice)?)))
        .collect()
}

fn read_choice(choice: &Value) -> Option<Source> {
    let text = |key: &str| choice.get(key)?.as_str();
    match text("kind")? {
        "service" => {
            let port = u16::try_from(choice.get("port")?.as_u64()?).ok()?;
            let mut service = MetricsService::new(text("namespace")?, text("name")?, port);
            service.https = choice.get("https").and_then(Value::as_bool) == Some(true);
            service.path = crate::monitoring::normalise_path(text("path").unwrap_or_default());
            Some(Source {
                service: Some(service),
                ..Source::default()
            })
        }
        // Its token is in the credential store, not in keys.json.
        "url" if choice.get("token").and_then(Value::as_bool) != Some(true) => Some(Source {
            url: Some(crate::monitoring::normalise_url(text("url")?).ok()?),
            ..Source::default()
        }),
        _ => None,
    }
}

/// Sets `coroot_reconnect` in `preferences.json` unless it is set, keeping
/// every other key. A file that exists and can't be read as a JSON object is
/// not written over.
fn keep_reconnect(file: &Path) -> std::io::Result<()> {
    let invalid = |why: &str| std::io::Error::new(std::io::ErrorKind::InvalidData, why.to_owned());
    let mut object = match read_bounded_regular_file(file, PREFERENCES_MAX) {
        Ok(bytes) => match serde_json::from_slice::<Value>(&bytes) {
            Ok(Value::Object(object)) => object,
            _ => return Err(invalid("preferences.json isn't a JSON object")),
        },
        Err(crate::BoundedReadError::NotFound) => serde_json::Map::new(),
        Err(_) => return Err(invalid("preferences.json can't be read")),
    };
    if object.contains_key(RECONNECT) {
        return Ok(());
    }
    object.insert(RECONNECT.into(), true.into());
    let text = format!("{:#}\n", Value::Object(object));
    if let Some(parent) = file.parent() {
        std::fs::create_dir_all(parent)?;
    }
    workspace::write_whole(file, &text)
}

/// Whose file is in place after [`link_into_place`].
enum Linked {
    Ours,
    Theirs,
}

/// Writes `text` to a temporary file beside `file`, created `0600`, and links
/// it to `file` without replacing anything: if `file` appeared meanwhile,
/// that one stays. The temporary file is removed either way.
fn link_into_place(
    file: &Path,
    text: &str,
    before_link: impl FnOnce(),
) -> std::io::Result<Linked> {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    if let Some(parent) = file.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut name = file.file_name().unwrap_or_default().to_os_string();
    name.push(format!(
        ".{}.{}.tmp",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    let temporary = file.with_file_name(name);
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let result = (|| {
        let mut output = options.open(&temporary)?;
        output.write_all(text.as_bytes())?;
        output.sync_all()?;
        before_link();
        match std::fs::hard_link(&temporary, file) {
            Ok(()) => Ok(Linked::Ours),
            Err(why) if why.kind() == std::io::ErrorKind::AlreadyExists => Ok(Linked::Theirs),
            Err(why) => Err(why),
        }
    })();
    let _ = std::fs::remove_file(&temporary);
    result
}

#[cfg(test)]
#[path = "migrate_tests.rs"]
mod tests;
