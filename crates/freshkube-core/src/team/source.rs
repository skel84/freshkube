//! Where a team file may come from, and how it is fetched safely.

use std::path::{Path, PathBuf};

use url::Url;

/// Why a source or a redirect is refused. The text never repeats the address.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Refused {
    NotAnAddress,
    PlainHttp,
    Scheme,
    Credentials,
    LocalPath,
    BadRef,
    BadPath,
    CrossHostRedirect,
}

impl std::fmt::Display for Refused {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Refused::NotAnAddress => "that isn’t an address",
            Refused::PlainHttp => "plain HTTP is refused; use an https:// address",
            Refused::Scheme => "only https://, ssh:// and git@host:path repositories are read",
            Refused::Credentials => "an address with a user name or password in it is refused",
            Refused::LocalPath => "a path on this computer is not a team source",
            Refused::BadRef => "that branch or tag name can’t be used",
            Refused::BadPath => "that file path can’t be used",
            Refused::CrossHostRedirect => "the address redirects to another host, which is refused",
        })
    }
}

impl std::error::Error for Refused {}

/// A team source: a git repository or an HTTPS address.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Source {
    Git(GitSource),
    Https(HttpsSource),
}

impl Source {
    /// Reads what a person typed: `https://…/file.yaml` is a file, an address
    /// ending in `.git`, an `ssh://` address or `git@host:path` is a repository.
    pub fn parse(text: &str, git_ref: &str, path: &str) -> Result<Source, Refused> {
        let text = text.trim();
        if text.starts_with("http://") {
            return Err(Refused::PlainHttp);
        }
        if text.starts_with("https://") && !text.trim_end_matches('/').ends_with(".git") {
            return HttpsSource::new(text).map(Source::Https);
        }
        GitSource::new(text, git_ref, path).map(Source::Git)
    }

    /// The name shown for the source, without any query.
    pub fn display(&self) -> String {
        match self {
            Source::Git(git) => git.url.clone(),
            Source::Https(https) => https.url.origin().ascii_serialization() + https.url.path(),
        }
    }
}

/// A file in a git repository at a branch or tag.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GitSource {
    pub url: String,
    pub git_ref: String,
    /// The setup file's path inside the repository.
    pub path: String,
}

impl GitSource {
    pub fn new(url: &str, git_ref: &str, path: &str) -> Result<Self, Refused> {
        let url = url.trim();
        if url.is_empty() || url.starts_with('-') || url.chars().any(|c| c.is_control() || c == ' ')
        {
            return Err(Refused::NotAnAddress);
        }
        if url.contains("::") {
            return Err(Refused::Scheme); // `ext::` and friends run commands
        }
        if let Some((scheme, rest)) = url.split_once("://") {
            match scheme {
                "https" | "ssh" => {}
                "http" => return Err(Refused::PlainHttp),
                _ => return Err(Refused::Scheme),
            }
            let parsed = Url::parse(url).map_err(|_| Refused::NotAnAddress)?;
            if parsed.password().is_some() || (scheme == "https" && !parsed.username().is_empty()) {
                return Err(Refused::Credentials);
            }
            if parsed.host_str().is_none() || rest.is_empty() {
                return Err(Refused::NotAnAddress);
            }
        } else if !is_scp_like(url) {
            return Err(if url.starts_with('/') || url.starts_with('.') {
                Refused::LocalPath
            } else {
                Refused::NotAnAddress
            });
        }
        let git_ref = if git_ref.trim().is_empty() {
            "HEAD"
        } else {
            git_ref.trim()
        };
        if !ref_ok(git_ref) {
            return Err(Refused::BadRef);
        }
        let path = if path.trim().is_empty() {
            "freshkube.yaml"
        } else {
            path.trim()
        };
        if !relative_ok(path) {
            return Err(Refused::BadPath);
        }
        Ok(Self {
            url: url.to_string(),
            git_ref: git_ref.to_string(),
            path: path.to_string(),
        })
    }
}

/// `git@host:owner/name.git`: a user, a host, a colon, then a path.
fn is_scp_like(url: &str) -> bool {
    match url.split_once(':') {
        Some((head, tail)) => {
            let host = head.rsplit_once('@').map_or(head, |(_, host)| host);
            !tail.is_empty()
                && !tail.starts_with('/')
                && !host.is_empty()
                && !head.contains('/')
                && host
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '-')
        }
        None => false,
    }
}

