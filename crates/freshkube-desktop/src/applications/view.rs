//! The page: its header, then the notes on what was read in an inset, and
//! the table edge to edge with the selection in the Inspector; or, when
//! nothing was found, the state that says why in place of the table.
use super::display::{AppRow, Body};
use super::*;
use crate::screens::{field, mono};
use crate::ui::{self, dp};
use freshkube_ui::inspector::{self, Inspector};
use freshkube_ui::page::{self, PageHeader};
use freshkube_ui::palette::palette;
use freshkube_ui::tooltip::FollowTooltip as _;
use gpui_kit::assets::IconName;
use gpui_kit::component::ActiveTheme as _;
use gpui_kit::component::h_flex;
use gpui_kit::component::{
    Disableable, Icon, Sizable,
    button::{Button, ButtonVariants},
    input::Input,
};

/// The page width, in dp, from which the chips say their words beside
/// their counts.
const CHIP_WORDS_WIDTH: f32 = 960.;

impl ApplicationsPage {
    fn render_header(&self, window: &mut Window, cx: &mut Context<Self>) -> Div {
        let header = PageHeader::new(PREFIX, "Applications");
        let filter = div().child(
            Input::new(&self.filter)
                .id("applications-filter")
                .small()
                .h(dp(ui::CONTROL_HEIGHT))
                .cleanable(true)
                .aria_label("Filter applications by name, what found them or cluster")
                .prefix(Icon::new(IconName::Search).size(dp(14.))),
        );
        let wide = crate::screens::page_width(window) >= CHIP_WORDS_WIDTH;
        let chips = (!self.display.rows.is_empty()).then(|| {
            kit::status_chips(
                header.id("tally"),
                marks().map(|mark| {
                    let id = header.id(&format!("tally-{}", mark.slug()));
                    // The words beside the count when there is room; the
                    // tooltip and the label always have them.
                    let words = wide.then(|| {
                        div()
                            .id(SharedString::from(format!("{id}-words")))
                            .test_support()
                            .font_family(cx.theme().font_family.clone())
                            .child(mark.short())
                    });
                    kit::status_chip(
                        id,
                        table::tone(mark),
                        self.counts[mark.index()],
                        mark.what(),
                        self.mark == Some(mark),
                        cx,
                    )
                    .children(words)
                    .on_click(cx.listener(move |this, _, _, cx| this.toggle_mark(mark, cx)))
                }),
                cx,
            )
        });
        let reading = self.pending;
        let label = "Read the applications again";
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
        header
            .filter(filter)
            .chips(chips)
            .control(refresh)
            .render(window, cx)
    }

    /// What may be missing, as a warning, and what isn't served or was
    /// read in one namespace, as plain facts; a refresh that failed over
    /// an earlier read says so first. Every string was derived with the
    /// display.
    fn render_notes(&self, cx: &App) -> Option<AnyElement> {
        let p = palette(cx);
        let refresh = self.stale.as_ref().map(|(text, label)| {
            ui::warning_banner(Some("Couldn't read again".into()), text.clone(), None, cx)
                .id("applications-stale")
                .aria_label(label.clone())
                .test_support()
                .role(Role::Status)
        });
        let missing = self.display.missing_text.as_ref().map(|(text, label)| {
            ui::warning_banner(Some("May be missing".into()), text.clone(), None, cx)
                .id("applications-missing")
                .aria_label(label.clone())
                .test_support()
                .role(Role::Status)
        });
        let legend = self.display.legend_text.as_ref().map(|text| {
            div()
                .id("applications-legend")
                .test_support()
                .aria_label(text.clone())
                .text_size(dp(12.))
                .text_color(p.muted)
                .child(text.clone())
        });
        if refresh.is_none() && missing.is_none() && legend.is_none() {
            return None;
        }
        Some(
            page::inset()
                .id("applications-notes")
                .test_support()
                .flex()
                .flex_col()
                .gap(dp(page::PANE_PADDING_Y))
                .children(refresh)
                .children(missing)
                .children(legend)
                .into_any_element(),
        )
    }

