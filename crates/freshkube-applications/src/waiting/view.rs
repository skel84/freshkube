//! The page: its header with the find box, the notes on what may be
//! missing, then the table edge to edge with the selection in the
//! Inspector; or, with no rows, the state that says why.
use super::display::{Detail, TraceLine};
use super::*;
use crate::ui::{self, dp};
use freshkube_ui::inspector::{self, Inspector};
use freshkube_ui::page::{self, PageHeader};
use freshkube_ui::palette::palette;
use freshkube_ui::tooltip::FollowTooltip as _;
use freshkube_ui::ui::{field, mono};
use gpui_kit::assets::IconName;
use gpui_kit::component::{
    Disableable, Icon, Sizable,
    button::{Button, ButtonVariants},
    h_flex,
    input::Input,
    v_flex,
};

impl WaitingPage {
    fn render_header(&self, window: &mut Window, cx: &mut Context<Self>) -> Div {
        let header = PageHeader::new(PREFIX, "Waiting to ship");
        let reading = self.loader.is_loading();
        let label = "Read what waits to ship again";
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
        let find = div().child(
            Input::new(&self.find)
                .id("waiting-find")
                .small()
                .h(dp(ui::CONTROL_HEIGHT))
                .cleanable(true)
                .aria_label("Find a pull request number or a commit, and trace it to production")
                .prefix(Icon::new(IconName::Search).size(dp(14.))),
        );
        header.filter(find).control(refresh).render(window, cx)
    }

