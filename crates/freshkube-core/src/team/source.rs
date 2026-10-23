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
    QueryOrFragment,
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
            Refused::QueryOrFragment => "an address with a query or a fragment is refused",
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
        if text.contains(['?', '#']) {
            return Err(Refused::QueryOrFragment);
        }
        if text.starts_with("https://") && !text.trim_end_matches('/').ends_with(".git") {
            return HttpsSource::new(text).map(Source::Https);
        }
        GitSource::new(text, git_ref, path).map(Source::Git)
    }

    /// The name shown for the source, without any query.
    pub fn display(&self) -> String {
        match self {
            Source::Git(git) => git.redacted(),
            Source::Https(https) => https.url.origin().ascii_serialization() + https.url.path(),
        }
    }
}

/// An address as logs, errors and the page may show it: no user name or
/// password, query or fragment.
pub fn redact(url: &str) -> String {
    if url.contains("://") {
        if let Ok(mut parsed) = Url::parse(url) {
            let _ = parsed.set_username("");
            let _ = parsed.set_password(None);
            parsed.set_query(None);
            parsed.set_fragment(None);
            return parsed.to_string();
        }
        return "<address>".into();
    }
    match url.rsplit_once('@') {
        Some((_, rest)) => rest.to_string(),
        None => url.to_string(),
    }
}

/// A file in a git repository at a branch or tag.
#[derive(Clone, PartialEq, Eq)]
pub struct GitSource {
    url: String,
    git_ref: String,
    /// The setup file's path inside the repository.
    path: String,
}

impl std::fmt::Debug for GitSource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GitSource")
            .field("url", &self.redacted())
            .field("git_ref", &self.git_ref)
            .field("path", &self.path)
            .finish()
    }
}

impl GitSource {
    pub fn url(&self) -> &str {
        &self.url
    }

    pub fn git_ref(&self) -> &str {
        &self.git_ref
    }

    pub fn path(&self) -> &str {
        &self.path
    }

    /// The address without anything that could be a credential.
    pub fn redacted(&self) -> String {
        redact(&self.url)
    }

    pub fn new(url: &str, git_ref: &str, path: &str) -> Result<Self, Refused> {
        let url = url.trim();
        if url.is_empty() || url.starts_with('-') || url.chars().any(|c| c.is_control() || c == ' ')
        {
            return Err(Refused::NotAnAddress);
        }
        if url.contains("::") {
            return Err(Refused::Scheme); // `ext::` and friends run commands
        }
        if url.contains(['?', '#']) {
            return Err(Refused::QueryOrFragment);
        }
        if let Some((scheme, rest)) = url.split_once("://") {
            match scheme {
                "https" | "ssh" => {}
                "http" => return Err(Refused::PlainHttp),
                _ => return Err(Refused::Scheme),
            }
            let parsed = Url::parse(url).map_err(|_| Refused::NotAnAddress)?;
            if parsed.password().is_some() || (scheme == "https" && !parsed.username().is_empty()) {
                // A user name alone is how ssh addresses name an account; any
                // password, or any userinfo on https, may be a credential.
                return Err(Refused::Credentials);
            }
            let dashed = |s: &str| s.starts_with('-');
            if parsed.host_str().is_none_or(dashed) || dashed(parsed.username()) || rest.is_empty()
            {
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
            let user = head.split_once('@').map_or("", |(user, _)| user);
            !tail.is_empty()
                && !host.starts_with('-')
                && !user.starts_with('-')
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
#[derive(Clone, PartialEq, Eq)]
pub struct HttpsSource {
    url: Url,
}

impl std::fmt::Debug for HttpsSource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HttpsSource")
            .field("url", &redact(self.url.as_str()))
            .finish()
    }
}

impl HttpsSource {
    pub fn url(&self) -> &Url {
        &self.url
    }

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
        if url.query().is_some() || url.fragment().is_some() {
            return Err(Refused::QueryOrFragment);
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
/// sign-in is used and none is stored here. A token never goes on a command
/// line, in an address or in a log.
#[derive(Clone, PartialEq, Eq)]
pub struct GitFetch {
    pub program: &'static str,
    pub args: Vec<String>,
    pub env: Vec<(&'static str, &'static str)>,
    /// The ref the fetch writes in the mirror.
    pub tracking_ref: &'static str,
    /// Where the address sits in `args`, so `Debug` can redact it.
    address_at: usize,
}

impl std::fmt::Debug for GitFetch {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let args: Vec<String> = self
            .args
            .iter()
            .enumerate()
            .map(|(i, arg)| {
                if i == self.address_at {
                    redact(arg)
                } else {
                    arg.clone()
                }
            })
            .collect();
        f.debug_struct("GitFetch")
            .field("program", &self.program)
            .field("args", &args)
            .field("env", &self.env)
            .finish()
    }
}

/// Where the fetch writes: one fixed ref in the app-owned mirror.
pub const TRACKING_REF: &str = "refs/freshkube/team";

impl GitFetch {
    /// `mirror` is the app-owned bare repository; `empty_hooks` an empty
    /// directory, so no hook of the repository or the person's config runs.
    /// `token_from_store` is true only when the token comes from our key store:
    /// then the person's credential helper is switched off for this fetch.
    /// Otherwise their helper is left alone.
    pub fn new(
        source: &GitSource,
        mirror: &Path,
        empty_hooks: &Path,
        token_from_store: bool,
    ) -> Self {
        let mut args: Vec<String> = vec!["-C".into(), mirror.display().to_string()];
        let mut config = |setting: String| {
            args.push("-c".into());
            args.push(setting);
        };
        config(format!("core.hooksPath={}", empty_hooks.display()));
        // Only https and ssh, whatever `url.insteadOf` rewrites an address to,
        // and no redirect is followed.
        config("protocol.allow=never".into());
        config("protocol.https.allow=always".into());
        config("protocol.ssh.allow=always".into());
        config("http.followRedirects=false".into());
        config("protocol.file.allow=never".into());
        config("protocol.ext.allow=never".into());
        config("submodule.recurse=false".into());
        if token_from_store {
            config("credential.helper=".into());
        }
        args.extend(
            [
                "fetch",
                "--no-recurse-submodules",
                "--no-tags",
                "--force",
                "--",
            ]
            .map(String::from),
        );
        let address_at = args.len();
        args.push(source.url().to_string());
        args.push(format!("+{}:{TRACKING_REF}", source.git_ref()));
        Self {
            program: "git",
            args,
            env: vec![("GIT_TERMINAL_PROMPT", "0")],
            tracking_ref: TRACKING_REF,
            address_at,
        }
    }

    /// The command that reads the setup file at the fetched commit: the raw
    /// blob, so no `textconv` or filter driver runs.
    pub fn show(source: &GitSource, mirror: &Path, empty_hooks: &Path) -> Vec<String> {
        vec![
            "-C".into(),
            mirror.display().to_string(),
            "-c".into(),
            format!("core.hooksPath={}", empty_hooks.display()),
            "cat-file".into(),
            "blob".into(),
            format!("{TRACKING_REF}:{}", source.path()),
        ]
    }

    pub fn mirror_dir(app_dir: &Path) -> PathBuf {
        app_dir.join("team-setup").join("mirror.git")
    }
}
