//! The page's owner of the team fetch and of joining (#849 slice 4): one
//! `Loader<Version, Source>` whose identity is the source, so an answer for
//! an address the person has since replaced never publishes. A fetch starts
//! only while the section shows, or from Fetch or Join; hiding drops the one
//! in flight, which kills its git child. `--fixture` answers from a script
//! and never touches the network.

use std::sync::Arc;
use std::time::Duration;

use chrono::Utc;
use freshkube_core::job::spawn_job;
use freshkube_core::team::fetch::TeamFetcher;
use freshkube_core::team::{
    Choice, FetchFailed, Membership, Overrides, Refused, Source, Version, example::Shape,
};
use freshkube_reads::{Loader, OnShow};
use gpui_kit::component::input::{InputEvent, InputState};
use gpui_kit::prelude::*;
use gpui_kit::*;
use tokio::runtime::Handle;

use super::{SettingsPage, SettingsSection};

/// A fetch is bounded by core's own deadline; this one only covers a task
/// that never reports.
const MARGIN: Duration = Duration::from_secs(75);
/// Showing the section again fetches when the last answer is this old.
const STALE: Duration = Duration::from_secs(5 * 60);

/// An example's script: what a fetch of a source answers, or `None` never.
pub(super) type Script = Arc<dyn Fn(&Source) -> Option<Result<Version, String>> + Send + Sync>;

/// What answers a fetch: the real one, or the example's script. A script
/// that answers `None` never answers, so the page stays on its loading row.
#[derive(Clone)]
pub(super) enum Fetcher {
    Live(Arc<TeamFetcher>),
    Example(Script),
}

/// Where joining stands once a first version is fetched.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) enum Stage {
    /// Everything the team would set or replace, read-only.
    #[default]
    Preview,
    /// "Map your contexts".
    Map,
    /// The two-step review, which applies.
    Review,
}

/// The Join form's three fields.
pub(super) struct JoinForm {
    pub(super) address: Entity<InputState>,
    pub(super) git_ref: Entity<InputState>,
    pub(super) path: Entity<InputState>,
    /// Masked. Read once at Join, then cleared; kept only in the key store.
    pub(super) token: Entity<InputState>,
    _subscriptions: Vec<Subscription>,
}

#[derive(Default)]
pub(super) struct Owner {
    pub(super) loader: Loader<Version, Source>,
    fetcher: Option<Fetcher>,
    runtime: Option<Handle>,
    /// The source a join is fetching, until its first version lands.
    pub(super) joining: Option<Source>,
    /// Why the last join's fetch or address was refused.
    pub(super) join_error: Option<String>,
    pub(super) stage: Stage,
    /// The contexts of the person's kubeconfig, for mapping.
    pub(super) contexts: Loader<Vec<String>, ()>,
    pub(super) form: Option<JoinForm>,
    /// The key store a token goes in, in a live window.
    pub(super) secrets: Option<freshkube_core::secrets::Secrets>,
}

impl Owner {
    /// The source a fetch is for: the team's, or the one being joined.
    pub(super) fn source<'a>(&'a self, team: Option<&'a Membership>) -> Option<&'a Source> {
        team.map(Membership::source).or(self.joining.as_ref())
    }

    pub(super) fn is_fetching(&self) -> bool {
        self.loader.is_loading()
    }

    /// Starts hidden, so the first time the section shows it reads.
    pub(super) fn hide(&mut self, cx: &App) {
        self.loader.set_visible(false, OnShow::Never, cx);
    }

    /// Drops what is in flight and what a join had begun.
    pub(super) fn stop(&mut self) {
        self.loader.reset();
        self.contexts.reset();
        self.joining = None;
        self.stage = Stage::Preview;
    }

    /// The contexts to choose among, or none until they are read.
    pub(super) fn available(&self) -> Vec<String> {
        self.contexts.data().cloned().unwrap_or_default()
    }
}

/// The invented contexts example data offers.
fn example_contexts() -> Vec<String> {
    ["core-fra", "cicd-fra", "stage-lon", "prod-ams"]
        .map(String::from)
        .into()
}

