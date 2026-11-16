//! The token `gh auth token` prints, for a setup that names `token: gh`.
//!
//! It runs only when the setup names it: `gh`'s token has wide scopes, so
//! nothing falls back to it. A GUI application doesn't inherit a shell's
//! `PATH`, so `gh` is looked for in the usual places first. The command is
//! pinned to github.com, and the environment's other GitHub hosts are taken
//! out of it, so the token that comes back is the one for the host it will
//! be sent to. Its standard error is dropped, never read, never shown; its
//! standard output is the token and is kept in memory only.

use std::path::PathBuf;
use std::time::Duration;

use tokio::io::AsyncReadExt;

use super::error::NotSetUp;

/// How long `gh auth token` may take.
pub(super) const DEADLINE: Duration = Duration::from_secs(10);
/// The most of its output read: a token is far shorter.
const MOST_OUTPUT_BYTES: u64 = 16 * 1024;

/// The usual places, then every directory of `PATH`.
pub(super) fn default_dirs() -> Vec<PathBuf> {
    freshkube_core::exec_plugin::default_dirs()
}

fn find(dirs: &[PathBuf]) -> Option<PathBuf> {
    freshkube_core::exec_plugin::find_program("gh", dirs)
}

/// How many times a busy `gh` is tried, and the pause between tries.
const BUSY_TRIES: u32 = 5;
const BUSY_PAUSE: Duration = Duration::from_millis(40);

/// What a failed spawn says: only a program that isn't there is "not found".
pub(super) fn spawn_failure(kind: std::io::ErrorKind) -> NotSetUp {
    match kind {
        std::io::ErrorKind::NotFound => NotSetUp::GhNotFound,
        other => NotSetUp::GhNotStarted(other),
    }
}

/// Starts `gh auth token`. A file that is busy (being written or updated)
/// is tried again a few times, never past `deadline`.
async fn spawn(
    gh: &std::path::Path,
    deadline: Duration,
) -> Result<tokio::process::Child, NotSetUp> {
    let started = tokio::time::Instant::now();
    let mut tries = 0;
    loop {
        let spawned = tokio::process::Command::new(gh)
            .args(["auth", "token", "--hostname", "github.com"])
            .env_remove("GH_HOST")
            .env_remove("GH_ENTERPRISE_TOKEN")
            .env_remove("GITHUB_ENTERPRISE_TOKEN")
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null())
            .kill_on_drop(true)
            .spawn();
        match spawned {
            Ok(child) => return Ok(child),
            Err(error) => {
                tries += 1;
                let busy = error.kind() == std::io::ErrorKind::ExecutableFileBusy;
                if !busy || tries >= BUSY_TRIES || started.elapsed() + BUSY_PAUSE >= deadline {
                    return Err(spawn_failure(error.kind()));
                }
                tokio::time::sleep(BUSY_PAUSE).await;
            }
        }
    }
}

/// The token `gh` holds for github.com, looking for `gh` in `dirs`.
pub(super) async fn token(dirs: &[PathBuf], deadline: Duration) -> Result<String, NotSetUp> {
    let gh = find(dirs).ok_or(NotSetUp::GhNotFound)?;
    let mut child = spawn(&gh, deadline).await?;
    let mut stdout = child.stdout.take().ok_or(NotSetUp::GhNotSignedIn)?;
    let run = async {
        let mut output = Vec::new();
        (&mut stdout)
            .take(MOST_OUTPUT_BYTES + 1)
            .read_to_end(&mut output)
            .await
            .ok()?;
        let status = child.wait().await.ok()?;
        status.success().then_some(output)
    };
    // On the deadline `run` is dropped, and with it the child, which is
    // killed.
    let output = tokio::time::timeout(deadline, run)
        .await
        .map_err(|_| NotSetUp::GhSlow)?
        .ok_or(NotSetUp::GhNotSignedIn)?;
    if output.len() as u64 > MOST_OUTPUT_BYTES {
        return Err(NotSetUp::TokenUnreadable);
    }
    let token = String::from_utf8(output).map_err(|_| NotSetUp::TokenUnreadable)?;
    let token = token.trim();
    if token.is_empty() {
        return Err(NotSetUp::GhNotSignedIn);
    }
    Ok(token.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::ErrorKind;

    #[test]
    fn only_a_missing_program_is_not_found() {
        assert_eq!(spawn_failure(ErrorKind::NotFound), NotSetUp::GhNotFound);
        for kind in [
            ErrorKind::ExecutableFileBusy,
            ErrorKind::PermissionDenied,
            ErrorKind::OutOfMemory,
        ] {
            let failure = spawn_failure(kind);
            assert_eq!(failure, NotSetUp::GhNotStarted(kind));
            let words = failure.to_string();
            assert!(!words.contains("not found"), "{words}");
            assert!(words.contains(&kind.to_string()), "{words}");
        }
    }
}
