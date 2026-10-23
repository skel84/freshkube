//! Versions of an agent and which of them a profile supports.

use std::fmt;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use tokio::io::AsyncReadExt;

use super::{Dirs, Host, Kind, Profile};
use crate::investigation::clip;

/// How long `<agent> --version` has to answer.
pub const CHECK_TIMEOUT: Duration = Duration::from_secs(5);

const MAX_OUTPUT: usize = 4 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Version {
    pub major: u32,
    pub minor: u32,
    pub patch: u32,
}

impl Version {
    pub const fn new(major: u32, minor: u32, patch: u32) -> Self {
        Self {
            major,
            minor,
            patch,
        }
    }

    /// The first word of `text` that reads as a version: `2.1.0`, `v2.1.0`,
    /// or `opencode v2.1.0`. A pre-release or build suffix is ignored, so a
    /// release candidate counts as its release.
    pub fn parse(text: &str) -> Option<Self> {
        text.split_whitespace().find_map(Self::parse_word)
    }

    fn parse_word(word: &str) -> Option<Self> {
        let word = word.strip_prefix(['v', 'V']).unwrap_or(word);
        let core = word.split(['-', '+']).next()?;
        let mut parts = core.split('.');
        let mut number = || {
            let part = parts.next()?;
            if part.is_empty() || part.len() > 9 || !part.bytes().all(|b| b.is_ascii_digit()) {
                return None;
            }
            part.parse::<u32>().ok()
        };
        let version = Self::new(number()?, number()?, number()?);
        parts.next().is_none().then_some(version)
    }
}

impl fmt::Display for Version {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}.{}.{}", self.major, self.minor, self.patch)
    }
}

/// The versions a profile supports: `min <= v < below`, and the one its live
/// check last ran against.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Range {
    pub min: Version,
    pub below: Version,
    pub checked: Version,
}

impl Range {
    pub fn need(&self) -> String {
        format!(">= {}, < {}", self.min, self.below)
    }

    pub fn classify(&self, found: Version) -> VersionState {
        if found < self.min || found >= self.below {
            VersionState::Unsupported {
                found: found.to_string(),
                need: self.need(),
            }
        } else if found > self.checked {
            VersionState::Newer {
                found,
                checked: self.checked,
            }
        } else {
            VersionState::Supported { found }
        }
    }