    /// The state in place of the table when nothing was found: refused and
    /// failed say what wasn't read; only when every source answered is it
    /// empty.
    fn render_state(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        // A retry in flight says so, and another press does nothing.
        let retrying = self.pending;
        let retry = || {
            let label = if retrying { "Retrying…" } else { "Retry" };
            Button::new("applications-retry")
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
                "applications-none",
                ui::empty_state(
                    IconName::Layers,
                    title.clone(),
                    description.clone(),
                    None,
                    Vec::new(),
                    cx,
                ),
            ),
            Body::Refused {
                title,
                description,
                reason,
            } => (
                "applications-refused",
                ui::empty_state(
                    IconName::ShieldX,
                    title.clone(),
                    description.clone(),
                    Some(reason.to_string()),
                    Vec::new(),
                    cx,
                ),
            ),
            Body::Failed {
                title,
                description,
                reason,
            } => (
                "applications-failed",
                ui::empty_state(
                    IconName::CircleDashed,
                    title.clone(),
                    description.clone(),
                    Some(reason.to_string()),
                    vec![retry()],
                    cx,
                ),
            ),
        };
        Some(
            page::inset()
                .flex_1()
                .child(state.id(id).test_support().role(Role::Status))
                .into_any_element(),
        )
    }

    /// The selection's details; nothing while no row is selected.
    fn render_details(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        if !self.inspect {
            return None;
        }
        let row: &AppRow = self.selected_row()?;
        let p = palette(cx);
        let title = div()
            .id("applications-detail-title")
            .test_support()
            .aria_label(row.name.clone())
            .min_w_0()
            .text_size(dp(14.))
            .font_weight(FontWeight::SEMIBOLD)
            .truncate()
            .follow_tooltip(row.name.clone())
            .child(row.name.clone());
        // The tag keeps to its short words, so the name keeps its room;
        // the whole sentence is its tooltip and label.
        let mark = div()
            .id("applications-detail-mark")
            .test_support()
            .aria_label(row.mark_words.clone())
            .flex_none()
            .follow_tooltip(row.mark_words.clone())
            .child(ui::tag(
                table::tone(row.mark),
                None,
                row.mark_short.clone(),
                cx,
            ));
        // A Kargo Project and an Argo CD Application may share a name.
        let what = div()
            .id("applications-detail-what")
            .test_support()
            .aria_label(row.what.clone())
            .flex_none()
            .text_color(p.muted)
            .child(row.what.clone());
        let heading = h_flex()
            .gap_2()
            .min_w_0()
            .child(title)
            .child(what)
            .child(mark);
        let notes = row.notes.iter().enumerate().map(|(ix, note)| {
            div()
                .id(SharedString::from(format!("applications-detail-note-{ix}")))
                .test_support()
                .aria_label(note.clone())
                .text_size(dp(12.5))
                .text_color(p.muted)
                .child(note.clone())
        });
        Some(
            Inspector::new("applications-detail")
                .heading(heading)
                .children(row.fields.iter().map(|(label, value)| {
                    field(label, mono(value.clone()).whitespace_normal(), cx)
                }))
                .children(notes)
                .render(cx)
                .into_any_element(),
        )
    }

    /// The table in the list's key context, with the Inspector beside it
    /// on a wide page and under it on a narrow one, and the height the
    /// split keeps while a short page scrolls its frame.
    fn render_table(&mut self, window: &mut Window, cx: &mut Context<Self>) -> (AnyElement, f32) {
        let table = div()
            .id("applications-table")
            .test_support()
            .flex()
            .flex_col()
            .size_full()
            .min_w_0()
            .min_h_0()
            .child(kit::data_table(self, window, cx).flex_1().min_h_0())
            .into_any_element();
        let beside = crate::screens::page_width(window) >= inspector::SPLIT_WIDTH;
        let details = self.render_details(cx);
        let least = self.split.short_height(beside, details.is_some());
        let split = inspector::split(
            "applications-split",
            &self.split,
            beside,
            table,
            details,
            window,
        );
        (split, least)
    }
}

impl Render for ApplicationsPage {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let _span = crate::perf::span("page.render");
        self.refocus(window, cx);
        // An open application's page shows in the list's place.
        if let Some((open, _)) = &self.open {
            return div()
                .flex()
                .flex_col()
                .size_full()
                .min_h_0()
                .child(open.clone());
        }
        let header = self.render_header(window, cx);
        let (body, least) = match self.render_state(cx) {
            Some(state) => (state, page::SHORT_LIST_HEIGHT),
            None => self.render_table(window, cx),
        };
        let short = page::is_short(window);
        // The keys live on a wrapper drawn in every state.
        div()
            .key_context(CONTEXT)
            .track_focus(&self.focus)
            .on_action(cx.listener(|this, _: &NextApplication, _, cx| this.step(1, cx)))
            .on_action(cx.listener(|this, _: &PreviousApplication, _, cx| this.step(-1, cx)))
            .on_action(cx.listener(|this, _: &ClearApplication, _, cx| this.clear_selection(cx)))
            .on_action(
                cx.listener(|this, _: &OpenApplication, window, cx| this.open_selected(window, cx)),
            )
            .flex()
            .flex_col()
            .size_full()
            .min_h_0()
            .child(
                page::page("applications-page")
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
                            // A short page scrolls its frame down to the
                            // split's least heights, the Inspector's too.
                            .when(short, |this| this.min_h(dp(least)))
                            .child(body),
                    ),
            )
    }
}
