//! The Identity group: the entry's role, the kubeconfig context it opens
//! and the talosconfig it is read with.
use super::{Field, Group, SettingsPage, saved_in};
use crate::ui::{self, dp};
use freshkube_core::workspace::{Entry, Role};
use freshkube_ui::form;
use gpui_kit::component::{
    Disableable, Sizable,
    button::{Button, ButtonGroup},
    input::{Input, InputState},
};
use gpui_kit::prelude::*;
use gpui_kit::*;
use std::path::PathBuf;

/// What the group's fields hold, as text, so a draft compares with what the
/// file holds without a parse.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Values {
    pub(super) role: Role,
    pub(super) context: String,
    pub(super) talosconfig: String,
    pub(super) talos_context: String,
}

/// Each field's error, shown in place of its hint.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(super) struct Errors {
    pub(super) context: Option<SharedString>,
    pub(super) talosconfig: Option<SharedString>,
    pub(super) talos_context: Option<SharedString>,
}

impl Errors {
    /// The first field, top to bottom, that fails.
    pub(super) fn first(&self) -> Option<Field> {
        if self.context.is_some() {
            Some(Field::Context)
        } else if self.talosconfig.is_some() {
            Some(Field::Talosconfig)
        } else if self.talos_context.is_some() {
            Some(Field::TalosContext)
        } else {
            None
        }
    }

    pub(super) fn set(&mut self, field: Field, error: Option<SharedString>) {
        match field {
            Field::Context => self.context = error,
            Field::Talosconfig => self.talosconfig = error,
            Field::TalosContext => self.talos_context = error,
            _ => {}
        }
    }

    pub(super) fn of(&self, field: Field) -> Option<SharedString> {
        match field {
            Field::Context => self.context.clone(),
            Field::Talosconfig => self.talosconfig.clone(),
            Field::TalosContext => self.talos_context.clone(),
            _ => None,
        }
    }
}

impl Values {
    pub(super) fn of(entry: &Entry) -> Self {
        Self {
            role: entry.role,
            context: entry.context.clone(),
            talosconfig: entry
                .talosconfig
                .as_deref()
                .map(|path| path.display().to_string())
                .unwrap_or_default(),
            talos_context: entry.talos_context.clone().unwrap_or_default(),
        }
    }

    /// The values as a save would write them, so text a save trims doesn't
    /// keep the group dirty.
    pub(super) fn normalized(&self) -> Self {
        Self {
            role: self.role,
            context: self.context.trim().to_owned(),
            talosconfig: self.talosconfig.trim().to_owned(),
            talos_context: self.talos_context.trim().to_owned(),
        }
    }

    /// Every field's error, the words the Add dialog used.
    pub(super) fn check(&self) -> Errors {
        let talosconfig = self.talosconfig.trim();
        Errors {
            context: self
                .context
                .trim()
                .is_empty()
                .then(|| "Enter the kubeconfig context this cluster opens".into()),
            talosconfig: (!talosconfig.is_empty() && !PathBuf::from(talosconfig).is_absolute())
                .then(|| "Talosconfig must be an absolute path".into()),
            talos_context: (!self.talos_context.trim().is_empty() && talosconfig.is_empty())
                .then(|| "A Talos context needs a talosconfig".into()),
        }
    }

    /// Writes the group's keys into `entry` and nothing else of it.
    pub(super) fn apply(&self, entry: &mut Entry) {
        let talosconfig = self.talosconfig.trim();
        let talos_context = self.talos_context.trim();
        entry.role = self.role;
        entry.context = self.context.trim().to_owned();
        entry.talosconfig = (!talosconfig.is_empty()).then(|| PathBuf::from(talosconfig));
        entry.talos_context = (!talos_context.is_empty()).then(|| talos_context.to_owned());
    }
}

pub(super) struct Identity {
    pub(super) role: Role,
    pub(super) context: Entity<InputState>,
    pub(super) talosconfig: Entity<InputState>,
    pub(super) talos_context: Entity<InputState>,
    /// What the file holds for the entry; the group is dirty when it differs.
    pub(super) baseline: Values,
    pub(super) errors: Errors,
}

