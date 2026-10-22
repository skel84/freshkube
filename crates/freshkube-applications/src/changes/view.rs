//! The page: its header, the notes on what may be missing, then the table
//! edge to edge with the selection's trail in the Inspector; or, with no
//! rows, the state that says why. A change opened from it shows in its
//! place.
use super::display::{Cell, TrailGroup};
use super::*;
use crate::ui::{self, dp};
use freshkube_ui::inspector::{self, Inspector};
use freshkube_ui::page::{self, PageHeader};
use freshkube_ui::palette::palette;
use freshkube_ui::tooltip::FollowTooltip as _;
use freshkube_ui::ui::{field, mono};
use gpui_kit::assets::IconName;
use gpui_kit::component::{
    Disableable, Sizable,
    button::{Button, ButtonVariants},
    h_flex, v_flex,
};

impl ChangesPage {
    fn render_header(&self, window: &mut Window, cx: &mut Context<Self>) -> Div {
        let header = PageHeader::new(PREFIX, "Changes");
        let reading = self.loader.is_loading();
        let label = "Read the changes again";
        let refresh = Button::new(header.id("refresh"))
            .ghost()
            .small()
            .size(dp(ui::CONTROL_HEIGHT))
            .icon(ui::refresh_icon(reading, cx))
            .accessibility_label(label)
            .tooltip_with_action(
                if reading { "Reading…" } else { label },
                &page::Refresh,
                Some(page::SHELL_CONTEXT),
            )
            .disabled(reading || self.source.is_none())
            // The button and ⌘R are one action, which the shell runs.
            .on_click(page::dispatch(page::Refresh, &self.focus));
        header.control(refresh).render(window, cx)
    }

    /// A refresh that failed over an earlier read, then what may be
    /// missing.
    fn render_notes(&self, cx: &App) -> Option<AnyElement> {
        let setup = self.setup_problem.as_ref().map(|text| {
            ui::warning_banner(None, text.clone(), None, cx)
                .id("changes-setup-problem")
                .aria_label(text.clone())
                .test_support()
                .role(Role::Status)
        });
        let refresh = self.stale.as_ref().map(|(text, label)| {
            ui::warning_banner(Some("Couldn't read again".into()), text.clone(), None, cx)
                .id("changes-stale")
                .aria_label(label.clone())
                .test_support()
                .role(Role::Status)
        });
        let missing = self.display.missing.as_ref().map(|(text, label)| {
            ui::warning_banner(Some("May be missing".into()), text.clone(), None, cx)
                .id("changes-missing")
                .aria_label(label.clone())
                .test_support()
                .role(Role::Status)
        });
        if setup.is_none() && refresh.is_none() && missing.is_none() {
            return None;
        }
        Some(
            page::inset()
                .id("changes-notes")
                .flex()
                .flex_col()
                .gap(dp(page::PANE_PADDING_Y))
                .children(setup)
                .children(refresh)
                .children(missing)
                .into_any_element(),
        )
    }

    /// With no rows: refused and failed say what wasn't read; only when
    /// Kargo answered is it empty.
    fn render_state(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let retrying = self.loader.is_loading();
        let retry = || {
            let label = if retrying { "Retrying…" } else { "Retry" };
            Button::new("changes-retry")
                .primary()
                .icon(ui::refresh_icon(retrying, cx))
                .label(label)
                .accessibility_label(label)
                .disabled(retrying)
                .on_click(cx.listener(|this, _, _, cx| this.refresh(cx)))
                .into_any_element()
        };
        let (id, state) = match &self.display.body {
            Body::Table => return None,
            Body::Empty { title, description } => (
                "changes-none",
                ui::empty_state(
                    IconName::Layers,
                    title.clone(),
                    description.clone(),
                    None,
                    Vec::new(),
                    cx,
                ),
            ),
            Body::Refused { reason } => (
                "changes-refused",
                ui::empty_state(
                    IconName::ShieldX,
                    "Not permitted to read Kargo",
                    "The changes come from Kargo's projects, Freight and Stages, and reading \
                     them was refused. A refused list isn't an empty one.",
                    Some(reason.to_string()),
                    Vec::new(),
                    cx,
                ),
            ),
            Body::Failed { reason } => (
                "changes-failed",
                ui::empty_state(
                    IconName::CircleDashed,
                    "Couldn't read the changes",
                    "Nothing of Kargo was read; the changes show once it answers.",
                    Some(reason.to_string()),
                    vec![retry()],
                    cx,
                ),
            ),
        };
        Some(
            page::inset()
                .id("changes-state-scroll")
                .test_support()
                .flex_1()
                .min_h_0()
                .overflow_y_scroll()
                .restrict_scroll_to_axis()
                .child(
                    state
                        .h_auto()
                        .flex_none()
                        .id(id)
                        .test_support()
                        .role(Role::Status),
                )
                .into_any_element(),
        )
    }