fn ref_ok(name: &str) -> bool {
    !name.starts_with('-')
        && !name.contains("..")
        && !name.ends_with('/')
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '/' | '.' | '_' | '-'))
}

fn relative_ok(path: &str) -> bool {
    !path.starts_with('/')
        && !path.contains('\\')
        && !path.chars().any(char::is_control)
        && path
            .split('/')
            .all(|part| !part.is_empty() && part != "." && part != "..")
}

/// A file at an HTTPS address, with an optional token kept in the key store.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HttpsSource {
    pub url: Url,
}

impl HttpsSource {
    pub fn new(text: &str) -> Result<Self, Refused> {
        let url = Url::parse(text.trim()).map_err(|_| Refused::NotAnAddress)?;
        match url.scheme() {
            "https" => {}
            "http" => return Err(Refused::PlainHttp),
            _ => return Err(Refused::Scheme),
        }
        if url.host_str().is_none() {
            return Err(Refused::NotAnAddress);
        }
        if !url.username().is_empty() || url.password().is_some() {
            return Err(Refused::Credentials);
        }
        Ok(Self { url })
    }

    /// The key store account for this source's token.
    pub fn token_account(&self) -> String {
        format!("Team setup {}", self.url.origin().ascii_serialization())
    }

    /// Whether a redirect from this source may be followed: HTTPS and the same
    /// host and port.
    pub fn check_redirect(&self, to: &Url) -> Result<(), Refused> {
        check_redirect(&self.url, to)
    }
}

/// Follows a redirect only to HTTPS on the same host and port.
pub fn check_redirect(from: &Url, to: &Url) -> Result<(), Refused> {
    if to.scheme() != "https" {
        return Err(if to.scheme() == "http" {
            Refused::PlainHttp
        } else {
            Refused::Scheme
        });
    }
    if from.host_str() != to.host_str()
        || from.port_or_known_default() != to.port_or_known_default()
    {
        return Err(Refused::CrossHostRedirect);
    }
    Ok(())
}

/// The git command that fetches a team repository, as data: tests assert its
/// flags and a later slice runs it. It is the person's own `git`, so their
/// sign-in is used and none is stored here.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GitFetch {
    pub program: &'static str,
    pub args: Vec<String>,
    pub env: Vec<(&'static str, &'static str)>,
    /// The ref the fetch writes in the mirror.
    pub tracking_ref: &'static str,
}

/// Where the fetch writes: one fixed ref in the app-owned mirror.
pub const TRACKING_REF: &str = "refs/freshkube/team";

impl GitFetch {
    /// `mirror` is the app-owned bare repository; `empty_hooks` an empty
    /// directory, so no hook of the repository or the person's config runs.
    pub fn new(source: &GitSource, mirror: &Path, empty_hooks: &Path) -> Self {
        let hooks = format!("core.hooksPath={}", empty_hooks.display());
        let args = [
            "-C".to_string(),
            mirror.display().to_string(),
            "-c".into(),
            hooks,
            "-c".into(),
            "protocol.file.allow=never".into(),
            "-c".into(),
            "protocol.ext.allow=never".into(),
            "-c".into(),
            "submodule.recurse=false".into(),
            "fetch".into(),
            "--no-recurse-submodules".into(),
            "--no-tags".into(),
            "--force".into(),
            "--".into(),
            source.url.clone(),
            format!("+{}:{TRACKING_REF}", source.git_ref),
        ];
        Self {
            program: "git",
            args: args.into(),
            env: vec![("GIT_TERMINAL_PROMPT", "0")],
            tracking_ref: TRACKING_REF,
        }
    }

    /// The command that reads the setup file at the fetched commit.
    pub fn show(source: &GitSource, mirror: &Path, empty_hooks: &Path) -> Vec<String> {
        vec![
            "-C".into(),
            mirror.display().to_string(),
            "-c".into(),
            format!("core.hooksPath={}", empty_hooks.display()),
            "show".into(),
            format!("{TRACKING_REF}:{}", source.path),
        ]
    }

    pub fn mirror_dir(app_dir: &Path) -> PathBuf {
        app_dir.join("team-setup").join("mirror.git")
    }
}
