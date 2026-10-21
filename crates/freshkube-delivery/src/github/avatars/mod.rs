//! Avatars on disk, for the pull request card.
//!
//! A second, separate client: it has no default headers and no token, and
//! is never handed one, so an avatar request cannot carry `Authorization`
//! whatever the REST client holds. What it fetches is built here from the
//! digits of a checked [`AvatarUrl`], never copied from an answer, and
//! nothing it fetches is followed anywhere: a redirect is a refusal.
//!
//! Files are named `u<id>-v<version>-s<size>` from those digits, so no
//! name can leave the folder. A file is written whole under a temporary
//! name and renamed into place, so no partial file is ever the avatar. It
//! is fresh for seven days by its modification time; an older one is still
//! shown while a new one is fetched. The folder is kept under 20 MiB: after
//! a write that goes over, the oldest files go until it is under 16 MiB.
//! A client change does not touch the folder, since avatars carry no token.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime};

use url::Url;

use super::rest::{AVATAR_HOST, AvatarUrl};

const DEADLINE: Duration = Duration::from_secs(5);
/// The most an avatar's body may be.
pub const MOST_BODY_BYTES: usize = 256 * 1024;
/// How long a file is the avatar before it is fetched again.
pub const FRESH_FOR: Duration = Duration::from_secs(7 * 24 * 3600);
/// The folder's size, past which the oldest files go.
pub const MOST_FOLDER_BYTES: u64 = 20 * 1024 * 1024;
/// What the folder is cut down to.
pub const KEPT_FOLDER_BYTES: u64 = 16 * 1024 * 1024;
/// The sizes asked for, in pixels.
const SIZES: std::ops::RangeInclusive<u32> = 16..=512;
/// A leftover temporary file this old is not another write's.
const TEMP_GRACE: Duration = Duration::from_secs(60);
/// How long an avatar that couldn't be fetched is not asked for again.
pub const FAILURE_BACKOFF: Duration = Duration::from_secs(5 * 60);

/// Why an avatar wasn't fetched, safe to display.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    Unreachable,
    /// Any status but 200, a redirect included.
    Refused(u16),
    /// Longer than [`MOST_BODY_BYTES`].
    TooBig,
    /// Not a PNG, JPEG or GIF, by its type or its first bytes.
    NotAnImage,
    /// The folder could not be written.
    Disk,
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Unreachable => f.write_str("the avatar host didn't answer"),
            Self::Refused(status) => write!(f, "the avatar host answered {status}"),
            Self::TooBig => f.write_str("the avatar is larger than 256 KiB"),
            Self::NotAnImage => f.write_str("the avatar is not a PNG, JPEG or GIF"),
            Self::Disk => f.write_str("the avatar could not be saved"),
        }
    }
}

impl std::error::Error for Error {}

/// An avatar on disk.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Avatar {
    pub path: PathBuf,
    /// Older than [`FRESH_FOR`]: show it, and fetch it again.
    pub stale: bool,
}

/// No `Debug`: the folder is the person's.
pub struct Avatars {
    dir: PathBuf,
    http: reqwest::Client,
    base: Url,
    swept: AtomicBool,
    writes: AtomicU64,
    /// One fetch of a file at a time: the others wait for it and find it.
    flights: Mutex<HashMap<String, Arc<tokio::sync::Mutex<()>>>>,
    /// Files that couldn't be fetched, and when, so they aren't asked for
    /// again for [`FAILURE_BACKOFF`].
    failures: Mutex<HashMap<String, (Instant, Error)>>,
}

fn file_name(avatar: &AvatarUrl, size: u32) -> String {
    format!(
        "u{}-v{}-s{}",
        avatar.id(),
        avatar.version().unwrap_or(0),
        size
    )
}

/// Whether `bytes` begin as a PNG, a JPEG or a GIF.
fn is_image(bytes: &[u8]) -> bool {
    bytes.starts_with(b"\x89PNG\r\n\x1a\n")
        || bytes.starts_with(b"\xFF\xD8\xFF")
        || bytes.starts_with(b"GIF87a")
        || bytes.starts_with(b"GIF89a")
}

fn image_type(value: Option<&http::HeaderValue>) -> bool {
    let Some(value) = value.and_then(|value| value.to_str().ok()) else {
        return false;
    };
    let kind = value.split(';').next().unwrap_or_default().trim();
    ["image/png", "image/jpeg", "image/gif"]
        .iter()
        .any(|allowed| kind.eq_ignore_ascii_case(allowed))
}

