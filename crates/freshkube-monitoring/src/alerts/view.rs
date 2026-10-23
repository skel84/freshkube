//! The page: its header with the filter, the notes on what was read in an
//! inset, then the table edge to edge with the selection in the Inspector;
//! or, when there is nothing to list, the state that says why.
use freshkube_ui::inspector::{self, Inspector};
use freshkube_ui::page::{self, PageHeader};
use freshkube_ui::palette::palette;
use freshkube_ui::tooltip::FollowTooltip as _;
use freshkube_ui::ui::{self, MONO_FONT, dp, field, mono};
use gpui_kit::assets::IconName;
use gpui_kit::component::{
    Disableable, Icon, Sizable,
    button::{Button, ButtonVariants},
    h_flex,
    input::Input,
    v_flex,
};
use gpui_kit::prelude::*;
use gpui_kit::*;

use super::display::{Body, Detail, State, StateIcon};
use super::{AlertsPage, ClearAlert, NextAlert, PreviousAlert, Which, object_of};
use freshkube_ui::table as kit;

impl AlertsPage {
    fn render_header(&mut self, window: &mut Window, cx: &mut Context<Self>) -> Div {
        let prefix = self.which.slug();
        let header = PageHeader::new(prefix, self.which.title());
        let input = self.input(window, cx);
        let filter = div().child(
            Input::new(&input)
                .id(SharedString::from(format!("{prefix}-filter")))
                .small()
                .h(dp(ui::CONTROL_HEIGHT))
                .cleanable(true)
                .aria_label(format!("Filter the {}", self.which.slug()))
                .prefix(Icon::new(IconName::Search).size(dp(14.))),
        );
        let reading = self.loader.is_loading();
        let label = match self.which {
            Which::Alerts => "Read the alerts again",
            Which::Rules => "Read the rules again",
            Which::Silences => "Read the silences again",
        };
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
        let mut header = header.filter(filter);
        if self.which == Which::Alerts {
            let shown = self.filter.muted;
            let toggle = ui::choice(
                Button::new(header.id("muted"))
                    .outline()
                    .small()
                    .h(dp(ui::CONTROL_HEIGHT))
                    .label("Silenced and inhibited")
                    .accessibility_label("Show silenced and inhibited alerts")
                    .tooltip("Show the alerts that are silenced or inhibited too"),
                shown,
            )
            .on_click(cx.listener(|this, _, _, cx| this.toggle_muted(cx)));
            header = header.control(toggle);
        }
        header.control(refresh).render(window, cx)
    }

    /// What may be stale or missing, and what was read, as banners.
    fn render_notes(&self, cx: &App) -> Option<AnyElement> {
        if self.display.notes.is_empty() {
            return None;
        }
        let prefix = self.which.slug();
        let banners = self.display.notes.iter().map(|note| {
            ui::banner(note.tone, note.lead.clone(), note.text.clone(), None, cx)
                .id(SharedString::from(format!("{prefix}-note-{}", note.id)))
                .aria_label(note.text.clone())
                .test_support()
                .role(Role::Status)
        });
        Some(
            page::inset()
                .id(SharedString::from(format!("{prefix}-notes")))
                .test_support()
                .flex()
                .flex_col()
                .gap(dp(page::PANE_PADDING_Y))
                .children(banners)
                .into_any_element(),
        )
    }

    /// With no rows: why, in words, and Retry where asking again can help.
    fn render_state(&self, state: &State, cx: &mut Context<Self>) -> AnyElement {
        let prefix = self.which.slug();
        let retrying = self.loader.is_loading();
        let actions = if state.retry {
            let label = if retrying { "Retrying…" } else { "Retry" };
            vec![
                Button::new(SharedString::from(format!("{prefix}-retry")))
                    .primary()
                    .icon(ui::refresh_icon(retrying, cx))
                    .label(label)
                    .accessibility_label(label)
                    .disabled(retrying)
                    .on_click(cx.listener(|this, _, _, cx| this.refresh(cx)))
                    .into_any_element(),
            ]
        } else {
            Vec::new()
        };
        let icon = match state.icon {
            StateIcon::Quiet => IconName::CircleCheck,
            StateIcon::Refused => IconName::ShieldX,
            StateIcon::Missing => IconName::BellOff,
            StateIcon::Broken => IconName::CircleDashed,
            StateIcon::Filtered => IconName::Search,
        };
        let view = ui::empty_state(
            icon,
            state.title.clone(),
            state.description.clone(),
            state.detail.as_ref().map(ToString::to_string),
            actions,
            cx,
        );
        page::inset()
            .id(SharedString::from(format!("{prefix}-state-scroll")))
            .test_support()
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .restrict_scroll_to_axis()
            .child(
                view.h_auto()
                    .flex_none()
                    .id(SharedString::from(state.id))
                    .test_support()
                    .role(Role::Status),
            )
            .into_any_element()
    }

