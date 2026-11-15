//! The person's own `git`, into an app-owned bare mirror, read through git
//! objects. No checkout is made, so no hook, filter or worktree file runs.

use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

use tokio::io::{AsyncRead, AsyncReadExt};
use tokio::process::Command;

use super::{Limits, redact_text};
use crate::team::file::{KEY_FILE, Version};
use crate::team::keyfile::{self, KeyFileError};
use crate::team::membership::FetchFailed;
use crate::team::source::{ENV, GitFetch, GitSource, TRACKING_REF};

/// What git needs to find the person's config and sign-in. Nothing else of
/// the app's environment reaches it.
const PASS: &[&str] = &[
    "HOME",
    "PATH",
    "XDG_CONFIG_HOME",
    "SSH_AUTH_SOCK",
    "USERPROFILE",
    "APPDATA",
    "SYSTEMROOT",
    "LANG",
    // How the person's own git reaches their remote: a proxy and a CA.
    "HTTPS_PROXY",
    "https_proxy",
    "NO_PROXY",
    "no_proxy",
    "GIT_SSL_CAINFO",
    "SSL_CERT_FILE",
];

pub struct GitRunner {
    app_dir: PathBuf,
    limits: Limits,
    /// Tests fetch from a bare repository in a temporary directory, which
    /// the allowed protocols refuse; only they set this.
    #[cfg(test)]
    allow_file: bool,
    /// A test's own home, so no global git config of the developer applies.
    #[cfg(test)]
    home: Option<PathBuf>,
    /// A test passes config that would allow every transport; only the
    /// environment may then still refuse one.
    #[cfg(test)]
    loosen_config: bool,
    /// A test's own `PATH`, to run a stand-in for git.
    #[cfg(test)]
    path: Option<PathBuf>,
    /// How many blobs `cat-file` was asked for.
    #[cfg(test)]
    pub(crate) blobs_read: std::sync::atomic::AtomicUsize,
}

/// One entry `git ls-tree` printed.
struct Entry {
    mode: String,
    kind: String,
    id: String,
    size: Option<u64>,
}

impl GitRunner {
    pub fn new(app_dir: PathBuf, limits: Limits) -> Self {
        Self {
            app_dir,
            limits,
            #[cfg(test)]
            allow_file: false,
            #[cfg(test)]
            home: None,
            #[cfg(test)]
            loosen_config: false,
            #[cfg(test)]
            path: None,
            #[cfg(test)]
            blobs_read: Default::default(),
        }
    }

    #[cfg(test)]
    pub(crate) fn allowing_file_for_test(mut self) -> Self {
        self.allow_file = true;
        self
    }

    #[cfg(test)]
    pub(crate) fn with_home_for_test(mut self, home: PathBuf) -> Self {
        self.home = Some(home);
        self
    }

    #[cfg(test)]
    pub(crate) fn with_path_for_test(mut self, dir: PathBuf) -> Self {
        self.path = Some(dir);
        self
    }

    #[cfg(test)]
    pub(crate) fn loosening_config_for_test(mut self) -> Self {
        self.loosen_config = true;
        self
    }

    fn failed(what: &str) -> FetchFailed {
        FetchFailed(what.to_string())
    }

    /// The whole fetch, under one deadline. A failed or cut-short fetch
    /// deletes the mirror: it is a cache, and a killed git leaves lock files
    /// that would fail every later fetch.
    pub async fn fetch(&self, source: &GitSource) -> Result<Version, FetchFailed> {
        let result =
            match tokio::time::timeout(self.limits.deadline, self.fetch_in_time(source)).await {
                Ok(result) => result,
                Err(_) => Err(Self::failed("git took too long")),
            };
        if result.is_err() {
            self.forget_mirror().await;
        }
        result
    }

    async fn forget_mirror(&self) {
        let mirror = GitFetch::mirror_dir(&self.app_dir);
        let _ = tokio::task::spawn_blocking(move || std::fs::remove_dir_all(mirror)).await;
    }

