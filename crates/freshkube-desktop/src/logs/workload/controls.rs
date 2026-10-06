//! A workload's Logs tab controls, above the shared filters and search:
//! where the pod watch stands, the notices, then one row of chips, one
//! per container read, which shows or hides lines. The chips take the
//! rows the panel has room for, two at most, and none when the panel is
//! short; a "+N" chip lists every container, so none is out of reach.

use freshkube_ui::tooltip::FollowTooltip as _;
use gpui_kit::assets::IconName;
use gpui_kit::{
    Anchor, AnyElement, App, AvailableSpace, Context, Pixels, Role, SharedString, TestSupportExt,
    Toggled, WeakEntity, Window,
    component::{Icon, Sizable, button::Button, h_flex, popover::Popover},
    div,
    prelude::*,
    size,
};

use super::{Chip, MAX_STREAMS, PodsState, Streams, WorkloadLogView, chips_that_fit, plural};
use crate::palette::palette;
use crate::ui::{self, Tone, dp};

/// The Logs tab's toolbar rows.
pub(super) trait Controls: Sized + 'static {
    fn render_controls(&self, cx: &mut Context<Self>) -> Vec<AnyElement>;

    /// Where the watch stands: a tag, and the counts.
    fn render_status(&self, cx: &mut Context<Self>) -> AnyElement;

    /// A watch failure with Retry, or the cap's notice.
    fn render_notices(&self, cx: &mut Context<Self>) -> Vec<AnyElement>;

    /// One chip per container read: its state, and whether its lines show.
    fn render_streams(&self, cx: &mut Context<Self>) -> Option<AnyElement>;

    /// "+N", which lists every container.
    fn render_more(&self, hidden: usize, cx: &mut Context<Self>) -> AnyElement;

    /// How many chips the rows have room for, from their widths, measured
    /// again only when the chips or the text size change.
    fn fit_chips(&mut self, width: Pixels, window: &mut Window, cx: &mut Context<Self>);
}

/// Chip rows by the panel's height: what's left of a short panel goes to
/// the filters, the search and the log.
fn chip_rows(panel: Option<Pixels>, window: &Window) -> usize {
    let Some(panel) = panel else {
        return 2;
    };
    if panel >= ui::dp_px(340., window) {
        2
    } else if panel >= ui::dp_px(290., window) {
        1
    } else {
        0
    }
}

/// One container's chip: click to show or hide its lines.
fn chip(chip: &Chip, id: SharedString, view: WeakEntity<WorkloadLogView>, cx: &App) -> AnyElement {
    let p = palette(cx);
    let service = chip.service.clone();
    h_flex()
        .id(id)
        .test_support()
        .role(Role::CheckBox)
        .aria_toggled(if chip.shown {
            Toggled::True
        } else {
            Toggled::False
        })
        .aria_label(chip.tooltip.clone())
        .tab_index(0)
        .flex_none()
        .h(dp(26.))
        .pl(dp(8.))
        .pr(dp(9.))
        .gap(dp(6.))
        .rounded_full()
        .border_1()
        .border_color(p.line_strong)
        .bg(p.surface)
        .cursor_pointer()
        .font_family(ui::MONO_FONT)
        .text_size(dp(12.))
        .text_color(if chip.shown { p.ink } else { p.muted })
        .children(ui::status_glyph(chip.tone, cx))
        .child(
            div()
                .when(!chip.shown, |this| this.line_through())
                .child(chip.label.clone()),
        )
        .child(
            Icon::new(if chip.shown {
                IconName::Eye
            } else {
                IconName::EyeOff
            })
            .size(dp(13.))
            .text_color(p.muted),
        )
        .follow_tooltip(chip.tooltip.clone())
        .on_click(move |_, _, cx| {
            _ = view.update(cx, |view, cx| view.toggle_stream(service.clone(), cx));
        })
        .into_any_element()
}

/// What "+N" reads: the hidden count, or every container when no row shows.
fn more_label(hidden: usize, total: usize) -> SharedString {
    if hidden == total {
        plural(total, "container", "containers").into()
    } else {
        format!("+{hidden}").into()
    }
}

impl Controls for WorkloadLogView {
    fn render_controls(&self, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let mut rows = vec![self.render_status(cx)];
        rows.extend(self.render_notices(cx));
        rows.extend(self.render_streams(cx));
        rows
    }

    fn render_status(&self, cx: &mut Context<Self>) -> AnyElement {
        let status = &self.source().status;
        let failed = matches!(self.source().pods_state, PodsState::Failed(_));
        h_flex()
            .id("workload-logs-status")
            .test_support()
            .role(Role::Status)
            .aria_label(status.label.clone())
            .gap_2()
            .min_h(dp(22.))
            .text_size(dp(12.))
            .text_color(palette(cx).muted)
            .when(!status.tag.is_empty(), |this| {
                this.child(ui::tag(status.tone, None, status.tag.clone(), cx))
            })
            .when(!failed, |this| {
                this.child(div().flex_1().min_w_0().child(status.text.clone()))
            })
            .into_any_element()
    }

