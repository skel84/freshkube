//! The setup's metrics draft. Monitoring retains per-context and folder settings.
use super::{SettingsPage, SettingsSection};
use crate::{
    desktop::Pilot,
    monitoring::page::MonitoringPage,
    setup_file::SetupFile,
    ui::{self, dp},
};
use freshkube_core::{
    cluster_source::ClusterAccess,
    job::spawn_job,
    monitoring::{
        self, Discovery,
        scope::{self, Check, Resolved, Verdict},
    },
    secrets::{STORE_NAME, Secrets},
    setup::{self, Metrics, MetricsService, Mode, Setup},
    workspace::Seen,
};
use freshkube_reads::Loader;
use freshkube_ui::{form, page};
use gpui_kit::component::{
    Disableable, Sizable,
    button::{Button, ButtonGroup},
    checkbox::Checkbox,
    h_flex,
    input::{Input, InputEvent, InputState},
    select::{Select, SelectEvent, SelectState},
};
use gpui_kit::prelude::*;
use gpui_kit::*;
use std::{collections::BTreeMap, future::Future, pin::Pin, sync::Arc, time::Duration};

type CheckWork = Pin<Box<dyn Future<Output = Vec<Check>> + Send>>;
type Checker = Arc<dyn Fn(Resolved, ClusterAccess, Option<String>) -> CheckWork + Send + Sync>;

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Field {
    Entry,
    Namespace,
    Name,
    Port,
    Url,
    Label,
    Token,
}

#[derive(Clone, PartialEq, Eq)]
struct Values {
    mode: Mode,
    by_url: bool,
    entry: Option<String>,
    namespace: String,
    name: String,
    port: String,
    https: bool,
    path: String,
    url: String,
    label: String,
    token: String,
    forget_token: bool,
}

pub(super) struct MonitoringForm {
    pilot: WeakEntity<Pilot>,
    page: Entity<MonitoringPage>,
    file: Entity<SetupFile>,
    baseline: Metrics,
    initial: Values,
    seen: Seen,
    mode: Mode,
    by_url: bool,
    entry: Entity<SelectState<Vec<SharedString>>>,
    missing_entry: Option<String>,
    namespace: Entity<InputState>,
    name: Entity<InputState>,
    port: Entity<InputState>,
    https: bool,
    path: Entity<InputState>,
    url: Entity<InputState>,
    label: Entity<InputState>,
    token: Entity<InputState>,
    replacing_token: bool,
    forget_token: bool,
    dirty: bool,
    entry_open: Option<String>,
    entry_in_workspace: bool,
    errors: BTreeMap<Field, String>,
    error: Option<String>,
    read: Loader<(), (Metrics, Option<String>)>,
    outcome: Option<(ui::Tone, String)>,
    checker: Checker,
    saving: Option<Task<()>>,
    _subscriptions: Vec<Subscription>,
}

impl MonitoringForm {
    fn values(&self, cx: &App) -> Values {
        let text = |state: &Entity<InputState>| state.read(cx).value().to_string();
        Values {
            mode: self.mode,
            by_url: self.by_url,
            entry: self
                .entry
                .read(cx)
                .selected_value()
                .map(ToString::to_string)
                .or_else(|| self.missing_entry.clone()),
            namespace: text(&self.namespace),
            name: text(&self.name),
            port: text(&self.port),
            https: self.https,
            path: text(&self.path),
            url: text(&self.url),
            label: text(&self.label),
            token: text(&self.token),
            forget_token: self.forget_token,
        }
    }

    fn changed(&mut self, cx: &App) {
        self.dirty = self.values(cx) != self.initial;
        self.errors.clear();
        self.error = None;
        self.read.reset();
        self.outcome = None;
    }

    fn reset(
        &mut self,
        metrics: Metrics,
        seen: Seen,
        entries: Vec<SharedString>,
        window: &mut Window,
        cx: &mut App,
    ) {
        self.mode = metrics.mode;
        let store = metrics.store.as_ref();
        self.by_url = store.is_some_and(|store| store.url.is_some());
        let service = store.and_then(|store| store.service.as_ref());
        self.https = service.is_some_and(|service| service.https);
        let entry = store.and_then(|store| store.entry.clone());
        self.missing_entry = entry
            .as_ref()
            .filter(|id| !entries.iter().any(|e| e.as_ref() == id.as_str()))
            .cloned();
        self.entry.update(cx, |state, cx| {
            state.set_items(entries, window, cx);
            if let Some(entry) = entry {
                state.set_selected_value(&SharedString::from(entry), window, cx);
            } else {
                state.set_selected_index(None, window, cx);
            }
        });
        let mut set = |state: &Entity<InputState>, value: String, cx: &mut App| {
            state.update(cx, |state, cx| state.set_value(value, window, cx));
        };
        set(
            &self.namespace,
            service.map(|s| s.namespace.clone()).unwrap_or_default(),
            cx,
        );
        set(
            &self.name,
            service.map(|s| s.name.clone()).unwrap_or_default(),
            cx,
        );
        set(
            &self.port,
            service.map(|s| s.port.to_string()).unwrap_or_default(),
            cx,
        );
        set(
            &self.path,
            service.map(|s| s.path.clone()).unwrap_or_default(),
            cx,
        );
        set(
            &self.url,
            store.and_then(|s| s.url.clone()).unwrap_or_default(),
            cx,
        );
        set(&self.label, metrics.label_name().to_owned(), cx);
        set(&self.token, String::new(), cx);
        self.replacing_token = false;
        self.forget_token = false;
        self.baseline = metrics;
        self.seen = seen;
        self.initial = self.values(cx);
        self.dirty = false;
        self.errors.clear();
        self.error = None;
        self.read.reset();
        self.outcome = None;
    }

