//! Teleport database access (#948, E3): `tsh proxy db` as a child process.
//!
//! The route changes, the rules don't: Teleport's certificate signs in, so no
//! password is read, and the session over the tunnel is the same read-only
//! session as ever. Freshkube runs `tsh` only from an explicit Explore, and
//! never runs `tsh login` for anyone: a person who isn't signed in is told
//! the command.
//!
//! - **The child** is `tsh proxy db --tunnel`, which signs in with `tsh`'s own
//!   certificate. Freshkube never opens the profile directory, names a
//!   certificate or key on a command line, or reads one. The child's
//!   environment is cleared down to what `tsh` itself needs.
//! - **Loopback only:** the port is the one `tsh` announces, and an announced
//!   address that isn't loopback ends the child.
//! - **Owned by one tab:** a [`Proxy`] kills its child when dropped, so closing
//!   the tab, connecting again or quitting the app leaves no process behind.
//! - **Stderr** is redacted ([`redact`]) and bounded before anyone shows it.
mod redact;
#[cfg(test)]
mod tests;

pub use redact::redact;
use std::{
    collections::VecDeque,
    net::{IpAddr, Ipv4Addr},
    path::{Path, PathBuf},
    process::Stdio,
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio::{
    io::{AsyncBufReadExt, AsyncRead, BufReader},
    process::{Child, Command},
    sync::mpsc,
    task::JoinHandle,
};

/// How long `tsh status` and `tsh db ls` may take.
pub const CHECK_TIMEOUT: Duration = Duration::from_secs(20);
/// How long the proxy may take to say it is listening.
pub const READY_TIMEOUT: Duration = Duration::from_secs(15);

/// Environment variables the child keeps, and no others. `tsh` needs a home
/// for its profile and a path to run; none of them is a secret.
const KEPT_ENV: [&str; 8] = [
    "PATH",
    "HOME",
    "TELEPORT_HOME",
    "USER",
    "LOGNAME",
    "TMPDIR",
    "SYSTEMROOT",
    "USERPROFILE",
];

/// What the team (or, until E4 reads the Team file, an example) configures.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TeleportAccess {
    /// The Teleport proxy, `host` or `host:port`.
    pub proxy: String,
    /// How a Teleport database is named from the Cluster's name; `{name}` is
    /// replaced, as in `{name}-ro`.
    pub name_pattern: String,
    /// The database user Teleport signs the session in as.
    pub db_user: String,
    /// The database to open.
    pub database: String,
    /// A `tsh` the person chose; else the one on `PATH`.
    pub tsh: Option<PathBuf>,
}

/// Why Teleport can't be used, each a state of the connect step.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum TeleportError {
    #[error("The Teleport settings are not valid: {0}")]
    BadSettings(String),
    #[error("tsh is not installed")]
    NotInstalled,
    #[error("Not signed in to Teleport at {proxy}")]
    NotLoggedIn { proxy: String },
    #[error("No role allows {user} to use {name}")]
    NoRole {
        name: String,
        user: String,
        proxy: String,
    },
    #[error("No database named {name} under the pattern {pattern}")]
    NotFound {
        name: String,
        pattern: String,
        proxy: String,
    },
    #[error("The Teleport proxy failed: {why}")]
    ProxyFailed { why: String, command: String },
}

impl TeleportError {
    /// The command the person can run themselves. Freshkube never runs a
    /// login for them.
    pub fn command(&self) -> Option<String> {
        match self {
            Self::BadSettings(_) => None,
            Self::NotInstalled => Some("brew install teleport".into()),
            Self::NotLoggedIn { proxy } => Some(format!("tsh login --proxy={proxy}")),
            Self::NoRole { proxy, .. } => Some(format!("tsh status --proxy={proxy}")),
            Self::NotFound { proxy, .. } => Some(format!("tsh db ls --proxy={proxy}")),
            Self::ProxyFailed { command, .. } => Some(command.clone()),
        }
    }

    /// Stable, for element ids and tests.
    pub fn slug(&self) -> &'static str {
        match self {
            Self::BadSettings(_) => "bad-settings",
            Self::NotInstalled => "not-installed",
            Self::NotLoggedIn { .. } => "not-logged-in",
            Self::NoRole { .. } => "no-role",
            Self::NotFound { .. } => "not-found",
            Self::ProxyFailed { .. } => "proxy-failed",
        }
    }
}

fn plain(text: &str, extra: &[char]) -> bool {
    !text.is_empty()
        && text.len() <= 253
        && !text.starts_with('-')
        && text.chars().all(|c| {
            c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-') || extra.contains(&c)
        })
}