    fn render_notices(&self, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let mut notices = Vec::new();
        if let PodsState::Failed(_) = &self.source().pods_state {
            let text = self.source().status.text.clone();
            notices.push(
                div()
                    .id("workload-logs-failed")
                    .test_support()
                    .role(Role::Alert)
                    .aria_label(text.clone())
                    .child(ui::banner(
                        Tone::Crit,
                        None,
                        text,
                        Some(
                            Button::new("workload-logs-retry")
                                .outline()
                                .small()
                                .icon(IconName::RefreshCw)
                                .label("Retry")
                                .on_click(cx.listener(|view, _, _, cx| view.retry(cx)))
                                .into_any_element(),
                        ),
                        cx,
                    ))
                    .into_any_element(),
            );
        }
        let left_out = self.source().left_out;
        if left_out > 0 {
            let text: SharedString = format!(
                "Reading {MAX_STREAMS} of {} containers, the newest pods first.",
                MAX_STREAMS + left_out
            )
            .into();
            notices.push(
                div()
                    .id("workload-logs-capped")
                    .test_support()
                    .role(Role::Status)
                    .aria_label(text.clone())
                    .child(ui::warning_banner(None, text, None, cx))
                    .into_any_element(),
            );
        }
        notices
    }

    fn render_streams(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let chips = self.source().chips.clone();
        if chips.is_empty() {
            return None;
        }
        let shown = self.source().fits.min(chips.len());
        let view = cx.entity().downgrade();
        Some(
            h_flex()
                .id("workload-logs-streams")
                .test_support()
                .role(Role::Group)
                .aria_label(plural(chips.len(), "container", "containers"))
                .w_full()
                .min_w_0()
                .flex_wrap()
                .gap(dp(6.))
                .children(
                    chips[..shown]
                        .iter()
                        .map(|item| chip(item, item.id.clone(), view.clone(), cx)),
                )
                .when(shown < chips.len(), |this| {
                    this.child(self.render_more(chips.len() - shown, cx))
                })
                .into_any_element(),
        )
    }

    fn render_more(&self, hidden: usize, cx: &mut Context<Self>) -> AnyElement {
        let chips = self.source().chips.clone();
        let label = more_label(hidden, chips.len());
        let view = cx.entity().downgrade();
        let open_view = view.clone();
        Popover::new("workload-logs-more-popover")
            .anchor(Anchor::TopLeft)
            .open(self.source().more_open)
            .on_open_change(move |open, _, cx| {
                let open = *open;
                _ = open_view.update(cx, |view, cx| {
                    view.source_mut().more_open = open;
                    cx.notify();
                });
            })
            .trigger(
                Button::new("workload-logs-more")
                    .outline()
                    .small()
                    .label(label)
                    .tooltip("Every container read: show or hide its lines"),
            )
            .content(move |_, _, cx| {
                h_flex()
                    .id("workload-logs-list")
                    .test_support()
                    .role(Role::Group)
                    .aria_label("Every container read")
                    .max_w(dp(420.))
                    .flex_wrap()
                    .gap(dp(6.))
                    .children(
                        chips
                            .iter()
                            .map(|item| chip(item, item.list_id.clone(), view.clone(), cx)),
                    )
            })
            .into_any_element()
    }

    fn fit_chips(&mut self, width: Pixels, window: &mut Window, cx: &mut Context<Self>) {
        let rows = chip_rows(self.panel_height(), window);
        let chips = self.source().chips.clone();
        let key = (self.source().chips_revision, window.rem_size());
        if self.source().measured != Some(key) {
            let view = cx.entity().downgrade();
            let mut elements: Vec<AnyElement> = chips
                .iter()
                .map(|item| chip(item, item.id.clone(), view.clone(), cx))
                .collect();
            elements.push(
                Button::new("workload-logs-more-measure")
                    .outline()
                    .small()
                    .label(more_label(MAX_STREAMS, MAX_STREAMS + 1))
                    .into_any_element(),
            );
            let mut widths: Vec<Pixels> = elements
                .iter_mut()
                .map(|element| {
                    element
                        .layout_as_root(
                            size(AvailableSpace::MinContent, AvailableSpace::MinContent),
                            window,
                            cx,
                        )
                        .width
                })
                .collect();
            let more = widths.pop().unwrap_or_default();
            let source = self.source_mut();
            source.chip_widths = widths;
            source.more_width = more;
            source.measured = Some(key);
        }
        let source = self.source();
        // The panel's border, and a little slack for rounding.
        let room = width - ui::dp_px(8., window);
        let fits = chips_that_fit(
            &source.chip_widths,
            source.more_width,
            ui::dp_px(6., window),
            room,
            rows,
        );
        let source = self.source_mut();
        source.fits = fits;
        // With every chip in its row there's no "+N" to hold the list open.
        if fits >= chips.len() {
            source.more_open = false;
        }
    }
}
