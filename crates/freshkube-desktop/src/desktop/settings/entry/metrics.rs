//! The Metrics group: the entry's value of the central store's label, and
//! its own store, which wins under any mode (docs/SETUP_FILE.md, Metrics).
use super::{Field, Group, SettingsPage, saved_in};
use crate::ui::{self, dp};
use freshkube_core::setup::{EntryMetrics, MetricsService, Source};
use freshkube_core::workspace::Entry;
use freshkube_ui::form;
use gpui_kit::component::{
    Disableable, Sizable,
    button::{Button, ButtonGroup},
    input::{Input, InputState},
};
use gpui_kit::prelude::*;
use gpui_kit::*;

/// Where the entry reads its metrics from, if it names a store of its own.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::desktop::settings) enum Kind {
    /// No store of its own: the mode decides.
    Mode,
    Service,
    Url,
}

impl Kind {
    const ALL: [Kind; 3] = [Kind::Mode, Kind::Service, Kind::Url];

    fn label(self) -> &'static str {
        match self {
            Kind::Mode => "None",
            Kind::Service => "Service",
            Kind::Url => "URL",
        }
    }

    fn id(self) -> &'static str {
        match self {
            Kind::Mode => "settings-metrics-source-none",
            Kind::Service => "settings-metrics-source-service",
            Kind::Url => "settings-metrics-source-url",
        }
    }
}

/// What the group's fields hold, as text. Fields of the kind not chosen keep
/// what they held, so choosing back loses nothing typed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Values {
    pub(super) value: String,
    pub(super) kind: Kind,
    pub(super) namespace: String,
    pub(super) name: String,
    pub(super) port: String,
    pub(super) url: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(super) struct Errors {
    pub(super) namespace: Option<SharedString>,
    pub(super) name: Option<SharedString>,
    pub(super) port: Option<SharedString>,
    pub(super) url: Option<SharedString>,
}

impl Errors {
    pub(super) fn first(&self) -> Option<Field> {
        if self.namespace.is_some() {
            Some(Field::Namespace)
        } else if self.name.is_some() {
            Some(Field::Service)
        } else if self.port.is_some() {
            Some(Field::Port)
        } else if self.url.is_some() {
            Some(Field::Address)
        } else {
            None
        }
    }

    pub(super) fn set(&mut self, field: Field, error: Option<SharedString>) {
        match field {
            Field::Namespace => self.namespace = error,
            Field::Service => self.name = error,
            Field::Port => self.port = error,
            Field::Address => self.url = error,
            _ => {}
        }
    }

    pub(super) fn of(&self, field: Field) -> Option<SharedString> {
        match field {
            Field::Namespace => self.namespace.clone(),
            Field::Service => self.name.clone(),
            Field::Port => self.port.clone(),
            Field::Address => self.url.clone(),
            _ => None,
        }
    }
}

/// A port, 1 to 65535.
fn port(text: &str) -> Option<u16> {
    text.trim().parse::<u16>().ok().filter(|port| *port > 0)
}

impl Values {
    pub(super) fn of(entry: &Entry) -> Self {
        let metrics = entry.metrics.as_deref();
        let source = metrics.and_then(|metrics| metrics.source.as_ref());
        let service = source.and_then(|source| source.service.as_ref());
        Self {
            value: metrics
                .and_then(|metrics| metrics.value.clone())
                .unwrap_or_default(),
            kind: match source {
                Some(Source {
                    service: Some(_), ..
                }) => Kind::Service,
                Some(Source { url: Some(_), .. }) => Kind::Url,
                _ => Kind::Mode,
            },
            namespace: service
                .map(|service| service.namespace.clone())
                .unwrap_or_default(),
            name: service
                .map(|service| service.name.clone())
                .unwrap_or_default(),
            port: service
                .map(|service| service.port.to_string())
                .unwrap_or_default(),
            url: source
                .and_then(|source| source.url.clone())
                .unwrap_or_default(),
        }
    }

    /// The values as a save would write them: trimmed, a port without its
    /// leading zeros, and only the fields of the store chosen. Text a save
    /// changes this way doesn't keep the group dirty.
    pub(super) fn normalized(&self) -> Self {
        let mut values = Self {
            value: self.value.trim().to_owned(),
            kind: self.kind,
            namespace: String::new(),
            name: String::new(),
            port: String::new(),
            url: String::new(),
        };
        match self.kind {
            Kind::Mode => {}
            Kind::Service => {
                values.namespace = self.namespace.trim().to_owned();
                values.name = self.name.trim().to_owned();
                values.port = port(&self.port)
                    .map(|port| port.to_string())
                    .unwrap_or_else(|| self.port.trim().to_owned());
            }
            Kind::Url => values.url = self.url.trim().to_owned(),
        }
        values
    }

