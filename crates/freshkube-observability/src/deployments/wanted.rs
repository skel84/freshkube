//! A revision asked for from outside the page: the change page's Compare
//! names a Deployment and its current ReplicaSet's pod-template hash, and
//! Deployments finds Coroot's revision by that hash, as Coroot names its
//! revisions. The page reads nothing more for it; what it says comes from
//! what it reads anyway.
use super::*;

/// A Deployment's revision, as the shell hands it to
/// [`ObservabilityPage::open_revision`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RevisionLink {
    /// The shell's connection id of the cluster that runs it; Coroot's
    /// cluster is the one associated with it.
    pub access: String,
    /// That cluster, as the change page names it.
    pub cluster: String,
    pub namespace: String,
    pub name: String,
    /// Its current ReplicaSet's `pod-template-hash`.
    pub hash: String,
}

/// What came of looking for the revision.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Outcome {
    /// Not answered yet: Coroot's connection, its applications or the
    /// application's page are still to come.
    Seeking,
    Found,
    /// The application's revisions don't hold the hash.
    NoRevision,
    /// Coroot lists no such application in what it was asked for.
    NotListed,
    /// No Coroot cluster is associated with the link's cluster.
    NoAssociation,
    /// The application's revisions couldn't be read; the list says why.
    Unread,
}

pub(super) struct Wanted {
    pub(super) link: RevisionLink,
    /// Coroot's application, once its cluster is known.
    pub(super) app: Option<api::AppId>,
    pub(super) outcome: Outcome,
    /// The application shown when the outcome was decided; the banner
    /// shows over its list only, so another application opened from the
    /// map or a report doesn't carry it.
    pub(super) shown: Option<api::AppId>,
}

impl Wanted {
    fn decide(&mut self, outcome: Outcome, shown: Option<&api::AppId>) {
        self.outcome = outcome;
        self.shown = shown.cloned();
    }
}

impl Revisions {
    /// Selects the wanted revision of `app` among `values`, when it is the
    /// one wanted, before the list's own choice of the newest stands.
    pub(super) fn find_wanted(
        &mut self,
        app: Option<&api::AppId>,
        revisions: Option<&Result<Vec<api::DeploymentRevision>, api::ReadError>>,
    ) {
        let Some(wanted) = self
            .wanted
            .as_mut()
            .filter(|wanted| wanted.outcome == Outcome::Seeking && wanted.app.as_ref() == app)
        else {
            return;
        };
        let outcome = match revisions {
            None => return,
            Some(Err(_)) => Outcome::Unread,
            Some(Ok(values)) => match values.iter().find(|r| r.hash == wanted.link.hash) {
                Some(revision) => {
                    self.selected = Some(key(revision));
                    Outcome::Found
                }
                None => Outcome::NoRevision,
            },
        };
        wanted.decide(outcome, app);
    }

    /// The window or Coroot's cluster link changed: what wasn't found is
    /// looked for again, from Coroot's cluster on.
    pub(crate) fn seek_again(&mut self) {
        if let Some(wanted) = &mut self.wanted
            && wanted.outcome != Outcome::Found
        {
            wanted.app = None;
            wanted.decide(Outcome::Seeking, None);
        }
    }

    #[cfg(test)]
    pub(crate) fn wanted_outcome(&self) -> Option<Outcome> {
        self.wanted.as_ref().map(|wanted| wanted.outcome)
    }
}

impl ObservabilityPage {
    /// Shows Deployments on `link`'s revision, as Coroot keeps it. Shown
    /// or not, the page reads only as it would for Deployments; hidden,
    /// it looks once it shows.
    pub fn open_revision(&mut self, link: RevisionLink, cx: &mut Context<Self>) {
        self.revision_observations.wanted = Some(Wanted {
            link,
            app: None,
            outcome: Outcome::Seeking,
            shown: None,
        });
        self.seek_revision();
        self.open(Destination::Deployments, cx);
    }