impl SettingsPage {
    /// A live window: fetches run the person's own git or an HTTPS request.
    pub(in crate::desktop::settings) fn set_team_io(
        &mut self,
        runtime: Handle,
        app_dir: Option<std::path::PathBuf>,
        secrets: Option<freshkube_core::secrets::Secrets>,
        preferences: Option<&std::path::Path>,
    ) {
        if let Some(preferences) = preferences {
            let store = super::store::Store::beside(preferences);
            let restored = store.load();
            self.team.membership = restored.membership;
            self.team.corrupt = restored.unreadable.is_some();
            self.team.notice = restored.unreadable;
            self.team.store = Some(store);
            self.team.derive();
            self.apply_team_floors();
        }
        let owner = &mut self.team.owner;
        owner.secrets = secrets.clone();
        owner.runtime = Some(runtime);
        owner.fetcher = app_dir
            .map(|dir| Fetcher::Live(Arc::new(TeamFetcher::new(dir, secrets, Default::default()))));
    }

    /// Plays an example team (`--fixture`): the fetch answers from a script.
    pub(in crate::desktop) fn set_team_example(&mut self, shape: Shape, cx: &mut Context<Self>) {
        self.team = super::TeamState::example(shape, Utc::now());
        self.team.owner.hide(cx);
        let newest = self
            .team
            .membership
            .as_ref()
            .and_then(|m| m.pending().or(m.applied()))
            .cloned()
            .unwrap_or_else(freshkube_core::team::example::first_version);
        self.team.owner.fetcher = Some(Fetcher::Example(Arc::new(move |_| {
            Some(match shape {
                Shape::Failed | Shape::JoinFailed => Err("the host didn’t answer".to_string()),
                _ => Ok(newest.clone()),
            })
        })));
        self.derive_levels();
        self.refit_columns();
        cx.notify();
    }

    /// A new page starts with its fetch hidden, so the first time Team setup
    /// shows it reads.
    pub(in crate::desktop::settings) fn start_team_hidden(&mut self, cx: &mut Context<Self>) {
        self.team.owner.hide(cx);
    }

    /// Fetches while the section shows; hiding drops the read in flight.
    pub(in crate::desktop::settings) fn sync_team(&mut self, cx: &mut Context<Self>) {
        let shown = self.visible && self.section == SettingsSection::Team;
        let again = self
            .team
            .owner
            .loader
            .set_visible(shown, OnShow::ReadIfOlder(STALE), cx);
        if shown && again && self.team.membership.is_some() {
            self.team_fetch(cx);
        }
    }

    /// Reads the team file again: Fetch, or the section showing.
    pub(super) fn team_fetch(&mut self, cx: &mut Context<Self>) {
        let Some(source) = self
            .team
            .owner
            .source(self.team.membership.as_ref())
            .cloned()
        else {
            return;
        };
        match self.team.owner.fetcher.clone() {
            Some(Fetcher::Example(script)) => match script(&source) {
                Some(result) => {
                    self.team.owner.loader.resolve(source, result.clone());
                    self.team_landed(result, cx);
                }
                // Held: never answers, as FRESHKUBE_FIXTURE_HOLD does.
                None => _ = self.team.owner.loader.start(source),
            },
            Some(Fetcher::Live(fetcher)) => {
                let Some(runtime) = self.team.owner.runtime.clone() else {
                    return;
                };
                let request = self.team.owner.loader.start(source.clone());
                let (job, receiver) = spawn_job(
                    &runtime,
                    MARGIN,
                    "Fetching the team file timed out".to_string(),
                    fetcher.work(source),
                );
                let task = cx.spawn(async move |this, cx| {
                    let result = receiver.await.unwrap_or_else(|_| {
                        Err("Fetching the team file stopped unexpectedly".to_string())
                    });
                    _ = this.update(cx, |page, cx| {
                        if page.team.owner.loader.land(&request, result.clone(), cx) {
                            page.team_landed(result, cx);
                        }
                        cx.notify();
                    });
                });
                self.team.owner.loader.run(Some(job), task);
            }
            None => {}
        }
        cx.notify();
    }