    /// What `<agent> --version` printed.
    pub fn read(&self, output: &str) -> VersionState {
        match Version::parse(output) {
            Some(found) => self.classify(found),
            None => VersionState::Unsupported {
                found: clip(output.trim(), 80).to_string(),
                need: self.need(),
            },
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VersionState {
    /// Nothing was found, or what was found doesn't run.
    NotInstalled,
    /// `found` is what it printed (a version, or the start of whatever else).
    Unsupported {
        found: String,
        need: String,
    },
    /// Newer than the live check ran against: warns and still starts.
    Newer {
        found: Version,
        checked: Version,
    },
    Supported {
        found: Version,
    },
}

impl VersionState {
    /// Whether a session may start.
    pub fn can_start(&self) -> bool {
        matches!(self, Self::Newer { .. } | Self::Supported { .. })
    }
}

/// Proof that the profile's binary passed its version check, and the only
/// thing [`Profile::spawn`] accepts. Only [`Profile::check`] makes one, so a
/// session never starts on a version the profile doesn't support: another
/// version may read its permissions under other keys and fall back to
/// defaults that allow a shell.
///
/// The proof is for one profile kind and range, and for the file that was
/// run, not only its path: a binary replaced since (an upgrade between the
/// check and the start) is refused until it is checked again.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Checked {
    binary: PathBuf,
    file: FileId,
    kind: Kind,
    range: Option<Range>,
    /// None for a command with no version range.
    state: Option<VersionState>,
}

/// What names a file, beyond its path.
#[derive(Debug, Clone, PartialEq, Eq)]
struct FileId {
    len: u64,
    modified: Option<SystemTime>,
    #[cfg(unix)]
    device: u64,
    #[cfg(unix)]
    inode: u64,
}

impl FileId {
    fn of(path: &Path) -> Option<Self> {
        let meta = std::fs::metadata(path).ok()?;
        #[cfg(unix)]
        use std::os::unix::fs::MetadataExt;
        Some(Self {
            len: meta.len(),
            modified: meta.modified().ok(),
            #[cfg(unix)]
            device: meta.dev(),
            #[cfg(unix)]
            inode: meta.ino(),
        })
    }
}

impl Checked {
    /// A proof without a check, for tests of what a start does next.
    #[cfg(all(test, unix))]
    pub(crate) fn for_tests(profile: &Profile) -> Self {
        Self {
            binary: profile.binary().to_path_buf(),
            file: FileId::of(profile.binary()).unwrap(),
            kind: profile.kind,
            range: profile.range(),
            state: None,
        }
    }

    /// The binary that was checked.
    pub fn binary(&self) -> &Path {
        &self.binary
    }

    /// `Supported`, or `Newer` (a warning that still starts); none for a
    /// command with no range.
    pub fn state(&self) -> Option<&VersionState> {
        self.state.as_ref()
    }

    /// Whether this proof is for `profile` as it is now: the same kind and
    /// range, the same path, and the same file at that path.
    pub(super) fn is_for(&self, profile: &Profile) -> Result<(), &'static str> {
        if self.kind != profile.kind || self.range != profile.range() {
            return Err("the version check was made for another kind of agent");
        }
        if self.binary != profile.binary() {
            return Err("the version check was made on another binary");
        }
        if FileId::of(profile.binary()).as_ref() != Some(&self.file) {
            return Err("the binary changed since its version was checked; check it again");
        }
        Ok(())
    }
}

impl Profile {
    /// Checks the version and returns the proof a start needs, or the state
    /// that stops it.
    pub async fn check(&self, host: &Host) -> Result<Checked, VersionState> {
        self.check_within(host, CHECK_TIMEOUT).await
    }

    pub async fn check_within(
        &self,
        host: &Host,
        timeout: Duration,
    ) -> Result<Checked, VersionState> {
        let Some(file) = FileId::of(self.binary()) else {
            return Err(VersionState::NotInstalled);
        };
        let checked = |state| Checked {
            binary: self.binary().to_path_buf(),
            file: file.clone(),
            kind: self.kind,
            range: self.range(),
            state,
        };
        if self.range().is_none() {
            return Ok(checked(None));
        }
        let state = self.check_version_within(host, timeout).await;
        // A file replaced while it was being run is not the one that answered.
        if FileId::of(self.binary()).as_ref() != Some(&file) {
            return Err(VersionState::NotInstalled);
        }
        if state.can_start() {
            Ok(checked(Some(state)))
        } else {
            Err(state)
        }
    }

    /// Runs `<binary> --version` with the locked environment and reads the
    /// version. Never installs, upgrades or runs `npx`.
    pub async fn check_version(&self, host: &Host) -> VersionState {
        self.check_version_within(host, CHECK_TIMEOUT).await
    }

