//! What an agent profile is: a launch command, an environment, a lock-down
//! and a supported version range.
//!
//! Every agent starts from a cleared environment. The profile adds a minimal
//! `PATH`, `HOME` and locale, an empty temporary config home, and empty
//! `KUBECONFIG` and `TALOSCONFIG` files, and works in an empty temporary
//! directory that goes with the session. The agent never inherits a provider
//! key or a cloud credential, and never gets a cluster credential.

mod locate;
mod opencode;
mod version;

use std::any::Any;
use std::collections::BTreeMap;
use std::ffi::{OsStr, OsString};
use std::io;
use std::path::{Path, PathBuf};
use std::process::Stdio;

pub use locate::{locate, search_dirs};
pub use opencode::{BUILTIN_TOOLS, LOCKDOWN_SOURCES, SUPPORTED};
pub use version::{CHECK_TIMEOUT, Range, Version, VersionState};

use crate::child::{Guard, Link, ProcessGroup};

/// What of the real environment a launch reads. It is read once, so a test
/// states it instead of changing the process's.
#[derive(Debug, Clone, Default)]
pub struct Host {
    pub home: Option<PathBuf>,
    /// Where the agent's logins live (`XDG_DATA_HOME`, or `~/.local/share`).
    pub data_home: Option<PathBuf>,
    pub cache_home: Option<PathBuf>,
    pub state_home: Option<PathBuf>,
    pub lang: Option<String>,
    /// `SystemRoot`, which Windows programs need.
    pub system_root: Option<OsString>,
    pub path: Option<OsString>,
}

impl Host {
    pub fn current() -> Self {
        let var = |name: &str| std::env::var_os(name).filter(|v| !v.is_empty());
        let home = var("HOME")
            .or_else(|| var("USERPROFILE"))
            .map(PathBuf::from);
        let xdg = |name: &str, fallback: &[&str]| {
            var(name).map(PathBuf::from).or_else(|| {
                home.as_ref()
                    .map(|home| fallback.iter().fold(home.clone(), |p, part| p.join(part)))
            })
        };
        Self {
            data_home: xdg("XDG_DATA_HOME", &[".local", "share"]),
            cache_home: xdg("XDG_CACHE_HOME", &[".cache"]),
            state_home: xdg("XDG_STATE_HOME", &[".local", "state"]),
            lang: var("LANG").and_then(|v| v.into_string().ok()),
            system_root: var("SystemRoot"),
            path: var("PATH"),
            home,
        }
    }

    /// Where to look for the agent's binary; see [`search_dirs`].
    pub fn search_dirs(&self) -> Vec<PathBuf> {
        search_dirs(self.home.as_deref(), self.path.as_deref())
    }
}

/// What a session makes on disk, removed with it: an empty workspace, an
/// empty home and config home, a temporary directory, and the empty
/// `KUBECONFIG` and `TALOSCONFIG` files.
#[derive(Debug)]
pub struct Dirs {
    root: tempfile::TempDir,
}

impl Dirs {
    pub fn create() -> io::Result<Self> {
        let root = tempfile::Builder::new()
            .prefix("freshkube-agent-")
            .tempdir()?;
        for dir in ["workspace", "home", "config", "tmp"] {
            std::fs::create_dir(root.path().join(dir))?;
        }
        for file in ["kubeconfig", "talosconfig"] {
            std::fs::File::create(root.path().join(file))?;
        }
        Ok(Self { root })
    }

    /// The agent's working directory; empty.
    pub fn workspace(&self) -> PathBuf {
        self.root.path().join("workspace")
    }

    pub fn home(&self) -> PathBuf {
        self.root.path().join("home")
    }

    /// Empty: the person's agent configuration isn't read.
    pub fn config(&self) -> PathBuf {
        self.root.path().join("config")
    }

    pub fn tmp(&self) -> PathBuf {
        self.root.path().join("tmp")
    }

    pub fn kubeconfig(&self) -> PathBuf {
        self.root.path().join("kubeconfig")
    }

    pub fn talosconfig(&self) -> PathBuf {
        self.root.path().join("talosconfig")
    }

    pub fn root(&self) -> &Path {
        self.root.path()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    Opencode,
    /// Any ACP agent command, such as the scripted one: the locked
    /// environment, no switches of its own.
    Generic,
}

#[derive(Debug, Clone)]
pub struct Profile {
    name: &'static str,
    command: PathBuf,
    args: Vec<String>,
    kind: Kind,
}

impl Profile {
    /// opencode at `binary`, started as `opencode acp`.
    pub fn opencode(binary: impl Into<PathBuf>) -> Self {
        Self {
            name: "opencode",
            command: binary.into(),
            args: vec!["acp".into()],
            kind: Kind::Opencode,
        }
    }

    /// Any ACP agent: the locked environment and workspace, and none of an
    /// agent's own switches. The scripted agent runs this way.
    pub fn command(
        name: &'static str,
        command: impl Into<PathBuf>,
        args: impl IntoIterator<Item = impl Into<String>>,
    ) -> Self {
        Self {
            name,
            command: command.into(),
            args: args.into_iter().map(Into::into).collect(),
            kind: Kind::Generic,
        }
    }

    pub fn name(&self) -> &'static str {
        self.name
    }

    pub fn binary(&self) -> &Path {
        &self.command
    }

    /// The versions this profile supports; none for a generic command.
    pub fn range(&self) -> Option<Range> {
        (self.kind == Kind::Opencode).then_some(SUPPORTED)
    }