    fn metrics(
        &self,
        workspace: &freshkube_core::workspace::Workspace,
        cx: &App,
    ) -> Result<Metrics, (Field, String)> {
        let v = self.values(cx);
        let mut next = self.baseline.clone();
        next.mode = v.mode;
        if v.mode != Mode::Central {
            return Ok(next);
        }
        let mut store = next.store.clone().unwrap_or_default();
        if v.by_url {
            let url = monitoring::normalise_url(v.url.trim()).map_err(|why| (Field::Url, why))?;
            let kept =
                store.url.as_ref() == Some(&url) && store.token.is_some() && !self.forget_token;
            store.entry = None;
            store.service = None;
            store.url = Some(url);
            store.token = (kept || !v.token.trim().is_empty()).then_some(setup::KeysJson);
            if !v.token.trim().is_empty() && self.page.read(cx).settings_secrets().is_none() {
                return Err((
                    Field::Token,
                    "This window has nowhere to save a token.".into(),
                ));
            }
        } else {
            let entry = v.entry.filter(|e| !e.trim().is_empty()).ok_or((
                Field::Entry,
                "Choose the entry that hosts the store.".into(),
            ))?;
            if !workspace.clusters.iter().any(|e| e.id == entry) {
                return Err((
                    Field::Entry,
                    "Choose an entry that is in the workspace.".into(),
                ));
            }
            if v.namespace.trim().is_empty() {
                return Err((Field::Namespace, "Enter the Service’s namespace.".into()));
            }
            if v.name.trim().is_empty() {
                return Err((Field::Name, "Enter the Service’s name.".into()));
            }
            let port = v
                .port
                .trim()
                .parse::<u16>()
                .ok()
                .filter(|port| *port > 0)
                .ok_or((
                    Field::Port,
                    "Enter the Service’s port as a number from 1 to 65535.".into(),
                ))?;
            let mut service = store
                .service
                .clone()
                .unwrap_or_else(|| MetricsService::new("", "", port));
            service.namespace = v.namespace.trim().into();
            service.name = v.name.trim().into();
            service.port = port;
            service.https = v.https;
            service.path = monitoring::normalise_path(v.path.trim());
            store.entry = Some(entry);
            store.service = Some(service);
            store.url = None;
            store.token = None;
        }
        next.store = Some(store);
        next.label = if v.label.trim() == setup::DEFAULT_LABEL && self.baseline.label.is_none() {
            None
        } else {
            Some(v.label.trim().into())
        };
        let mut check = Setup::new(workspace.clone());
        check.metrics = next.clone();
        check
            .validate()
            .map_err(|why| (Field::Label, why.to_string()))?;
        Ok(next)
    }

    fn input(
        &self,
        id: &'static str,
        state: &Entity<InputState>,
        field: Option<Field>,
        cx: &App,
    ) -> Input {
        let input = Input::new(state)
            .id(id)
            .small()
            .h(dp(ui::CONTROL_HEIGHT))
            .disabled(self.saving.is_some());
        if field.is_some_and(|field| self.errors.contains_key(&field)) {
            form::invalid(input, cx)
        } else {
            input
        }
    }

    fn error(&self, field: Field) -> Option<String> {
        self.errors.get(&field).cloned()
    }
}

