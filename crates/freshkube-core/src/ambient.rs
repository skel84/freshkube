//! The process environment core depends on, read once by the binary.
//!
//! Core reads no environment variable itself. The process builds an
//! [`Ambient`] from its own environment
//! (`freshkube_desktop::install_ambient_from_env`) and [`install`]s it before
//! its first client. The readers below sit under many callers
//! (`kube_client::client`, the access revision, the audit log, every
//! `kubeconfig_sources`), so one set-once value stands in for a parameter on
//! each of them; each decision is a pure function of an `Ambient` that the
//! tests call with values.
//!
//! **Invariant:** `Ambient` must mirror the environment kube itself reads in
//! the same process. kube reads `HTTPS_PROXY`/`https_proxy` when it builds a
//! `Config` and `KUBECONFIG` (then `~/.kube/config`) through
//! `Kubeconfig::read`; core can only tell that a proxy came from the
//! environment, and which files the Automatic selection watches, by comparing
//! with this value. So every process that calls core's kube functions (the
//! binaries, examples and test harnesses) installs it, from the real
//! environment or, in tests, a hermetic value that keeps the proxy pair, before
//! the first client. A process that installs nothing sees an empty `Ambient`:
//! kube still proxies, but core's loopback and `NO_PROXY` exemption and the
//! kubeconfig file list are off.
//!
//! A second install with a different value fails loudly; an identical one is
//! a no-op.

use std::ffi::OsString;
use std::path::PathBuf;
use std::sync::OnceLock;

/// The environment values core reads, as plain data.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Ambient {
    /// `FRESHKUBE_KUBECONFIG`: a path list.
    pub freshkube_kubeconfig: Option<OsString>,
    /// `KUBECONFIG`: a path list.
    pub kubeconfig: Option<OsString>,
    /// The home directory (`HOME`, or the platform's home).
    pub home: Option<PathBuf>,
    /// `HTTPS_PROXY`, else `https_proxy`, when not empty.
    pub https_proxy: Option<String>,
    /// `NO_PROXY`, else `no_proxy`, when not empty.
    pub no_proxy: Option<String>,
    /// `USER`, else `USERNAME`.
    pub user: Option<String>,
    /// `HOME` as the audit file reads it (not the platform's home).
    pub home_variable: Option<OsString>,
}

impl Ambient {
    /// Builds the value from a lookup of variables by name; `home` is the
    /// platform's home directory. The binary passes `std::env::var_os`.
    pub fn from_lookup(lookup: impl Fn(&str) -> Option<OsString>, home: Option<PathBuf>) -> Self {
        let first_set = |names: &[&str]| {
            names
                .iter()
                .filter_map(|name| lookup(name)?.into_string().ok())
                .find(|value| !value.is_empty())
        };
        Self {
            freshkube_kubeconfig: lookup("FRESHKUBE_KUBECONFIG"),
            kubeconfig: lookup("KUBECONFIG"),
            home,
            https_proxy: first_set(&["HTTPS_PROXY", "https_proxy"]),
            no_proxy: first_set(&["NO_PROXY", "no_proxy"]),
            user: ["USER", "USERNAME"]
                .iter()
                .find_map(|name| lookup(name)?.into_string().ok()),
            home_variable: lookup("HOME"),
        }
    }

    /// Whether `KUBECONFIG` is set and not empty.
    pub fn has_kubeconfig(&self) -> bool {
        self.kubeconfig
            .as_ref()
            .is_some_and(|value| !value.is_empty())
    }
}

static AMBIENT: OnceLock<Ambient> = OnceLock::new();

/// Installs the process's ambient values. A repeat with an equal value is a
/// no-op; a different one panics, since a silently dropped value would change
/// which kubeconfig or proxy is used.
pub fn install(ambient: Ambient) {
    set_once(&AMBIENT, ambient).expect("a different Ambient is already installed");
}

fn set_once(cell: &OnceLock<Ambient>, ambient: Ambient) -> Result<(), ()> {
    match cell.set(ambient) {
        Ok(()) => Ok(()),
        Err(rejected) if cell.get() == Some(&rejected) => Ok(()),
        Err(_) => Err(()),
    }
}

/// The installed values, or an empty set when none were installed.
pub fn get() -> &'static Ambient {
    static EMPTY: Ambient = Ambient {
        freshkube_kubeconfig: None,
        kubeconfig: None,
        home: None,
        https_proxy: None,
        no_proxy: None,
        user: None,
        home_variable: None,
    };
    AMBIENT.get().unwrap_or(&EMPTY)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lookup(pairs: &'static [(&'static str, &'static str)]) -> impl Fn(&str) -> Option<OsString> {
        move |name| {
            pairs
                .iter()
                .find(|(key, _)| *key == name)
                .map(|(_, value)| OsString::from(value))
        }
    }

    #[test]
    fn a_repeat_install_must_be_equal() {
        let cell = OnceLock::new();
        let one = Ambient {
            user: Some("ana".into()),
            ..Ambient::default()
        };
        assert!(set_once(&cell, one.clone()).is_ok());
        assert!(set_once(&cell, one).is_ok());
        assert!(set_once(&cell, Ambient::default()).is_err());
    }

    #[test]
    fn nothing_set_is_empty() {
        assert_eq!(Ambient::from_lookup(lookup(&[]), None), Ambient::default());
    }

    #[test]
    fn the_proxy_pair_reads_upper_case_first_and_skips_empty() {
        let ambient = Ambient::from_lookup(
            lookup(&[
                ("HTTPS_PROXY", ""),
                ("https_proxy", "http://p:1"),
                ("NO_PROXY", "a"),
                ("no_proxy", "b"),
            ]),
            None,
        );
        assert_eq!(ambient.https_proxy.as_deref(), Some("http://p:1"));
        assert_eq!(ambient.no_proxy.as_deref(), Some("a"));
    }

    #[test]
    fn user_prefers_user_over_username() {
        let both = Ambient::from_lookup(lookup(&[("USER", "ana"), ("USERNAME", "bo")]), None);
        assert_eq!(both.user.as_deref(), Some("ana"));
        let empty = Ambient::from_lookup(lookup(&[("USER", ""), ("USERNAME", "bo")]), None);
        assert_eq!(empty.user.as_deref(), Some(""));
        let only = Ambient::from_lookup(lookup(&[("USERNAME", "bo")]), None);
        assert_eq!(only.user.as_deref(), Some("bo"));
    }

    #[test]
    fn kubeconfig_variables_and_home_are_kept_as_given() {
        let ambient = Ambient::from_lookup(
            lookup(&[
                ("FRESHKUBE_KUBECONFIG", "/a"),
                ("KUBECONFIG", ""),
                ("HOME", "/h"),
            ]),
            Some("/home/me".into()),
        );
        assert_eq!(ambient.freshkube_kubeconfig, Some("/a".into()));
        assert!(!ambient.has_kubeconfig());
        assert!(Ambient::from_lookup(lookup(&[("KUBECONFIG", "/k")]), None).has_kubeconfig());
        let lower = Ambient::from_lookup(lookup(&[("NO_PROXY", ""), ("no_proxy", "b")]), None);
        assert_eq!(lower.no_proxy.as_deref(), Some("b"));
        assert_eq!(ambient.home_variable, Some("/h".into()));
        assert_eq!(ambient.home, Some("/home/me".into()));
    }
}