    /// The errors of the fields of the store chosen; the others aren't
    /// written, so they can't fail.
    pub(super) fn check(&self) -> Errors {
        let mut errors = Errors::default();
        match self.kind {
            Kind::Mode => {}
            Kind::Service => {
                if self.namespace.trim().is_empty() {
                    errors.namespace = Some("Enter the Service’s namespace".into());
                }
                if self.name.trim().is_empty() {
                    errors.name = Some("Enter the Service’s name".into());
                }
                if port(&self.port).is_none() {
                    errors.port = Some("Enter a port from 1 to 65535".into());
                }
            }
            Kind::Url => {
                let url = self.url.trim();
                let host = url
                    .strip_prefix("https://")
                    .or_else(|| url.strip_prefix("http://"));
                if host.is_none_or(|host| host.is_empty() || host.contains(char::is_whitespace)) {
                    errors.url = Some("Enter an http:// or https:// address".into());
                }
            }
        }
        errors
    }

    /// Writes the group's keys into `entry`, keeping the keys of its
    /// metrics block this version doesn't know and a URL's token.
    pub(super) fn apply(&self, entry: &mut Entry) {
        let mut metrics = entry.metrics.as_deref().cloned().unwrap_or_default();
        let value = self.value.trim();
        metrics.value = (!value.is_empty()).then(|| value.to_owned());
        let mut source = metrics.source.take().unwrap_or_default();
        metrics.source = match self.kind {
            Kind::Mode => None,
            Kind::Service => {
                let mut service = source
                    .service
                    .take()
                    .unwrap_or_else(|| MetricsService::new("", "", 0));
                service.namespace = self.namespace.trim().to_owned();
                service.name = self.name.trim().to_owned();
                service.port = port(&self.port).unwrap_or_default();
                source.service = Some(service);
                source.url = None;
                // A Service needs no token.
                source.token = None;
                Some(source)
            }
            Kind::Url => {
                source.service = None;
                source.url = Some(self.url.trim().to_owned());
                Some(source)
            }
        };
        entry.metrics = (metrics != EntryMetrics::default()).then(|| Box::new(metrics));
    }
}

pub(super) struct Metrics {
    pub(super) kind: Kind,
    pub(super) value: Entity<InputState>,
    pub(super) namespace: Entity<InputState>,
    pub(super) name: Entity<InputState>,
    pub(super) port: Entity<InputState>,
    pub(super) url: Entity<InputState>,
    pub(super) baseline: Values,
    pub(super) errors: Errors,
    /// Whether the entry's own store has a token, which is kept and never
    /// shown.
    pub(super) has_token: bool,
}

impl Metrics {
    pub(super) fn new(entry: &Entry, window: &mut Window, cx: &mut Context<SettingsPage>) -> Self {
        let id = entry.id.clone();
        let value = cx.new(|cx| InputState::new(window, cx).placeholder(id));
        let namespace = cx.new(|cx| InputState::new(window, cx).placeholder("monitoring"));
        let name = cx.new(|cx| InputState::new(window, cx).placeholder("prometheus"));
        let port = cx.new(|cx| InputState::new(window, cx).placeholder("9090"));
        let url = cx.new(|cx| {
            InputState::new(window, cx).placeholder("https://metrics.example.test/prometheus")
        });
        let mut this = Self {
            kind: Kind::Mode,
            value,
            namespace,
            name,
            port,
            url,
            baseline: Values::of(entry),
            errors: Errors::default(),
            has_token: has_token(entry),
        };
        let values = this.baseline.clone();
        this.load(&values, window, cx);
        this
    }

    pub(super) fn input(&self, field: Field) -> Option<&Entity<InputState>> {
        match field {
            Field::Label => Some(&self.value),
            Field::Namespace => Some(&self.namespace),
            Field::Service => Some(&self.name),
            Field::Port => Some(&self.port),
            Field::Address => Some(&self.url),
            _ => None,
        }
    }

    pub(super) fn values(&self, cx: &App) -> Values {
        let text = |input: &Entity<InputState>| input.read(cx).value().to_string();
        Values {
            value: text(&self.value),
            kind: self.kind,
            namespace: text(&self.namespace),
            name: text(&self.name),
            port: text(&self.port),
            url: text(&self.url),
        }
    }

    pub(super) fn dirty(&self, cx: &App) -> bool {
        self.values(cx).normalized() != self.baseline.normalized()
    }

