//! Fetching a team file for real (#849 slice 3): the person's own `git`, or
//! an HTTPS request. Both are bounded in time and size, run on Tokio, and
//! stop when their future is dropped, so a read owner (`freshkube-reads`'
//! `Loader`, which keeps the source identity and the request generation)
//! cancels one by replacing it.
//!
//! Nothing from the repository runs: git reads objects, never a checkout.

mod git;
mod https;

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use super::file::Version;
use super::membership::FetchFailed;
use super::source::{Source, redact};
use crate::secrets::Secrets;

pub use git::GitRunner;
pub use https::HttpsFetch;

/// How long a fetch may take and how much it may read.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Limits {
    /// The whole fetch: git is killed, or the request dropped, at this time.
    pub deadline: Duration,
    /// What the mirror may hold after a fetch, and while it runs.
    pub mirror_bytes: u64,
    /// The setup file.
    pub file_bytes: u64,
    /// One key file.
    pub key_bytes: u64,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            deadline: Duration::from_secs(60),
            mirror_bytes: 64 * 1024 * 1024,
            file_bytes: crate::workspace::MAX_BYTES,
            key_bytes: super::keyfile::MOST_KEY_BYTES,
        }
    }
}

/// Fetches the team file from either kind of source.
pub struct TeamFetcher {
    git: GitRunner,
    https: HttpsFetch,
}

impl TeamFetcher {
    /// `app_dir` holds the mirror; `secrets` is the key store the optional
    /// HTTPS token is read from.
    pub fn new(app_dir: PathBuf, secrets: Option<Secrets>, limits: Limits) -> Self {
        Self {
            git: GitRunner::new(app_dir, limits),
            https: HttpsFetch::new(secrets, limits),
        }
    }

    pub async fn fetch(&self, source: &Source) -> Result<Version, FetchFailed> {
        match source {
            Source::Git(git) => self.git.fetch(git).await,
            Source::Https(https) => self.https.fetch(https).await,
        }
    }

    /// The work for a read owner: the fetch with its error as text.
    pub async fn work(self: Arc<Self>, source: Source) -> Result<Version, String> {
        self.fetch(&source).await.map_err(|error| error.0)
    }
}

/// Text a child printed, as an error may show it: every address cut to what
/// [`redact`] allows, and short.
pub(crate) fn redact_text(text: &str) -> String {
    let words: Vec<String> = text
        .split_whitespace()
        .map(|word| {
            let word = word.trim_matches(|c| matches!(c, '\'' | '"' | '`' | ',' | ';'));
            if word.contains("://") || word.contains('@') {
                redact(word)
            } else {
                word.to_string()
            }
        })
        .collect();
    let mut shown = words.join(" ");
    if shown.chars().count() > 300 {
        shown = shown.chars().take(300).collect::<String>() + "…";
    }
    shown
}

#[cfg(test)]
mod fake;
#[cfg(test)]
mod tests;