    pub async fn check_version_within(&self, host: &Host, timeout: Duration) -> VersionState {
        let Some(range) = self.range() else {
            return VersionState::NotInstalled;
        };
        let Ok(dirs) = Dirs::create() else {
            return VersionState::NotInstalled;
        };
        let plan = self.plan(&dirs, host).with_args(["--version"]);
        let Ok(mut running) = plan.spawn(&dirs) else {
            return VersionState::NotInstalled;
        };
        let Some(mut stdout) = running.child.stdout.take() else {
            return VersionState::NotInstalled;
        };
        let (guard, status) = running.guard(Box::new(dirs));
        let read = async {
            let mut buffer = Vec::new();
            let mut limited = (&mut stdout).take(MAX_OUTPUT as u64);
            limited.read_to_end(&mut buffer).await.ok()?;
            let status = status.await.ok()?.ok()?;
            status
                .success()
                .then(|| String::from_utf8_lossy(&buffer).into_owned())
        };
        let output = tokio::time::timeout(timeout, read).await;
        guard.end();
        match output {
            Ok(Some(text)) => range.read(&text),
            _ => VersionState::NotInstalled,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const RANGE: Range = Range {
        min: Version::new(2, 1, 0),
        below: Version::new(3, 0, 0),
        checked: Version::new(2, 1, 3),
    };

    fn state(output: &str) -> VersionState {
        RANGE.read(output)
    }

    #[test]
    fn version_states() {
        let need = ">= 2.1.0, < 3.0.0".to_string();
        let unsupported = |found: &str| VersionState::Unsupported {
            found: found.into(),
            need: need.clone(),
        };
        assert_eq!(
            state("opencode v2.1.0\n"),
            VersionState::Supported {
                found: Version::new(2, 1, 0)
            }
        );
        assert_eq!(
            state("2.1.3"),
            VersionState::Supported {
                found: Version::new(2, 1, 3)
            }
        );
        assert_eq!(state("2.0.9"), unsupported("2.0.9"));
        assert_eq!(
            state("2.4.0"),
            VersionState::Newer {
                found: Version::new(2, 4, 0),
                checked: Version::new(2, 1, 3)
            }
        );
        assert_eq!(state("3.0.0"), unsupported("3.0.0"));
        assert_eq!(state("1.19.4"), unsupported("1.19.4"));
        assert_eq!(state("not a version"), unsupported("not a version"));
        assert_eq!(state(""), unsupported(""));
        assert_eq!(state("2.1"), unsupported("2.1"));
        assert_eq!(state("2.1.0.1"), unsupported("2.1.0.1"));
        // A release candidate counts as its release.
        assert_eq!(state("v3.0.0-rc.1"), unsupported("3.0.0"));
        assert!(state("2.1.0").can_start() && state("2.9.9").can_start());
        assert!(!state("2.0.9").can_start() && !VersionState::NotInstalled.can_start());
    }

    #[test]
    fn a_long_unreadable_answer_is_cut() {
        let VersionState::Unsupported { found, .. } = state(&"x".repeat(10_000)) else {
            panic!("unsupported")
        };
        assert_eq!(found.len(), 80);
    }

    #[cfg(unix)]
    mod process {
        use std::os::unix::fs::PermissionsExt;
        use std::path::Path;

        use super::*;
        use crate::profile::Data;

        fn script(dir: &Path, body: &str) -> Profile {
            let path = dir.join("opencode");
            std::fs::write(&path, format!("#!/bin/sh\n{body}\n")).unwrap();
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
            Profile::opencode(path, Data::Shared)
        }

        #[tokio::test]
        async fn the_binary_is_asked_for_its_version() {
            let dir = tempfile::tempdir().unwrap();
            // It sees the locked environment: a cleared one.
            let profile = script(
                dir.path(),
                r#"[ "$1" = "--version" ] || exit 3; [ -z "$AWS_SECRET_ACCESS_KEY" ] || exit 4; echo "opencode v2.1.0""#,
            );
            let state = profile.check_version(&Host::default()).await;
            assert_eq!(
                state,
                VersionState::Supported {
                    found: Version::new(2, 1, 0)
                }
            );
        }

        #[tokio::test]
        async fn a_missing_binary_a_failing_one_and_a_hanging_one_are_not_installed() {
            let dir = tempfile::tempdir().unwrap();
            let host = Host::default();
            let missing = Profile::opencode(dir.path().join("nothing"), Data::Shared);
            assert_eq!(
                missing.check_version(&host).await,
                VersionState::NotInstalled
            );

            let failing = script(dir.path(), "echo 2.1.0; exit 1");
            assert_eq!(
                failing.check_version(&host).await,
                VersionState::NotInstalled
            );

            let hang = tempfile::tempdir().unwrap();
            let hanging = script(hang.path(), "sleep 30");
            let started = std::time::Instant::now();
            let state = hanging
                .check_version_within(&host, Duration::from_millis(300))
                .await;
            assert_eq!(state, VersionState::NotInstalled);
            assert!(started.elapsed() < Duration::from_secs(5));
        }

        #[tokio::test]
        async fn garbage_is_unsupported() {
            let dir = tempfile::tempdir().unwrap();
            let profile = script(dir.path(), "echo 'hello there'");
            assert!(matches!(
                profile.check_version(&Host::default()).await,
                VersionState::Unsupported { .. }
            ));
        }

        #[tokio::test]
        async fn only_a_passing_check_gives_the_proof_a_start_needs() {
            let host = Host::default();
            let old = tempfile::tempdir().unwrap();
            let unsupported = script(old.path(), "echo 2.0.9");
            assert!(matches!(
                unsupported.check(&host).await,
                Err(VersionState::Unsupported { .. })
            ));
            let missing = Profile::opencode(old.path().join("nothing"), Data::Shared);
            assert_eq!(missing.check(&host).await, Err(VersionState::NotInstalled));

            let new = tempfile::tempdir().unwrap();
            let supported = script(new.path(), "echo 2.1.3");
            let checked = supported.check(&host).await.unwrap();
            assert_eq!(
                checked.state(),
                Some(&VersionState::Supported {
                    found: Version::new(2, 1, 3)
                })
            );
            // A newer version warns and still starts.
            let newer_dir = tempfile::tempdir().unwrap();
            let newer = script(newer_dir.path(), "echo 2.9.0");
            assert!(matches!(
                newer.check(&host).await.unwrap().state(),
                Some(VersionState::Newer { .. })
            ));
            // The proof is for the binary that was checked.
            let other = script(old.path(), "echo 2.0.9");
            let error = other.spawn(&host, &checked).err().expect("refused");
            assert!(error.to_string().contains("another binary"), "{error}");
            // And for the right one it starts.
            assert!(supported.spawn(&host, &checked).is_ok());
        }

        #[tokio::test]
        async fn a_proof_for_another_kind_of_agent_is_refused() {
            let host = Host::default();
            let dir = tempfile::tempdir().unwrap();
            let opencode = script(dir.path(), "echo 2.0.9");
            // A generic command needs no version, so its proof says none was checked.
            let generic = Profile::command("generic", opencode.binary(), ["acp"]);
            let proof = generic
                .check(&host)
                .await
                .expect("a generic command passes");
            assert_eq!(proof.state(), None);
            let error = opencode.spawn(&host, &proof).err().expect("refused");
            assert!(
                error.to_string().contains("another kind of agent"),
                "{error}"
            );
        }

        #[tokio::test]
        async fn a_binary_replaced_after_its_check_is_refused() {
            let host = Host::default();
            let dir = tempfile::tempdir().unwrap();
            let profile = script(dir.path(), "echo 2.1.3");
            let checked = profile.check(&host).await.unwrap();
            assert!(profile.spawn(&host, &checked).is_ok());
            // An upgrade in place: the same path, another file.
            let path = profile.binary().to_path_buf();
            std::fs::write(&path, "#!/bin/sh\necho 9.9.9 # upgraded\n").unwrap();
            let error = profile.spawn(&host, &checked).err().expect("refused");
            assert!(error.to_string().contains("changed since"), "{error}");
            // Checked again, it is judged on what it is now.
            assert!(matches!(
                profile.check(&host).await,
                Err(VersionState::Unsupported { .. })
            ));
            // A file renamed over the path is another file too.
            let again = script(dir.path(), "echo 2.1.3");
            let checked = again.check(&host).await.unwrap();
            let other = dir.path().join("other");
            std::fs::write(&other, "#!/bin/sh\necho 2.1.3\n").unwrap();
            {
                use std::os::unix::fs::PermissionsExt;
                std::fs::set_permissions(&other, std::fs::Permissions::from_mode(0o755)).unwrap();
            }
            std::fs::rename(&other, again.binary()).unwrap();
            assert!(again.spawn(&host, &checked).is_err());
        }
    }
}
