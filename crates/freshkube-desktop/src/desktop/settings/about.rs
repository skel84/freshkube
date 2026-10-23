//! Settings › About: the version, the system and the log, with Reveal log
//! and Copy diagnostics for problems that aren't crashes (#953). The files
//! and the redaction are `freshkube_core::crash_log`; `desktop/crash.rs` holds
//! the actions the crash notice shares.

use freshkube_ui::{form, page};
use gpui_kit::component::{Disableable, Sizable, button::Button, h_flex, v_flex};
use gpui_kit::prelude::*;
use gpui_kit::*;

use super::SettingsPage;
use crate::desktop::crash;
use crate::ui::dp;

impl SettingsPage {
    pub(super) fn render_about(&self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let narrow = page::content_width(window) < form::NARROW;
        let header = page::PageHeader::new("settings-about", "About").render(window, cx);
        let session = crash::session(cx);
        let log = match &session {
            Some(info) => freshkube_core::crash_log::diagnostics::display_path(&info.log),
            None => "This window keeps no log".to_owned(),
        };
        let kept = session.is_some();
        let actions = h_flex()
            .gap(dp(8.))
            .child(
                Button::new("about-reveal-log")
                    .outline()
                    .small()
                    .disabled(!kept)
                    .label("Reveal log")
                    .on_click(|_, window, cx| crash::reveal_log(window, cx)),
            )
            .child(
                Button::new("about-copy-diagnostics")
                    .outline()
                    .small()
                    .disabled(!kept)
                    .label("Copy diagnostics")
                    .on_click(|_, window, cx| crash::copy_diagnostics(window, cx)),
            );
        let mut group = form::group("about-freshkube", "Freshkube")
            .narrow(narrow)
            .row(form::row(
                "about-version",
                "Version",
                env!("CARGO_PKG_VERSION"),
            ))
            .row(form::row(
                "about-system",
                "System",
                freshkube_core::crash_log::diagnostics::system(),
            ))
            .row(form::row("about-log", "Log", log));
        if session
            .as_ref()
            .is_some_and(|info| info.last_crash.is_some())
        {
            group = group.row(form::row(
                "about-last-session",
                "Last session",
                "Ended unexpectedly",
            ));
        }
        let problems = form::group("about-problems", "Problems")
            .narrow(narrow)
            .row(form::row("about-actions", "Diagnostics", actions).hint(
                "The log keeps the last three sessions. Copy diagnostics holds the version, \
                     the system and the last 200 log lines, with credentials removed; nothing \
                     is sent anywhere.",
            ));
        page::padded("settings-page")
            .track_focus(&self.focus)
            .overflow_y_scroll()
            .restrict_scroll_to_axis()
            .child(header)
            .child(
                v_flex()
                    .id("settings-about-form")
                    .test_support()
                    .w_full()
                    .gap(dp(20.))
                    .flex_none()
                    .child(group.render(cx))
                    .child(problems.render(cx)),
            )
            .into_any_element()
    }
}
