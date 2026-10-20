//! One guard for leaving any Settings section with a draft. The section
//! owns validation and saving; navigation waits until its draft is clean.
use super::*;
use freshkube_ui::form;
use gpui_kit::component::WindowExt;

type Destination = Box<dyn FnOnce(&mut Pilot, &mut Window, &mut Context<Pilot>)>;

pub(super) struct Leave {
    section: SettingsSection,
    destination: Destination,
    saving: bool,
}

impl Pilot {
    /// Returns true when navigation must wait for Save, Discard or Stay.
    pub(in crate::desktop) fn guard_settings(
        &mut self,
        destination: impl FnOnce(&mut Self, &mut Window, &mut Context<Self>) + 'static,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        if self.navigation.page != Page::Settings {
            return false;
        }
        if self.navigation.settings_leave.is_some() {
            return true;
        }
        let page = self.settings_page.read(cx);
        let section = page.section();
        if !page.has_unsaved(section, cx) {
            return false;
        }
        self.navigation.settings_leave = Some(Leave {
            section,
            destination: Box::new(destination),
            saving: false,
        });
        let focus = cx.focus_handle();
        let content_focus = focus.clone();
        let pilot = cx.weak_entity();
        window.open_dialog(cx, move |dialog, window, cx| {
            let busy = pilot.upgrade().is_some_and(|pilot| pilot.read(cx).settings_page.read(cx).unsaved_save_in_progress());
            let (save, discard, stay, cancel) =
                (pilot.clone(), pilot.clone(), pilot.clone(), pilot.clone());
            dialog
                .w(ui::dp_px(460., window).min(window.viewport_size().width - ui::dp_px(32., window)))
                .title(format!("Save changes to {}?", section.label()))
                .overlay_closable(false)
                .on_cancel(move |_, _, cx| {
                    _ = cancel.update(cx, |pilot, _| pilot.navigation.settings_leave = None);
                    true
                })
                .child(
                    v_flex()
                        .id("settings-unsaved-dialog")
                        .test_support()
                        .role(Role::Dialog)
                        .aria_label("Unsaved changes")
                        .track_focus(&content_focus)
                        .gap(dp(12.))
                        .text_size(dp(13.))
                        .child("Save your changes before leaving this section, discard them, or stay to keep editing.")
                        .child(form::actions(
                            form::Action::new("settings-unsaved-save", "Save", move |_, window, cx| {
                                window.close_dialog(cx);
                                _ = save.update(cx, |pilot, cx| {
                                    if let Some(leave) = &mut pilot.navigation.settings_leave {
                                        leave.saving = true;
                                    }
                                    pilot.settings_page.update(cx, |page, cx| page.save_unsaved(window, cx));
                                    pilot.finish_settings_leave(window, cx);
                                });
                            }).disabled(busy),
                            form::Action::new("settings-unsaved-stay", "Stay", move |_, window, cx| {
                                window.close_dialog(cx);
                                _ = stay.update(cx, |pilot, _| pilot.navigation.settings_leave = None);
                            }),
                        ).extra(
                            form::Action::new("settings-unsaved-discard", "Discard", move |_, window, cx| {
                                    window.close_dialog(cx);
                                    _ = discard.update(cx, |pilot, cx| {
                                        pilot.settings_page.update(cx, |page, cx| page.discard_unsaved(window, cx));
                                        if let Some(leave) = pilot.navigation.settings_leave.take() {
                                            (leave.destination)(pilot, window, cx);
                                        }
                                    });
                                }).disabled(busy),
                        )),
                )
        });
        // Kit remembers the previous focus on open, so move it afterwards.
        window.focus(&focus, cx);
        true
    }

    /// Called on section notifications and immediately after Save, including
    /// synchronous validation. A failed save keeps both the page and draft.
    pub(in crate::desktop) fn finish_settings_leave(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(leave) = &self.navigation.settings_leave else {
            return;
        };
        if !leave.saving || self.settings_page.read(cx).unsaved_save_in_progress() {
            return;
        }
        let clean = !self.settings_page.read(cx).has_unsaved(leave.section, cx);
        let leave = self.navigation.settings_leave.take().unwrap();
        if clean {
            (leave.destination)(self, window, cx);
        }
    }
}