    async fn fetch_in_time(&self, source: &GitSource) -> Result<Version, FetchFailed> {
        let mirror = GitFetch::mirror_dir(&self.app_dir);
        let hooks = self.app_dir.join("team-setup").join("no-hooks");
        std::fs::create_dir_all(&hooks)
            .map_err(|_| Self::failed("the team setup folder can’t be written"))?;
        if !mirror.join("HEAD").is_file() {
            self.init(&mirror, &hooks).await?;
        }
        #[allow(unused_mut)]
        let mut fetch = GitFetch::new(source, &mirror, &hooks, false);
        #[cfg(test)]
        if self.loosen_config {
            fetch.loosen_for_test();
        }
        self.run(&fetch.args, 64 * 1024, Some(&mirror)).await?;
        if self.mirror_size(&mirror).await > self.limits.mirror_bytes {
            return Err(Self::failed("the repository is larger than the limit"));
        }

        let commit = self
            .read(
                &mirror,
                &hooks,
                &["rev-parse", "--verify", "--quiet"],
                &format!("{TRACKING_REF}^{{commit}}"),
            )
            .await?;
        let commit = String::from_utf8_lossy(&commit).trim().to_string();
        if commit.len() < 12 || !commit.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err(Self::failed("git didn’t name a commit"));
        }
        let text = self
            .blob(
                &mirror,
                &hooks,
                &commit,
                source.path(),
                Blob::TeamFile,
                self.limits.file_bytes,
            )
            .await?;
        // The key file the file names, if any, is read at the same commit.
        let first = Version::parse(&commit[..12], &text, Default::default())?;
        let mut prints = std::collections::BTreeMap::new();
        if let Some(path) = first.values().get(KEY_FILE).and_then(|v| v.as_str()) {
            let pem = self
                .blob(
                    &mirror,
                    &hooks,
                    &commit,
                    path,
                    Blob::KeyFile,
                    self.limits.key_bytes,
                )
                .await?;
            let print = keyfile::fingerprint(&pem)
                .map_err(|error| FetchFailed(format!("the key file: {error}")))?;
            prints.insert(path.to_string(), print);
        }
        Ok(Version::parse(&commit[..12], &text, prints)?)
    }

    async fn mirror_size(&self, mirror: &Path) -> u64 {
        let mirror = mirror.to_path_buf();
        tokio::task::spawn_blocking(move || dir_size(&mirror))
            .await
            .unwrap_or(0)
    }

    async fn init(&self, mirror: &Path, hooks: &Path) -> Result<(), FetchFailed> {
        let args: Vec<String> = [
            "-c".to_string(),
            format!("core.hooksPath={}", hooks.display()),
            "-c".into(),
            "init.templateDir=".into(),
            "init".into(),
            "--bare".into(),
            "--quiet".into(),
            mirror.display().to_string(),
        ]
        .into();
        if let Some(parent) = mirror.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|_| Self::failed("the team setup folder can’t be written"))?;
        }
        self.run(&args, 4096, None).await.map(|_| ())
    }

    async fn read(
        &self,
        mirror: &Path,
        hooks: &Path,
        rest: &[&str],
        last: &str,
    ) -> Result<Vec<u8>, FetchFailed> {
        let mut args = GitFetch::read(mirror, hooks, rest);
        args.push(last.to_string());
        self.run(&args, 4096, None).await
    }

    /// The bytes of the plain file at `path` in `commit`, by its tree entry.
    /// `what` says which file it is, so every refusal names it.
    async fn blob(
        &self,
        mirror: &Path,
        hooks: &Path,
        commit: &str,
        path: &str,
        what: Blob,
        most: u64,
    ) -> Result<Vec<u8>, FetchFailed> {
        let refuse = |error: KeyFileError| FetchFailed(what.words(&error, most));
        keyfile::check_repo_path(path).map_err(refuse)?;
        let mut args = GitFetch::read(mirror, hooks, &["ls-tree", "-l", "-z", commit, "--"]);
        args.push(path.to_string());
        // A git that fails or runs out of time is not "not in the repository".
        let out = self.run(&args, 4096, None).await?;
        let entry = parse_entry(&out).map_err(refuse)?;
        match (entry.mode.as_str(), entry.kind.as_str()) {
            ("120000", _) => return Err(refuse(KeyFileError::Symlink)),
            ("100644" | "100755", "blob") => {}
            _ => return Err(refuse(KeyFileError::NotAFile)),
        }
        // The size is known before a byte is read.
        if entry.size.is_none_or(|size| size > most) {
            return Err(refuse(KeyFileError::TooLarge));
        }
        #[cfg(test)]
        self.blobs_read
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let args = GitFetch::read(mirror, hooks, &["cat-file", "blob", &entry.id]);
        let bytes = self.run(&args, most + 1, None).await?;
        if bytes.len() as u64 > most {
            return Err(refuse(KeyFileError::TooLarge));
        }
        Ok(bytes)
    }

    /// Runs git in a process group of its own, reading at most `cap` bytes of
    /// its output. A fetch (`watch`) also ends when the mirror outgrows its
    /// limit. Whatever ends it first, dropped future included, the whole
    /// group is killed, so `git-remote-https` and `index-pack` stop too.
    async fn run(
        &self,
        args: &[String],
        cap: u64,
        watch: Option<&Path>,
    ) -> Result<Vec<u8>, FetchFailed> {
        let mut command = Command::new("git");
        command
            .args(args)
            .env_clear()
            .current_dir(&self.app_dir)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        #[cfg(unix)]
        command.process_group(0);
        for name in PASS {
            if let Some(value) = std::env::var_os(name) {
                command.env(name, value);
            }
        }
        for (name, value) in ENV {
            command.env(name, value);
        }
        // A test fetches from a repository in its own temporary directory, a
        // `file` transport the shipped environment refuses, under its own
        // home so the developer's git config never applies.
        #[cfg(test)]
        {
            if self.allow_file {
                command.env("GIT_ALLOW_PROTOCOL", "https:ssh:file");
            }
            if let Some(dir) = &self.path {
                command.env("PATH", format!("{}:/usr/bin:/bin", dir.display()));
            }
            if let Some(home) = &self.home {
                command
                    .env("HOME", home)
                    .env("XDG_CONFIG_HOME", home)
                    .env("GIT_CONFIG_NOSYSTEM", "1");
            }
        }
        let mut child = command
            .spawn()
            .map_err(|_| Self::failed("git couldn’t be started; is it installed?"))?;
        let stdout = child.stdout.take().expect("piped");
        let stderr = child.stderr.take().expect("piped");
        // The group owns the child, so that on a kill it can reap it before
        // it asks whether the group is gone.
        let mut group = Group::new(child);
        let work = async {
            let (out, err) = tokio::join!(capped(stdout, cap), capped(stderr, 4096));
            let status = group.wait().await;
            (out, err, status)
        };
        let guard = async {
            let Some(mirror) = watch else {
                return std::future::pending::<()>().await;
            };
            loop {
                tokio::time::sleep(Duration::from_millis(50)).await;
                if self.mirror_size(mirror).await > self.limits.mirror_bytes {
                    return;
                }
            }
        };
        tokio::select! {
            (out, err, status) = work => {
                group.disarm();
                if status.is_ok_and(|s| s.success()) && out.len() as u64 <= cap {
                    Ok(out)
                } else {
                    let said = redact_text(&String::from_utf8_lossy(&err));
                    Err(FetchFailed(if said.is_empty() {
                        "git didn’t finish".to_string()
                    } else {
                        format!("git said: {said}")
                    }))
                }
            }
            () = guard => Err(Self::failed("the repository is larger than the limit")),
        }
    }
}