    /// The selection: what the change is, each Stage's cell with why, and
    /// its trail, with Open change.
    fn render_detail(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        if !self.inspect {
            return None;
        }
        let row = self.selected_row()?;
        let p = palette(cx);
        let title = div()
            .id("changes-detail-title")
            .test_support()
            .aria_label(format!("{} {}", row.named, row.title))
            .min_w_0()
            .text_size(dp(14.))
            .font_weight(FontWeight::SEMIBOLD)
            .truncate()
            .follow_tooltip(row.title.clone())
            .child(format!("{} {}", row.named, row.title));
        let heading = h_flex().gap_2().min_w_0().child(title).child(
            div()
                .id("changes-detail-state")
                .test_support()
                .aria_label(row.state.clone())
                .flex_none()
                .follow_tooltip(row.state.clone())
                .child(ui::tag(row.tone, None, short(row.tone), cx)),
        );
        let mut fields = vec![
            field("Application", mono(row.project.clone()), cx),
            field("Freight", mono(row.freight.clone()), cx),
        ];
        if let Some(made) = &row.made {
            fields.push(field(
                "Made",
                div().child(format!("{made} · {} ago", row.age)),
                cx,
            ));
        }
        let stages = section("Stages", cx)
            .id("changes-detail-stages")
            .test_support()
            .children(
                row.stages
                    .iter()
                    .enumerate()
                    .map(|(ix, cell)| stage_line(ix, cell, cx)),
            );
        let trail = match &row.trail {
            Ok(groups) => section("Trail", cx)
                .id("changes-detail-trail")
                .test_support()
                .children(groups.iter().map(|group| trail_group(group, cx)))
                .into_any_element(),
            Err(why) => section("Trail", cx)
                .child(
                    div()
                        .id("changes-detail-no-trail")
                        .test_support()
                        .aria_label(why.clone())
                        .text_color(p.muted)
                        .child(why.clone()),
                )
                .into_any_element(),
        };
        let open = Button::new("changes-detail-open")
            .outline()
            .xsmall()
            .label("Open change")
            .tooltip_with_action("Open the change's page", &OpenChange, Some(CONTEXT))
            .on_click(cx.listener(|this, _, window, cx| this.open_selected(window, cx)));
        Some(
            Inspector::new("changes-detail")
                .selection(row.key.clone())
                .heading(heading)
                .children(fields)
                .child(stages)
                .child(trail)
                .footer(Some(open))
                .render(cx)
                .into_any_element(),
        )
    }

    fn render_table(&mut self, window: &mut Window, cx: &mut Context<Self>) -> (AnyElement, f32) {
        let table = div()
            .id("changes-table")
            .test_support()
            .flex()
            .flex_col()
            .size_full()
            .min_w_0()
            .min_h_0()
            .child(kit::data_table(self, window, cx).flex_1().min_h_0())
            .into_any_element();
        let beside = freshkube_ui::page::page_width(window) >= inspector::SPLIT_WIDTH;
        let details = self.render_detail(cx);
        let least = self.split.short_height(beside, details.is_some());
        let split = inspector::split("changes-split", &self.split, beside, table, details, window);
        (split, least)
    }
}

/// The tag's short words for a row's tone.
fn short(tone: Tone) -> &'static str {
    match tone {
        Tone::Good => "Confirmed",
        Tone::Info => "Claimed",
        Tone::Outline => "Not running",
        _ => "Not confirmed",
    }
}

fn section(title: &str, cx: &App) -> Div {
    v_flex()
        .gap(dp(7.))
        .text_size(dp(12.))
        .child(ui::caption(title, cx))
}

/// A Stage, its glyph and word, and why under them.
fn stage_line(ix: usize, cell: &Cell, cx: &mut Context<ChangesPage>) -> AnyElement {
    let p = palette(cx);
    v_flex()
        .id(SharedString::from(format!("changes-detail-stage-{ix}")))
        .test_support()
        .aria_label(format!("{}: {}. {}", cell.stage, cell.word, cell.why))
        .gap(dp(2.))
        .child(
            h_flex()
                .gap(dp(6.))
                .items_center()
                .children(cell.tone.and_then(|tone| ui::status_glyph(tone, cx)))
                .child(
                    div()
                        .font_weight(FontWeight::MEDIUM)
                        .child(cell.stage.clone()),
                )
                .child(div().text_color(p.ink_2).child(cell.word.clone())),
        )
        .child(
            div()
                .text_color(p.muted)
                .whitespace_normal()
                .child(cell.why.clone()),
        )
        .children(cell.after.as_ref().map(|line| outcome_line(line, cx)))
        .into_any_element()
}

