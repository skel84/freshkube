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
use gpui_kit::component::{
    Disableable, Sizable,
    button::{Button, ButtonGroup, ButtonVariants},
    checkbox::Checkbox,
    h_flex,
    input::{Input, InputState},
    v_flex,
};
use gpui_kit::prelude::*;
use gpui_kit::{
    AnyElement, App, AppContext, Context, Entity, FontWeight, SharedString, TestSupportExt, Window,
    div,
};

use super::connection::describe;
use super::{MonitoringEvent, MonitoringPage, Request};
use crate::palette::palette;
use crate::store::{Choice, token_account};
use crate::ui::{self, dp};
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

/// The form's inputs and what Test last said. The inputs are filled from
/// what is saved whenever the context changes.
pub(crate) struct SourceForm {
    /// The context the inputs were filled for; none before the first fill.
    filled: Option<Option<String>>,
    pub(super) mode: Mode,
    pub(super) namespace: Entity<InputState>,
    pub(super) name: Entity<InputState>,
    pub(super) port: Entity<InputState>,
    pub(super) path: Entity<InputState>,
    pub(super) url: Entity<InputState>,
    pub(super) token: Entity<InputState>,
    https: bool,
    pub(super) test: Test,
    /// Why the form can't be used as typed, or that it was saved.
    note: Option<SharedString>,
}

/// The form as typed. A URL's token is only what was typed now.
enum Draft {
    Automatic,
    Service(PrometheusService),
    Url { url: String, typed: Option<String> },
}

type Work = Pin<Box<dyn Future<Output = Result<String, QueryError>> + Send>>;

impl MonitoringPage {
    /// Creates the form on first use and fills it again when the context
    /// changed since. The Settings popover calls this before it draws.
    pub fn source_form(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.form.is_none() {
            let input = |placeholder: &'static str, window: &mut Window, cx: &mut Context<Self>| {
                cx.new(|cx| InputState::new(window, cx).placeholder(placeholder))
            };
            self.form = Some(SourceForm {
                filled: None,
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
                note: None,
            });
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
        form.test = Test::Idle;
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
    }

    pub(super) fn set_mode(&mut self, mode: Mode, cx: &mut Context<Self>) {
        if let Some(form) = &mut self.form {
            form.mode = mode;
            form.test = Test::Idle;
            form.note = None;
        }
        cx.notify();
    }

    fn set_https(&mut self, https: bool, cx: &mut Context<Self>) {
        if let Some(form) = &mut self.form {
            form.https = https;
            form.test = Test::Idle;
        }
        cx.notify();
    }

    /// The form as typed, or why it can't be used.
    fn draft(&self, cx: &App) -> Result<Draft, String> {
        let form = self.form.as_ref().ok_or_else(String::new)?;
        let text = |input: &Entity<InputState>| input.read(cx).value().trim().to_owned();
        match form.mode {
            Mode::Automatic => Ok(Draft::Automatic),
            Mode::Service => {
                let (namespace, name) = (text(&form.namespace), text(&form.name));
                if namespace.is_empty() || name.is_empty() {
                    return Err("Enter the Service's namespace and name".into());
                }
                let port = text(&form.port)
                    .parse::<u16>()
                    .ok()
                    .filter(|port| *port > 0)
                    .ok_or_else(|| "Enter the Service's port as a number".to_owned())?;
                Ok(Draft::Service(PrometheusService {
                    https: form.https,
                    ..PrometheusService::new(namespace, name, port)
                        .with_path(&normalise_path(&text(&form.path)))
                }))
            }
            Mode::Url => {
                let url = normalise_url(&text(&form.url))?;
                let typed = Some(text(&form.token)).filter(|token| !token.is_empty());
                Ok(Draft::Url { url, typed })
            }
        }
    }