    /// An answer that is the latest for its source: a join's first version,
    /// or a later one for the team.
    pub(in crate::desktop::settings) fn team_landed(
        &mut self,
        result: Result<Version, String>,
        cx: &mut Context<Self>,
    ) {
        let now = Utc::now();
        let result = result.map_err(FetchFailed);
        if let Some(team) = self.team.membership.as_mut() {
            if team.fetched(result, now) {
                self.team.review = team.review();
                self.team.later = false;
                self.team.refusal = None;
            }
        } else if let Some(source) = self.team.owner.joining.take() {
            match result {
                Ok(version) => {
                    let mut team = Membership::join(source, Overrides::new());
                    team.fetched(Ok(version), now);
                    self.team.review = team.review();
                    self.team.membership = Some(team);
                    self.team.owner.stage = Stage::Preview;
                    self.team.owner.join_error = None;
                    self.refit_columns();
                }
                Err(error) => {
                    // Nothing joined: the token typed for it goes too.
                    self.team_forget_token(&source);
                    self.team.owner.join_error = Some(error.0);
                }
            }
        }
        self.team.derive();
        self.derive_levels();
        cx.notify();
    }

    /// Creates the Join form's inputs, after the current update.
    pub(in crate::desktop::settings) fn prepare_team(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.team.owner.form.is_some() || self.section != SettingsSection::Team {
            return;
        }
        let address = cx.new(|cx| {
            InputState::new(window, cx).placeholder("https://git.acme.example/ops/freshkube-setup")
        });
        let git_ref = cx.new(|cx| InputState::new(window, cx).placeholder("main"));
        let path = cx.new(|cx| InputState::new(window, cx).placeholder("freshkube.yaml"));
        let token = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("Only for an https:// file")
                .masked(true)
        });
        let mut subscriptions = Vec::new();
        for input in [&address, &git_ref, &path, &token] {
            subscriptions.push(
                cx.subscribe_in(input, window, |page, _, event, window, cx| match event {
                    InputEvent::PressEnter { .. } => page.team_join(window, cx),
                    InputEvent::Change => {
                        page.team.owner.join_error = None;
                        page.team.derive();
                        cx.notify();
                    }
                    _ => {}
                }),
            );
        }
        self.team.owner.form = Some(JoinForm {
            address,
            git_ref,
            path,
            token,
            _subscriptions: subscriptions,
        });
        cx.notify();
    }

    /// Join: checks the address, then fetches it. A refusal never repeats
    /// the address.
    pub(super) fn team_join(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.team.membership.is_some() || self.team.owner.is_fetching() {
            return;
        }
        let Some(form) = self.team.owner.form.as_ref() else {
            return;
        };
        let address = form.address.clone();
        let token_input = form.token.clone();
        let token = token_input.read(cx).value().to_string();
        let text = |input: &Entity<InputState>| input.read(cx).value().trim().to_owned();
        let parsed = Source::parse(
            &text(&form.address),
            &text(&form.git_ref),
            &text(&form.path),
        );
        match parsed {
            Ok(source) => {
                if !token.trim().is_empty() {
                    if let Err(words) = self.team_keep_token(&source, token.trim()) {
                        self.team.owner.join_error = Some(words.into());
                        self.team.derive();
                        cx.notify();
                        return;
                    }
                    token_input.update(cx, |input, cx| input.set_value("", window, cx));
                }
                self.team.owner.join_error = None;
                self.team.owner.joining = Some(source);
                self.team.left = None;
                self.team_fetch(cx);
            }
            Err(refused) => {
                // What was typed carried a credential: it does not stay in
                // the field.
                if refused == Refused::Credentials || refused == Refused::QueryOrFragment {
                    address.update(cx, |input, cx| input.set_value("", window, cx));
                }
                self.team.owner.join_error = Some(refused.to_string());
                self.team.derive();
                cx.notify();
            }
        }
    }

    /// Keeps the typed token in the key store, under the source's account,
    /// before the first fetch reads it. Only an https:// file takes one: a
    /// git repository uses the person's own sign-in. The words never repeat
    /// the token or the address.
    fn team_keep_token(&self, source: &Source, token: &str) -> Result<(), &'static str> {
        let Source::Https(https) = source else {
            return Err(
                "A token is only for an https:// file; a git repository uses your own sign-in.",
            );
        };
        let Some(secrets) = &self.team.owner.secrets else {
            return Err("There is no key store here, so a token can’t be kept.");
        };
        secrets
            .write(&https.token_account(), token)
            .map_err(|_| "The token couldn’t be saved to your key store.")
    }

    /// Forgets the token a source's account holds: a join that didn't
    /// finish, or leaving. Nothing to do for a git source.
    pub(super) fn team_forget_token(&self, source: &Source) {
        if let (Source::Https(https), Some(secrets)) = (source, &self.team.owner.secrets)
            && let Err(error) = secrets.forget(&https.token_account())
        {
            tracing::warn!("a team token could not be removed from the key store: {error}");
        }
    }

    /// Steps on from the preview, then from the mapping, to the review.
    pub(in crate::desktop::settings) fn team_continue(&mut self, cx: &mut Context<Self>) {
        let Some(team) = self.team.membership.as_ref() else {
            return;
        };
        match self.team.owner.stage {
            Stage::Preview if team.needs_mapping() => {
                self.team.owner.stage = Stage::Map;
                self.team_load_contexts(cx);
            }
            Stage::Preview => self.team.owner.stage = Stage::Review,
            Stage::Map if team.mapping().is_complete() => self.team.owner.stage = Stage::Review,
            Stage::Map | Stage::Review => {}
        }
        self.team.derive();
        cx.notify();
    }

    /// Leaves a join that has not applied: nothing was kept.
    pub(in crate::desktop::settings) fn team_cancel_join(&mut self, cx: &mut Context<Self>) {
        if self
            .team
            .membership
            .as_ref()
            .is_some_and(|m| m.applied().is_none())
        {
            if let Some(team) = &self.team.membership {
                self.team_forget_token(team.source());
            }
            self.team.membership = None;
            self.team.review = None;
            self.team.owner.stage = Stage::Preview;
            self.team.owner.loader.reset();
            self.team.derive();
            self.derive_levels();
            self.refit_columns();
            cx.notify();
        }
    }

    /// Reads the contexts of the person's kubeconfig for the mapping.
    fn team_load_contexts(&mut self, cx: &mut Context<Self>) {
        let owner = &mut self.team.owner;
        if matches!(owner.fetcher, Some(Fetcher::Example(_))) {
            owner.contexts.resolve((), Ok(example_contexts()));
            return;
        }
        let Some(runtime) = owner.runtime.clone() else {
            return;
        };
        let explicit = self.workspace.kubeconfig.clone();
        let request = owner.contexts.start(());
        let (job, receiver) = spawn_job(
            &runtime,
            Duration::from_secs(15),
            "Reading your kubeconfig timed out".to_string(),
            async move {
                tokio::task::spawn_blocking(move || {
                    let sources =
                        freshkube_core::resources::kubeconfig_sources(explicit.as_deref());
                    freshkube_core::resources::discover_contexts(&sources)
                        .contexts
                        .into_iter()
                        .map(|context| context.name)
                        .collect::<Vec<_>>()
                })
                .await
                .map_err(|_| "Reading your kubeconfig stopped unexpectedly".to_string())
            },
        );
        let task = cx.spawn(async move |this, cx| {
            let result = receiver
                .await
                .unwrap_or_else(|_| Err("Reading your kubeconfig stopped unexpectedly".into()));
            _ = this.update(cx, |page, cx| {
                if page.team.owner.contexts.land(&request, result, cx) {
                    page.team.derive();
                }
                cx.notify();
            });
        });
        owner.contexts.run(Some(job), task);
    }

    /// Chooses the context of one team cluster, or to leave it out.
    pub(in crate::desktop::settings) fn team_map_choose(
        &mut self,
        id: &str,
        choice: Choice,
        cx: &mut Context<Self>,
    ) {
        let available = self.team.owner.available();
        if let Some(team) = self.team.membership.as_mut()
            && team.applied().is_none()
        {
            _ = team.map_context(id, choice, &available);
            self.team.derive();
            cx.notify();
        }
    }

    /// Chooses the context named like each cluster, on request.
    pub(in crate::desktop::settings) fn team_map_suggested(&mut self, cx: &mut Context<Self>) {
        let available = self.team.owner.available();
        if let Some(team) = self.team.membership.as_mut()
            && team.applied().is_none()
        {
            team.map_suggested(&available);
            self.team.derive();
            cx.notify();
        }
    }

    /// Replaces the example's script, for the tests; `None` never answers.
    #[cfg(test)]
    pub(in crate::desktop::settings) fn set_team_script(
        &mut self,
        script: impl Fn(&Source) -> Option<Result<Version, String>> + Send + Sync + 'static,
    ) {
        self.team.owner.fetcher = Some(Fetcher::Example(Arc::new(script)));
    }

    /// What the Join form's address field holds, for the tests.
    #[cfg(test)]
    pub(in crate::desktop::settings) fn team_address_text(&self, cx: &App) -> String {
        self.team
            .owner
            .form
            .as_ref()
            .map(|form| form.address.read(cx).value().to_string())
            .unwrap_or_default()
    }

    /// Why the last join was refused, for the tests.
    #[cfg(test)]
    pub(in crate::desktop::settings) fn team_join_error(&self) -> String {
        self.team.owner.join_error.clone().unwrap_or_default()
    }

    /// Joins the example team and takes every step to Apply through the
    /// page's own methods, for the tests that need Settings on another section.
    #[cfg(test)]
    pub(in crate::desktop::settings) fn team_join_and_apply_for_test(
        &mut self,
        cx: &mut Context<Self>,
    ) {
        self.team.owner.joining = Some(freshkube_core::team::example::source());
        self.team_fetch(cx);
        self.team_continue(cx);
        self.team_map_suggested(cx);
        self.team_continue(cx);
        let pending: Vec<String> = self
            .team
            .review
            .iter()
            .flat_map(|review| review.trust_pending().map(|c| c.at.clone()))
            .collect();
        for at in pending {
            self.team_confirm_trust(&at, cx);
        }
        self.team_apply(cx);
    }

    /// Saves beside `preferences` from now on, for the tests of a live window.
    #[cfg(test)]
    pub(in crate::desktop::settings) fn team_use_store_for_test(
        &mut self,
        preferences: &std::path::Path,
    ) {
        self.team.store = Some(super::store::Store::beside(preferences));
    }

    /// A key store for the token field, for the tests of a window on example
    /// data, which has none.
    #[cfg(test)]
    pub(in crate::desktop::settings) fn team_use_secrets_for_test(
        &mut self,
        secrets: freshkube_core::secrets::Secrets,
    ) {
        self.team.owner.secrets = Some(secrets);
    }

    /// What the Join form's token field holds, for the tests.
    #[cfg(test)]
    pub(in crate::desktop::settings) fn team_token_text(&self, cx: &App) -> String {
        self.team
            .owner
            .form
            .as_ref()
            .map(|form| form.token.read(cx).value().to_string())
            .unwrap_or_default()
    }

    /// Sets one of your own values on the membership, for the tests.
    #[cfg(test)]
    pub(in crate::desktop::settings) fn team_set_override_for_test(
        &mut self,
        key: &str,
        value: serde_json::Value,
    ) {
        if let Some(team) = self.team.membership.as_mut() {
            team.set_override(key, value);
        }
        self.team.derive();
    }

    /// Whether the membership still holds your value for `key`.
    #[cfg(test)]
    pub(in crate::desktop::settings) fn team_has_override(&self, key: &str) -> bool {
        self.team
            .membership
            .as_ref()
            .is_some_and(|team| team.overrides().contains_key(key))
    }

    /// "Forget it", for the tests.
    #[cfg(test)]
    pub(in crate::desktop::settings) fn team_forget_unreadable_for_test(
        &mut self,
        cx: &mut Context<Self>,
    ) {
        self.team_forget_unreadable(cx);
    }

    /// Marks the saved team unreadable, as a restore that found it so does.
    #[cfg(test)]
    pub(in crate::desktop::settings) fn team_mark_unreadable_for_test(&mut self) {
        self.team.corrupt = true;
        self.team.notice = Some("The file that keeps your team setup couldn’t be read.".into());
        self.team.derive();
        self.apply_team_floors();
    }

    /// The words of the last save's failure notice, for the tests.
    #[cfg(test)]
    pub(in crate::desktop::settings) fn notice_failed_words(&self) -> Option<String> {
        match &self.notice {
            Some(super::super::Notice::Failed(words)) => Some(words.to_string()),
            _ => None,
        }
    }

    /// The notice above the section, for the tests.
    #[cfg(test)]
    pub(in crate::desktop::settings) fn team_notice(&self) -> Option<String> {
        self.team.shown.notice.clone()
    }

    /// Whether the section is in the middle of joining.
    pub(super) fn team_is_joining(&self) -> bool {
        self.team
            .membership
            .as_ref()
            .is_some_and(|m| m.applied().is_none())
    }
}