impl TeleportAccess {
    /// Refuses a setting that could be taken for an option or hold a space.
    pub fn validate(&self) -> Result<(), TeleportError> {
        let bad = |what: &str| {
            Err(TeleportError::BadSettings(format!(
                "the {what} has characters Teleport names don't"
            )))
        };
        if !plain(&self.proxy, &[':']) {
            return bad("proxy");
        }
        if !plain(&self.db_user, &['@']) {
            return bad("database user");
        }
        if !plain(&self.database, &[]) {
            return bad("database");
        }
        if !self.name_pattern.contains("{name}") {
            return Err(TeleportError::BadSettings(
                "the name pattern has no {name} in it".into(),
            ));
        }
        if !plain(&self.name_pattern.replace("{name}", "x"), &[]) {
            return bad("name pattern");
        }
        Ok(())
    }

    /// The Teleport database name for a Cluster.
    pub fn database_name(&self, cluster: &str) -> Result<String, TeleportError> {
        let name = self.name_pattern.replace("{name}", cluster);
        if plain(&name, &[]) {
            Ok(name)
        } else {
            Err(TeleportError::BadSettings(
                "the database name from the pattern has characters Teleport names don't".into(),
            ))
        }
    }

    pub fn status_args(&self) -> Vec<String> {
        vec![
            format!("--proxy={}", self.proxy),
            "status".into(),
            "--format=json".into(),
        ]
    }

    pub fn list_args(&self) -> Vec<String> {
        vec![
            format!("--proxy={}", self.proxy),
            "db".into(),
            "ls".into(),
            "--format=json".into(),
        ]
    }

    /// The proxy's arguments: no certificate, key, password or token. `--tunnel`
    /// has `tsh` sign in with its own certificate.
    pub fn proxy_args(&self, name: &str) -> Vec<String> {
        vec![
            format!("--proxy={}", self.proxy),
            "proxy".into(),
            "db".into(),
            "--tunnel".into(),
            format!("--db-user={}", self.db_user),
            format!("--db-name={}", self.database),
            name.into(),
        ]
    }

    fn proxy_command(&self, name: &str) -> String {
        format!("tsh {}", self.proxy_args(name).join(" "))
    }
}

/// `tsh` on `PATH`, or the file the person chose.
pub fn locate_tsh(chosen: Option<&Path>) -> Option<PathBuf> {
    if let Some(path) = chosen {
        return path.is_file().then(|| path.to_owned());
    }
    let program = format!("tsh{}", std::env::consts::EXE_SUFFIX);
    std::env::split_paths(&std::env::var_os("PATH")?)
        .map(|dir| dir.join(&program))
        .find(|candidate| candidate.is_file())
}

fn command(tsh: &Path) -> Command {
    let mut command = Command::new(tsh);
    command
        .env_clear()
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    for key in KEPT_ENV {
        if let Some(value) = std::env::var_os(key) {
            command.env(key, value);
        }
    }
    command
}

/// What the person's own paths are, to cut out of anything shown.
fn private_paths() -> Vec<String> {
    ["HOME", "TELEPORT_HOME", "USERPROFILE", "TMPDIR"]
        .iter()
        .filter_map(|k| std::env::var(k).ok())
        .collect()
}

fn shown(stderr: &str) -> String {
    let private = private_paths();
    let private: Vec<&str> = private.iter().map(String::as_str).collect();
    redact(stderr, &private)
}

struct Ran {
    ok: bool,
    stdout: String,
    stderr: String,
}

async fn run(tsh: &Path, args: &[String]) -> Result<Ran, TeleportError> {
    let child = command(tsh).args(args).spawn().map_err(spawn_error)?;
    let output = tokio::time::timeout(CHECK_TIMEOUT, child.wait_with_output())
        .await
        .map_err(|_| failed("tsh did not answer in time", ""))?
        .map_err(|e| failed(&e.to_string(), ""))?;
    Ok(Ran {
        ok: output.status.success(),
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
    })
}

fn spawn_error(error: std::io::Error) -> TeleportError {
    if error.kind() == std::io::ErrorKind::NotFound {
        TeleportError::NotInstalled
    } else {
        failed(&error.to_string(), "")
    }
}

fn failed(why: &str, command: &str) -> TeleportError {
    TeleportError::ProxyFailed {
        why: shown(why),
        command: command.to_owned(),
    }
}

