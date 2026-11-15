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
];

pub struct GitRunner {
    app_dir: PathBuf,
    limits: Limits,
    /// Tests fetch from a bare repository in a temporary directory, which
    /// `protocol.file.allow=never` refuses; only they set this.
    #[cfg(test)]
    allow_file: bool,
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
        }
    }

    #[cfg(test)]
    pub(crate) fn allowing_file_for_test(mut self) -> Self {
        self.allow_file = true;
        self
    }

    fn failed(what: &str) -> FetchFailed {
        FetchFailed(what.to_string())
    }

    pub async fn fetch(&self, source: &GitSource) -> Result<Version, FetchFailed> {
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
        if self.allow_file {
            fetch.allow_file_for_test();
        }
        let fetched = self.run(&fetch.args, 64 * 1024, Some(&mirror)).await;
        // A mirror past its limit is forgotten, so it never grows on disk.
        let too_big = dir_size(&mirror) > self.limits.mirror_bytes;
        if too_big {
            let _ = std::fs::remove_dir_all(&mirror);
            return Err(Self::failed("the repository is larger than the limit"));
        }
        fetched?;

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
                self.limits.file_bytes,
            )
            .await
            .map_err(|error| FetchFailed(format!("the team file: {error}")))?;
        // The key file the file names, if any, is read at the same commit.
        let first = Version::parse(&commit[..12], &text, Default::default())?;
        let mut prints = std::collections::BTreeMap::new();
        if let Some(path) = first.values().get(KEY_FILE).and_then(|v| v.as_str()) {
            let pem = self
                .blob(&mirror, &hooks, &commit, path, self.limits.key_bytes)
                .await
                .map_err(|error| FetchFailed(format!("the key file: {error}")))?;
            let print = keyfile::fingerprint(&pem)
                .map_err(|error| FetchFailed(format!("the key file: {error}")))?;
            prints.insert(path.to_string(), print);
        }
        Ok(Version::parse(&commit[..12], &text, prints)?)
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
    async fn blob(
        &self,
        mirror: &Path,
        hooks: &Path,
        commit: &str,
        path: &str,
        most: u64,
    ) -> Result<Vec<u8>, KeyFileError> {
        keyfile::check_repo_path(path)?;
        let mut args = GitFetch::read(mirror, hooks, &["ls-tree", "-l", "-z", commit, "--"]);
        args.push(path.to_string());
        let out = self
            .run(&args, 4096, None)
            .await
            .map_err(|_| KeyFileError::NotFound)?;
        let entry = parse_entry(&out)?;
        match (entry.mode.as_str(), entry.kind.as_str()) {
            ("120000", _) => return Err(KeyFileError::Symlink),
            ("100644" | "100755", "blob") => {}
            _ => return Err(KeyFileError::NotAFile),
        }
        // The size is known before a byte is read.
        if entry.size.is_none_or(|size| size > most) {
            return Err(KeyFileError::TooLarge);
        }
        let args = GitFetch::read(mirror, hooks, &["cat-file", "blob", &entry.id]);
        let bytes = self
            .run(&args, most + 1, None)
            .await
            .map_err(|_| KeyFileError::NotFound)?;
        if bytes.len() as u64 > most {
            return Err(KeyFileError::TooLarge);
        }
        Ok(bytes)
    }

    /// Runs git, reading at most `cap` bytes of its output, for at most the
    /// deadline, and (for a fetch) until the mirror outgrows its limit. A
    /// dropped future kills the child.
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
        for name in PASS {
            if let Some(value) = std::env::var_os(name) {
                command.env(name, value);
            }
        }
        for (name, value) in ENV {
            command.env(name, value);
        }
        let mut child = command
            .spawn()
            .map_err(|_| Self::failed("git couldn’t be started; is it installed?"))?;
        let stdout = child.stdout.take().expect("piped");
        let stderr = child.stderr.take().expect("piped");
        let work = async {
            let (out, err) = tokio::join!(capped(stdout, cap), capped(stderr, 4096));
            let status = child.wait().await;
            (out, err, status)
        };
        let guard = async {
            let Some(mirror) = watch else {
                return std::future::pending::<()>().await;
            };
            loop {
                tokio::time::sleep(Duration::from_millis(50)).await;
                if dir_size(mirror) > self.limits.mirror_bytes {
                    return;
                }
            }
        };
        tokio::select! {
            (out, err, status) = work => {
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
            () = tokio::time::sleep(self.limits.deadline) => Err(Self::failed("git took too long")),
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