impl SettingsPage {
    pub(super) fn prepare_monitoring(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.monitoring.is_some() || self.section != SettingsSection::Monitoring {
            return;
        }
        let Some(root) = window.root::<gpui_kit::component::Root>().flatten() else {
            return;
        };
        let Ok(pilot) = root.read(cx).view().clone().downcast::<Pilot>() else {
            return;
        };
        let example = self.origin == super::Origin::Example;
        let file = pilot.read(cx).setup_file.clone();
        let page = pilot.read(cx).pages.monitoring.clone();
        page.update(cx, |page, cx| page.source_form(window, cx));
        let metrics = if example {
            example_setup().metrics
        } else {
            file.read(cx)
                .setup
                .as_ref()
                .map(|s| s.metrics.clone())
                .unwrap_or_default()
        };
        let input = |placeholder: &'static str, window: &mut Window, cx: &mut Context<Self>| {
            cx.new(|cx| InputState::new(window, cx).placeholder(placeholder))
        };
        let namespace = input("monitoring", window, cx);
        let name = input("prometheus-operated", window, cx);
        let port = input("9090", window, cx);
        let path = input("Optional path prefix", window, cx);
        let url = input("https://metrics.example.test", window, cx);
        let label = input("cluster", window, cx);
        let token = cx.new(|cx| InputState::new(window, cx).masked(true));
        let entry = cx.new(|cx| {
            SelectState::new(Vec::<SharedString>::new(), None, window, cx).searchable(true)
        });
        let mut draft = MonitoringForm {
            pilot: pilot.downgrade(),
            page: page.clone(),
            file: file.clone(),
            baseline: metrics.clone(),
            initial: Values {
                mode: Mode::Automatic,
                by_url: false,
                entry: None,
                namespace: String::new(),
                name: String::new(),
                port: String::new(),
                https: false,
                path: String::new(),
                url: String::new(),
                label: String::new(),
                token: String::new(),
                forget_token: false,
            },
            seen: file.read(cx).seen.clone(),
            mode: metrics.mode,
            by_url: false,
            entry: entry.clone(),
            missing_entry: None,
            namespace: namespace.clone(),
            name: name.clone(),
            port: port.clone(),
            path: path.clone(),
            url: url.clone(),
            label: label.clone(),
            token: token.clone(),
            https: false,
            replacing_token: false,
            forget_token: false,
            dirty: false,
            entry_open: pilot.read(cx).source().entry,
            entry_in_workspace: pilot
                .read(cx)
                .source()
                .entry
                .as_ref()
                .is_some_and(|id| self.workspace.clusters.iter().any(|entry| &entry.id == id)),
            errors: BTreeMap::new(),
            error: None,
            read: Loader::default(),
            outcome: None,
            checker: Arc::new(|resolved, access, token| {
                Box::pin(check_source(resolved, access, token))
            }),
            saving: None,
            _subscriptions: Vec::new(),
        };
        draft.reset(
            metrics,
            file.read(cx).seen.clone(),
            self.metric_entries(),
            window,
            cx,
        );
        for (state, field) in [
            (&namespace, Some(Field::Namespace)),
            (&name, Some(Field::Name)),
            (&port, Some(Field::Port)),
            (&path, None),
            (&url, Some(Field::Url)),
            (&label, Some(Field::Label)),
            (&token, Some(Field::Token)),
        ] {
            draft._subscriptions.push(cx.subscribe_in(
                state,
                window,
                move |page, _, event, window, cx| match event {
                    InputEvent::Change => {
                        if let Some(draft) = &mut page.monitoring {
                            draft.changed(cx);
                        }
                        cx.notify();
                    }
                    InputEvent::PressEnter { .. } => page.save_monitoring(window, cx),
                    InputEvent::Blur => {
                        page.validate_monitoring(false, field, window, cx);
                    }
                    _ => {}
                },
            ));
        }
        draft._subscriptions.push(cx.subscribe(
            &entry,
            |page, _, _: &SelectEvent<Vec<SharedString>>, cx| {
                if let Some(draft) = &mut page.monitoring {
                    draft.missing_entry = None;
                    draft.changed(cx);
                }
                cx.notify();
            },
        ));
        let source = pilot.read(cx).source();
        let mut identity = (source.kube_identity, source.entry);
        draft._subscriptions.push(
            cx.observe_in(&pilot, window, move |page, pilot, window, cx| {
                let source = pilot.read(cx).source();
                let next = (source.kube_identity, source.entry);
                if next == identity {
                    return;
                }
                identity = next;
                if let Some(draft) = &mut page.monitoring {
                    draft.entry_open = identity.1.clone();
                    draft.entry_in_workspace = draft.entry_open.as_ref().is_some_and(|id| {
                        page.workspace.clusters.iter().any(|entry| &entry.id == id)
                    });
                    draft.read.reset();
                    draft.outcome = None;
                    draft
                        .page
                        .update(cx, |page, cx| page.source_form(window, cx));
                }
                cx.notify();
            }),
        );
        draft
            ._subscriptions
            .push(cx.observe(&page, |_, _, cx| cx.notify()));
        draft
            ._subscriptions
            .push(cx.subscribe_in(&file, window, |page, file, _, window, cx| {
                if page.origin == super::Origin::Example {
                    return;
                }
                let Some(draft) = &mut page.monitoring else {
                    return;
                };
                let model = file.read(cx);
                if let Some(setup) = &model.setup {
                    draft.entry_in_workspace = draft.entry_open.as_ref().is_some_and(|id| {
                        setup.workspace.clusters.iter().any(|entry| &entry.id == id)
                    });
                }
                if draft.dirty && model.seen != draft.seen && draft.saving.is_none() {
                    draft.error = Some(crate::setup_file::SaveError::Changed.to_string());
                }
                if !draft.dirty
                    && draft.saving.is_none()
                    && let Some(setup) = &model.setup
                {
                    let entries = setup
                        .workspace
                        .clusters
                        .iter()
                        .map(|entry| entry.id.clone().into())
                        .collect();
                    draft.reset(
                        setup.metrics.clone(),
                        model.seen.clone(),
                        entries,
                        window,
                        cx,
                    );
                }
                cx.notify();
            }));
        self.monitoring = Some(draft);
        cx.notify();
    }

    fn metric_entries(&self) -> Vec<SharedString> {
        let workspace = if self.origin == super::Origin::Example {
            example_setup().workspace
        } else {
            self.workspace.clone()
        };
        workspace
            .clusters
            .iter()
            .map(|entry| entry.id.clone().into())
            .collect()
    }

    fn validate_monitoring(
        &mut self,
        focus: bool,
        field: Option<Field>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<Metrics> {
        let draft = self.monitoring.as_mut()?;
        match draft.metrics(&self.workspace, cx) {
            Ok(metrics) => Some(metrics),
            Err((failed, message)) => {
                if focus || field == Some(failed) {
                    draft.errors.insert(failed, message);
                }
                if focus {
                    let handle = match failed {
                        Field::Entry => draft.entry.read(cx).focus_handle(cx),
                        Field::Namespace => draft.namespace.read(cx).focus_handle(cx),
                        Field::Name => draft.name.read(cx).focus_handle(cx),
                        Field::Port => draft.port.read(cx).focus_handle(cx),
                        Field::Url => draft.url.read(cx).focus_handle(cx),
                        Field::Label => draft.label.read(cx).focus_handle(cx),
                        Field::Token => draft.token.read(cx).focus_handle(cx),
                    };
                    window.focus(&handle, cx);
                }
                cx.notify();
                None
            }
        }
    }

    fn revert_monitoring(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let entries = self.metric_entries();
        if let Some(draft) = &mut self.monitoring {
            if draft.saving.is_some() {
                return;
            }
            let model = draft.file.read(cx);
            let metrics = if self.origin == super::Origin::Example {
                example_setup().metrics
            } else {
                model
                    .setup
                    .as_ref()
                    .map(|s| s.metrics.clone())
                    .unwrap_or_default()
            };
            draft.reset(metrics, model.seen.clone(), entries, window, cx);
            let page = draft.page.clone();
            page.update(cx, |page, cx| page.source_form(window, cx));
            cx.notify();
        }
    }

    fn save_monitoring(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.origin == super::Origin::Example
            || self
                .monitoring
                .as_ref()
                .is_none_or(|d| !d.dirty || d.saving.is_some())
        {
            return;
        }
        if !self.editable() {
            let why = self.why_not_editable().to_owned();
            self.monitoring.as_mut().unwrap().error = Some(why);
            cx.notify();
            return;
        }
        let Some(metrics) = self.validate_monitoring(true, None, window, cx) else {
            return;
        };
        let entries = self.metric_entries();
        let draft = self.monitoring.as_mut().unwrap();
        draft.error = None;
        let file = draft.file.clone();
        let expected = draft.seen.clone();
        let typed = if metrics.mode == Mode::Central && draft.by_url {
            draft.token.read(cx).value().trim().to_owned()
        } else {
            String::new()
        };
        let secrets = draft.page.read(cx).settings_secrets();
        let url = metrics.store.as_ref().and_then(|s| s.url.clone());
        let forget = metrics.mode == Mode::Central && draft.by_url && draft.forget_token;
        let saved_metrics = metrics.clone();
        draft.saving = Some(cx.spawn_in(window, async move |this, cx| {
            let staged = cx
                .background_spawn(async move { stage_token(secrets, url, typed, forget) })
                .await;
            let mut result = match &staged {
                Ok(_) => Ok(()),
                Err(why) => Err(why.clone()),
            };
            if result.is_ok() {
                let save = file.update(cx, |file, cx| {
                    file.save_with(expected, move |setup| setup.metrics = saved_metrics, cx)
                });
                result = save.await.map_err(|why| why.to_string());
            }
            if result.is_err()
                && let Ok(Some(staged)) = staged
            {
                let rollback = cx
                    .background_spawn(async move {
                        match staged.previous {
                            Some(previous) => staged.secrets.write(&staged.account, &previous),
                            None => staged.secrets.forget(&staged.account),
                        }
                    })
                    .await;
                if rollback.is_err() {
                    result = Err(
                        "The setup wasn’t saved, and its previous token couldn’t be restored."
                            .into(),
                    );
                }
            }
            _ = this.update_in(cx, |page, window, cx| {
                let Some(draft) = &mut page.monitoring else {
                    return;
                };
                draft.saving = None;
                match result {
                    Ok(()) => {
                        if let Some(url) =
                            metrics.store.as_ref().and_then(|store| store.url.as_ref())
                        {
                            draft
                                .page
                                .update(cx, |page, _| page.settings_token_changed(url));
                        }
                        draft.reset(metrics, file.read(cx).seen.clone(), entries, window, cx);
                    }
                    Err(why) => draft.error = Some(why),
                }
                cx.notify();
            });
        }));
        cx.notify();
    }

    fn test_monitoring(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.origin == super::Origin::Example {
            return;
        }
        let Some(metrics) = self.validate_monitoring(true, None, window, cx) else {
            return;
        };
        let draft = self.monitoring.as_mut().unwrap();
        let Some(pilot) = draft.pilot.upgrade() else {
            return;
        };
        let pilot = pilot.read(cx);
        let source = pilot.source();
        let Some(entry) = source.entry.clone() else {
            draft.outcome = Some((
                ui::Tone::Unknown,
                "Unknown · Open a workspace entry to test this setup.".into(),
            ));
            cx.notify();
            return;
        };
        let Some(open) = draft.page.read(cx).open_access() else {
            draft.outcome = Some((
                ui::Tone::Unknown,
                "Unknown · This entry has no Kubernetes connection.".into(),
            ));
            cx.notify();
            return;
        };
        let mut setup = draft
            .file
            .read(cx)
            .setup
            .clone()
            .unwrap_or_else(|| Setup::new(self.workspace.clone()));
        setup.metrics = metrics.clone();
        if !setup
            .workspace
            .clusters
            .iter()
            .any(|defined| defined.id == entry)
        {
            draft.outcome = Some((
                ui::Tone::Unknown,
                "Unknown · The open context isn’t a workspace entry.".into(),
            ));
            cx.notify();
            return;
        }
        let resolved = draft
            .page
            .read(cx)
            .settings_resolution(scope::resolve(&setup, Some(&entry)));
        let access = match resolved.host() {
            Some(host) if host != entry => crate::desktop::entity_pages::hosted_by(
                &setup.workspace,
                host,
                &mut BTreeMap::new(),
            )
            .map(ClusterAccess::Direct),
            _ => Some(open),
        };
        let Some(access) = access else {
            return;
        };
        let typed = draft.token.read(cx).value().trim().to_owned();
        let secrets = draft.page.read(cx).settings_secrets();
        let token_url = match &resolved {
            Resolved::Central {
                place: scope::Place::Url { url, token },
                ..
            }
            | Resolved::Own(scope::Place::Url { url, token }) => Some((url.clone(), *token)),
            _ => None,
        };
        let typed = if token_url.as_ref().is_some_and(|(url, _)| {
            setup
                .metrics
                .store
                .as_ref()
                .and_then(|store| store.url.as_ref())
                == Some(url)
        }) {
            typed
        } else {
            String::new()
        };
        let checker = draft.checker.clone();
        let request = draft.read.start((metrics, source.kube_identity));
        let (job, receiver) = spawn_job(
            &pilot.runtime,
            Duration::from_secs(45),
            "The metrics check didn’t answer in time.".to_owned(),
            async move {
                let token = read_token(secrets, token_url, typed).await?;
                Ok(checker(resolved, access, token).await)
            },
        );
        let task = cx.spawn(async move |this, cx| {
            let Ok(result) = receiver.await else {
                return;
            };
            _ = this.update(cx, |page, cx| {
                let Some(draft) = &mut page.monitoring else {
                    return;
                };
                let complete = result.as_ref().map(|_| ()).map_err(Clone::clone);
                if draft.read.land(&request, complete, cx) {
                    draft.outcome = Some(match result {
                        Ok(checks) => describe_checks(&checks),
                        Err(why) => (ui::Tone::Unknown, format!("Unknown · {why}")),
                    });
                    cx.notify();
                }
            });
        });
        draft.read.run(Some(job), task);
        draft.outcome = None;
        cx.notify();
    }

    pub(super) fn sync_monitoring_visibility(&mut self, cx: &mut Context<Self>) {
        if (!self.visible || self.section != SettingsSection::Monitoring)
            && let Some(draft) = &mut self.monitoring
        {
            draft.read.reset();
            draft.page.update(cx, |page, _| page.pause_settings());
        }
    }

    fn monitoring_edit(&mut self, edit: impl FnOnce(&mut MonitoringForm), cx: &mut Context<Self>) {
        if let Some(draft) = &mut self.monitoring {
            edit(draft);
            draft.changed(cx);
            cx.notify();
        }
    }

    pub(super) fn render_monitoring(
        &self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let narrow = page::content_width(window) < form::NARROW;
        let mut content = div()
            .id("settings-monitoring-form")
            .test_support()
            .w_full()
            .flex()
            .flex_col()
            .gap(dp(20.))
            .flex_none();
        if let Some(draft) = &self.monitoring {
            content = content.child(self.render_metrics(draft, narrow, cx));
            let in_workspace = draft.entry_in_workspace;
            content = content.child(if in_workspace || self.origin == super::Origin::Example {
                form::group("monitoring-this-context", "This context")
                    .narrow(narrow)
                    .row(form::row(
                        "monitoring-context-decided",
                        "Source",
                        "The setup file decides this entry’s metrics source.",
                    ))
                    .saved_in(form::SavedIn::file("freshkube.yaml"))
                    .render(cx)
                    .into_any_element()
            } else {
                crate::monitoring::page::source_section(&draft.page, narrow, cx)
            });
            content = content.child(crate::monitoring::page::settings_section(
                &draft.page,
                narrow,
                cx,
            ));
        }
        page::padded("settings-page")
            .track_focus(&self.focus)
            .capture_key_down(cx.listener(|page, event: &KeyDownEvent, window, cx| {
                if event.keystroke.key == "escape"
                    && page.monitoring.as_ref().is_some_and(|d| {
                        d.values(cx) != d.initial
                            && !d.page.read(cx).settings_source_focused(window, cx)
                    })
                {
                    page.revert_monitoring(window, cx);
                    cx.stop_propagation();
                }
            }))
            .overflow_y_scroll()
            .restrict_scroll_to_axis()
            .child(page::PageHeader::new("settings-monitoring", "Monitoring").render(window, cx))
            .children(self.banner.as_ref().map(|why| {
                ui::warning_banner(Some("Freshkube setup".into()), why.clone(), None, cx)
            }))
            .children(
                self.warning
                    .as_ref()
                    .map(|why| ui::warning_banner(None, why.clone(), None, cx)),
            )
            .children(
                self.monitoring
                    .as_ref()
                    .and_then(|draft| draft.error.as_ref())
                    .map(|why| {
                        ui::banner(
                            ui::Tone::Crit,
                            Some("Not saved".into()),
                            why.clone(),
                            None,
                            cx,
                        )
                    }),
            )
            .child(content)
            .into_any_element()
    }

    fn render_central_fields(
        &self,
        mut group: form::Group,
        draft: &MonitoringForm,
        cx: &Context<Self>,
    ) -> form::Group {
        let saving = draft.saving.is_some();
        let stores = ButtonGroup::new("metrics-store-mode")
            .outline()
            .small()
            .child(ui::choice(
                Button::new("metrics-store-service")
                    .label("Service")
                    .disabled(saving),
                !draft.by_url,
            ))
            .child(ui::choice(
                Button::new("metrics-store-url")
                    .label("URL")
                    .disabled(saving),
                draft.by_url,
            ))
            .on_click(cx.listener(|page, selected: &Vec<usize>, _, cx| {
                page.monitoring_edit(|d| d.by_url = selected.first() == Some(&1), cx)
            }));
        group = group.row(form::row("metrics-store-row", "Store", stores));
        group = if draft.by_url {
            self.render_metrics_url(group, draft, cx)
        } else {
            self.render_metrics_service(group, draft, cx)
        };
        group = group.row(
            form::row(
                "metrics-label-row",
                "Cluster label",
                draft.input("metrics-label", &draft.label, Some(Field::Label), cx),
            )
            .error(draft.error(Field::Label)),
        );
        group
    }

    fn render_metrics_url(
        &self,
        mut group: form::Group,
        draft: &MonitoringForm,
        cx: &Context<Self>,
    ) -> form::Group {
        let kept = draft
            .baseline
            .store
            .as_ref()
            .is_some_and(|store| store.token.is_some())
            && !draft.replacing_token
            && !draft.forget_token;
        let secret = if kept {
            form::Secret::Kept
        } else {
            form::Secret::Typing(draft.token.clone())
        };
        group = group
            .row(
                form::row(
                    "metrics-url-row",
                    "URL",
                    draft.input("metrics-url", &draft.url, Some(Field::Url), cx),
                )
                .error(draft.error(Field::Url)),
            )
            .row(
                form::row(
                    "metrics-token-row",
                    "Token",
                    form::secret("metrics-token", secret, cx)
                        .store_name(STORE_NAME)
                        .on_replace(cx.listener(|page, _, _, cx| {
                            page.monitoring_edit(|d| d.replacing_token = true, cx)
                        }))
                        .on_forget(cx.listener(|page, _, _, cx| {
                            page.monitoring_edit(
                                |d| {
                                    d.forget_token = true;
                                    d.replacing_token = true;
                                },
                                cx,
                            )
                        })),
                )
                .hint("A bearer token; leave empty when the store needs none.")
                .error(draft.error(Field::Token)),
            );
        group
    }

    fn render_metrics_service(
        &self,
        mut group: form::Group,
        draft: &MonitoringForm,
        cx: &Context<Self>,
    ) -> form::Group {
        let saving = draft.saving.is_some();
        group = group
            .row(
                form::row(
                    "metrics-entry-row",
                    "Hosting entry",
                    Select::new(&draft.entry)
                        .id("metrics-entry")
                        .when(draft.errors.contains_key(&Field::Entry), |select| {
                            select.border_color(crate::palette::palette(cx).crit)
                        })
                        .small()
                        .disabled(saving)
                        .placeholder(
                            draft
                                .missing_entry
                                .clone()
                                .unwrap_or_else(|| "Choose an entry".into()),
                        ),
                )
                .error(draft.error(Field::Entry)),
            )
            .row(
                form::row(
                    "metrics-namespace-row",
                    "Namespace",
                    draft.input(
                        "metrics-namespace",
                        &draft.namespace,
                        Some(Field::Namespace),
                        cx,
                    ),
                )
                .error(draft.error(Field::Namespace)),
            )
            .row(
                form::row(
                    "metrics-name-row",
                    "Service",
                    draft.input("metrics-name", &draft.name, Some(Field::Name), cx),
                )
                .error(draft.error(Field::Name)),
            )
            .row(
                form::row(
                    "metrics-port-row",
                    "Port",
                    draft.input("metrics-port", &draft.port, Some(Field::Port), cx),
                )
                .error(draft.error(Field::Port)),
            )
            .row(form::row(
                "metrics-https-row",
                "HTTPS",
                Checkbox::new("metrics-https")
                    .small()
                    .checked(draft.https)
                    .disabled(saving)
                    .on_click(cx.listener(|page, checked, _, cx| {
                        page.monitoring_edit(|d| d.https = *checked, cx)
                    })),
            ))
            .row(form::row(
                "metrics-path-row",
                "Path",
                draft.input("metrics-path", &draft.path, None, cx),
            ));
        group
    }

    fn render_metrics(
        &self,
        draft: &MonitoringForm,
        narrow: bool,
        cx: &Context<Self>,
    ) -> AnyElement {
        let example = self.origin == super::Origin::Example;
        let saving = draft.saving.is_some();
        let mut modes = h_flex()
            .id("metrics-mode")
            .flex_wrap()
            .gap(dp(2.))
            .bg(crate::palette::palette(cx).surface_2)
            .rounded(px(8.));
        for (id, label, mode) in [
            ("metrics-automatic", "Automatic", Mode::Automatic),
            ("metrics-central", "Central", Mode::Central),
            ("metrics-per-entry", "Per entry", Mode::PerEntry),
        ] {
            modes = modes.child(
                ui::segment(
                    Button::new(id)
                        .label(label)
                        .h(dp(ui::CONTROL_HEIGHT))
                        .disabled(saving),
                    draft.mode == mode,
                    cx,
                )
                .on_click(
                    cx.listener(move |page, _, _, cx| page.monitoring_edit(|d| d.mode = mode, cx)),
                ),
            );
        }
        let mut group = form::group("monitoring-metrics", "Metrics source")
            .narrow(narrow)
            .dirty(draft.dirty)
            .row(
                form::row(
                    "metrics-mode-row",
                    "Mode",
                    div().flex().min_w_0().child(modes.max_w_full()),
                )
                .hint(if example {
                    "Example data isn’t saved"
                } else if !self.editable() {
                    self.why_not_editable()
                } else {
                    "Choose where workspace entries read their metrics."
                }),
            );
        match draft.mode {
            Mode::Automatic => {}
            Mode::PerEntry => {
                group = group.row(form::row("metrics-per-entry-row", "Entries", div().text_size(dp(12.)).text_color(crate::palette::palette(cx).muted)
                .child("Each entry names its own source under its clusters entry in freshkube.yaml.")));
            }
            Mode::Central => {
                group = self.render_central_fields(group, draft, cx);
            }
        }
        let result = if draft.read.is_loading() {
            "Testing…".to_owned()
        } else {
            draft
                .outcome
                .as_ref()
                .map(|(_, words)| words.clone())
                .unwrap_or_else(|| {
                    "Checks the values above against the open entry without saving them.".into()
                })
        };
        let line = h_flex()
            .gap(dp(8.))
            .items_start()
            .child(
                Button::new("metrics-test")
                    .outline()
                    .small()
                    .label("Test")
                    .h(dp(ui::CONTROL_HEIGHT))
                    .disabled(example || draft.read.is_loading() || saving)
                    .on_click(cx.listener(|page, _, window, cx| page.test_monitoring(window, cx))),
            )
            .child(
                div()
                    .id("metrics-test-result")
                    .test_support()
                    .text_size(dp(12.))
                    .children(
                        draft
                            .outcome
                            .as_ref()
                            .and_then(|(tone, _)| ui::status_glyph(*tone, cx)),
                    )
                    .child(result),
            );
        let saved = if draft
            .baseline
            .store
            .as_ref()
            .is_some_and(|s| s.token.is_some())
            || !draft.token.read(cx).value().is_empty()
        {
            form::SavedIn::file(format!("freshkube.yaml; the token in {STORE_NAME}"))
        } else {
            form::SavedIn::file("freshkube.yaml")
        };
        group
            .row(form::row("metrics-test-row", "Test", line))
            .saved_in(saved)
            .actions(form::actions(
                form::Action::new(
                    "metrics-save",
                    "Save",
                    cx.listener(|page, _, window, cx| page.save_monitoring(window, cx)),
                )
                .disabled(!self.editable() || !draft.dirty || saving),
                form::Action::new(
                    "metrics-revert",
                    "Revert",
                    cx.listener(|page, _, window, cx| page.revert_monitoring(window, cx)),
                )
                .disabled(!draft.dirty || saving),
            ))
            .render(cx)
            .into_any_element()
    }
}