/// Which file a blob read is, for the words of its refusal.
#[derive(Clone, Copy)]
enum Blob {
    TeamFile,
    KeyFile,
}

impl Blob {
    fn words(self, error: &KeyFileError, most: u64) -> String {
        match self {
            Blob::KeyFile => format!("the key file: {error}"),
            Blob::TeamFile => format!(
                "the team file: {}",
                match error {
                    KeyFileError::NotFound => "it isn’t in the repository at that path".to_string(),
                    KeyFileError::NotAFile => "it isn’t a regular file".to_string(),
                    KeyFileError::Symlink => "it is a link, which is refused".to_string(),
                    KeyFileError::TooLarge => format!("it is larger than {} KiB", most / 1024),
                    KeyFileError::Absolute
                    | KeyFileError::ParentSegment
                    | KeyFileError::Escapes => "its path leaves the repository".to_string(),
                    KeyFileError::NotAKey => "it isn’t readable".to_string(),
                }
            ),
        }
    }
}

/// A running git and its process group. While armed, dropping it kills every
/// process in the group and waits briefly for them to go (reaping git itself
/// on each pass, since a zombie keeps its group "there"), so a download that
/// `git` started stops with it. Disarmed once git has finished by itself.
struct Group {
    child: tokio::process::Child,
    armed: bool,
}