    /// The command, arguments and environment a launch uses. Nothing is
    /// started and nothing secret is in it.
    pub fn plan(&self, dirs: &Dirs, host: &Host) -> Plan {
        let mut env = base_environment(dirs, host, &self.command);
        if self.kind == Kind::Opencode {
            env.extend(opencode::environment(dirs, host));
        }
        Plan {
            command: self.command.clone(),
            args: self.args.clone(),
            env,
        }
    }

    /// Starts the agent with a cleared environment in its own process group
    /// and returns both ends of its stdio. Ending the session ends the group
    /// and removes the directories.
    pub fn spawn(&self, host: &Host) -> io::Result<Link> {
        let dirs = Dirs::create()?;
        let plan = self.plan(&dirs, host);
        let workspace = dirs.workspace();
        let mut running = plan.spawn(&dirs)?;
        let writer = running.child.stdin.take().ok_or_else(no_pipe)?;
        let reader = running.child.stdout.take().ok_or_else(no_pipe)?;
        let (guard, _) = running.guard(Box::new(dirs));
        Ok(Link {
            reader: Box::new(reader),
            writer: Box::new(writer),
            workspace,
            guard,
        })
    }
}

fn no_pipe() -> io::Error {
    io::Error::other("the agent's stdio isn't piped")
}

/// A launch, as data.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Plan {
    pub command: PathBuf,
    pub args: Vec<String>,
    pub env: BTreeMap<String, String>,
}

impl Plan {
    pub fn with_args(mut self, args: impl IntoIterator<Item = impl Into<String>>) -> Self {
        self.args = args.into_iter().map(Into::into).collect();
        self
    }

    /// The configuration the profile hands the agent inline, if it does.
    pub fn config(&self) -> Option<serde_json::Value> {
        serde_json::from_str(self.env.get(opencode::CONFIG_VAR)?).ok()
    }

    pub(crate) fn spawn(&self, dirs: &Dirs) -> io::Result<Running> {
        let mut command = tokio::process::Command::new(&self.command);
        command
            .args(&self.args)
            .env_clear()
            .envs(&self.env)
            .current_dir(dirs.workspace())
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            // Nothing the agent logs is kept, and none of it reaches ours.
            .stderr(Stdio::null())
            .kill_on_drop(false);
        #[cfg(unix)]
        command.process_group(0);
        #[cfg(windows)]
        command.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
        let child = command.spawn()?;
        let group = ProcessGroup::of(&child)?;
        Ok(Running { child, group })
    }
}

/// A started agent whose group isn't yet owned by a guard.
pub(crate) struct Running {
    pub(crate) child: tokio::process::Child,
    group: ProcessGroup,
}

impl Running {
    /// Hands the process to a guard that ends it; `keep` goes with it. The
    /// child is reaped in the background, and its exit status is the task's.
    pub(crate) fn guard(
        self,
        keep: Box<dyn Any + Send>,
    ) -> (
        Guard,
        tokio::task::JoinHandle<io::Result<std::process::ExitStatus>>,
    ) {
        let Self { mut child, group } = self;
        let status = tokio::spawn(async move { child.wait().await });
        (Guard::with_group(group, keep), status)
    }
}

/// The environment every agent starts from, whatever its profile.
fn base_environment(dirs: &Dirs, host: &Host, binary: &Path) -> BTreeMap<String, String> {
    let text = |p: &OsStr| p.to_string_lossy().into_owned();
    let mut env = BTreeMap::new();
    let mut set = |name: &str, value: String| {
        env.insert(name.to_string(), value);
    };
    let binary_dir = binary
        .parent()
        .filter(|dir| dir.is_absolute())
        .map(|dir| text(dir.as_os_str()));
    let lang = host
        .lang
        .clone()
        .filter(|lang| lang.to_ascii_lowercase().contains("utf"))
        .unwrap_or_else(|| "en_US.UTF-8".into());
    set("LANG", lang);
    set("KUBECONFIG", text(dirs.kubeconfig().as_os_str()));
    set("TALOSCONFIG", text(dirs.talosconfig().as_os_str()));
    set("XDG_CONFIG_HOME", text(dirs.config().as_os_str()));
    #[cfg(not(windows))]
    {
        // The agent's own directory is on the path so a script that starts
        // `node` beside it still runs.
        let mut path = String::from("/usr/bin:/bin:/usr/sbin:/sbin");
        if let Some(dir) = binary_dir.filter(|dir| !path.split(':').any(|p| p == dir)) {
            path.push(':');
            path.push_str(&dir);
        }
        set("PATH", path);
        set("HOME", text(dirs.home().as_os_str()));
        set("TMPDIR", text(dirs.tmp().as_os_str()));
    }
    #[cfg(windows)]
    {
        let root = host
            .system_root
            .as_deref()
            .map(|r| r.to_string_lossy().into_owned())
            .unwrap_or_else(|| r"C:\Windows".into());
        let mut path = format!(r"{root}\System32");
        if let Some(dir) = binary_dir {
            path.push(';');
            path.push_str(&dir);
        }
        set("PATH", path);
        set("SystemRoot", root);
        set("USERPROFILE", text(dirs.home().as_os_str()));
        set("HOME", text(dirs.home().as_os_str()));
        set("TEMP", text(dirs.tmp().as_os_str()));
        set("TMP", text(dirs.tmp().as_os_str()));
    }
    env
}