    /// Resolves the wanted revision's application once Coroot's cluster
    /// for it is known, and chooses it. Called before every read.
    pub(crate) fn seek_revision(&mut self) {
        let Some(wanted) = &self.revision_observations.wanted else {
            return;
        };
        // Another connection since: the link, and whatever came of it,
        // was of the one before.
        if self.live.access.as_deref() != Some(wanted.link.access.as_str()) {
            self.revision_observations.wanted = None;
            return;
        }
        if wanted.outcome != Outcome::Seeking {
            return;
        }
        if let Some(app) = &wanted.app {
            // Another application opened since, from the map or a report:
            // the revision asked for no longer shows.
            if self.selected_app.as_ref() != Some(app) {
                self.revision_observations.wanted = None;
            }
            return;
        }
        // Hidden, what Coroot answered may be old; the page looks once it
        // shows and reads again.
        if !self.fixture && !self.live.visible {
            return;
        }
        let cluster = if self.fixture {
            Some("fixture".to_owned())
        } else {
            let Some(source) = &self.live.source else {
                // Coroot isn't connected yet; it is looked for once it is.
                return;
            };
            source
                .association()
                .filter(|association| association.access() == wanted.link.access)
                .map(|association| association.cluster().to_owned())
        };
        let Some(cluster) = cluster else {
            if let Some(wanted) = &mut self.revision_observations.wanted {
                wanted.decide(Outcome::NoAssociation, self.selected_app.as_ref());
            }
            return;
        };
        let app = api::AppId::new(format!(
            "{cluster}:{}:Deployment:{}",
            wanted.link.namespace, wanted.link.name
        ));
        // Coroot's applications, when they have answered, or else once
        // they do; example data's are known at once.
        let listed = (!self.fixture && self.applications.is_empty())
            || self.applications.iter().any(|a| a.id == app);
        if let Some(wanted) = &mut self.revision_observations.wanted {
            wanted.app = Some(app.clone());
            if !listed {
                wanted.decide(Outcome::NotListed, self.selected_app.as_ref());
                return;
            }
        }
        if self.selected_app.as_ref() != Some(&app) {
            self.selected_app = Some(app);
            self.report_snapshot = None;
            self.app_page = None;
        }
    }

    /// Coroot's applications answered: the wanted one is among them, or
    /// Coroot lists no such application.
    pub(crate) fn check_wanted_listed(&mut self) {
        let applications = &self.applications;
        if let Some(wanted) = &mut self.revision_observations.wanted
            && wanted.outcome == Outcome::Seeking
            && let Some(app) = &wanted.app
            && !applications.iter().any(|a| &a.id == app)
        {
            wanted.decide(Outcome::NotListed, self.selected_app.as_ref());
        }
    }

    /// The user chose another application or revision: what was asked for
    /// is no longer what shows.
    pub(crate) fn forget_wanted(&mut self) {
        self.revision_observations.wanted = None;
    }

    /// What came of the revision asked for, above the list.
    pub(crate) fn render_wanted(&self, cx: &Context<Self>) -> Option<AnyElement> {
        if self.destination != Destination::Deployments {
            return None;
        }
        let wanted = self
            .revision_observations
            .wanted
            .as_ref()
            .filter(|wanted| wanted.shown == self.selected_app)?;
        let link = &wanted.link;
        let app = format!("{}/{}", link.namespace, link.name);
        let (tone, lead, body) = match wanted.outcome {
            // The list's loading rows say it is being read.
            Outcome::Seeking | Outcome::Unread => return None,
            // Found is a lookup, not a verdict: the row's glyph says how
            // Coroot judges the revision.
            Outcome::Found => (
                Tone::Info,
                format!("Revision {} of {app}.", link.hash),
                format!(
                    "The revision {} runs, by its pod-template hash, as Coroot keeps it.",
                    link.cluster
                ),
            ),
            Outcome::NoRevision => (
                Tone::Warn,
                format!("Coroot keeps no revision {} of {app}.", link.hash),
                format!(
                    "Coroot keeps a Deployment's last 100 revisions, recorded as its pods roll \
                     out.{}",
                    if self.revision_observations.rows.is_empty() {
                        ""
                    } else {
                        " Another revision it keeps is selected instead."
                    }
                ),
            ),
            Outcome::NotListed => (
                Tone::Warn,
                format!("Coroot lists no application {app}."),
                format!(
                    "Coroot's applications for {} don't include the Deployment, so it keeps \
                     no revision {} of it.",
                    link.cluster, link.hash
                ),
            ),
            Outcome::NoAssociation => (
                Tone::Warn,
                format!("No Coroot cluster is linked to {}.", link.cluster),
                format!(
                    "Link one in Coroot's settings to find revision {} of {app}.",
                    link.hash
                ),
            ),
        };
        Some(
            div()
                .id("obs-wanted-revision")
                .test_support()
                .child(ui::banner(tone, Some(lead.into()), body, None, cx))
                .into_any_element(),
        )
    }
}