impl Group {
    fn new(child: tokio::process::Child) -> Self {
        Self { child, armed: true }
    }

    async fn wait(&mut self) -> std::io::Result<std::process::ExitStatus> {
        self.child.wait().await
    }

    fn disarm(&mut self) {
        self.armed = false;
    }
}

impl Drop for Group {
    fn drop(&mut self) {
        if !std::mem::take(&mut self.armed) {
            return;
        }
        #[cfg(unix)]
        if let Some(pgid) = self.child.id().map(|pid| pid as i32) {
            // SAFETY: kill(2) with a negative pid signals a process group,
            // here the one git was started as the leader of.
            unsafe { libc::kill(-pgid, libc::SIGKILL) };
            for _ in 0..200 {
                // Reap git, so a zombie doesn't hold its group open.
                let _ = self.child.try_wait();
                // SAFETY: signal 0 only checks that the group exists.
                if unsafe { libc::kill(-pgid, 0) } != 0 {
                    break;
                }
                std::thread::sleep(Duration::from_millis(10));
            }
        }
        #[cfg(windows)]
        if let Some(pid) = self.child.id() {
            use std::os::windows::process::CommandExt;
            // The tree under git, by the system's own taskkill, with no
            // environment and no console window. `/T` follows parent links,
            // so a helper whose parent already exited is missed: a stopgap
            // for a job object.
            let root = std::env::var_os("SystemRoot").unwrap_or_else(|| "C:\\Windows".into());
            let _ = std::process::Command::new(
                std::path::Path::new(&root)
                    .join("System32")
                    .join("taskkill.exe"),
            )
            .args(["/T", "/F", "/PID", &pid.to_string()])
            .env_clear()
            .creation_flags(0x0800_0000)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
        }
    }
}

/// Reads up to `cap` bytes, then keeps draining so the child isn't blocked,
/// returning one byte more than `cap` when there was more.
async fn capped(mut stream: impl AsyncRead + Unpin, cap: u64) -> Vec<u8> {
    let mut kept = Vec::new();
    let _ = (&mut stream).take(cap).read_to_end(&mut kept).await;
    if kept.len() as u64 >= cap {
        // More than the cap is an answer too big to keep: say so by one byte
        // beyond it, and drain the rest so the child isn't blocked.
        let mut extra = [0u8; 1];
        if matches!(stream.read(&mut extra).await, Ok(n) if n > 0) {
            kept.push(extra[0]);
            let _ = tokio::io::copy(&mut stream, &mut tokio::io::sink()).await;
        }
    }
    kept
}

fn dir_size(dir: &Path) -> u64 {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return 0;
    };
    entries
        .flatten()
        .map(|entry| match entry.metadata() {
            Ok(meta) if meta.is_dir() => dir_size(&entry.path()),
            Ok(meta) => meta.len(),
            Err(_) => 0,
        })
        .sum()
}

fn parse_entry(out: &[u8]) -> Result<Entry, KeyFileError> {
    let text = std::str::from_utf8(out).map_err(|_| KeyFileError::NotFound)?;
    let mut lines = text.split('\0').filter(|line| !line.is_empty());
    let line = lines.next().ok_or(KeyFileError::NotFound)?;
    if lines.next().is_some() {
        return Err(KeyFileError::NotAFile);
    }
    let (meta, _name) = line.split_once('\t').ok_or(KeyFileError::NotFound)?;
    let mut parts = meta.split_whitespace();
    match (parts.next(), parts.next(), parts.next(), parts.next()) {
        (Some(mode), Some(kind), Some(id), size) => Ok(Entry {
            mode: mode.into(),
            kind: kind.into(),
            id: id.into(),
            size: size.and_then(|size| size.parse().ok()),
        }),
        _ => Err(KeyFileError::NotFound),
    }
}