    /// The selection: what it is, its fields, text and labels.
    fn render_detail(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        if !self.inspect {
            return None;
        }
        let key = self.selected.as_ref()?;
        let detail = &self.display.rows.iter().find(|row| &row.key == key)?.detail;
        let prefix = self.which.slug();
        let id = format!("{prefix}-detail");
        let p = palette(cx);
        let title = div()
            .id(SharedString::from(format!("{id}-title")))
            .test_support()
            .aria_label(detail.title.clone())
            .min_w_0()
            .text_size(dp(14.))
            .font_weight(FontWeight::SEMIBOLD)
            .truncate()
            .follow_tooltip(detail.title.clone())
            .child(detail.title.clone());
        let heading = h_flex().gap_2().min_w_0().child(title).child(
            div()
                .id(SharedString::from(format!("{id}-state")))
                .test_support()
                .aria_label(detail.state.clone())
                .flex_none()
                .child(ui::tag(detail.tone, None, detail.state.clone(), cx)),
        );
        let fields: Vec<Div> = detail
            .fields
            .iter()
            .map(|(label, value, is_mono)| {
                let value = value.clone();
                field(
                    label,
                    if *is_mono {
                        mono(value)
                    } else {
                        div().child(value)
                    },
                    cx,
                )
            })
            .collect();
        let text = |name: &str, body: &SharedString| {
            section(name, cx).child(
                div()
                    .id(SharedString::from(format!("{id}-{}", name.to_lowercase())))
                    .test_support()
                    .whitespace_normal()
                    .text_color(p.ink_2)
                    .child(body.clone()),
            )
        };
        let mut inspector = Inspector::new(id.clone())
            .selection(key)
            .heading(heading)
            .children(fields);
        if let Some(summary) = &detail.summary {
            inspector = inspector.child(text("Summary", summary));
        }
        if let Some(description) = &detail.description
            && detail.summary.as_ref() != Some(description)
        {
            inspector = inspector.child(text("Description", description));
        }
        if let Some(error) = &detail.error {
            inspector = inspector.child(
                section("Last error", cx).child(
                    div()
                        .id(SharedString::from(format!("{id}-last-error")))
                        .test_support()
                        .whitespace_normal()
                        .text_color(p.crit_ink)
                        .child(error.clone()),
                ),
            );
        }
        if let Some(query) = &detail.query {
            inspector = inspector.child(
                section("Expression", cx).child(
                    div()
                        .id(SharedString::from(format!("{id}-query")))
                        .test_support()
                        .font_family(MONO_FONT)
                        .text_size(dp(11.5))
                        .whitespace_normal()
                        .text_color(p.ink_2)
                        .child(query.clone()),
                ),
            );
        }
        if !detail.labels.is_empty() {
            inspector = inspector.child(self.labels_section(detail, &id, cx));
        }
        if !detail.matchers.is_empty() {
            inspector = inspector.child(
                section("Matchers", cx)
                    .id(SharedString::from(format!("{id}-matchers")))
                    .test_support()
                    .children(
                        detail
                            .matchers
                            .iter()
                            .map(|matcher| mono(matcher.clone()).text_size(dp(11.5))),
                    ),
            );
        }
        if self.which == Which::Silences {
            inspector = inspector.child(muted_section(detail, &id, cx));
        }
        let runbook = detail.runbook.clone().map(|address| {
            let open = address.clone();
            Button::new(SharedString::from(format!("{id}-runbook")))
                .outline()
                .xsmall()
                .label("Open runbook")
                .tooltip(address.clone())
                .accessibility_label(format!("Open the runbook, {address}"))
                .on_click(cx.listener(move |_, _, _, cx| cx.open_url(&open)))
        });
        Some(inspector.footer(runbook).render(cx).into_any_element())
    }

