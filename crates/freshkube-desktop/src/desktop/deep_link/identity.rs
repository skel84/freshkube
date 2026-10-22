//! What the window knows of each cluster's identity, and how a link's
//! fingerprint is matched to it (#873). Pure: the reads, the cache file and
//! the dialog live beside it.
use freshkube_core::fingerprint::{Fingerprint, Strength, matches};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// The most contexts the cache keeps.
pub(super) const CACHE_LIMIT: usize = 256;

/// Where a link can be opened: a workspace entry, or a kubeconfig context no
/// entry covers.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub(super) enum Target {
    Entry(String),
    Context(String),
}

/// What reading a context left: the fingerprint's inputs, kept while the
/// kubeconfig still names the same server and CA. Hashes only: never a
/// credential, a UID or an address.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub(super) struct Record {
    /// A hash of the server address the context named when it was read.
    pub server: String,
    /// The local CA value then, if the kubeconfig had one.
    #[serde(default)]
    pub ca: Option<String>,
    /// The `kube-system` value, when the read could see it.
    #[serde(default)]
    pub strong: Option<String>,
    /// The read was refused (401 or 403): the context is "CA only".
    #[serde(default)]
    pub refused: bool,
}

/// What the kubeconfig says of a context now, locally.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(super) struct Local {
    pub server: String,
    pub ca: Option<String>,
}

/// `fingerprints.json`: the records by context name.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub(super) struct Cache {
    #[serde(default)]
    pub contexts: BTreeMap<String, Record>,
}

impl Cache {
    /// The record of `context`, if it still describes it.
    pub(super) fn valid(&self, context: &str, local: Option<&Local>) -> Option<&Record> {
        let local = local?;
        self.contexts
            .get(context)
            .filter(|record| record.server == local.server && record.ca == local.ca)
    }

    /// Keeps `record` for `context`, forgetting the first names past the
    /// limit.
    pub(super) fn put(&mut self, context: &str, record: Record) {
        self.contexts.insert(context.to_owned(), record);
        while self.contexts.len() > CACHE_LIMIT {
            let Some(first) = self.contexts.keys().next().cloned() else {
                break;
            };
            self.contexts.remove(&first);
        }
    }
}

/// How far a candidate's strong value is known.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum Checked {
    /// A value: read, or from the entry's override.
    Yes,
    /// Never read: the context was never opened here, and nothing reads it
    /// without a Check.
    No,
    /// The read was refused: the CA is all there is.
    Refused,
    /// The last Check failed, and why.
    Failed(String),
}

/// A cluster a link may open in.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Candidate {
    pub target: Target,
    pub label: String,
    pub context: String,
    pub identity: Fingerprint,
    pub checked: Checked,
    /// This is the cluster the window is on.
    pub open: bool,
}

/// How a candidate stands against a link.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Standing {
    Exact,
    CaOnly,
    /// Both sides have a strong value and they differ.
    Different,
    /// Nothing known ties it to the link yet.
    Unknown,
}

pub(super) fn standing(link: &Fingerprint, candidate: &Candidate) -> Standing {
    match matches(link, &candidate.identity) {
        Some(Strength::Exact) => Standing::Exact,
        Some(Strength::CaOnly) => Standing::CaOnly,
        None if link.strong.is_some() && candidate.identity.strong.is_some() => Standing::Different,
        None => Standing::Unknown,
    }
}

/// What opening a link does.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum Decision {
    /// The cluster that is open is the one: nothing to switch.
    Here,
    /// Exactly one cluster matches: switch to it, if it isn't open.
    Open(Target),
    /// Ask which: several match and none is open, or a match is on the CA
    /// alone, which never opens without asking.
    Ask,
    /// Nothing matches: say so and offer the contexts.
    NoMatch,
}

