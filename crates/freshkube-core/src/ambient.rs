//! The process environment core depends on, read once by the binary.
//!
//! Core reads no environment variable. The binary builds an [`Ambient`] from
//! its own environment (`freshkube_desktop::ambient_from_env`) and
//! [`install`]s it before any thread starts. The readers below sit under many
//! callers (`kube_client::client`, the access revision, the audit log, every
//! `kubeconfig_sources`), so one set-once value stands in for a parameter on
//! each of them; each decision is a pure function of an `Ambient` that the
//! tests call with values.
//!
//! Nothing installed means nothing set: no kubeconfig list, home, proxy or
//! user, never a fall back to the real environment.

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

/// Installs the process's ambient values; the first call wins.
pub fn install(ambient: Ambient) {
    let _ = AMBIENT.set(ambient);
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
        assert_eq!(ambient.home_variable, Some("/h".into()));
        assert_eq!(ambient.home, Some("/home/me".into()));
    }
}
