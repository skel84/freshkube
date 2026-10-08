//! The header Refresh button's tooltip, which reads the countdown again
//! while it shows. The header is a cached view that the shell's tick
//! doesn't redraw, so text built there would keep the number it opened
//! with (#409).
use std::time::Duration;

use gpui_kit::component::{button::Button, tooltip::Tooltip};
use gpui_kit::prelude::*;
use gpui_kit::{
    AnyView, App, Context, ElementId, SharedString, Task, TestSupportExt as _, WeakEntity, Window,
    div,
};

use super::{AUTO_REFRESH, Pilot};

/// A button whose tooltip is a view of its own, which may change while it
/// shows. Kit's button takes only text, fixed when the button draws.
pub(super) trait TooltipView {
    fn tooltip_view(
        self,
        id: impl Into<ElementId>,
        build: impl Fn(&mut Window, &mut App) -> AnyView + 'static,
    ) -> impl IntoElement;
}

impl TooltipView for Button {
    fn tooltip_view(
        self,
        id: impl Into<ElementId>,
        build: impl Fn(&mut Window, &mut App) -> AnyView + 'static,
    ) -> impl IntoElement {
        div().id(id).child(self).tooltip(build).test_support()
    }
}

/// How often an open tooltip reads the shell again. The ring moves once
/// a second; reading more often keeps the two in step whatever the phase.
const READ_EVERY: Duration = Duration::from_millis(250);

/// The tooltip while it shows. GPUI drops it when the tooltip hides, and
/// its timer with it, so nothing ticks for a closed tooltip.
pub(super) struct RefreshTip {
    pilot: WeakEntity<Pilot>,
    text: SharedString,
    tip: AnyView,
    _read: Task<()>,
}

impl RefreshTip {
    pub(super) fn build(pilot: WeakEntity<Pilot>, window: &mut Window, cx: &mut App) -> AnyView {
        let text = pilot
            .upgrade()
            .map(|pilot| pilot.read(cx).refresh_tip())
            .unwrap_or_default();
        let tip = shown(text.clone(), window, cx);
        cx.new(|cx| RefreshTip {
            pilot,
            text,
            tip,
            _read: cx.spawn_in(window, async move |this, cx| {
                loop {
                    cx.background_executor().timer(READ_EVERY).await;
                    if this
                        .update_in(cx, |tip: &mut RefreshTip, window, cx| {
                            tip.reread(window, cx)
                        })
                        .is_err()
                    {
                        break;
                    }
                }
            }),
        })
        .into()
    }

    /// Redraws only this tooltip, and only when its text changed.
    fn reread(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(pilot) = self.pilot.upgrade() else {
            return;
        };
        let text = pilot.read(cx).refresh_tip();
        if text != self.text {
            self.tip = shown(text.clone(), window, cx);
            self.text = text;
            cx.notify();
        }
    }
}

/// Kit's tooltip for `text`, counted by UI tests as other tooltips are.
fn shown(text: SharedString, window: &mut Window, cx: &mut App) -> AnyView {
    let tip = Tooltip::new(text.clone()).build(window, cx);
    freshkube_ui::tooltip::unless_menu(tip, text, cx)
}

impl Render for RefreshTip {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        self.tip.clone()
    }
}

impl Pilot {
    /// What the Refresh button's tooltip says now.
    pub(in crate::desktop) fn refresh_tip(&self) -> SharedString {
        if self.loading() {
            "Refreshing…".into()
        } else if self.countdown_state().1 {
            let next_in = AUTO_REFRESH.saturating_sub(self.elapsed).as_secs();
            format!("Refresh now · next Talos refresh in {next_in} s").into()
        } else {
            "Refresh now".into()
        }
    }
}
