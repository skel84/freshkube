//! The form in the dialog that adds a cluster to the workspace: its role and
//! the kubeconfig context it opens. The id is derived from the context, and
//! everything else about the cluster is edited in the Inspector. The page
//! checks and saves what the dialog sends.
use super::*;
use freshkube_core::workspace::Role as ClusterRole;
use freshkube_ui::form as kit_form;
use gpui_kit::component::{
    Sizable, WindowExt,
    button::{Button, ButtonGroup},
    input::{Input, InputEvent, InputState},
    v_flex,
};

pub(super) struct ClusterForm {
    page: Entity<SettingsPage>,
    /// A save the form started is in flight: it closes when the page says it
    /// landed, and shows why when it did not.
    pending: bool,
    role: ClusterRole,
    context: Entity<InputState>,
    error: Option<SharedString>,
    _subscriptions: Vec<Subscription>,
}

pub(super) fn open(page: Entity<SettingsPage>, window: &mut Window, cx: &mut App) {
    let form = cx.new(|cx| ClusterForm::new(page, window, cx));
    let context = form.read(cx).context.clone();
    window.open_dialog(cx, move |dialog, window, cx| {
        freshkube_ui::dialog::frame(dialog, 460., ClusterForm::actions(&form, cx), window)
            .title("Add a cluster")
            .child(form.clone())
    });
    // A dialog remembers what had focus when it opened: focus its field after.
    context.update(cx, |state, cx| state.focus(window, cx));
}

impl ClusterForm {
    fn actions(form: &Entity<Self>, cx: &App) -> impl IntoElement {
        let pending = form.read(cx).pending;
        let form = form.downgrade();
        kit_form::actions(
            kit_form::Action::new("settings-form-save", "Save", move |_, _, cx| {
                _ = form.update(cx, |form, cx| form.submit(cx));
            })
            .disabled(pending),
            kit_form::Action::cancel("settings-form-cancel"),
        )
    }

    fn new(page: Entity<SettingsPage>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let context =
            cx.new(|cx| InputState::new(window, cx).placeholder("Context name in the kubeconfig"));
        let subscriptions = vec![
            cx.subscribe_in(&context, window, |this, _, event: &InputEvent, _, cx| {
                if matches!(event, InputEvent::PressEnter { .. }) {
                    this.submit(cx);
                }
            }),
            cx.observe_in(&page, window, |this, page, window, cx| {
                this.saved(&page, window, cx)
            }),
        ];
        Self {
            page,
            pending: false,
            role: ClusterRole::Environment,
            context,
            error: None,
            _subscriptions: subscriptions,
        }
    }

    fn submit(&mut self, cx: &mut Context<Self>) {
        if self.pending {
            return;
        }
        let context = self.context.read(cx).value().to_string();
        let role = self.role;
        let started = self
            .page
            .update(cx, |page, cx| page.add_cluster(role, &context, cx));
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
        let this = cx.entity().downgrade();
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
            .children(self.error.clone().map(|why| {
                div()
                    .id("settings-form-error")
                    .test_support()
                    .role(Role::Alert)
                    .aria_label(why.clone())
                    .child(ui::warning_banner(None, why, None, cx))
            }))
            .child(
                kit_form::group("settings-cluster", "Cluster")
                    .narrow(true)
                    .row(kit_form::row(
                        "settings-form-role",
                        "Role",
                        roles.on_click(move |selected: &Vec<usize>, _, cx| {
                            let Some(role) =
                                selected.first().and_then(|ix| ClusterRole::ALL.get(*ix))
                            else {
                                return;
                            };
                            let role = *role;
                            _ = this.update(cx, |form, cx| {
                                form.role = role;
                                cx.notify();
                            });
                        }),
                    ))
                    .row(
                        kit_form::row(
                            "settings-form-context-row",
                            "Context",
                            Input::new(&self.context)
                                .id("settings-form-context")
                                .aria_label("Kubeconfig context")
                                .small(),
                        )
                        .hint("Talosconfig and the rest are set in the Inspector."),
                    )
                    .render(cx),
            )
    }
}
