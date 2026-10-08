//! The form in the dialog that adds a cluster to the workspace or changes
//! one: its role, the kubeconfig context it opens, and an optional
//! talosconfig. The page checks and saves what it sends.
use super::*;
use freshkube_core::workspace::{Entry, Role as ClusterRole};
use gpui_kit::component::{
    Disableable, Sizable, WindowExt,
    button::{Button, ButtonGroup, ButtonVariants},
    h_flex,
    input::{Input, InputEvent, InputState},
    v_flex,
};

pub(super) struct ClusterForm {
    page: Entity<SettingsPage>,
    /// A save the form started is in flight: it closes when the page says it
    /// landed, and shows why when it did not.
    pending: bool,
    /// The id of the cluster being changed; none when adding.
    editing: Option<SharedString>,
    role: ClusterRole,
    context: Entity<InputState>,
    talosconfig: Entity<InputState>,
    talos_context: Entity<InputState>,
    error: Option<SharedString>,
    _subscriptions: Vec<Subscription>,
}

pub(super) fn open(
    page: Entity<SettingsPage>,
    editing: Option<Entry>,
    window: &mut Window,
    cx: &mut App,
) {
    let title = match &editing {
        Some(entry) => format!("Change {}", entry.id),
        None => "Add a cluster".to_owned(),
    };
    let form = cx.new(|cx| ClusterForm::new(page, editing, window, cx));
    let context = form.read(cx).context.clone();
    window.open_dialog(cx, move |dialog, window, _| {
        dialog
            .w(ui::dp_px(460., window).min(window.viewport_size().width - ui::dp_px(32., window)))
            .overlay_closable(false)
            .title(title.clone())
            .child(form.clone())
    });
    // A dialog remembers what had focus when it opened: focus its field after.
    context.update(cx, |state, cx| state.focus(window, cx));
}

impl ClusterForm {
    fn new(
        page: Entity<SettingsPage>,
        editing: Option<Entry>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let context =
            cx.new(|cx| InputState::new(window, cx).placeholder("Context name in the kubeconfig"));
        let talosconfig = cx.new(|cx| {
            InputState::new(window, cx).placeholder("Optional: absolute path of a talosconfig")
        });
        let talos_context = cx.new(|cx| {
            InputState::new(window, cx).placeholder("Optional: the talosconfig's own selected one")
        });
        let mut role = ClusterRole::Environment;
        let mut id = None;
        let saved_talos_context = editing
            .as_ref()
            .and_then(|entry| entry.talos_context.clone());
        if let Some(entry) = editing {
            role = entry.role;
            id = Some(SharedString::from(entry.id.clone()));
            context.update(cx, |input, cx| {
                input.set_value(entry.context.clone(), window, cx)
            });
            if let Some(path) = &entry.talosconfig {
                talosconfig.update(cx, |input, cx| {
                    input.set_value(path.display().to_string(), window, cx)
                });
            }
        }
        if let Some(name) = saved_talos_context {
            talos_context.update(cx, |input, cx| input.set_value(name, window, cx));
        }
        let submit = |this: &mut Self,
                      _: &Entity<InputState>,
                      event: &InputEvent,
                      _: &mut Window,
                      cx: &mut Context<Self>| {
            if matches!(event, InputEvent::PressEnter { .. }) {
                this.submit(cx);
            }
        };
        let subscriptions = vec![
            cx.subscribe_in(&context, window, submit),
            cx.subscribe_in(&talosconfig, window, submit),
            cx.subscribe_in(&talos_context, window, submit),
            cx.observe_in(&page, window, |this, page, window, cx| {
                this.saved(&page, window, cx)
            }),
        ];
        Self {
            page,
            pending: false,
            editing: id,
            role,
            context,
            talosconfig,
            talos_context,
            error: None,
            _subscriptions: subscriptions,
        }
    }

