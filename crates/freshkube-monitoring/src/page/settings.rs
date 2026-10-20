//! Monitoring's Settings groups; the page retains their handlers and fields.
use freshkube_core::secrets::STORE_NAME;
use freshkube_ui::{form, palette::palette};
use gpui_kit::component::{
    Disableable, Sizable,
    button::{Button, ButtonGroup, ButtonVariants},
    checkbox::Checkbox,
    h_flex,
    input::{Input, InputState},
};
use gpui_kit::prelude::*;
use gpui_kit::{AnyElement, App, Entity, SharedString, TestSupportExt, div};

use super::{
    MonitoringPage,
    source::{Field, Mode, Test},
};
use crate::store::Choice;
use crate::ui::{self, dp};

pub fn settings_section(page: &Entity<MonitoringPage>, narrow: bool, cx: &App) -> AnyElement {
    let p = palette(cx);
    let this = page.read(cx);
    let folder = this.folder().map(|path| path.display().to_string());
    let example = this.open_access().is_some_and(|access| access.is_example());
    let (choose, forget) = (page.downgrade(), page.downgrade());
    let controls = h_flex()
        .gap(dp(8.))
        .flex_wrap()
        .child(
            div()
                .id("monitoring-folder")
                .test_support()
                .flex_1()
                .min_w_0()
                .font_family(ui::MONO_FONT)
                .text_size(dp(12.))
                .text_color(p.muted)
                .child(folder.clone().unwrap_or_else(|| "None".into())),
        )
        .child(
            Button::new("monitoring-folder-choose")
                .outline()
                .small()
                .label("Choose…")
                .h(dp(ui::CONTROL_HEIGHT))
                .disabled(example)
                .on_click(move |_, window, cx| {
                    _ = choose.update(cx, |page, cx| page.choose_folder(window, cx));
                }),
        )
        .child(
            Button::new("monitoring-folder-forget")
                .ghost()
                .small()
                .label("Forget")
                .h(dp(ui::CONTROL_HEIGHT))
                .disabled(example || folder.is_none())
                .on_click(move |_, _, cx| {
                    _ = forget.update(cx, |page, cx| page.set_folder(None, cx));
                }),
        );
    form::group("monitoring-dashboards", "Dashboards folder").narrow(narrow)
        .row(form::row("monitoring-folder-row", "Folder", controls).hint(if example {
            "Example data isn’t saved"
        } else {
            "Grafana JSON exports in this folder join the built-in dashboards under Monitoring. Their colours are not used."
        }).error(this.save_error.clone()))
        .saved_in(form::SavedIn::file("monitoring.json")).render(cx).into_any_element()
}