impl Avatars {
    /// Avatars kept in `dir`, fetched from GitHub's avatar host.
    pub fn new(dir: PathBuf) -> Result<Self, Error> {
        let base = Url::parse(&format!("https://{AVATAR_HOST}/")).map_err(|_| Error::Disk)?;
        Self::from(reqwest::Client::builder(), dir, base)
    }

    fn from(builder: reqwest::ClientBuilder, dir: PathBuf, base: Url) -> Result<Self, Error> {
        let http = builder
            .use_rustls_tls()
            .https_only(true)
            .redirect(reqwest::redirect::Policy::none())
            .connect_timeout(DEADLINE)
            .timeout(DEADLINE)
            .build()
            .map_err(|_| Error::Unreachable)?;
        Ok(Self {
            dir,
            http,
            base,
            swept: AtomicBool::new(false),
            writes: AtomicU64::new(0),
            flights: Mutex::default(),
            failures: Mutex::default(),
        })
    }

    fn path(&self, avatar: &AvatarUrl, size: u32) -> PathBuf {
        self.dir
            .join(file_name(avatar, size.clamp(*SIZES.start(), *SIZES.end())))
    }

    /// What is on disk for the avatar at `size` pixels, if anything. It
    /// reads only the file's metadata.
    pub fn cached(&self, avatar: &AvatarUrl, size: u32) -> Option<Avatar> {
        let path = self.path(avatar, size);
        let modified = std::fs::metadata(&path).ok()?.modified().ok();
        // A file from the future, by a clock that was wrong, is not fresh
        // for ever: it is as old as it can be, and is fetched again.
        let stale = modified
            .and_then(|at| SystemTime::now().duration_since(at).ok())
            .is_none_or(|age| age >= FRESH_FOR);
        Some(Avatar { path, stale })
    }

    /// The avatar to show: the file when it is fresh, else the one fetch
    /// that replaces it, however many ask at once. A fetch that failed is
    /// not tried again for [`FAILURE_BACKOFF`], and meanwhile an old file
    /// stands, or the same refusal is the answer.
    pub async fn ensure(&self, avatar: &AvatarUrl, size: u32) -> Result<Avatar, Error> {
        let size = size.clamp(*SIZES.start(), *SIZES.end());
        let name = file_name(avatar, size);
        if let Some(held) = self.cached(avatar, size).filter(|held| !held.stale) {
            return Ok(held);
        }
        let flight = self
            .flights
            .lock()
            .unwrap()
            .entry(name.clone())
            .or_default()
            .clone();
        let answer = {
            let _turn = flight.lock().await;
            self.ensure_in_turn(avatar, size, &name).await
        };
        // The last one out takes the flight away.
        let mut flights = self.flights.lock().unwrap();
        drop(flight);
        if flights
            .get(&name)
            .is_some_and(|held| Arc::strong_count(held) == 1)
        {
            flights.remove(&name);
        }
        answer
    }

    async fn ensure_in_turn(
        &self,
        avatar: &AvatarUrl,
        size: u32,
        name: &str,
    ) -> Result<Avatar, Error> {
        // Another call may have fetched it while this one waited its turn.
        let held = self.cached(avatar, size);
        if let Some(held) = held.clone().filter(|held| !held.stale) {
            return Ok(held);
        }
        let failed = self
            .failures
            .lock()
            .unwrap()
            .get(name)
            .filter(|(at, _)| at.elapsed() < FAILURE_BACKOFF)
            .map(|(_, error)| *error);
        if let Some(error) = failed {
            return held.ok_or(error);
        }
        match self.fetch(avatar, size).await {
            Ok(fetched) => {
                self.failures.lock().unwrap().remove(name);
                Ok(fetched)
            }
            Err(error) => {
                self.failures
                    .lock()
                    .unwrap()
                    .insert(name.to_owned(), (Instant::now(), error));
                held.ok_or(error)
            }
        }
    }

