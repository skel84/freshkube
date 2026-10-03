//! Kubeconfig files and the contexts in them, for connecting without Talos.
//!
//! Files are read with the same bounded reader as the Settings file choice,
//! and several files merge the way kubectl merges `KUBECONFIG`: the first
//! file to define a name wins, and the first `current-context` wins. Nothing
//! here loads credentials or runs an auth plugin.

use std::ffi::OsString;
use std::path::{Path, PathBuf};

use kube::config::Kubeconfig;

use crate::kubeconfig_selection::read_selected_file;

/// One context as kubectl would see it after merging every source.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct KubeContext {
    pub name: String,
    pub cluster: String,
    pub namespace: Option<String>,
    /// The file that defines this context.
    pub source: PathBuf,
}

/// Contexts found in the sources, plus any file that couldn't be read. A
/// broken file never hides the contexts of the others.
#[derive(Clone, Debug, Default)]
pub struct KubeconfigReport {
    /// Revision inspected on the worker, before an access session is created.
    pub revision: crate::ConfigurationRevision,
    /// The files read, in merge order.
    pub sources: Vec<PathBuf>,
    /// Every context, in merge order, each name once.
    pub contexts: Vec<KubeContext>,
    /// The merged `current-context`, which need not name a listed context.
    pub current: Option<String>,
    pub errors: Vec<(PathBuf, String)>,
}

impl KubeconfigReport {
    /// The current context, when it names one that exists.
    pub fn current_context(&self) -> Option<&KubeContext> {
        let current = self.current.as_deref()?;
        self.contexts.iter().find(|context| context.name == current)
    }

    pub fn context(&self, name: &str) -> Option<&KubeContext> {
        self.contexts.iter().find(|context| context.name == name)
    }
}

/// The kubeconfig files to read: `explicit` when given, else
/// `FRESHKUBE_KUBECONFIG`, else `KUBECONFIG`, else `~/.kube/config`. Both
/// variables take a path list, as kubectl does. Missing files are skipped,
/// except an explicit one, which is reported when read.
pub fn kubeconfig_sources(explicit: Option<&Path>) -> Vec<PathBuf> {
    if let Some(path) = explicit {
        return vec![expand_home(path, dirs_next::home_dir().as_deref())];
    }
    sources_from(
        std::env::var_os("FRESHKUBE_KUBECONFIG"),
        std::env::var_os("KUBECONFIG"),
        dirs_next::home_dir().as_deref(),
    )
    .into_iter()
    .filter(|path| path.is_file())
    .collect()
}

fn sources_from(
    freshkube: Option<OsString>,
    kubeconfig: Option<OsString>,
    home: Option<&Path>,
) -> Vec<PathBuf> {
    let listed = |value: Option<OsString>| {
        value
            .filter(|value| !value.is_empty())
            .map(|value| std::env::split_paths(&value).collect::<Vec<_>>())
    };
    let paths = listed(freshkube)
        .or_else(|| listed(kubeconfig))
        .unwrap_or_else(|| {
            home.map(|home| vec![home.join(".kube").join("config")])
                .unwrap_or_default()
        });
    let mut unique = Vec::new();
    for path in paths {
        if path.as_os_str().is_empty() {
            continue;
        }
        let path = expand_home(&path, home);
        if !unique.contains(&path) {
            unique.push(path);
        }
    }
    unique
}

fn expand_home(path: &Path, home: Option<&Path>) -> PathBuf {
    match (path.strip_prefix("~"), home) {
        (Ok(rest), Some(home)) => home.join(rest),
        _ => path.to_path_buf(),
    }
}

/// Reads every source and lists its contexts. This is blocking file I/O; run
/// it on a worker.
pub fn discover_contexts(sources: &[PathBuf]) -> KubeconfigReport {
    load(sources).0
}

/// Reads each source once: the report, and every readable source merged into
/// one kubeconfig as kubectl merges them (`None` when none could be read).
pub(crate) fn load(sources: &[PathBuf]) -> (KubeconfigReport, Option<Kubeconfig>) {
    let mut report = KubeconfigReport {
        sources: sources.to_vec(),
        revision: crate::ConfigurationRevision::from_kubeconfig_sources(sources),
        ..KubeconfigReport::default()
    };
    let mut merged: Option<Kubeconfig> = None;
    for source in sources {
        let config = match read_selected_file(source) {
            Ok(config) => config,
            Err(error) => {
                report.errors.push((source.clone(), error.to_string()));
                continue;
            }
        };
        if report.current.is_none() {
            report.current = config
                .current_context
                .clone()
                .filter(|name| !name.is_empty());
        }
        for named in &config.contexts {
            let Some(context) = &named.context else {
                continue;
            };
            if report.context(&named.name).is_some() {
                continue;
            }
            report.contexts.push(KubeContext {
                name: named.name.clone(),
                cluster: context.cluster.clone(),
                namespace: context.namespace.clone(),
                source: source.clone(),
            });
        }
        merged = match merged {
            None => Some(config),
            Some(previous) => match previous.clone().merge(config) {
                Ok(next) => Some(next),
                Err(_) => {
                    report.errors.push((
                        source.clone(),
                        "can't be merged with the files before it".into(),
                    ));
                    Some(previous)
                }
            },
        };
    }
    (report, merged)
}