fn example_setup() -> Setup {
    setup::parse(include_bytes!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../freshkube-prefs/src/examples/freshkube.yaml"
    )))
    .expect("the example setup is valid")
}

struct TokenEdit {
    secrets: Secrets,
    account: String,
    previous: Option<String>,
}

/// Stage the token before writing its reference; a refused setup restores it.
fn stage_token(
    secrets: Option<Secrets>,
    url: Option<String>,
    typed: String,
    forget: bool,
) -> Result<Option<TokenEdit>, String> {
    if typed.is_empty() && !forget {
        return Ok(None);
    }
    let Some((secrets, url)) = secrets.zip(url) else {
        return Ok(None);
    };
    let account = format!("Prometheus {url}");
    let previous = secrets.read(&account)?;
    if forget {
        secrets.forget(&account)?;
    } else {
        secrets.write(&account, &typed)?;
    }
    Ok(Some(TokenEdit {
        secrets,
        account,
        previous,
    }))
}

async fn read_token(
    secrets: Option<Secrets>,
    url: Option<(String, bool)>,
    typed: String,
) -> Result<Option<String>, String> {
    if !typed.is_empty() {
        return Ok(Some(typed));
    }
    let Some((url, true)) = url else {
        return Ok(None);
    };
    let Some(secrets) = secrets else {
        return Ok(None);
    };
    tokio::task::spawn_blocking(move || secrets.read(&format!("Prometheus {url}")))
        .await
        .map_err(|_| "The token read stopped.".to_owned())?
}