    fn submit(&mut self, cx: &mut Context<Self>) {
        if self.pending {
            return;
        }
        let context = self.context.read(cx).value().to_string();
        let talosconfig = self.talosconfig.read(cx).value().to_string();
        let talos_context = self.talos_context.read(cx).value().to_string();
        let role = self.role;
        let editing = self.editing.clone();
        let started = self.page.update(cx, |page, cx| {
            page.upsert(
                editing.as_deref(),
                role,
                &context,
                &talosconfig,
                &talos_context,
                cx,
            )
        });
        match started {
            // The save has started; the form stays until it answers, so a
            // failed write loses nothing that was typed.
            Ok(()) => self.pending = true,
            Err(why) => self.error = Some(why),
        }
        cx.notify();
    }

    /// The page changed: if our save has answered, close on success, or show
    /// why not and keep what was typed.
    fn saved(&mut self, page: &Entity<SettingsPage>, window: &mut Window, cx: &mut Context<Self>) {
        if !self.pending {
            return;
        }
        let (saving, notice) = {
            let page = page.read(cx);
            (page.saving.is_some(), page.notice.clone())
        };
        if saving {
            return;
        }
        self.pending = false;
        match notice {
            Some(super::Notice::Failed(why)) => {
                self.error = Some(why);
                page.update(cx, |page, cx| {
                    page.notice = None;
                    cx.notify();
                });
            }
            _ => window.close_dialog(cx),
        }
        cx.notify();
    }
}

impl Render for ClusterForm {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let p = crate::palette::palette(cx);
        let this = cx.entity().downgrade();
        let label = |text: &'static str| {
            div()
                .text_size(dp(12.5))
                .font_weight(FontWeight::SEMIBOLD)
                .child(text)
        };
        let selected = ClusterRole::ALL.iter().position(|role| *role == self.role);
        let roles = ClusterRole::ALL.iter().enumerate().fold(
            ButtonGroup::new("settings-role").outline().small(),
            |group, (ix, role)| {
                group.child(ui::choice(
                    Button::new(("settings-role", ix)).label(role.label()),
                    selected == Some(ix),
                ))
            },
        );
        v_flex()
            .id("settings-cluster-form")
            .test_support()
            .role(Role::Dialog)
            .aria_label("Cluster")
            .gap_3()
            .text_size(dp(13.))
            .when_some(self.editing.clone(), |this, id| {
                this.child(
                    h_flex().gap_2().child(label("Cluster")).child(
                        div()
                            .id("settings-form-id")
                            .test_support()
                            .text_color(p.muted)
                            .child(id),
                    ),
                )
            })
            .child(
                v_flex()
                    .gap_1p5()
                    .child(label("Role"))
                    .child(roles.on_click(move |selected: &Vec<usize>, _, cx| {
                        let Some(role) = selected.first().and_then(|ix| ClusterRole::ALL.get(*ix))
                        else {
                            return;
                        };
                        let role = *role;
                        _ = this.update(cx, |form, cx| {
                            form.role = role;
                            cx.notify();
                        });
                    })),
            )
            .child(
                v_flex().gap_1p5().child(label("Context")).child(
                    Input::new(&self.context)
                        .id("settings-form-context")
                        .aria_label("Kubeconfig context")
                        .small(),
                ),
            )
            .child(
                v_flex().gap_1p5().child(label("Talosconfig")).child(
                    Input::new(&self.talosconfig)
                        .id("settings-form-talosconfig")
                        .aria_label("Talosconfig path")
                        .small(),
                ),
            )
            .child(
                v_flex().gap_1p5().child(label("Talos context")).child(
                    Input::new(&self.talos_context)
                        .id("settings-form-talos-context")
                        .aria_label("Talos context")
                        .small(),
                ),
            )
            .children(self.error.clone().map(|why| {
                div()
                    .id("settings-form-error")
                    .test_support()
                    .role(Role::Alert)
                    .aria_label(why.clone())
                    .child(ui::warning_banner(None, why, None, cx))
            }))
            .child(
                h_flex()
                    .gap_2()
                    .justify_end()
                    .child(
                        Button::new("settings-form-cancel")
                            .outline()
                            .label("Cancel")
                            .on_click(|_, window, cx| window.close_dialog(cx)),
                    )
                    .child(
                        Button::new("settings-form-save")
                            .primary()
                            .label("Save")
                            .disabled(self.pending)
                            .on_click(cx.listener(|form, _, _, cx| form.submit(cx))),
                    ),
            )
    }
}