#[cfg(test)]
mod tests {
    use super::*;

    const LAB: &str = "apiVersion: v1\nkind: Config\ncurrent-context: b\nclusters:\n- name: c\n  cluster:\n    server: https://127.0.0.1:1\ncontexts:\n- name: a\n  context:\n    cluster: c\n    namespace: team\n- name: b\n  context:\n    cluster: c\nusers: []\n";
    const OTHER: &str = "apiVersion: v1\nkind: Config\ncurrent-context: z\nclusters: []\ncontexts:\n- name: b\n  context:\n    cluster: elsewhere\n- name: z\n  context:\n    cluster: d\nusers: []\n";

    struct Scratch(PathBuf);

    impl Scratch {
        fn new(name: &str) -> Self {
            let directory = std::env::temp_dir()
                .join(format!("freshkube-contexts-{name}-{}", std::process::id()));
            std::fs::create_dir_all(&directory).unwrap();
            Self(directory)
        }

        fn file(&self, name: &str, text: &str) -> PathBuf {
            let path = self.0.join(name);
            std::fs::write(&path, text).unwrap();
            path
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn contexts_merge_first_wins_and_bad_files_are_reported() {
        let scratch = Scratch::new("merge");
        let lab = scratch.file("lab.yaml", LAB);
        let other = scratch.file("other.yaml", OTHER);
        let bad = scratch.file("bad.yaml", ": not yaml [");
        let report = discover_contexts(&[lab.clone(), bad.clone(), other.clone()]);
        let names: Vec<_> = report.contexts.iter().map(|c| c.name.as_str()).collect();
        assert_eq!(names, ["a", "b", "z"]);
        // The first file to define `b` wins.
        assert_eq!(report.context("b").unwrap().cluster, "c");
        assert_eq!(report.context("b").unwrap().source, lab);
        assert_eq!(report.context("z").unwrap().source, other);
        assert_eq!(
            report.context("a").unwrap().namespace.as_deref(),
            Some("team")
        );
        // The first current-context wins.
        assert_eq!(report.current.as_deref(), Some("b"));
        assert_eq!(report.current_context().unwrap().name, "b");
        assert_eq!(report.errors.len(), 1);
        assert_eq!(report.errors[0].0, bad);
        // The reader's message never echoes file contents.
        assert!(!report.errors[0].1.contains("not yaml"));
    }

    #[test]
    fn a_current_context_that_does_not_exist_is_kept_but_not_resolved() {
        let scratch = Scratch::new("dangling");
        let path = scratch.file(
            "config",
            &LAB.replace("current-context: b", "current-context: gone"),
        );
        let report = discover_contexts(&[path]);
        assert_eq!(report.current.as_deref(), Some("gone"));
        assert!(report.current_context().is_none());
    }

    #[test]
    fn merged_kubeconfig_keeps_first_definitions() {
        let scratch = Scratch::new("merged");
        let lab = scratch.file("lab.yaml", LAB);
        let other = scratch.file("other.yaml", OTHER);
        let merged = load(&[lab, other]).1.unwrap();
        assert_eq!(merged.current_context.as_deref(), Some("b"));
        assert_eq!(merged.contexts.len(), 3);
        assert!(load(&[]).1.is_none());
    }

    #[test]
    fn sources_follow_kubectl_precedence() {
        let home = Path::new("/home/me");
        let joined = std::env::join_paths(["/a/one", "~/two", "", "/a/one"]).unwrap();
        assert_eq!(
            sources_from(Some(joined.clone()), Some("/ignored".into()), Some(home)),
            [PathBuf::from("/a/one"), PathBuf::from("/home/me/two")]
        );
        assert_eq!(
            sources_from(Some("".into()), Some("/b".into()), Some(home)),
            [PathBuf::from("/b")]
        );
        assert_eq!(
            sources_from(None, None, Some(home)),
            [PathBuf::from("/home/me/.kube/config")]
        );
        assert!(sources_from(None, None, None).is_empty());
        assert_eq!(
            kubeconfig_sources(Some(Path::new("/explicit/missing"))),
            [PathBuf::from("/explicit/missing")]
        );
    }
}