async fn check_source(
    resolved: Resolved,
    access: ClusterAccess,
    token: Option<String>,
) -> Vec<Check> {
    let verdict = match &resolved {
        Resolved::Central { .. } => return scope::check(&resolved, &access, token).await,
        Resolved::Unknown(why) => Verdict::Unknown(why.clone()),
        Resolved::Own(place) => match scope::connect(place, &access, token).await {
            Ok(_) => Verdict::Pass,
            Err(why) => Verdict::Fail(why.message),
        },
        Resolved::Discovery => match monitoring::cluster_client(&access).await {
            Ok(client) => match monitoring::discover(&client, None).await {
                Ok(Discovery::Found { .. }) => Verdict::Pass,
                Ok(Discovery::Missing { .. }) => Verdict::Unknown(format!(
                    "Nothing answered. Discovery looks for {}.",
                    monitoring::LOOKED_FOR
                )),
                Err(why) => {
                    monitoring::forget_after(&access, &why);
                    Verdict::Fail(why.message)
                }
            },
            Err(why) => Verdict::Fail(why.message),
        },
    };
    vec![Check {
        what: "The source answers",
        verdict,
    }]
}

fn describe_checks(checks: &[Check]) -> (ui::Tone, String) {
    let (tone, state) = if checks.iter().any(|c| matches!(c.verdict, Verdict::Fail(_))) {
        (ui::Tone::Crit, "Failed")
    } else if checks.is_empty()
        || checks
            .iter()
            .any(|c| matches!(c.verdict, Verdict::Unknown(_)))
    {
        (ui::Tone::Unknown, "Unknown")
    } else {
        (ui::Tone::Good, "Passed")
    };
    let why = checks
        .iter()
        .map(|c| match &c.verdict {
            Verdict::Pass => c.what.to_owned(),
            Verdict::Fail(why) | Verdict::Unknown(why) => format!("{}: {why}", c.what),
        })
        .collect::<Vec<_>>()
        .join("; ");
    (tone, format!("{state} · {why}"))
}

#[cfg(test)]
#[path = "monitoring_tests.rs"]
mod tests;
