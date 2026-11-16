//! "Map your contexts" (#849): the team file never names a context, so
//! joining asks, for each cluster the team names, which of the contexts in
//! your kubeconfig it is, or to leave it out. A context named like the
//! cluster is suggested and never chosen for you.

use std::collections::BTreeMap;

use super::file::Version;

/// What a person chose for one team cluster.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum Choice {
    /// One of the contexts of your kubeconfig.
    Context(String),
    /// Use no context for it: the team's cluster stays unopened.
    LeftOut,
}

/// A context that is not one of the person's own.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UnknownContext;

impl std::fmt::Display for UnknownContext {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("That context is not in your kubeconfig.")
    }
}

/// The team's clusters, each with the person's choice or none yet.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ContextMap {
    clusters: BTreeMap<String, Option<Choice>>,
}

/// The ids of the clusters a version names.
pub fn cluster_ids(version: &Version) -> impl Iterator<Item = &str> {
    version.values().keys().filter_map(|key| {
        let rest = key.strip_prefix("clusters[")?;
        rest.strip_suffix(']').filter(|id| !id.contains(']'))
    })
}

impl ContextMap {
    /// Every cluster `version` names, none chosen. A choice already made for
    /// a cluster that is still named is kept.
    pub fn for_version(version: &Version, before: &ContextMap) -> Self {
        Self::for_ids(cluster_ids(version), before)
    }

    /// Each of `ids`, with the choice `before` held for it, if any.
    pub fn for_ids<'a>(ids: impl IntoIterator<Item = &'a str>, before: &ContextMap) -> Self {
        Self {
            clusters: ids
                .into_iter()
                .map(|id| {
                    let kept = before.clusters.get(id).cloned().flatten();
                    (id.to_string(), kept)
                })
                .collect(),
        }
    }

    /// What was saved: each of `ids`, with the choice kept for it. A saved
    /// choice for any other cluster is dropped. A context is not checked
    /// against a kubeconfig here: the file may name one that has since gone,
    /// and a mapping only ever tightens a level.
    pub fn restore<'a>(
        ids: impl IntoIterator<Item = &'a str>,
        chosen: &BTreeMap<String, Choice>,
    ) -> Self {
        Self {
            clusters: ids
                .into_iter()
                .map(|id| (id.to_string(), chosen.get(id).cloned()))
                .collect(),
        }
    }

    /// The choices made, for saving.
    pub fn chosen(&self) -> BTreeMap<String, Choice> {
        self.clusters
            .iter()
            .filter_map(|(id, choice)| Some((id.clone(), choice.clone()?)))
            .collect()
    }

    /// Each cluster with its choice, by id.
    pub fn clusters(&self) -> impl Iterator<Item = (&str, Option<&Choice>)> {
        self.clusters
            .iter()
            .map(|(id, choice)| (id.as_str(), choice.as_ref()))
    }

    pub fn choice(&self, id: &str) -> Option<&Choice> {
        self.clusters.get(id)?.as_ref()
    }

    /// Chooses for one cluster. A context must be one of `available`; a
    /// cluster the team doesn't name is ignored.
    pub fn choose(
        &mut self,
        id: &str,
        choice: Choice,
        available: &[String],
    ) -> Result<(), UnknownContext> {
        if let Choice::Context(name) = &choice
            && !available.contains(name)
        {
            return Err(UnknownContext);
        }
        if let Some(slot) = self.clusters.get_mut(id) {
            *slot = Some(choice);
        }
        Ok(())
    }

    /// Whether every cluster has a choice, even if it is to leave it out.
    pub fn is_complete(&self) -> bool {
        self.clusters.values().all(Option::is_some)
    }

    /// How many clusters still have no choice.
    pub fn unchosen(&self) -> usize {
        self.clusters.values().filter(|c| c.is_none()).count()
    }

    /// The cluster ids and contexts chosen, for the person's own file.
    pub fn contexts(&self) -> BTreeMap<&str, &str> {
        self.clusters
            .iter()
            .filter_map(|(id, choice)| match choice {
                Some(Choice::Context(name)) => Some((id.as_str(), name.as_str())),
                _ => None,
            })
            .collect()
    }

    /// The context named like the cluster, offered and never taken.
    pub fn suggested<'a>(id: &str, available: &'a [String]) -> Option<&'a str> {
        available
            .iter()
            .map(String::as_str)
            .find(|name| *name == id)
    }
}