    /// A refresh that failed over an earlier read, then what may be
    /// missing.
    fn render_notes(&self, cx: &App) -> Option<AnyElement> {
        let setup = self.setup_problem.as_ref().map(|text| {
            ui::warning_banner(None, text.clone(), None, cx)
                .id("waiting-setup-problem")
                .aria_label(text.clone())
                .test_support()
                .role(Role::Status)
        });
        let refresh = self.stale.as_ref().map(|(text, label)| {
            ui::warning_banner(Some("Couldn't read again".into()), text.clone(), None, cx)
                .id("waiting-stale")
                .aria_label(label.clone())
                .test_support()
                .role(Role::Status)
        });
        let missing = self.display.missing.as_ref().map(|(text, label)| {
            ui::warning_banner(Some("May be missing".into()), text.clone(), None, cx)
                .id("waiting-missing")
                .aria_label(label.clone())
                .test_support()
                .role(Role::Status)
        });
        if setup.is_none() && refresh.is_none() && missing.is_none() {
            return None;
        }
        Some(
            page::inset()
                .id("waiting-notes")
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
    /// Kargo answered, or the find box found nothing, is it empty.
    fn render_state(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let retrying = self.loader.is_loading();
        let retry = || {
            let label = if retrying { "Retrying…" } else { "Retry" };
            Button::new("waiting-retry")
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
                "waiting-none",
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
                "waiting-refused",
                ui::empty_state(
                    IconName::ShieldX,
                    "Not permitted to read Kargo",
                    "What waits to ship comes from Kargo's projects, Freight and Stages, and \
                     reading them was refused. A refused list isn't an empty one.",
                    Some(reason.to_string()),
                    Vec::new(),
                    cx,
                ),
            ),
            Body::Failed { reason } => (
                "waiting-failed",
                ui::empty_state(
                    IconName::CircleDashed,
                    "Couldn't read what waits",
                    "Nothing of Kargo was read; the page fills once it answers.",
                    Some(reason.to_string()),
                    vec![retry()],
                    cx,
                ),
            ),
        };
        Some(
            page::inset()
                .id("waiting-state-scroll")
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

    /// The selection: what it is, why it waits, where it stands in each
    /// Stage, which Stages count as production, and what promoting now
    /// would ship.
    fn render_detail(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        if !self.inspect {
            return None;
        }
        let row = self.selected_row()?;
        let detail: &Detail = &row.detail;
        let p = palette(cx);
        let title = div()
            .id("waiting-detail-title")
            .test_support()
            .aria_label(detail.heading.clone())
            .min_w_0()
            .text_size(dp(14.))
            .font_weight(FontWeight::SEMIBOLD)
            .truncate()
            .follow_tooltip(detail.heading.clone())
            .child(detail.heading.clone());
        let heading = h_flex().gap_2().min_w_0().child(title).child(
            div()
                .id("waiting-detail-state")
                .test_support()
                .aria_label(row.state.clone())
                .flex_none()
                .follow_tooltip(row.state.clone())
                .child(ui::tag(detail.tag.0, None, detail.tag.1.clone(), cx)),
        );
        let fields: Vec<_> = detail
            .fields
            .iter()
            .map(|(label, value, code)| {
                let value = if *code {
                    mono(value.clone()).into_any_element()
                } else {
                    div().child(value.clone()).into_any_element()
                };
                field(label, value, cx)
            })
            .collect();
        let why = detail.why.as_ref().map(|why| {
            section("Why", cx)
                .id("waiting-detail-why")
                .test_support()
                .child(
                    div()
                        .text_color(p.ink_2)
                        .whitespace_normal()
                        .child(why.clone()),
                )
        });
        let trace = (!detail.trace.is_empty()).then(|| {
            section("Trace to production", cx)
                .id("waiting-detail-trace")
                .test_support()
                .children(
                    detail
                        .trace
                        .iter()
                        .enumerate()
                        .map(|(ix, line)| trace_line(ix, line, cx)),
                )
        });
        let production = (!detail.production.is_empty()).then(|| {
            section("Production", cx)
                .id("waiting-detail-production")
                .test_support()
                .children(lines(&detail.production, cx))
        });
        let ship = (!detail.ship.is_empty()).then(|| {
            section("Promoting now would ship", cx)
                .id("waiting-detail-ship")
                .test_support()
                .children(lines(&detail.ship, cx))
        });
        let basis = div()
            .id("waiting-detail-basis")
            .test_support()
            .aria_label(detail.basis.clone())
            .text_size(dp(12.))
            .text_color(p.muted)
            .whitespace_normal()
            .child(detail.basis.clone());
        Some(
            Inspector::new("waiting-detail")
                .heading(heading)
                .children(fields)
                .children(why)
                .children(trace)
                .children(production)
                .children(ship)
                .child(basis)
                .render(cx)
                .into_any_element(),
        )
    }

    fn render_table(&mut self, window: &mut Window, cx: &mut Context<Self>) -> (AnyElement, f32) {
        let table = div()
            .id("waiting-table")
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
        let split = inspector::split("waiting-split", &self.split, beside, table, details, window);
        (split, least)
    }
}

fn section(title: &str, cx: &App) -> Div {
    v_flex()
        .gap(dp(7.))
        .text_size(dp(12.))
        .child(ui::caption(title, cx))
}

fn lines(lines: &[SharedString], cx: &App) -> Vec<AnyElement> {
    let p = palette(cx);
    lines
        .iter()
        .map(|line| {
            div()
                .text_color(p.ink_2)
                .whitespace_normal()
                .child(line.clone())
                .into_any_element()
        })
        .collect()
}

/// A Stage, its glyph and where the Freight stands in it.
fn trace_line(ix: usize, line: &TraceLine, cx: &App) -> AnyElement {
    let p = palette(cx);
    h_flex()
        .id(SharedString::from(format!("waiting-detail-stage-{ix}")))
        .test_support()
        .aria_label(format!(
            "{}{}: {}",
            line.stage,
            if line.production { " (production)" } else { "" },
            line.word
        ))
        .gap(dp(6.))
        .items_center()
        .children(line.tone.and_then(|tone| ui::status_glyph(tone, cx)))
        .child(
            div()
                .font_weight(FontWeight::MEDIUM)
                .child(line.stage.clone()),
        )
        .when(line.production, |this| {
            this.child(div().text_color(p.muted).child("production"))
        })
        .child(div().text_color(p.ink_2).child(line.word.clone()))
        .into_any_element()
}

impl Render for WaitingPage {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let _span = crate::perf::span("page.render");
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
            .on_action(cx.listener(|this, _: &NextWaiting, window, cx| this.step(1, window, cx)))
            .on_action(
                cx.listener(|this, _: &PreviousWaiting, window, cx| this.step(-1, window, cx)),
            )
            .on_action(cx.listener(|this, _: &ClearWaiting, _, cx| this.clear_selection(cx)))
            .flex()
            .flex_col()
            .size_full()
            .min_h_0()
            .child(
                page::page("waiting-page")
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