/// Reads what `tsh`'s stderr says into one of the states.
fn classify(stderr: &str, access: &TeleportAccess, name: &str, command: &str) -> TeleportError {
    let lower = stderr.to_lowercase();
    let has = |words: &[&str]| words.iter().any(|w| lower.contains(w));
    if has(&[
        "not logged in",
        "please login",
        "please log in",
        "expired",
        "re-login",
        "relogin",
    ]) {
        TeleportError::NotLoggedIn {
            proxy: access.proxy.clone(),
        }
    } else if has(&[
        "access to db denied",
        "access denied",
        "not allowed to access",
        "not authorized",
    ]) {
        TeleportError::NoRole {
            name: name.to_owned(),
            user: access.db_user.clone(),
            proxy: access.proxy.clone(),
        }
    } else if has(&["database not found", "not found"]) {
        TeleportError::NotFound {
            name: name.to_owned(),
            pattern: access.name_pattern.clone(),
            proxy: access.proxy.clone(),
        }
    } else {
        failed(stderr, command)
    }
}

/// Signed in? A nonzero exit is "not signed in or expired", which is what
/// `tsh status` says for both. Only the exit and whether a profile is active
/// are read, never what is in the profile.
async fn check_login(tsh: &Path, access: &TeleportAccess) -> Result<(), TeleportError> {
    let not_logged_in = || TeleportError::NotLoggedIn {
        proxy: access.proxy.clone(),
    };
    let ran = run(tsh, &access.status_args()).await?;
    if !ran.ok {
        return Err(not_logged_in());
    }
    let active = serde_json::from_str::<serde_json::Value>(&ran.stdout)
        .ok()
        .map(|v| v.get("active").is_some_and(|a| !a.is_null()));
    if active == Some(false) {
        return Err(not_logged_in());
    }
    Ok(())
}

/// Is the database listed under this name? A role that can't see it never
/// lists it, so the two read alike here; a role that sees it but may not log
/// in is told by the proxy.
async fn check_listed(
    tsh: &Path,
    access: &TeleportAccess,
    name: &str,
) -> Result<(), TeleportError> {
    let ran = run(tsh, &access.list_args()).await?;
    if !ran.ok {
        return Err(classify(
            &ran.stderr,
            access,
            name,
            &access.proxy_command(name),
        ));
    }
    let listed = serde_json::from_str::<serde_json::Value>(&ran.stdout)
        .ok()
        .and_then(|v| v.as_array().cloned())
        .ok_or_else(|| {
            failed(
                "tsh db ls answered something other than a list",
                &access.proxy_command(name),
            )
        })?;
    let names = |entry: &serde_json::Value| {
        entry
            .pointer("/metadata/name")
            .or_else(|| entry.get("name"))
            .and_then(|n| n.as_str())
            .map(str::to_owned)
    };
    if listed.iter().filter_map(names).any(|n| n == name) {
        Ok(())
    } else {
        Err(TeleportError::NotFound {
            name: name.to_owned(),
            pattern: access.name_pattern.clone(),
            proxy: access.proxy.clone(),
        })
    }
}

/// The loopback address and port `tsh` announced it listens on.
enum Listen {
    Loopback(IpAddr, u16),
    /// Announced an address other than loopback.
    Exposed(String),
}

fn parse_listen(line: &str) -> Option<Listen> {
    for word in line.split_whitespace() {
        let word =
            word.trim_matches(|c: char| matches!(c, '"' | '\'' | ',' | ';' | '(' | ')' | '.'));
        // A trailing sentence dot was trimmed above; keep the port's digits.
        let Some((host, port)) = word.rsplit_once(':') else {
            continue;
        };
        let Ok(port) = port.trim_end_matches('.').parse::<u16>() else {
            continue;
        };
        let host = host.trim_matches(['[', ']']);
        if host == "localhost" {
            return Some(Listen::Loopback(Ipv4Addr::LOCALHOST.into(), port));
        }
        if let Ok(ip) = host.parse::<IpAddr>() {
            return Some(if ip.is_loopback() {
                Listen::Loopback(ip, port)
            } else {
                Listen::Exposed(ip.to_string())
            });
        }
    }
    None
}

type Tail = Arc<Mutex<VecDeque<String>>>;
const TAIL_LINES: usize = 40;

fn watch<R: AsyncRead + Unpin + Send + 'static>(
    stream: R,
    tail: Tail,
    ready: mpsc::UnboundedSender<Listen>,
) -> JoinHandle<()> {
    tokio::spawn(async move {
        let mut lines = BufReader::new(stream).lines();
        // Keep reading after the ready line so a full pipe never stalls `tsh`.
        while let Ok(Some(line)) = lines.next_line().await {
            if let Some(listen) = parse_listen(&line) {
                let _ = ready.send(listen);
            }
            let mut tail = tail.lock().unwrap_or_else(|e| e.into_inner());
            if tail.len() == TAIL_LINES {
                tail.pop_front();
            }
            tail.push_back(line);
        }
    })
}