    /// Every label as a chip; one that names an object opens it.
    fn labels_section(&self, detail: &Detail, id: &str, cx: &mut Context<Self>) -> AnyElement {
        let p = palette(cx);
        section("Labels", cx)
            .id(SharedString::from(format!("{id}-labels")))
            .test_support()
            .child(
                h_flex()
                    .flex_wrap()
                    .gap(dp(5.))
                    .children(detail.labels.iter().map(|(name, value)| {
                        let text = format!("{name}={value}");
                        let chip_id = SharedString::from(format!("{id}-label-{name}"));
                        let object = object_of(name, value, &detail.labels);
                        let (name, value) = (name.to_string(), value.to_string());
                        // One chip, whether it opens an object or not.
                        let chip = div()
                            .id(chip_id)
                            .test_support()
                            .h(dp(20.))
                            .px(dp(6.))
                            .flex()
                            .items_center()
                            .rounded(px(3.))
                            .border_1()
                            .border_color(p.line)
                            .bg(p.surface_2)
                            .text_size(dp(11.5))
                            .font_family(MONO_FONT)
                            .text_color(p.ink_2)
                            .aria_label(text.clone())
                            .child(text.clone());
                        match object {
                            Some(object) => chip
                                .role(Role::Button)
                                .cursor_pointer()
                                .text_color(p.accent)
                                .hover(|style| style.bg(p.hover))
                                .follow_tooltip(format!("Open {} {}", object.kind, object.name))
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.open_label(&name, &value, cx)
                                }))
                                .into_any_element(),
                            None => chip.into_any_element(),
                        }
                    })),
            )
            .into_any_element()
    }

    fn render_table(&mut self, window: &mut Window, cx: &mut Context<Self>) -> (AnyElement, f32) {
        let prefix = self.which.slug();
        let table = div()
            .id(SharedString::from(format!("{prefix}-table")))
            .test_support()
            .flex()
            .flex_col()
            .size_full()
            .min_w_0()
            .min_h_0()
            .child(kit::data_table(self, window, cx).flex_1().min_h_0())
            .into_any_element();
        let beside = page::page_width(window) >= inspector::SPLIT_WIDTH;
        let details = self.render_detail(cx);
        let least = self.split.short_height(beside, details.is_some());
        let split = inspector::split(
            format!("{prefix}-split"),
            &self.split,
            beside,
            table,
            details,
            window,
        );
        (split, least)
    }
}

fn section(title: &str, cx: &App) -> Div {
    v_flex()
        .gap(dp(7.))
        .text_size(dp(12.))
        .child(ui::caption(title, cx))
}

/// The alerts a silence mutes now, by name.
fn muted_section(detail: &Detail, id: &str, cx: &App) -> AnyElement {
    let p = palette(cx);
    let body = if detail.muted.is_empty() {
        div()
            .text_color(p.muted)
            .child("It mutes no alert now")
            .into_any_element()
    } else {
        v_flex()
            .gap(dp(3.))
            .children(detail.muted.iter().map(|name| div().child(name.clone())))
            .into_any_element()
    };
    section("Muting now", cx)
        .id(SharedString::from(format!("{id}-muting")))
        .test_support()
        .child(body)
        .into_any_element()
}

impl Render for AlertsPage {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let prefix = self.which.slug();
        let header = self.render_header(window, cx);
        let state = match &self.display.body {
            Body::State(state) => Some(state.clone()),
            _ => None,
        };
        // A state scrolls its full content beneath the fixed toolbar and
        // notes. Only a table needs the short window's minimum list height.
        let short = state.is_none() && page::is_short(window);
        let (body, least) = match state {
            Some(state) => (self.render_state(&state, cx), page::SHORT_LIST_HEIGHT),
            None => self.render_table(window, cx),
        };
        // The keys live on a wrapper drawn in every state.
        div()
            .key_context(self.which.context())
            .track_focus(&self.focus)
            .on_action(cx.listener(|this, _: &NextAlert, window, cx| this.step(1, window, cx)))
            .on_action(cx.listener(|this, _: &PreviousAlert, window, cx| this.step(-1, window, cx)))
            .on_action(cx.listener(|this, _: &ClearAlert, _, cx| this.clear_selection(cx)))
            .flex()
            .flex_col()
            .size_full()
            .min_h_0()
            .child(
                page::page(SharedString::from(format!("{prefix}-page")))
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