/// Today's choice for a context outside the workspace.
pub fn source_section(page: &Entity<MonitoringPage>, narrow: bool, cx: &App) -> AnyElement {
    let this = page.read(cx);
    let group = form::group("monitoring-source-settings", "This context").narrow(narrow);
    let (Some(context), Some(draft)) = (this.context(), this.form.as_ref()) else {
        return group
            .row(form::row(
                "monitoring-context-unavailable",
                "Source",
                "Choose a context to set where its metrics come from.",
            ))
            .render(cx)
            .into_any_element();
    };
    let weak = page.downgrade();
    let mode = draft.mode;
    let group = group.dirty(draft.dirty);
    let saved_token = matches!(
        this.saved.choices.get(&context),
        Some(Choice::Url { token: true, .. })
    );
    let modes = ButtonGroup::new("monitoring-source-mode")
        .outline()
        .small()
        .child(ui::choice(
            Button::new("monitoring-source-automatic").label("Automatic"),
            mode == Mode::Automatic,
        ))
        .child(ui::choice(
            Button::new("monitoring-source-service").label("Service"),
            mode == Mode::Service,
        ))
        .child(ui::choice(
            Button::new("monitoring-source-url").label("URL"),
            mode == Mode::Url,
        ))
        .on_click(move |selected: &Vec<usize>, _, cx| {
            let mode = match selected.first() {
                Some(0) => Mode::Automatic,
                Some(1) => Mode::Service,
                Some(_) => Mode::Url,
                None => return,
            };
            _ = weak.update(cx, |page, cx| page.set_mode(mode, cx));
        });
    let hint: SharedString = match mode {
        Mode::Automatic => "Finds Prometheus, VictoriaMetrics, Thanos or Mimir in the cluster and reads it through the Kubernetes API’s service proxy.".into(),
        Mode::Service => "Reads through the Kubernetes API’s service proxy with this context’s credentials. VictoriaMetrics’ vmselect uses the prefix select/0/prometheus; Mimir uses prometheus.".into(),
        Mode::Url if this.secrets.is_some() => format!("Reads directly from this computer. A token is sent as a bearer token and kept in {STORE_NAME}.").into(),
        Mode::Url => "Reads directly from this computer. A token is kept until Freshkube quits.".into(),
    };
    let mut group = group
        .row(form::row(
            "monitoring-context-current",
            "Context",
            context.clone(),
        ))
        .row(form::row("monitoring-context-mode", "Source", modes).hint(hint));
    let input = |id: &'static str, state: &Entity<InputState>| {
        let input = Input::new(state).id(id).small().h(dp(ui::CONTROL_HEIGHT));
        if draft.error.as_ref().is_some_and(|(field, _)| match field {
            Field::Namespace => state == &draft.namespace,
            Field::Name => state == &draft.name,
            Field::Port => state == &draft.port,
            Field::Url => state == &draft.url,
        }) {
            form::invalid(input, cx)
        } else {
            input
        }
    };
    match mode {
        Mode::Automatic => {}
        Mode::Service => {
            let weak = page.downgrade();
            group = group
                .row(
                    form::row(
                        "monitoring-context-namespace",
                        "Namespace",
                        input("monitoring-source-namespace", &draft.namespace),
                    )
                    .hint("Where the store’s Service runs.")
                    .error(draft.error(Field::Namespace)),
                )
                .row(
                    form::row(
                        "monitoring-context-name",
                        "Service",
                        input("monitoring-source-name", &draft.name),
                    )
                    .hint("The store’s Service in that namespace.")
                    .error(draft.error(Field::Name)),
                )
                .row(
                    form::row(
                        "monitoring-context-port",
                        "Port",
                        input("monitoring-source-port", &draft.port),
                    )
                    .hint("The Service’s port, such as 9090.")
                    .error(draft.error(Field::Port)),
                )
                .row(form::row(
                    "monitoring-context-path",
                    "Path",
                    input("monitoring-source-path", &draft.path),
                ))
                .row(form::row(
                    "monitoring-context-https",
                    "HTTPS",
                    Checkbox::new("monitoring-source-https")
                        .small()
                        .checked(draft.https)
                        .on_click(move |checked, _, cx| {
                            _ = weak.update(cx, |page, cx| page.set_https(*checked, cx));
                        }),
                ));
        }
        Mode::Url => {
            let (replace, forget) = (page.downgrade(), page.downgrade());
            let secret = if saved_token && !draft.replacing_token {
                form::Secret::Kept
            } else {
                form::Secret::Typing(draft.token.clone())
            };
            group = group
                .row(
                    form::row(
                        "monitoring-context-url",
                        "URL",
                        input("monitoring-source-address", &draft.url),
                    )
                    .hint("An http:// or https:// address for the store.")
                    .error(draft.error(Field::Url)),
                )
                .row(form::row(
                    "monitoring-context-token",
                    "Token",
                    form::secret("monitoring-source-token", secret, cx)
                        .store_name(STORE_NAME)
                        .on_replace(move |_, _, cx| {
                            _ = replace.update(cx, |page, cx| {
                                if let Some(form) = &mut page.form {
                                    form.replacing_token = true;
                                }
                                cx.notify();
                            });
                        })
                        .on_forget(move |_, window, cx| {
                            _ = forget.update(cx, |page, cx| page.forget_token(window, cx));
                        }),
                ));
        }
    }
    let testing = matches!(draft.test, Test::Testing(_));
    let (test, save, revert) = (page.downgrade(), page.downgrade(), page.downgrade());
    let (tone, outcome): (_, SharedString) = match &draft.test {
        Test::Idle => (
            ui::Tone::Unknown,
            draft
                .note
                .clone()
                .unwrap_or_else(|| "Checks the values above without saving them.".into()),
        ),
        Test::Testing(_) => (ui::Tone::Unknown, "Testing…".into()),
        Test::Passed(message) => (ui::Tone::Good, format!("Passed · {message}").into()),
        Test::Failed(message) => (ui::Tone::Crit, format!("Failed · {message}").into()),
    };
    let line = h_flex()
        .gap(dp(8.))
        .items_start()
        .child(
            Button::new("monitoring-source-test")
                .outline()
                .small()
                .label("Test")
                .h(dp(ui::CONTROL_HEIGHT))
                .disabled(testing)
                .on_click(move |_, window, cx| {
                    _ = test.update(cx, |page, cx| page.test_source(window, cx));
                }),
        )
        .child(
            div()
                .id("monitoring-context-test-result")
                .test_support()
                .role(gpui_kit::Role::Status)
                .aria_label(outcome.clone())
                .flex_1()
                .min_w_0()
                .text_size(dp(12.))
                .children(ui::status_glyph(tone, cx))
                .child(outcome),
        );
    let saved = if saved_token {
        form::SavedIn::file(format!("monitoring.json; the token in {STORE_NAME}"))
    } else {
        form::SavedIn::file("monitoring.json")
    };
    let content = group
        .row(form::row("monitoring-context-test", "Test", line))
        .saved_in(saved)
        .actions(form::actions(
            form::Action::new("monitoring-source-save", "Save", move |_, window, cx| {
                _ = save.update(cx, |page, cx| page.save_source(window, cx));
            })
            .disabled(!draft.dirty),
            form::Action::new(
                "monitoring-source-revert",
                "Revert",
                move |_, window, cx| {
                    _ = revert.update(cx, |page, cx| page.revert_source(window, cx));
                },
            )
            .disabled(!draft.dirty),
        ))
        .render(cx)
        .into_any_element();
    let weak = page.downgrade();
    div()
        .track_focus(&draft.focus)
        .capture_key_down(move |event, window, cx| {
            if event.keystroke.key == "escape" {
                _ = weak.update(cx, |page, cx| {
                    if page
                        .form
                        .as_ref()
                        .is_some_and(|form| form.initial.as_ref() != Some(&form.values(cx)))
                    {
                        page.revert_source(window, cx);
                        cx.stop_propagation();
                    }
                });
            }
        })
        .child(content)
        .into_any_element()
}
