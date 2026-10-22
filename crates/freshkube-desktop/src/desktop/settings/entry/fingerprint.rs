//! The Fingerprint group: what deep links match this cluster by (#873), read
//! only. The window reads it, never the file: the shell hands the page the
//! value of each entry, and Copy puts it on the clipboard.
use super::SettingsPage;
use crate::ui::{self, MONO_FONT, dp};
use freshkube_ui::form;
use gpui_kit::assets::IconName;
use gpui_kit::component::{Sizable, button::Button, h_flex};
use gpui_kit::prelude::*;
use gpui_kit::*;

/// What one entry's fingerprint is, and how well it is known.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Line {
    /// The value a link carries, if there is one to copy.
    pub(crate) value: Option<SharedString>,
    /// What the value is, or why there is none.
    pub(crate) note: SharedString,
}

impl SettingsPage {
    /// The shell's word on every entry's fingerprint, replacing the last.
    pub(in crate::desktop) fn set_fingerprints(
        &mut self,
        lines: std::collections::BTreeMap<String, Line>,
        cx: &mut Context<Self>,
    ) {
        if self.fingerprints != lines {
            self.fingerprints = lines;
            cx.notify();
        }
    }

    pub(super) fn render_fingerprint(&self, id: &str, cx: &mut Context<Self>) -> AnyElement {
        let line = self.fingerprints.get(id).cloned().unwrap_or(Line {
            value: None,
            note: "Not checked yet. It is read when the cluster opens.".into(),
        });
        let shown = line.value.clone().unwrap_or_else(|| "Not available".into());
        let copy = line.value.clone();
        let control = h_flex()
            .gap(dp(8.))
            .items_center()
            .child(
                div()
                    .id("settings-fingerprint-value")
                    .test_support()
                    .aria_label(shown.clone())
                    .min_w_0()
                    .truncate()
                    .font_family(MONO_FONT)
                    .text_size(dp(12.5))
                    .child(shown),
            )
            .child(
                Button::new("settings-fingerprint-copy")
                    .small()
                    .h(dp(ui::CONTROL_HEIGHT))
                    .outline()
                    .icon(IconName::Copy)
                    .label("Copy")
                    .on_click(move |_, _, cx| {
                        if let Some(value) = &copy {
                            cx.write_to_clipboard(ClipboardItem::new_string(value.to_string()));
                        }
                    }),
            );
        form::group("settings-fingerprint", "Link fingerprint")
            .narrow(true)
            .row(form::row("settings-fingerprint-row", "Fingerprint", control).hint(line.note))
            .render(cx)
            .into_any_element()
    }
}