/// The rule of #873: exact matches only, one → open there; several and one
/// is open → that one, without asking; several, none open → ask; any
/// CA-only candidate → always ask, even for one match or the open cluster;
/// none → no matching cluster.
pub(super) fn decide(link: &Fingerprint, candidates: &[Candidate]) -> Decision {
    let of = |wanted: Standing| {
        candidates
            .iter()
            .filter(move |candidate| standing(link, candidate) == wanted)
    };
    if of(Standing::CaOnly).next().is_some() {
        return Decision::Ask;
    }
    let exact: Vec<&Candidate> = of(Standing::Exact).collect();
    match exact.as_slice() {
        [] => Decision::NoMatch,
        [only] if only.open => Decision::Here,
        [only] => Decision::Open(only.target.clone()),
        several if several.iter().any(|candidate| candidate.open) => Decision::Here,
        _ => Decision::Ask,
    }
}

/// The candidates the picker lists, best first: exact, then CA only, then
/// the ones not checked yet (which a Check may turn into either); a cluster
/// known to differ is left out. With `everything`, as for no match, every
/// context is offered, in their own order.
pub(super) fn listed<'a>(
    link: &Fingerprint,
    candidates: &'a [Candidate],
    everything: bool,
) -> Vec<(&'a Candidate, Standing)> {
    let mut rows: Vec<_> = candidates
        .iter()
        .map(|candidate| (candidate, standing(link, candidate)))
        .filter(|(candidate, standing)| {
            everything
                || matches!(standing, Standing::Exact | Standing::CaOnly)
                || (*standing == Standing::Unknown && candidate.checked == Checked::No)
        })
        .collect();
    if !everything {
        rows.sort_by_key(|(_, standing)| match standing {
            Standing::Exact => 0,
            Standing::CaOnly => 1,
            _ => 2,
        });
    }
    rows
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fp(strong: Option<&str>, ca: Option<&str>) -> Fingerprint {
        Fingerprint {
            strong: strong.map(str::to_owned),
            ca: ca.map(str::to_owned),
        }
    }

    fn candidate(name: &str, strong: Option<&str>, ca: Option<&str>, open: bool) -> Candidate {
        Candidate {
            target: Target::Entry(name.into()),
            label: name.into(),
            context: name.into(),
            identity: fp(strong, ca),
            checked: if strong.is_some() {
                Checked::Yes
            } else {
                Checked::No
            },
            open,
        }
    }

    fn entry(name: &str) -> Target {
        Target::Entry(name.into())
    }

    #[test]
    fn one_exact_match_opens_there_or_stays_when_it_is_open() {
        let link = fp(Some("a"), Some("x"));
        let cands = [
            candidate("one", Some("a"), Some("x"), false),
            candidate("two", Some("b"), Some("x"), true),
        ];
        assert_eq!(decide(&link, &cands), Decision::Open(entry("one")));
        let cands = [
            candidate("one", Some("a"), Some("x"), true),
            candidate("two", Some("b"), Some("x"), false),
        ];
        assert_eq!(decide(&link, &cands), Decision::Here);
    }

    #[test]
    fn several_exact_matches_keep_the_open_one_without_asking_else_ask() {
        let link = fp(Some("a"), None);
        let cands = [
            candidate("one", Some("a"), None, false),
            candidate("two", Some("a"), None, true),
        ];
        assert_eq!(decide(&link, &cands), Decision::Here);
        let cands = [
            candidate("one", Some("a"), None, false),
            candidate("two", Some("a"), None, false),
            candidate("three", Some("a"), None, false),
        ];
        assert_eq!(decide(&link, &cands), Decision::Ask);
    }

    #[test]
    fn any_ca_only_candidate_always_asks_even_for_one_or_the_open_cluster() {
        let link = fp(Some("a"), Some("x"));
        let one = [candidate("one", None, Some("x"), false)];
        assert_eq!(decide(&link, &one), Decision::Ask);
        let open = [candidate("one", None, Some("x"), true)];
        assert_eq!(decide(&link, &open), Decision::Ask);
        // An exact match beside it doesn't open without asking either.
        let both = [
            candidate("exact", Some("a"), Some("x"), true),
            candidate("ca", None, Some("x"), false),
        ];
        assert_eq!(decide(&link, &both), Decision::Ask);
        // A link with a CA alone matches on the CA alone, always asking.
        let link = fp(None, Some("x"));
        let strong = [candidate("one", Some("a"), Some("x"), true)];
        assert_eq!(decide(&link, &strong), Decision::Ask);
    }

    #[test]
    fn a_shared_ca_does_not_make_a_checked_cluster_match() {
        let link = fp(Some("a"), Some("proxy"));
        let cands = [
            candidate("one", Some("b"), Some("proxy"), true),
            candidate("two", Some("c"), Some("proxy"), false),
        ];
        assert_eq!(decide(&link, &cands), Decision::NoMatch);
        assert_eq!(standing(&link, &cands[0]), Standing::Different);
    }

    #[test]
    fn no_candidate_is_no_match() {
        let link = fp(Some("a"), Some("x"));
        assert_eq!(decide(&link, &[]), Decision::NoMatch);
        let cands = [candidate("one", None, Some("y"), false)];
        assert_eq!(decide(&link, &cands), Decision::NoMatch);
        assert_eq!(standing(&link, &cands[0]), Standing::Unknown);
    }

    #[test]
    fn the_picker_lists_exact_then_ca_only_then_unchecked_and_never_a_known_different() {
        let link = fp(Some("a"), Some("x"));
        let cands = [
            candidate("unchecked", None, Some("y"), false),
            candidate("different", Some("b"), Some("x"), false),
            candidate("ca", None, Some("x"), false),
            candidate("exact", Some("a"), Some("x"), false),
        ];
        let rows: Vec<_> = listed(&link, &cands, false)
            .into_iter()
            .map(|(candidate, standing)| (candidate.label.as_str(), standing))
            .collect();
        assert_eq!(
            rows,
            [
                ("exact", Standing::Exact),
                ("ca", Standing::CaOnly),
                ("unchecked", Standing::Unknown),
            ]
        );
        // For no match, every context is offered in its own order.
        assert_eq!(listed(&link, &cands, true).len(), 4);
    }

    #[test]
    fn a_record_describes_a_context_only_while_its_server_and_ca_are_unchanged() {
        let mut cache = Cache::default();
        let record = Record {
            server: "s".into(),
            ca: Some("c".into()),
            strong: Some("u".into()),
            refused: false,
        };
        cache.put("ctx", record.clone());
        let local = |server: &str, ca: Option<&str>| Local {
            server: server.into(),
            ca: ca.map(str::to_owned),
        };
        assert_eq!(
            cache.valid("ctx", Some(&local("s", Some("c")))),
            Some(&record)
        );
        assert_eq!(cache.valid("ctx", Some(&local("other", Some("c")))), None);
        assert_eq!(cache.valid("ctx", Some(&local("s", Some("other")))), None);
        assert_eq!(cache.valid("ctx", Some(&local("s", None))), None);
        assert_eq!(cache.valid("ctx", None), None);
        assert_eq!(cache.valid("another", Some(&local("s", Some("c")))), None);
    }

    #[test]
    fn the_cache_keeps_at_most_its_limit() {
        let mut cache = Cache::default();
        for ix in 0..CACHE_LIMIT + 5 {
            cache.put(&format!("ctx-{ix:04}"), Record::default());
        }
        assert_eq!(cache.contexts.len(), CACHE_LIMIT);
    }

    #[test]
    fn the_cache_file_holds_hashes_and_unknown_keys_do_not_break_it() {
        let text = r#"{"contexts":{"ctx":{"server":"s","ca":"c","strong":"u","refused":false,"later":1}},"later":2}"#;
        let cache: Cache = serde_json::from_str(text).unwrap();
        assert_eq!(cache.contexts["ctx"].strong.as_deref(), Some("u"));
        let written = serde_json::to_string(&cache).unwrap();
        assert!(!written.contains("later"));
    }
}