    /// Checks the form as typed against the cluster or the URL. Nothing
    /// is saved, and the page's own connection is left alone.
    pub(super) fn test_source(&mut self, cx: &mut Context<Self>) {
        let Some(source) = self.source.clone() else {
            return;
        };
        let draft = match self.draft(cx) {
            Ok(draft) => draft,
            Err(why) => return self.form_note(why, cx),
        };
        let access = source.access.clone();
        let work: Work = match draft {
            Draft::Automatic => Box::pin(async move {
                let client = cluster_client(&access).await?;
                let discovery = discover(&client, None)
                    .await
                    .inspect_err(|error| forget_after(&access, error))?;
                match discovery {
                    Discovery::Found {
                        prometheus, build, ..
                    } => Ok(answered(&prometheus, &build)),
                    Discovery::Missing { .. } => Err(QueryError::new(
                        ErrorKind::NotFound,
                        format!("Nothing answered. Discovery looks for {LOOKED_FOR}."),
                    )),
                }
            }),
            Draft::Service(service) => Box::pin(async move {
                let client = cluster_client(&access).await?;
                let (prometheus, build) = confirm(&client, service)
                    .await
                    .inspect_err(|error| forget_after(&access, error))?;
                Ok(answered(&prometheus, &build))
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
                    let (prometheus, build) = confirm_url(url, token).await?;
                    Ok(answered(&prometheus, &build))
                })
            }
        };
        let request = self.run(work, cx, |this, result, cx| {
            if let Some(form) = &mut this.form {
                form.test = match result {
                    Ok(message) => Test::Passed(message.into()),
                    Err(error) => Test::Failed(error.message.into()),
                };
            }
            cx.notify();
        });
        if let Some(form) = &mut self.form {
            form.test = Test::Testing(request);
            form.note = None;
        }
        cx.notify();
    }

    /// Keeps the form for the context and connects with it.
    pub(super) fn save_source(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(context) = self.context() else {
            return;
        };
        let draft = match self.draft(cx) {
            Ok(draft) => draft,
            Err(why) => return self.form_note(why, cx),
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
        // A token for a URL no longer used is forgotten.
        if let Some(Choice::Url { url: old, .. }) = &previous
            && !matches!(&choice, Some(Choice::Url { url, .. }) if url == old)
        {
            self.tokens.remove(old);
            let account = token_account(old);
            self.secret_step(move |store| store.forget(&account), cx);
        }
        self.keep_choice(context, choice.clone(), cx);
        if let Some(form) = &mut self.form {
            form.test = Test::Idle;
            form.note = Some("Saved. Monitoring reads from it now.".into());
            let token = matches!(choice, Some(Choice::Url { token: true, .. }));
            form.token.update(cx, |input, cx| {
                input.set_value("", window, cx);
                input.set_placeholder(token_placeholder(token), window, cx);
            });
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
        let account = token_account(&url);
        self.secret_step(move |store| store.forget(&account), cx);
        self.keep_choice(context, Some(Choice::Url { url, token: false }), cx);
        if let Some(form) = &mut self.form {
            form.note = Some("Token forgotten.".into());
            form.token.update(cx, |input, cx| {
                input.set_placeholder(token_placeholder(false), window, cx)
            });
        }
        self.reconnect(cx);
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
        self.connection = super::connection::Connection::None;
        cx.emit(MonitoringEvent::History);
        self.connect(cx);
        cx.notify();
    }

    fn form_note(&mut self, note: String, cx: &mut Context<Self>) {
        if let Some(form) = &mut self.form {
            form.note = Some(note.into());
            form.test = Test::Idle;
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
            form.note = None;
        }
    }
}

fn answered(
    prometheus: &freshkube_core::monitoring::Prometheus,
    build: &freshkube_core::monitoring::BuildInfo,
) -> String {
    format!(
        "{} answered at {}",
        describe(prometheus.endpoint().backend(), build),
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

/// The Metrics source section of Settings, for the shell's context.
pub fn source_section(page: &Entity<MonitoringPage>, cx: &App) -> AnyElement {
    let p = palette(cx);
    let this = page.read(cx);
    let heading = div()
        .text_size(dp(12.5))
        .font_weight(FontWeight::SEMIBOLD)
        .child("Metrics source");
    let hint = |text: SharedString| div().text_size(dp(12.)).text_color(p.muted).child(text);
    let section = v_flex()
        .id("monitoring-source-settings")
        .test_support()
        .gap_1p5();
    let (Some(context), Some(form)) = (this.context(), this.form.as_ref()) else {
        return section
            .child(heading)
            .child(hint(
                "Choose a context to set where its metrics come from.".into(),
            ))
            .into_any_element();
    };
    if this
        .source
        .as_ref()
        .is_some_and(|source| source.access.is_example())
    {
        return section
            .child(heading)
            .child(hint("Example data answers in example mode.".into()))
            .into_any_element();
    }
    let weak = page.downgrade();
    let on = |weak: &gpui_kit::WeakEntity<MonitoringPage>| weak.clone();
    let mode = form.mode;
    let saved_token = matches!(
        this.saved.choices.get(&context),
        Some(Choice::Url { token: true, .. })
    );
    let segment = |id: &'static str, label: &'static str, value: Mode| {
        ui::choice(Button::new(id).label(label), mode == value)
    };
    let modes = {
        let weak = on(&weak);
        ButtonGroup::new("monitoring-source-mode")
            .outline()
            .small()
            .child(segment(
                "monitoring-source-automatic",
                "Automatic",
                Mode::Automatic,
            ))
            .child(segment(
                "monitoring-source-service",
                "Service",
                Mode::Service,
            ))
            .child(segment("monitoring-source-url", "URL", Mode::Url))
            .on_click(move |selected: &Vec<usize>, _, cx| {
                let mode = match selected.first() {
                    Some(0) => Mode::Automatic,
                    Some(1) => Mode::Service,
                    Some(_) => Mode::Url,
                    None => return,
                };
                _ = weak.update(cx, |page, cx| page.set_mode(mode, cx));
            })
    };
    let field = |label: &'static str, id: &'static str, input: &Entity<InputState>| {
        v_flex()
            .flex_1()
            .min_w_0()
            .gap(dp(3.))
            .child(div().text_size(dp(11.)).text_color(p.muted).child(label))
            .child(Input::new(input).id(id).small())
    };
    let fields: AnyElement = match mode {
        Mode::Automatic => hint(
            "Finds Prometheus, VictoriaMetrics, Thanos or Mimir in the cluster and reads it through the Kubernetes API's service proxy."
                .into(),
        )
        .into_any_element(),
        Mode::Service => {
            let weak = on(&weak);
            v_flex()
                .gap_2()
                .child(
                    h_flex()
                        .gap_2()
                        .child(field("Namespace", "monitoring-source-namespace", &form.namespace))
                        .child(field("Service", "monitoring-source-name", &form.name)),
                )
                .child(
                    h_flex()
                        .gap_2()
                        .child(div().w(dp(80.)).child(field("Port", "monitoring-source-port", &form.port)))
                        .child(field("Path prefix", "monitoring-source-path", &form.path)),
                )
                .child(
                    Checkbox::new("monitoring-source-https")
                        .small()
                        .label("The port speaks HTTPS")
                        .checked(form.https)
                        .on_click(move |checked: &bool, _, cx| {
                            let checked = *checked;
                            _ = weak.update(cx, |page, cx| page.set_https(checked, cx));
                        }),
                )
                .child(hint(
                    "Read through the Kubernetes API's service proxy with this context's credentials. VictoriaMetrics' vmselect uses the prefix select/0/prometheus; Mimir uses prometheus."
                        .into(),
                ))
                .into_any_element()
        }
        Mode::Url => {
            let weak = on(&weak);
            v_flex()
                .gap_2()
                .child(field("URL", "monitoring-source-address", &form.url))
                .child(
                    h_flex()
                        .gap_2()
                        .items_end()
                        .child(field("Token", "monitoring-source-token", &form.token))
                        .when(saved_token, |row| {
                            row.child(
                                Button::new("monitoring-source-forget-token")
                                    .ghost()
                                    .small()
                                    .label("Forget")
                                    .on_click(move |_, window, cx| {
                                        _ = weak.update(cx, |page, cx| page.forget_token(window, cx));
                                    }),
                            )
                        }),
                )
                .child(hint(
                    if this.secrets.is_some() {
                        format!("Read from this computer, not through the cluster. A token is sent as a bearer token and kept in the {STORE_NAME}.")
                    } else {
                        "Read from this computer, not through the cluster. A token is kept until Freshkube quits.".to_owned()
                    }
                    .into(),
                ))
                .into_any_element()
        }
    };
    let testing = matches!(form.test, Test::Testing(_));
    let (test_weak, save_weak) = (on(&weak), on(&weak));
    let outcome: Option<AnyElement> = match &form.test {
        Test::Idle => form.note.clone().map(|note| hint(note).into_any_element()),
        Test::Testing(_) => Some(hint("Testing…".into()).into_any_element()),
        Test::Passed(message) => Some(
            h_flex()
                .id("monitoring-source-passed")
                .gap(dp(6.))
                .text_size(dp(12.))
                .children(ui::status_glyph(ui::Tone::Good, cx))
                .child(message.clone())
                .into_any_element(),
        ),
        Test::Failed(message) => Some(
            h_flex()
                .id("monitoring-source-failed")
                .gap(dp(6.))
                .items_start()
                .text_size(dp(12.))
                .text_color(p.crit_ink)
                .children(ui::status_glyph(ui::Tone::Crit, cx))
                .child(div().min_w_0().child(message.clone()))
                .into_any_element(),
        ),
    };
    section
        .child(
            h_flex().justify_between().gap_2().child(heading).child(
                div()
                    .min_w_0()
                    .truncate()
                    .font_family(ui::MONO_FONT)
                    .text_size(dp(11.))
                    .text_color(p.muted)
                    .child(context),
            ),
        )
        .child(modes)
        .child(fields)
        .child(
            h_flex()
                .gap_2()
                .child(
                    Button::new("monitoring-source-test")
                        .outline()
                        .small()
                        .label("Test")
                        .disabled(testing)
                        .on_click(move |_, _, cx| {
                            _ = test_weak.update(cx, |page, cx| page.test_source(cx));
                        }),
                )
                .child(
                    Button::new("monitoring-source-save")
                        .primary()
                        .small()
                        .label("Save")
                        .on_click(move |_, window, cx| {
                            _ = save_weak.update(cx, |page, cx| page.save_source(window, cx));
                        }),
                ),
        )
        .children(outcome)
        .into_any_element()
}
