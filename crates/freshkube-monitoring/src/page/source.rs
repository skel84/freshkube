//! Settings' Metrics source: per context, discovery (automatic), one
//! Service through the Kubernetes API's proxy, or a URL with an optional
//! bearer token. Test checks the form as typed, without saving it; Save
//! keeps it in `monitoring.json` and the token in the credential store,
//! then connects again. Nothing here runs in example mode.
use std::future::Future;
use std::pin::Pin;

use freshkube_core::monitoring::{
    Discovery, ErrorKind, LOOKED_FOR, PrometheusService, QueryError, cluster_client, confirm,
    confirm_url, discover, forget_after, normalise_path, normalise_url,
};
use futures::channel::oneshot;
use gpui_kit::component::input::{InputEvent, InputState};
use gpui_kit::{App, AppContext, Context, Entity, Focusable, SharedString, Subscription, Window};

use super::connection::describe;
use super::{MonitoringEvent, MonitoringPage, Read, ReadKind, Request};
use crate::store::{Choice, token_account};
use freshkube_core::secrets::{STORE_NAME, SecretStore};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Mode {
    Automatic,
    Service,
    Url,
}

pub(super) enum Test {
    Idle,
    Testing(#[allow(dead_code)] Request),
    Passed(SharedString),
    Failed(SharedString),
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Field {
    Namespace,
    Name,
    Port,
    Url,
}

#[derive(PartialEq, Eq)]
pub(super) struct Values {
    mode: Mode,
    https: bool,
    fields: [String; 6],
}

/// The form's inputs and what Test last said. The inputs are filled from
/// what is saved whenever the context changes.
pub(crate) struct SourceForm {
    /// The context the inputs were filled for; none before the first fill.
    filled: Option<Option<String>>,
    pub(super) focus: gpui_kit::FocusHandle,
    pub(super) mode: Mode,
    pub(super) namespace: Entity<InputState>,
    pub(super) name: Entity<InputState>,
    pub(super) port: Entity<InputState>,
    pub(super) path: Entity<InputState>,
    pub(super) url: Entity<InputState>,
    pub(super) token: Entity<InputState>,
    pub(super) https: bool,
    pub(super) test: Test,
    pub(super) replacing_token: bool,
    pub(super) dirty: bool,
    subscriptions: Vec<Subscription>,
    pub(super) initial: Option<Values>,
    pub(super) error: Option<(Field, SharedString)>,
    read: Read,
    /// Why the form can't be used as typed, or that it was saved.
    pub(super) note: Option<SharedString>,
}

/// The form as typed. A URL's token is only what was typed now.
enum Draft {
    Automatic,
    Service(PrometheusService),
    Url { url: String, typed: Option<String> },
}

type Work = Pin<Box<dyn Future<Output = Result<String, QueryError>> + Send>>;

impl SourceForm {
    pub(super) fn values(&self, cx: &App) -> Values {
        Values {
            mode: self.mode,
            https: self.https,
            fields: [
                &self.namespace,
                &self.name,
                &self.port,
                &self.path,
                &self.url,
                &self.token,
            ]
            .map(|input| input.read(cx).value().to_string()),
        }
    }
    fn changed(&mut self, cx: &App) {
        self.dirty = self.initial.as_ref() != Some(&self.values(cx));
        self.test = Test::Idle;
        self.read.reset();
        self.note = None;
        self.error = None;
    }
    pub(super) fn error(&self, field: Field) -> Option<SharedString> {
        self.error
            .as_ref()
            .filter(|(failed, _)| *failed == field)
            .map(|(_, why)| why.clone())
    }
    fn field(&self, field: Field) -> &Entity<InputState> {
        match field {
            Field::Namespace => &self.namespace,
            Field::Name => &self.name,
            Field::Port => &self.port,
            Field::Url => &self.url,
        }
    }
}

impl MonitoringPage {
    /// Creates the form on first use and fills it again when the context
    /// changed since. Settings calls this before it draws.
    pub fn source_form(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.form.is_none() {
            let input = |placeholder: &'static str, window: &mut Window, cx: &mut Context<Self>| {
                cx.new(|cx| InputState::new(window, cx).placeholder(placeholder))
            };
            self.form = Some(SourceForm {
                filled: None,
                focus: cx.focus_handle(),
                mode: Mode::Automatic,
                namespace: input("monitoring", window, cx),
                name: input("prometheus-operated", window, cx),
                port: input("9090", window, cx),
                path: input("Optional, such as select/0/prometheus", window, cx),
                url: input(
                    "https://vmselect.example.com/select/0/prometheus",
                    window,
                    cx,
                ),
                token: cx.new(|cx| InputState::new(window, cx).masked(true)),
                https: false,
                test: Test::Idle,
                replacing_token: false,
                dirty: false,
                subscriptions: Vec::new(),
                initial: None,
                error: None,
                read: Read::default(),
                note: None,
            });
        }
        if let Some(form) = &mut self.form
            && form.subscriptions.is_empty()
        {
            for (input, field) in [
                (&form.namespace, Some(Field::Namespace)),
                (&form.name, Some(Field::Name)),
                (&form.port, Some(Field::Port)),
                (&form.path, None),
                (&form.url, Some(Field::Url)),
                (&form.token, None),
            ] {
                form.subscriptions.push(cx.subscribe_in(
                    input,
                    window,
                    move |page, _, event, window, cx| match event {
                        InputEvent::PressEnter { .. } => page.save_source(window, cx),
                        InputEvent::Change => {
                            if let Some(form) = &mut page.form {
                                form.changed(cx);
                            }
                            cx.notify();
                        }
                        InputEvent::Blur => {
                            if let Err((failed, why)) = page.draft(cx)
                                && field == Some(failed)
                                && let Some(form) = &mut page.form
                            {
                                form.error = Some((failed, why.into()));
                                cx.notify();
                            }
                        }
                        _ => {}
                    },
                ));
            }
        }
        let context = self.context();
        if self.form.as_ref().and_then(|form| form.filled.as_ref()) != Some(&context) {
            self.fill_form(context, window, cx);
        }
    }

    fn fill_form(&mut self, context: Option<String>, window: &mut Window, cx: &mut Context<Self>) {
        let choice = context
            .as_ref()
            .and_then(|context| self.saved.choices.get(context))
            .cloned();
        // Automatic starts the Service fields from what discovery found.
        let found = context
            .as_ref()
            .and_then(|context| self.saved.services.get(context))
            .cloned();
        let (mode, service, url, token) = match choice {
            Some(Choice::Service(service)) => (Mode::Service, Some(service), String::new(), false),
            Some(Choice::Url { url, token }) => (Mode::Url, None, url, token),
            None => (Mode::Automatic, found, String::new(), false),
        };
        let Some(form) = &mut self.form else {
            return;
        };
        form.filled = Some(context);
        form.mode = mode;
        form.replacing_token = false;
        form.dirty = false;
        form.test = Test::Idle;
        form.read.resolve(None, Ok(()));
        form.note = None;
        form.https = service.as_ref().is_some_and(|service| service.https);
        let set = |input: &Entity<InputState>, value: String, window: &mut Window, cx: &mut App| {
            input.update(cx, |input, cx| input.set_value(value, window, cx));
        };
        let field =
            |pick: fn(&PrometheusService) -> String| service.as_ref().map(pick).unwrap_or_default();
        set(&form.namespace, field(|s| s.namespace.clone()), window, cx);
        set(&form.name, field(|s| s.name.clone()), window, cx);
        set(&form.port, field(|s| s.port.to_string()), window, cx);
        set(&form.path, field(|s| s.path.clone()), window, cx);
        set(&form.url, url, window, cx);
        set(&form.token, String::new(), window, cx);
        let placeholder = token_placeholder(token);
        form.token.update(cx, |input, cx| {
            input.set_placeholder(placeholder, window, cx)
        });
        form.initial = Some(form.values(cx));
        form.error = None;
    }

    pub(super) fn set_mode(&mut self, mode: Mode, cx: &mut Context<Self>) {
        if let Some(form) = &mut self.form {
            form.mode = mode;
            form.changed(cx);
        }
        cx.notify();
    }

    pub(super) fn set_https(&mut self, https: bool, cx: &mut Context<Self>) {
        if let Some(form) = &mut self.form {
            form.https = https;
            form.changed(cx);
        }
        cx.notify();
    }

    /// The form as typed, or why it can't be used.
    fn draft(&self, cx: &App) -> Result<Draft, (Field, String)> {
        let form = self.form.as_ref().ok_or((Field::Url, String::new()))?;
        let text = |input: &Entity<InputState>| input.read(cx).value().trim().to_owned();
        match form.mode {
            Mode::Automatic => Ok(Draft::Automatic),
            Mode::Service => {
                let (namespace, name) = (text(&form.namespace), text(&form.name));
                if namespace.is_empty() {
                    return Err((Field::Namespace, "Enter the Service’s namespace.".into()));
                }
                if name.is_empty() {
                    return Err((Field::Name, "Enter the Service’s name.".into()));
                }
                let port = text(&form.port)
                    .parse::<u16>()
                    .ok()
                    .filter(|port| *port > 0)
                    .ok_or_else(|| {
                        (
                            Field::Port,
                            "Enter the Service’s port as a number from 1 to 65535.".to_owned(),
                        )
                    })?;
                Ok(Draft::Service(PrometheusService {
                    https: form.https,
                    ..PrometheusService::new(namespace, name, port)
                        .with_path(&normalise_path(&text(&form.path)))
                }))
            }
            Mode::Url => {
                let url = normalise_url(&text(&form.url)).map_err(|why| (Field::Url, why))?;
                let typed = Some(text(&form.token)).filter(|token| !token.is_empty());
                Ok(Draft::Url { url, typed })
            }
        }
    }

    /// Checks the form as typed against the cluster or the URL. Nothing
    /// is saved, and the page's own connection is left alone.
    pub(super) fn test_source(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(source) = self.source.clone() else {
            return;
        };
        let draft = match self.draft(cx) {
            Ok(draft) => draft,
            Err((field, why)) => return self.form_error(field, why, None, cx),
        };
        if let Some(form) = &self.form {
            window.focus(&form.focus, cx);
        }
        let access = source.access.clone();
        let work: Work = match draft {
            Draft::Automatic => Box::pin(async move {
                let client = cluster_client(&access).await?;
                let discovery = discover(&client, None)
                    .await
                    .inspect_err(|error| forget_after(&access, error))?;
                match discovery {
                    Discovery::Found {
                        prometheus,
                        version,
                        ..
                    } => Ok(answered(&prometheus, version.as_deref())),
                    Discovery::Missing { .. } => Err(QueryError::new(
                        ErrorKind::NotFound,
                        format!("Nothing answered. Discovery looks for {LOOKED_FOR}."),
                    )),
                }
            }),
            Draft::Service(service) => Box::pin(async move {
                let client = cluster_client(&access).await?;
                let (prometheus, version) = confirm(&client, service)
                    .await
                    .inspect_err(|error| forget_after(&access, error))?;
                Ok(answered(&prometheus, version.as_deref()))
            }),
            Draft::Url { url, typed } => {
                // An empty token field tests with the saved token, if the
                // URL is the saved one.
                let saved = matches!(
                    self.chosen_source(),
                    Some(Choice::Url { url: saved, token: true }) if *saved == url
                );
                let token = self.token_reader(&url, saved);
                Box::pin(async move {
                    let token = match typed {
                        Some(typed) => Some(typed),
                        None => token.await?,
                    };
                    let (prometheus, version) = confirm_url(url, token).await?;
                    Ok(answered(&prometheus, version.as_deref()))
                })
            }
        };
        let request = self.run(
            self.read_identity(ReadKind::Test {
                context: source.context.clone(),
            }),
            |this| this.form.as_mut().map(|form| &mut form.read),
            work,
            cx,
            |this, result, cx| {
                if let Some(form) = &mut this.form {
                    form.test = match result {
                        Ok(message) => Test::Passed(message.into()),
                        Err(error) => Test::Failed(error.message.into()),
                    };
                }
                cx.notify();
            },
        );
        if let Some(form) = &mut self.form {
            form.test = Test::Testing(request);
            form.note = None;
        }
        cx.notify();
    }

    pub(super) fn revert_source(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.fill_form(self.context(), window, cx);
        cx.notify();
    }

    /// Drops an explicit Settings check when the section hides.
    pub fn pause_settings(&mut self) {
        if let Some(form) = &mut self.form {
            form.read.reset();
            form.test = Test::Idle;
        }
    }

    /// Invalidates a token cached before a setup edit changed its store.
    pub fn settings_token_changed(&mut self, url: &str) {
        self.tokens.remove(url);
    }

    /// The effective choice under automatic mode, as the dashboard reads it.
    pub fn settings_resolution(
        &self,
        resolved: freshkube_core::monitoring::scope::Resolved,
    ) -> freshkube_core::monitoring::scope::Resolved {
        use freshkube_core::monitoring::scope::{Place, Resolved};
        if !resolved.is_discovery() {
            return resolved;
        }
        match self.chosen_source() {
            Some(Choice::Service(service)) => Resolved::Own(Place::Service(service.clone())),
            Some(Choice::Url { url, token }) => Resolved::Own(Place::Url {
                url: url.clone(),
                token: *token,
            }),
            None => resolved,
        }
    }

    /// The credential store already used by Monitoring's URL sources.
    pub fn settings_secrets(&self) -> Option<freshkube_core::secrets::Secrets> {
        self.secrets.clone()
    }

    /// Keeps the form for the context and connects with it.
    pub(super) fn save_source(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(context) = self.context() else {
            return;
        };
        let draft = match self.draft(cx) {
            Ok(draft) => draft,
            Err((field, why)) => return self.form_error(field, why, Some(window), cx),
        };
        let previous = self.saved.choices.get(&context).cloned();
        let choice = match draft {
            Draft::Automatic => None,
            Draft::Service(service) => Some(Choice::Service(service)),
            Draft::Url { url, typed } => {
                let kept = matches!(
                    &previous,
                    Some(Choice::Url { url: saved, token: true }) if *saved == url
                );
                if let Some(token) = &typed {
                    self.tokens.insert(url.clone(), token.clone());
                    let (account, token) = (token_account(&url), token.clone());
                    self.secret_step(move |store| store.write(&account, &token), cx);
                }
                let token = self.secrets.is_some() && (typed.is_some() || kept);
                Some(Choice::Url { url, token })
            }
        };
        self.keep_choice(context, choice.clone(), cx);
        // The other contexts and the setup share this token account.
        if let Some(Choice::Url { url: old, .. }) = &previous
            && !matches!(&choice, Some(Choice::Url { url, .. }) if url == old)
        {
            self.tokens.remove(old);
            self.forget_unused_token(old, cx);
        }
        if let Some(form) = &mut self.form {
            form.test = Test::Idle;
            form.read.resolve(None, Ok(()));
            form.dirty = false;
            form.replacing_token = false;
            form.note = Some("Saved monitoring.json".into());
            let token = matches!(choice, Some(Choice::Url { token: true, .. }));
            form.token.update(cx, |input, cx| {
                input.set_value("", window, cx);
                input.set_placeholder(token_placeholder(token), window, cx);
            });
            form.initial = Some(form.values(cx));
            form.error = None;
        }
        self.reconnect(cx);
    }

    /// Forgets the saved token for the context's URL.
    pub(super) fn forget_token(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(context) = self.context() else {
            return;
        };
        let Some(Choice::Url { url, .. }) = self.saved.choices.get(&context).cloned() else {
            return;
        };
        self.tokens.remove(&url);
        self.keep_choice(
            context,
            Some(Choice::Url {
                url: url.clone(),
                token: false,
            }),
            cx,
        );
        self.forget_unused_token(&url, cx);
        if let Some(form) = &mut self.form {
            form.note = Some("Token forgotten.".into());
            form.token.update(cx, |input, cx| {
                input.set_placeholder(token_placeholder(false), window, cx)
            });
        }
        self.reconnect(cx);
    }

    fn forget_unused_token(&mut self, url: &str, cx: &mut Context<Self>) {
        let mut named = self.saved.token_urls();
        if let Some(setup) = &self.inputs.setup {
            named.extend(freshkube_core::monitoring::scope::token_urls(setup));
        }
        if !named.contains(url) {
            let account = token_account(url);
            self.secret_step(move |store| store.forget(&account), cx);
        }
    }

    fn keep_choice(&mut self, context: String, choice: Option<Choice>, cx: &mut Context<Self>) {
        match &choice {
            Some(choice) => self.saved.choices.insert(context.clone(), choice.clone()),
            None => self.saved.choices.remove(&context),
        };
        self.save(
            move |saved| match choice {
                Some(choice) => {
                    saved.choices.insert(context, choice);
                }
                None => {
                    saved.choices.remove(&context);
                }
            },
            cx,
        );
    }

    /// Drops the current connection and looks again if the page shows.
    fn reconnect(&mut self, cx: &mut Context<Self>) {
        self.connection_reads.forget();
        self.connection = super::connection::Connection::None;
        cx.emit(MonitoringEvent::History);
        self.connect(cx);
        cx.notify();
    }

    fn form_error(
        &mut self,
        field: Field,
        note: String,
        window: Option<&mut Window>,
        cx: &mut Context<Self>,
    ) {
        if let Some(form) = &mut self.form {
            form.error = Some((field, note.into()));
            if let Some(window) = window {
                window.focus(&form.field(field).read(cx).focus_handle(cx), cx);
            }
            form.test = Test::Idle;
            form.read.resolve(None, Ok(()));
        }
        cx.notify();
    }

    /// Runs credential store work in order, off the UI thread. A failure
    /// shows in Settings; the token stays in memory for this session.
    fn secret_step(
        &mut self,
        step: impl FnOnce(&dyn SecretStore) -> Result<(), String> + Send + 'static,
        cx: &mut Context<Self>,
    ) {
        let Some(secrets) = self.secrets.clone() else {
            return;
        };
        let previous = self.secret_queue.take();
        let (done, next) = oneshot::channel();
        self.secret_queue = Some(next);
        let work = cx.background_spawn(async move {
            if let Some(previous) = previous {
                let _ = previous.await;
            }
            let result = step(secrets.as_ref());
            let _ = done.send(());
            result
        });
        cx.spawn(async move |this, cx| {
            if let Err(error) = work.await {
                _ = this.update(cx, |this, cx| {
                    this.save_error = Some(error.into());
                    cx.notify();
                });
            }
        })
        .detach();
    }

    /// Clears what Test said when the context changes.
    pub(super) fn reset_form(&mut self) {
        if let Some(form) = &mut self.form {
            form.test = Test::Idle;
            form.read.resolve(None, Ok(()));
            form.note = None;
        }
    }
}

fn answered(prometheus: &freshkube_core::monitoring::Prometheus, version: Option<&str>) -> String {
    format!(
        "{} answered at {}",
        describe(prometheus.endpoint().backend(), version),
        prometheus.endpoint().label()
    )
}

fn token_placeholder(saved: bool) -> String {
    if saved {
        format!("Saved in {STORE_NAME}; type to replace")
    } else {
        "Bearer token, optional".into()
    }
}