    /// Fetches the avatar at `size` pixels and saves it, whatever is on disk.
    pub async fn fetch(&self, avatar: &AvatarUrl, size: u32) -> Result<Avatar, Error> {
        let size = size.clamp(*SIZES.start(), *SIZES.end());
        let mut url = self
            .base
            .join(&format!("u/{}", avatar.id()))
            .map_err(|_| Error::Disk)?;
        {
            let mut query = url.query_pairs_mut();
            if let Some(version) = avatar.version() {
                query.append_pair("v", &version.to_string());
            }
            query.append_pair("s", &size.to_string());
        }
        let mut response = self
            .http
            .get(url)
            .header(http::header::ACCEPT, "image/png, image/jpeg, image/gif")
            .header(
                http::header::USER_AGENT,
                concat!("freshkube/", env!("CARGO_PKG_VERSION")),
            )
            .send()
            .await
            .map_err(|_| Error::Unreachable)?;
        // A redirect is not followed, and an error's body is not read.
        if response.status().as_u16() != 200 {
            return Err(Error::Refused(response.status().as_u16()));
        }
        if !image_type(response.headers().get(http::header::CONTENT_TYPE)) {
            return Err(Error::NotAnImage);
        }
        let mut body = Vec::new();
        while let Some(chunk) = response.chunk().await.map_err(|_| Error::Unreachable)? {
            if body.len() + chunk.len() > MOST_BODY_BYTES {
                return Err(Error::TooBig);
            }
            body.extend_from_slice(&chunk);
        }
        if !is_image(&body) {
            return Err(Error::NotAnImage);
        }
        let dir = self.dir.clone();
        let path = self.path(avatar, size);
        let first = !self.swept.swap(true, Ordering::SeqCst);
        let n = self.writes.fetch_add(1, Ordering::SeqCst);
        let saved = path.clone();
        tokio::task::spawn_blocking(move || save(&dir, &saved, &body, first, n))
            .await
            .map_err(|_| Error::Disk)??;
        Ok(Avatar { path, stale: false })
    }
}

/// Writes `body` to `path` whole: under a temporary name in the same
/// folder, then renamed over it. The first write also sweeps what an
/// earlier run left.
fn save(dir: &Path, path: &Path, body: &[u8], sweep: bool, n: u64) -> Result<(), Error> {
    std::fs::create_dir_all(dir).map_err(|_| Error::Disk)?;
    if sweep {
        sweep_temporary(dir);
    }
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or(Error::Disk)?;
    let temporary = dir.join(format!("{name}.tmp-{}-{n}", std::process::id()));
    let written = std::fs::write(&temporary, body).and_then(|()| std::fs::rename(&temporary, path));
    if written.is_err() {
        _ = std::fs::remove_file(&temporary);
        return Err(Error::Disk);
    }
    cut(dir);
    Ok(())
}

fn is_temporary(name: &str) -> bool {
    name.contains(".tmp-")
}

/// Removes temporary files no write is still making.
fn sweep_temporary(dir: &Path) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let old = entry
            .metadata()
            .and_then(|meta| meta.modified())
            .ok()
            .and_then(|at| SystemTime::now().duration_since(at).ok())
            .is_some_and(|age| age >= TEMP_GRACE);
        if old && entry.file_name().to_str().is_some_and(is_temporary) {
            _ = std::fs::remove_file(entry.path());
        }
    }
}

/// Past [`MOST_FOLDER_BYTES`], removes the oldest files by modification
/// time until the folder is under [`KEPT_FOLDER_BYTES`].
fn cut(dir: &Path) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    let mut files: Vec<(SystemTime, u64, PathBuf)> = entries
        .flatten()
        .filter(|entry| {
            entry
                .file_name()
                .to_str()
                .is_some_and(|name| !is_temporary(name))
        })
        .filter_map(|entry| {
            let meta = entry.metadata().ok()?;
            meta.is_file().then(|| {
                (
                    meta.modified().unwrap_or(SystemTime::UNIX_EPOCH),
                    meta.len(),
                    entry.path(),
                )
            })
        })
        .collect();
    let mut total: u64 = files.iter().map(|(_, len, _)| len).sum();
    if total <= MOST_FOLDER_BYTES {
        return;
    }
    files.sort();
    for (_, len, path) in files {
        if total < KEPT_FOLDER_BYTES {
            break;
        }
        if std::fs::remove_file(path).is_ok() {
            total -= len;
        }
    }
}

#[cfg(test)]
impl Avatars {
    /// Makes every remembered failure `by` older.
    pub(crate) fn age_failures(&self, by: Duration) {
        for (at, _) in self.failures.lock().unwrap().values_mut() {
            *at -= by;
        }
    }

    /// Avatars fetched from the fake at `address`, which this trusts alone.
    pub(crate) fn trusting(dir: PathBuf, address: &str) -> Self {
        let root = reqwest::Certificate::from_pem(super::fake::CERT).unwrap();
        let builder = reqwest::Client::builder()
            .add_root_certificate(root)
            .no_proxy();
        Self::from(builder, dir, Url::parse(&format!("{address}/")).unwrap()).unwrap()
    }
}

#[cfg(test)]
mod tests;