/// A text link before a read, then the change page's exact summary.
fn outcome_line(line: &super::outcome::Line, cx: &mut Context<ChangesPage>) -> AnyElement {
    let Some(summary) = &line.summary else {
        return Button::new(line.id.clone())
            .link()
            .xsmall()
            .h_auto()
            .p_0()
            .whitespace_normal()
            .w_full()
            .min_w_0()
            .accessibility_label(line.label.clone())
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .whitespace_normal()
                    .text_size(dp(12.))
                    .child(line.label.clone()),
            )
            .on_click(cx.listener(|this, _, window, cx| this.open_selected(window, cx)))
            .into_any_element();
    };
    h_flex()
        .id(line.id.clone())
        .test_support()
        .aria_label(line.label.clone())
        .gap(dp(4.))
        .flex_wrap()
        .items_center()
        .child(super::outcome::AFTER_DEPLOY)
        .children(summary.mark.as_ref().and_then(|mark| {
            ui::status_glyph(mark.tone, cx).map(|glyph| {
                div()
                    .id(line.mark_id.clone())
                    .test_support()
                    .aria_label(mark.tooltip.clone())
                    .follow_tooltip(mark.tooltip.clone())
                    .child(glyph)
            })
        }))
        .child(
            div()
                .min_w_0()
                .whitespace_normal()
                .child(summary.words.clone()),
        )
        .children(summary.stale.as_ref().map(|why| {
            div()
                .id(line.stale_id.clone())
                .test_support()
                .aria_label(why.clone())
                .text_color(palette(cx).muted)
                .follow_tooltip(why.clone())
                .child("· last read")
        }))
        .into_any_element()
}

/// A group of the trail: its hops, each with its glyph, its link's word
/// and why.
fn trail_group(group: &TrailGroup, cx: &App) -> AnyElement {
    let p = palette(cx);
    v_flex()
        .gap(dp(4.))
        .child(
            div()
                .text_color(p.muted)
                .font_weight(FontWeight::MEDIUM)
                .child(group.label.clone()),
        )
        .children(group.hops.iter().map(|hop| {
            v_flex()
                .pl(dp(4.))
                .gap(dp(1.))
                .child(
                    h_flex()
                        .gap(dp(6.))
                        .items_center()
                        .children(ui::status_glyph(hop.tone, cx))
                        .child(div().min_w_0().truncate().child(hop.name.clone()))
                        .children(hop.link.map(|word| {
                            div()
                                .flex_none()
                                .text_color(p.ink_2)
                                .child(format!("· link {word}"))
                        })),
                )
                .child(
                    div()
                        .pl(dp(16.))
                        .text_color(p.muted)
                        .whitespace_normal()
                        .child(hop.why.clone()),
                )
        }))
        .into_any_element()
}

impl Render for ChangesPage {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let _span = crate::perf::span("page.render");
        if let Some((change, _)) = &self.change {
            return div()
                .flex()
                .flex_col()
                .size_full()
                .min_h_0()
                .child(change.clone());
        }
        let header = self.render_header(window, cx);
        let state = self.render_state(cx);
        // A state scrolls its full content beneath the fixed toolbar and
        // notes. Only a table needs the short window's minimum list height.
        let short = state.is_none() && page::is_short(window);
        let (body, least) = match state {
            Some(state) => (state, page::SHORT_LIST_HEIGHT),
            None => self.render_table(window, cx),
        };
        // The keys live on a wrapper drawn in every state.
        div()
            .key_context(CONTEXT)
            .track_focus(&self.focus)
            .on_action(cx.listener(|this, _: &NextChange, window, cx| this.step(1, window, cx)))
            .on_action(
                cx.listener(|this, _: &PreviousChange, window, cx| this.step(-1, window, cx)),
            )
            .on_action(cx.listener(|this, _: &ClearChange, _, cx| this.clear_selection(cx)))
            .on_action(cx.listener(|this, _: &OpenChange, window, cx| {
                if this.focus.is_focused(window) {
                    this.open_selected(window, cx);
                } else {
                    cx.propagate();
                }
            }))
            .flex()
            .flex_col()
            .size_full()
            .min_h_0()
            .child(
                page::page("changes-page")
                    .track_scroll(&self.page_scroll)
                    .when(short, |this| {
                        this.overflow_y_scroll().restrict_scroll_to_axis()
                    })
                    .child(page::toolbar(cx).child(header))
                    .children(self.render_notes(cx))
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .flex_1()
                            .min_h_0()
                            .when(short, |this| this.min_h(dp(least)))
                            .child(body),
                    ),
            )
    }
}