impl Identity {
    pub(super) fn new(entry: &Entry, window: &mut Window, cx: &mut Context<SettingsPage>) -> Self {
        let context =
            cx.new(|cx| InputState::new(window, cx).placeholder("Context name in the kubeconfig"));
        let talosconfig = cx.new(|cx| {
            InputState::new(window, cx).placeholder("Optional: absolute path of a talosconfig")
        });
        let talos_context = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("Optional: else the Context above, if the talosconfig has it")
        });
        let mut this = Self {
            role: entry.role,
            context,
            talosconfig,
            talos_context,
            baseline: Values::of(entry),
            errors: Errors::default(),
        };
        let values = this.baseline.clone();
        this.load(&values, window, cx);
        this
    }

    pub(super) fn input(&self, field: Field) -> Option<&Entity<InputState>> {
        match field {
            Field::Context => Some(&self.context),
            Field::Talosconfig => Some(&self.talosconfig),
            Field::TalosContext => Some(&self.talos_context),
            _ => None,
        }
    }

    pub(super) fn values(&self, cx: &App) -> Values {
        Values {
            role: self.role,
            context: self.context.read(cx).value().to_string(),
            talosconfig: self.talosconfig.read(cx).value().to_string(),
            talos_context: self.talos_context.read(cx).value().to_string(),
        }
    }

    pub(super) fn dirty(&self, cx: &App) -> bool {
        self.values(cx).normalized() != self.baseline.normalized()
    }

    /// Puts `values` in the fields, without errors.
    pub(super) fn load(&mut self, values: &Values, window: &mut Window, cx: &mut App) {
        self.role = values.role;
        self.errors = Errors::default();
        for (input, text) in [
            (&self.context, &values.context),
            (&self.talosconfig, &values.talosconfig),
            (&self.talos_context, &values.talos_context),
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
        let roles = Role::ALL.iter().enumerate().fold(
            ButtonGroup::new("settings-identity-role")
                .outline()
                .small()
                .disabled(!editable),
            |group, (ix, role)| {
                group.child(ui::choice(
                    Button::new(("settings-identity-role", ix))
                        .label(role.label())
                        .disabled(!editable),
                    self.role == *role,
                ))
            },
        );
        let page = cx.weak_entity();
        let browse = {
            let page = page.clone();
            move |_: &ClickEvent, window: &mut Window, cx: &mut App| {
                _ = page.update(cx, |page, cx| page.browse_talosconfig(window, cx));
            }
        };
        let talosconfig = form::path(
            "settings-identity-talosconfig",
            field(
                &self.talosconfig,
                "settings-identity-talosconfig-field",
                "Talosconfig path",
                self.errors.talosconfig.is_some(),
            ),
            browse,
        )
        .disabled(!editable);
        form::group("settings-identity", "Identity")
            .narrow(true)
            .dirty(dirty)
            .row(form::row(
                "settings-identity-role-row",
                "Role",
                roles.on_click({
                    let page = page.clone();
                    move |selected: &Vec<usize>, _, cx| {
                        let Some(role) = selected.first().and_then(|ix| Role::ALL.get(*ix)) else {
                            return;
                        };
                        let role = *role;
                        _ = page.update(cx, |page, cx| page.set_entry_role(role, cx));
                    }
                }),
            ))
            .row(
                form::row(
                    "settings-identity-context-row",
                    "Context",
                    field(
                        &self.context,
                        "settings-identity-context",
                        "Kubeconfig context",
                        self.errors.context.is_some(),
                    ),
                )
                .hint("The kubeconfig context this cluster opens.")
                .error(self.errors.context.clone()),
            )
            .row(
                form::row(
                    "settings-identity-talosconfig-row",
                    "Talosconfig",
                    talosconfig,
                )
                .hint("Optional: the file this cluster’s Talos API is read with.")
                .error(self.errors.talosconfig.clone()),
            )
            .row(
                form::row(
                    "settings-identity-talos-context-row",
                    "Talos context",
                    field(
                        &self.talos_context,
                        "settings-identity-talos-context",
                        "Talos context",
                        self.errors.talos_context.is_some(),
                    ),
                )
                .hint("Optional: unset, the Context above names it, if the talosconfig has it.")
                .error(self.errors.talos_context.clone()),
            )
            .saved_in(saved_in())
            .actions(form::actions(
                form::Action::new(
                    "settings-identity-save",
                    "Save",
                    cx.listener(|page, _, window, cx| page.save_group(Group::Identity, window, cx)),
                )
                .disabled(!editable || !dirty),
                form::Action::new(
                    "settings-identity-revert",
                    "Revert",
                    cx.listener(|page, _, window, cx| {
                        page.revert_group(Group::Identity, window, cx);
                        // The button is spent: the ring leaves with it.
                        window.focus(&page.focus, cx);
                    }),
                )
                .disabled(!dirty),
            ))
            .render(cx)
            .into_any_element()
    }
}
