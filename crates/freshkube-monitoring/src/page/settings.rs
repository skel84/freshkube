//! Settings' Dashboards section: the folder of the user's own Grafana JSON.
use super::source::{Mode, Test};
use crate::store::Choice;
use freshkube_core::secrets::STORE_NAME;
use gpui_kit::assets::IconName;
use gpui_kit::component::{
    Disableable, Sizable,
    button::{Button, ButtonGroup, ButtonVariants},
    checkbox::Checkbox,
    h_flex,
    input::{Input, InputState},
    popover::PopoverState,
    v_flex,
};
use gpui_kit::prelude::*;
use gpui_kit::{AnyElement, App, Entity, FontWeight, SharedString, TestSupportExt, div};

use super::MonitoringPage;
use crate::palette::palette;
use crate::ui::{self, dp};

pub fn settings_section(
    page: &Entity<MonitoringPage>,
    popover: Entity<PopoverState>,
    cx: &App,
) -> AnyElement {
    let p = palette(cx);
    let folder = page
        .read(cx)
        .folder()
        .map(|path| path.display().to_string());
    let save_error = page.read(cx).save_error.clone();
    let (choose, forget) = (page.downgrade(), page.downgrade());
    v_flex()
        .gap_1p5()
        .child(
            div()
                .text_size(dp(12.5))
                .font_weight(FontWeight::SEMIBOLD)
                .child("Dashboards folder"),
        )
        .child(
            h_flex()
                .gap_2()
                .child(
                    div()
                        .id("monitoring-folder")
                        .flex_1()
                        .min_w_0()
                        .truncate()
                        .font_family(ui::MONO_FONT)
                        .text_size(dp(12.))
                        .text_color(if folder.is_some() { p.ink } else { p.muted })
                        .child(folder.clone().unwrap_or_else(|| "None".into())),
                )
                .child(
                    Button::new("monitoring-folder-choose")
                        .outline()
                        .small()
                        .icon(IconName::FolderOpen)
                        .label("Choose…")
                        .on_click(move |_, window, cx| {
                            popover.update(cx, |state, cx| state.dismiss(window, cx));
                            _ = choose.update(cx, |page, cx| page.choose_folder(window, cx));
                        }),
                )
                .child(
                    Button::new("monitoring-folder-forget")
                        .ghost()
                        .small()
                        .label("Forget")
                        .disabled(folder.is_none())
                        .on_click(move |_, _, cx| {
                            _ = forget.update(cx, |page, cx| page.set_folder(None, cx));
                        }),
                ),
        )
        .child(
            div()
                .text_size(dp(12.))
                .text_color(p.muted)
                .child("Grafana JSON exports in this folder join the built-in dashboards under Monitoring. Their colours are not used."),
        )
        .children(save_error.map(|error| {
            div()
                .text_size(dp(12.))
                .text_color(p.crit_ink)
                .child(format!("Couldn't remember this for next launch: {error}"))
        }))
        .into_any_element()
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
                        format!("Read from this computer, not through the cluster. A token is sent as a bearer token and kept in {STORE_NAME}.")
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
