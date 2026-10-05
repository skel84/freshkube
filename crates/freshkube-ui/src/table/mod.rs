//! The table every list page draws, as Pods does: its cells, group rows,
//! the bars above its rows, its legend and the status chips that filter
//! it (docs/DESIGN.md#components).
use gpui_kit::base::ObservedElement as Observed;
use gpui_kit::component::{
    Selectable, Sizable,
    button::{Button, ButtonVariants},
    h_flex,
    tooltip::Tooltip,
};
use gpui_kit::prelude::*;
use gpui_kit::{
    AnyElement, App, Div, ElementId, FontWeight, Role, SharedString, Stateful, TestSupportExt, div,
};

use crate::palette::palette;

mod data;
use crate::ui::{self, MONO_FONT, Tone, dp};
pub use data::{
    DataTable, Line, RowStyle, SortOrder, TableRow, TableSource, TableState, data_table, reveal,
    step,
};

/// Comfortable rows, and the compact ones the density toggle picks. Group
/// rows take the same height, so the list stays uniform.
pub const ROW_HEIGHT: f32 = 34.;
pub const COMPACT_ROW_HEIGHT: f32 = 26.;
/// The header row's height.
pub const HEADER_HEIGHT: f32 = 30.;
/// The hover group of a row, for cells that brighten with it.
pub const ROW_GROUP: &str = "table-row";

/// What the table needs to know of a column.
pub trait TableColumn {
    fn label(&self) -> &SharedString;
    /// Its width, or with [`flexible`](Self::flexible) its least width.
    fn width(&self) -> f32;
    /// Takes the room left over; at most one column does.
    fn flexible(&self) -> bool;
}

/// A cell's frame: padded, truncated, at its column's width.
pub fn cell(column: &impl TableColumn) -> Div {
    let cell = div().px_3().min_w_0().whitespace_nowrap().truncate();
    if column.flexible() {
        cell.flex_1().min_w(dp(column.width()))
    } else {
        cell.flex_none().w(dp(column.width()))
    }
}

/// A group's header row: its glyph and label, an optional subject such as
/// a node, the details after them and the group's actions.
pub struct GroupRow {
    id: ElementId,
    tone: Tone,
    label: SharedString,
    subject: Option<SharedString>,
    detail: Vec<String>,
    actions: Vec<AnyElement>,
    height: f32,
}

impl GroupRow {
    pub fn new(
        id: impl Into<ElementId>,
        tone: Tone,
        label: impl Into<SharedString>,
        height: f32,
    ) -> Self {
        Self {
            id: id.into(),
            tone,
            label: label.into(),
            subject: None,
            detail: Vec::new(),
            actions: Vec::new(),
            height,
        }
    }

    pub fn subject(mut self, subject: Option<impl Into<SharedString>>) -> Self {
        self.subject = subject.map(Into::into);
        self
    }

    /// The details, joined with `·`.
    pub fn detail(mut self, detail: Vec<String>) -> Self {
        self.detail = detail;
        self
    }

    pub fn action(mut self, action: impl IntoElement) -> Self {
        self.actions.push(action.into_any_element());
        self
    }

    pub fn render(self, cx: &App) -> Observed<Stateful<Div>> {
        let p = palette(cx);
        let color = match self.tone {
            Tone::Crit => p.crit_ink,
            Tone::Warn => p.warn_ink,
            Tone::Good => p.good_ink,
            _ => p.muted,
        };
        let detail = self.detail.join(" · ");
        h_flex()
            .id(self.id)
            .test_support()
            .role(Role::Heading)
            .aria_label(format!("{} · {detail}", self.label))
            .w_full()
            .h(dp(self.height))
            .px_3()
            .gap(dp(10.))
            .bg(p.track.opacity(0.45))
            .border_b_1()
            .border_color(p.line)
            .text_size(dp(12.))
            .children(ui::status_glyph(self.tone, cx))
            .child(
                div()
                    .flex_none()
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(color)
                    .child(self.label),
            )
            .when_some(self.subject, |this, subject| {
                this.child(div().flex_none().font_family(MONO_FONT).child(subject))
            })
            // The actions follow the text, so a table wider than its view
            // still shows them.
            .child(
                div()
                    .min_w_0()
                    .truncate()
                    .text_color(p.muted)
                    .child(format!("· {detail}")),
            )
            .child(h_flex().flex_none().gap_1().children(self.actions))
    }
}