/// A running `tsh proxy db --tunnel`. Dropping it kills the child.
pub struct Proxy {
    child: Child,
    address: IpAddr,
    port: u16,
    name: String,
    tail: Tail,
    readers: Vec<JoinHandle<()>>,
}

impl std::fmt::Debug for Proxy {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Proxy")
            .field("name", &self.name)
            .field("port", &self.port)
            .finish_non_exhaustive()
    }
}

impl Proxy {
    /// Checks `tsh`, the login and the name, then starts the proxy and waits
    /// for it to listen. Every failure is one of [`TeleportError`]'s states and
    /// leaves no child behind.
    pub async fn start(access: &TeleportAccess, cluster: &str) -> Result<Self, TeleportError> {
        Self::start_within(access, cluster, READY_TIMEOUT).await
    }

    pub async fn start_within(
        access: &TeleportAccess,
        cluster: &str,
        ready_within: Duration,
    ) -> Result<Self, TeleportError> {
        access.validate()?;
        let name = access.database_name(cluster)?;
        let tsh = locate_tsh(access.tsh.as_deref()).ok_or(TeleportError::NotInstalled)?;
        check_login(&tsh, access).await?;
        check_listed(&tsh, access, &name).await?;
        Self::spawn(&tsh, access, &name, ready_within).await
    }

    async fn spawn(
        tsh: &Path,
        access: &TeleportAccess,
        name: &str,
        ready_within: Duration,
    ) -> Result<Self, TeleportError> {
        let again = access.proxy_command(name);
        let mut child = command(tsh)
            .args(access.proxy_args(name))
            .spawn()
            .map_err(spawn_error)?;
        let tail = Tail::default();
        let (send, mut ready) = mpsc::unbounded_channel();
        let mut readers = Vec::new();
        if let Some(out) = child.stdout.take() {
            readers.push(watch(out, tail.clone(), send.clone()));
        }
        if let Some(err) = child.stderr.take() {
            readers.push(watch(err, tail.clone(), send.clone()));
        }
        drop(send);
        let stop = |readers: &[JoinHandle<()>]| readers.iter().for_each(JoinHandle::abort);
        let outcome = tokio::select! {
            listen = ready.recv() => listen.map(Ok),
            _ = child.wait() => None,
            _ = tokio::time::sleep(ready_within) => Some(Err(failed("tsh did not start listening in time", &again))),
        };
        match outcome {
            Some(Ok(Listen::Loopback(address, port))) => Ok(Self {
                child,
                address,
                port,
                name: name.to_owned(),
                tail,
                readers,
            }),
            Some(Ok(Listen::Exposed(address))) => {
                let _ = child.start_kill();
                stop(&readers);
                Err(failed(
                    &format!("tsh would listen on {address}, which is not this computer only"),
                    &again,
                ))
            }
            Some(Err(error)) => {
                let _ = child.start_kill();
                stop(&readers);
                Err(error)
            }
            None => {
                // It ended before it listened: let the readers finish, then say why.
                for reader in readers.iter_mut() {
                    let _ = tokio::time::timeout(Duration::from_secs(1), reader).await;
                }
                let text = tail_text(&tail);
                Err(classify(&text, access, name, &again))
            }
        }
    }

    /// Where the proxy listens: always loopback.
    pub fn address(&self) -> IpAddr {
        self.address
    }

    pub fn port(&self) -> u16 {
        self.port
    }

    /// The Teleport database it reaches.
    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn pid(&self) -> Option<u32> {
        self.child.id()
    }

    /// Whether the child is still running.
    pub fn running(&mut self) -> bool {
        matches!(self.child.try_wait(), Ok(None))
    }

    /// What the child last said, redacted and bounded, for a proxy that ended.
    pub fn tail(&self) -> String {
        shown(&tail_text(&self.tail))
    }
}

fn tail_text(tail: &Tail) -> String {
    let tail = tail.lock().unwrap_or_else(|e| e.into_inner());
    tail.iter().cloned().collect::<Vec<_>>().join("\n")
}

impl Drop for Proxy {
    fn drop(&mut self) {
        let _ = self.child.start_kill();
        self.readers.iter().for_each(JoinHandle::abort);
    }
}