    pub(super) fn load(&mut self, values: &Values, window: &mut Window, cx: &mut App) {
        self.kind = values.kind;
        self.errors = Errors::default();
        for (input, text) in [
            (&self.value, &values.value),
            (&self.namespace, &values.namespace),
            (&self.name, &values.name),
            (&self.port, &values.port),
            (&self.url, &values.url),
        ] {
            input.update(cx, |input, cx| input.set_value(text.clone(), window, cx));
        }
    }

    pub(super) fn render(
        &self,
        editable: bool,
        dirty: bool,
        cx: &Context<SettingsPage>,
    ) -> AnyElement {
        let field =
            |input: &Entity<InputState>, id: &'static str, label: &'static str, error: bool| {
                let input = Input::new(input)
                    .id(id)
                    .aria_label(label)
                    .small()
                    .map(|input| Styled::h(input, dp(ui::CONTROL_HEIGHT)))
                    .disabled(!editable);
                if error {
                    form::invalid(input, cx)
                } else {
                    input
                }
            };
        let page = cx.weak_entity();
        let kinds = Kind::ALL.iter().fold(
            ButtonGroup::new("settings-metrics-source")
                .outline()
                .small(),
            |group, kind| {
                group.child(ui::choice(
                    Button::new(kind.id())
                        .label(kind.label())
                        .disabled(!editable),
                    self.kind == *kind,
                ))
            },
        );
        let mut group = form::group("settings-metrics", "Metrics")
            .narrow(true)
            .dirty(dirty)
            .row(
                form::row(
                    "settings-metrics-value-row",
                    "Label value",
                    field(&self.value, "settings-metrics-value", "Label value", false),
                )
                .hint(
                    "This cluster’s value of the central store’s label. Unset, the cluster’s id.",
                ),
            )
            .row(
                form::row(
                    "settings-metrics-source-row",
                    "Own store",
                    kinds.on_click(move |selected: &Vec<usize>, _, cx| {
                        let Some(kind) = selected.first().and_then(|ix| Kind::ALL.get(*ix)) else {
                            return;
                        };
                        let kind = *kind;
                        _ = page.update(cx, |page, cx| page.set_entry_store(kind, cx));
                    }),
                )
                .hint("A store of its own wins under any mode."),
            );
        match self.kind {
            Kind::Mode => {}
            Kind::Service => {
                group = group
                    .row(
                        form::row(
                            "settings-metrics-namespace-row",
                            "Namespace",
                            field(
                                &self.namespace,
                                "settings-metrics-namespace",
                                "Service namespace",
                                self.errors.namespace.is_some(),
                            ),
                        )
                        .error(self.errors.namespace.clone()),
                    )
                    .row(
                        form::row(
                            "settings-metrics-service-row",
                            "Service",
                            field(
                                &self.name,
                                "settings-metrics-service",
                                "Service name",
                                self.errors.name.is_some(),
                            ),
                        )
                        .hint("Reached through this cluster’s API server.")
                        .error(self.errors.name.clone()),
                    )
                    .row(
                        form::row(
                            "settings-metrics-port-row",
                            "Port",
                            field(
                                &self.port,
                                "settings-metrics-port",
                                "Service port",
                                self.errors.port.is_some(),
                            ),
                        )
                        .error(self.errors.port.clone()),
                    );
            }
            Kind::Url => {
                group = group.row(
                    form::row(
                        "settings-metrics-url-row",
                        "Address",
                        field(
                            &self.url,
                            "settings-metrics-url",
                            "Store address",
                            self.errors.url.is_some(),
                        ),
                    )
                    .hint(if self.has_token {
                        "Its token stays where it is kept; this form doesn’t show or change it."
                    } else {
                        "An http:// or https:// address."
                    })
                    .error(self.errors.url.clone()),
                );
            }
        }
        group
            .saved_in(saved_in())
            .actions(form::actions(
                form::Action::new(
                    "settings-metrics-save",
                    "Save",
                    cx.listener(|page, _, window, cx| page.save_group(Group::Metrics, window, cx)),
                )
                .disabled(!editable || !dirty),
                form::Action::new(
                    "settings-metrics-revert",
                    "Revert",
                    cx.listener(|page, _, window, cx| {
                        page.revert_group(Group::Metrics, window, cx)
                    }),
                )
                .disabled(!dirty),
            ))
            .render(cx)
            .into_any_element()
    }
}

fn has_token(entry: &Entry) -> bool {
    entry
        .metrics
        .as_deref()
        .and_then(|metrics| metrics.source.as_ref())
        .is_some_and(|source| source.token.is_some())
}