/// The frame of a bar above the rows.
fn bar(id: impl Into<ElementId>, cx: &App) -> Observed<Stateful<Div>> {
    let p = palette(cx);
    h_flex()
        .bg(p.accent_soft)
        .id(id)
        .test_support()
        .role(Role::Status)
        .px_3()
        .py(dp(5.))
        .gap_2()
        .border_b_1()
        .border_color(p.line)
        .text_size(dp(12.5))
}

/// How many rows are marked, and what can be done with them.
pub fn selection_bar(
    id: impl Into<ElementId>,
    count: usize,
    actions: impl IntoIterator<Item = AnyElement>,
    cx: &App,
) -> Observed<Stateful<Div>> {
    bar(id, cx)
        .child(
            div()
                .flex_1()
                .font_weight(FontWeight::SEMIBOLD)
                .child(format!("{count} selected")),
        )
        .children(actions)
}

/// `Showing 40 of 212` while rows are folded, and the action that shows
/// them all.
pub fn showing_bar(
    id: impl Into<ElementId>,
    shown: usize,
    total: usize,
    show_all: impl IntoElement,
    cx: &App,
) -> Observed<Stateful<Div>> {
    bar(id, cx)
        .text_color(palette(cx).ink_2)
        .flex_none()
        .child(
            div()
                .flex_1()
                .min_w_0()
                .child(format!("Showing {shown} of {total}")),
        )
        .child(show_all)
}

/// The table's footer: what its marks mean, wrapping.
pub fn legend(items: impl IntoIterator<Item = AnyElement>, cx: &App) -> Div {
    let p = palette(cx);
    h_flex()
        .flex_none()
        .px(dp(12.))
        .py(dp(7.))
        .gap(dp(12.))
        .flex_wrap()
        .border_t_1()
        .border_color(p.line)
        .text_size(dp(11.))
        .text_color(p.muted)
        .children(items)
}

/// One entry of a [`legend`]: an example mark and what it means.
pub fn legend_item(mark: impl IntoElement, text: impl Into<SharedString>) -> Div {
    h_flex().gap(dp(5.)).child(mark).child(text.into())
}

/// The legend in a narrow table: one line, the whole legend in its tooltip.
pub fn legend_line(
    id: impl Into<ElementId>,
    text: impl Into<SharedString>,
    tooltip: &'static str,
    cx: &App,
) -> Stateful<Div> {
    let p = palette(cx);
    h_flex()
        .id(id)
        .h(dp(26.))
        .flex_none()
        .px(dp(12.))
        .border_t_1()
        .border_color(p.line)
        .text_size(dp(11.))
        .text_color(p.muted)
        .child(text.into())
        .tooltip(move |window, cx| Tooltip::new(tooltip).build(window, cx))
}

/// One status chip: the glyph and how many rows it marks. The caller
/// handles the click, which filters to that status or clears the filter.
pub fn status_chip(
    id: impl Into<ElementId>,
    tone: Tone,
    count: usize,
    what: &str,
    selected: bool,
    cx: &App,
) -> Button {
    let p = palette(cx);
    Button::new(id)
        .ghost()
        .small()
        .px(dp(6.))
        .selected(selected)
        .children(ui::status_glyph(tone, cx))
        .child(div().child(count.to_string()))
        .text_color(p.ink_2)
        .accessibility_label(format!("{count} {what}"))
        .tooltip(format!("{count} {what} · click to filter"))
}

/// The row of [`status_chip`]s, critical first.
pub fn status_chips(
    id: impl Into<ElementId>,
    chips: impl IntoIterator<Item = Button>,
    cx: &App,
) -> Observed<Stateful<Div>> {
    h_flex()
        .id(id)
        .test_support()
        .gap(dp(2.))
        .font_family(MONO_FONT)
        .text_size(dp(12.))
        .text_color(palette(cx).muted)
        .children(chips)
}
