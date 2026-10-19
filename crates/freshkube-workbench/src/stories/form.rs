//! The form helpers, one state at a time, on invented connection settings.

use super::motion::{option, segments};
use freshkube_ui::form::{self, Action, Secret};
use freshkube_ui::page::{self, PageHeader};
use freshkube_ui::ui::dp;
use gpui_kit::component::input::{Input, InputState};
use gpui_kit::component::{Sizable, v_flex};
use gpui_kit::prelude::*;
use gpui_kit::{AnyView, App, Context, Entity, Window};

#[derive(Clone, Copy, PartialEq, Eq)]
enum State {
    Plain,
    Error,
    Secret,
    Dirty,
    Narrow,
}

pub(super) fn build(window: &mut Window, cx: &mut App) -> AnyView {
    cx.new(|cx| FormStory {
        state: State::Plain,
        context: cx.new(|cx| InputState::new(window, cx).default_value("cluster-a")),
        path: cx.new(|cx| InputState::new(window, cx).default_value("/example/config")),
        token: cx.new(|cx| InputState::new(window, cx).masked(true)),
        dirty: false,
        replacing: false,
    })
    .into()
}

struct FormStory {
    state: State,
    context: Entity<InputState>,
    path: Entity<InputState>,
    token: Entity<InputState>,
    dirty: bool,
    replacing: bool,
}

impl FormStory {
    fn choose(&mut self, state: State, cx: &mut Context<Self>) {
        self.state = state;
        self.dirty = matches!(state, State::Dirty | State::Narrow);
        self.replacing = false;
        cx.notify();
    }

    fn render_group(&self, window: &Window, cx: &Context<Self>) -> impl IntoElement {
        let narrow = self.state == State::Narrow || page::content_width(window) < form::NARROW;
        let mut group = form::group("form-connection", "Connection")
            .dirty(self.dirty)
            .narrow(narrow);
        if self.state == State::Secret {
            let secret = if self.replacing {
                Secret::Typing(self.token.clone())
            } else {
                Secret::Kept
            };
            group = group
                .row(
                    form::row(
                        "form-token",
                        "Token",
                        form::secret("form-credential", secret, cx)
                            .on_replace(cx.listener(|this, _, _, cx| {
                                this.replacing = true;
                                cx.notify();
                            }))
                            .on_forget(cx.listener(|this, _, _, cx| {
                                this.replacing = true;
                                cx.notify();
                            })),
                    )
                    .hint("Used only for this connection."),
                )
                .saved_in(form::SavedIn::file("monitoring.json").and_credential("the token"));
        } else {
            let failing = self.state == State::Error;
            let input = gpui_kit::Styled::h(
                Input::new(&self.context).small().id("form-context-input"),
                dp(24.),
            );
            let input = if failing {
                form::invalid(input, cx)
            } else {
                input
            };
            group = group
                .row(
                    form::row("form-context", "Context", input)
                        .hint("The context to use for cluster-a.")
                        .error(failing.then_some("Choose a context from the file.")),
                )
                .row(
                    form::row(
                        "form-config",
                        "Configuration",
                        form::path(
                            "form-config",
                            Input::new(&self.path),
                            cx.listener(|this, _, window, cx| {
                                this.path.update(cx, |input, cx| {
                                    input.set_value("/example/cluster-a", window, cx)
                                });
                                cx.notify();
                            }),
                        )
                        .chosen_only(),
                    )
                    .hint("Choose a file; its contents stay outside the form."),
                )
                .saved_in(form::SavedIn::file("workspace.json"));
        }
        if matches!(self.state, State::Dirty | State::Narrow) {
            group = group.actions(form::actions(
                Action::new(
                    "form-save",
                    "Save",
                    cx.listener(|this, _, _, cx| {
                        this.dirty = false;
                        cx.notify();
                    }),
                )
                .disabled(!self.dirty),
                Action::new(
                    "form-revert",
                    "Revert",
                    cx.listener(|this, _, _, cx| {
                        this.dirty = false;
                        cx.notify();
                    }),
                )
                .disabled(!self.dirty),
            ));
        }
        v_flex()
            .w_full()
            .when(self.state == State::Narrow, |this| {
                this.max_w(dp(form::NARROW - 1.))
            })
            .child(group.render(cx))
    }
}

impl Render for FormStory {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let choices = segments(
            [
                (State::Plain, "plain", "Plain"),
                (State::Error, "error", "Error"),
                (State::Secret, "secret", "Secret"),
                (State::Dirty, "dirty", "Dirty"),
                (State::Narrow, "narrow", "Narrow"),
            ]
            .map(|(state, id, label)| {
                option(format!("form-{id}"), label, self.state == state, cx)
                    .on_click(cx.listener(move |this, _, _, cx| this.choose(state, cx)))
            }),
            cx,
        );
        let header = PageHeader::new("form", "Form")
            .control(choices)
            .render(window, cx);
        page::page("form-page")
            .child(page::toolbar(cx).child(header))
            .child(page::padded("form-content").child(self.render_group(window, cx)))
    }
}
