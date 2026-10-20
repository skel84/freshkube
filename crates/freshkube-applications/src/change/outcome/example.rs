//! Example data's outcomes: Coroot's example keeps two revisions of acme's
//! `checkout-api`, so a Deployment hop whose hash names one finds it and any
//! other finds none. `FRESHKUBE_APPLICATIONS=outcome-<name>` makes every
//! Deployment hop show one outcome, so a capture can show each.
use std::rc::Rc;

use chrono::Utc;
use freshkube_core::coroot as api;
use freshkube_delivery::change::example::POD_HASH;
use freshkube_observability::{After, after_example};
use gpui_kit::*;

use super::{ChangePage, Memo, ask_of};

/// An outcome every Deployment hop shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Found,
    EndsEarly,
    NoRevision,
    NoApplication,
    NoProject,
    NoConnection,
    NotLinked,
    NoProjectForEntry,
    Refused,
    RefusedWindow,
    NoData,
    Failed,
    NotDeployment,
    NotLooked,
    /// Found, then a later read failed: the last answer, marked stale.
    Stale,
}

impl Kind {
    /// Each kind and its name in `FRESHKUBE_APPLICATIONS=outcome-<name>`.
    pub const ALL: [(Kind, &'static str); 15] = [
        (Kind::Found, "found"),
        (Kind::EndsEarly, "ends-early"),
        (Kind::NoRevision, "no-revision"),
        (Kind::NoApplication, "no-application"),
        (Kind::NoProject, "no-project"),
        (Kind::NoConnection, "no-connection"),
        (Kind::NotLinked, "not-linked"),
        (Kind::NoProjectForEntry, "no-project-for-entry"),
        (Kind::Refused, "refused"),
        (Kind::RefusedWindow, "refused-window"),
        (Kind::NoData, "no-data"),
        (Kind::Failed, "failed"),
        (Kind::NotDeployment, "not-deployment"),
        (Kind::NotLooked, "not-looked"),
        (Kind::Stale, "stale"),
    ];

    pub fn parse(name: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find_map(|(kind, known)| (known == name).then_some(kind))
    }

    pub fn name(self) -> &'static str {
        Self::ALL
            .into_iter()
            .find_map(|(kind, name)| (kind == self).then_some(name))
            .unwrap_or("found")
    }

    /// The outcome and, for a stale one, the failed read over it.
    fn outcome(self) -> (api::Outcome, Option<api::ReadError>) {
        let window = |hash| {
            after_example::around(hash).map_or(
                api::Outcome::Failed(api::ReadError::InvalidResponse),
                api::Outcome::of_window,
            )
        };
        match self {
            Self::Found => (window(after_example::OLDER_HASH), None),
            Self::EndsEarly => (window(POD_HASH), None),
            Self::Stale => (
                window(after_example::OLDER_HASH),
                Some(api::ReadError::Unreachable),
            ),
            Self::NoRevision => (api::Outcome::NoRevision, None),
            Self::NoApplication => (api::Outcome::NoApplication, None),
            Self::NoProject => (api::Outcome::NoProject, None),
            Self::NoConnection => (api::Outcome::NoConnection, None),
            Self::NotLinked => (api::Outcome::NotLinked, None),
            Self::NoProjectForEntry => (api::Outcome::NoProjectForEntry, None),
            Self::Refused => (api::Outcome::Refused(api::Refused::Application), None),
            Self::RefusedWindow => (api::Outcome::Refused(api::Refused::Window), None),
            Self::NoData => (api::Outcome::NoData, None),
            Self::Failed => (api::Outcome::Failed(api::ReadError::Unreachable), None),
            Self::NotDeployment => (api::Outcome::NotDeployment, None),
            Self::NotLooked => (api::Outcome::NotLooked, None),
        }
    }
}

impl ChangePage {
    /// Answers the selected hop from example data at once: the kind every
    /// hop shows, else what Coroot's example keeps for its hash.
    pub(super) fn read_example(
        &mut self,
        hop: &str,
        kind: Option<Kind>,
        force: bool,
        cx: &mut Context<Self>,
    ) {
        let Some(ask) = ask_of(&self.change, hop) else {
            return;
        };
        if !force && self.outcomes.memo.contains_key(hop) {
            return;
        }
        let (outcome, failed) = match kind {
            Some(kind) => kind.outcome(),
            None if ask.object.kind == "Rollout" => (api::Outcome::NotDeployment, None),
            None if ask.why.is_some() || ask.hash.is_none() => (api::Outcome::NotLooked, None),
            None => (
                ask.hash
                    .as_deref()
                    .and_then(after_example::around)
                    .map_or(api::Outcome::NoRevision, api::Outcome::of_window),
                None,
            ),
        };
        let after = match &outcome {
            api::Outcome::Found(view) => Some((view.as_ref(), None)),
            api::Outcome::EndsEarly(view, at) => Some((view.as_ref(), Some(*at))),
            _ => None,
        }
        .map(|(view, ends)| Rc::new(After::new(view, ends, Utc::now(), cx)));
        self.outcomes.memo.insert(
            hop.to_owned(),
            Memo {
                query: None,
                window: self.outcomes.window,
                outcome,
                after,
                read: Utc::now(),
                at: cx.background_executor().now(),
                stale: failed.map(|error| error.to_string()),
            },
        );
        self.detail = self.derive_detail();
        self.sync_marks();
        cx.notify();
    }
}
