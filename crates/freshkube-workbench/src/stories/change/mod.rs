//! The change page (docs/DESIGN.md, "The change page"): one change, from
//! the pull request to the pods that run it, as a trail of hops on the
//! shared table, grouped by phase and by Stage, with the selected hop in
//! the shared Inspector. The data is invented ([`trail`]); the story reads
//! nothing and opens nothing, and says where an action would lead.

mod detail;
pub mod trail;

use std::collections::BTreeSet;

use freshkube_ui::inspector::{self, InspectorSplit, Pane, Stacked};
use freshkube_ui::page::{self, PageHeader};
use freshkube_ui::palette::palette;
use freshkube_ui::table::{
    self, GLYPH_WIDTH, GroupRow, Line, ROW_HEIGHT, RowStyle, SortOrder, TableColumn, TableRow,
    TableSource, TableState,
};
use freshkube_ui::ui::{self, MONO_FONT, Tone, dp};
use gpui_kit::assets::IconName;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::{Sizable, h_flex};
use gpui_kit::prelude::*;
use gpui_kit::{
    AnyElement, AnyView, App, ClickEvent, Context, Div, ElementId, SharedString, Window, div,
};

use trail::{Confidence, Trail};

/// The id prefix of everything the story draws.
const PREFIX: &str = "change-trail";
/// The table's own ids' prefix: `change-trail-hops-list`, `change-trail-hops-rows`.
const TABLE: &str = "change-trail-hops";

pub fn build(window: &mut Window, cx: &mut App) -> AnyView {
    let story = cx.new(ChangeStory::new);
    // The first row selected sits below a short window's stacked table.
    table::reveal_when_settled(story.downgrade(), |_| true, window);
    story.into()
}

/// The row selected when the story opens: prod-ams waiting for promotion,
/// whose Inspector shows a Stage's three gates apart.
pub const FIRST: &str = "prod-ams-promotion";

/// Stacked, the trail and the Inspector share the height evenly: the
/// trail is what the page is for, and a short window keeps rows of it.
const STACKED: Stacked = Stacked {
    lead: Pane::new(280.).least(inspector::LIST_MIN_HEIGHT),
    trail: Pane::new(280.).least(inspector::MIN_HEIGHT),
};

/// The status chips, critical first, and what each counts.
const TONES: [(Tone, &str); 4] = [
    (Tone::Crit, "failing"),
    (Tone::Warn, "warning"),
    (Tone::Unknown, "waiting or unknown"),
    (Tone::Good, "ok"),
];

#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    Glyph,
    Hop,
    Detail,
    From,
    Link,
    Time,
}

pub struct Column {
    kind: Kind,
    label: SharedString,
    width: f32,
}

impl TableColumn for Column {
    fn label(&self) -> &SharedString {
        &self.label
    }

    fn width(&self) -> f32 {
        self.width
    }

    fn flexible(&self) -> bool {
        self.kind == Kind::Detail
    }

    fn pinned(&self) -> bool {
        matches!(self.kind, Kind::Glyph | Kind::Hop)
    }
}

/// One line of the table.
enum Item {
    Group(usize),
    Row(usize),
}

/// A row's cells' text, made with the trail.
pub struct Cells {
    /// The row's id: `change-trail-hop-<key>`.
    id: SharedString,
    name: SharedString,
    detail: SharedString,
    from: SharedString,
    time: SharedString,
    link: Option<Confidence>,
}

pub struct ChangeStory {
    table: TableState,
    columns: Vec<Column>,
    trail: Trail,
    cells: Vec<Cells>,
    /// Each group's tone: its worst row's.
    group_tones: Vec<Tone>,
    /// Derived from the trail, the folds and the filter.
    lines: Vec<Item>,
    /// How many rows each tone has, folded or not.
    counts: [usize; TONES.len()],
    /// The Stage groups folded: those whose every row is fine, while
    /// anything else needs a look.
    folded: BTreeSet<usize>,
    filter: Option<Tone>,
    selected: Option<&'static str>,
    split: InspectorSplit,
    /// What the last action pressed would have opened.
    opened: Option<SharedString>,
}

/// The status a row counts as: a blue dot (a Freight approved by hand,
/// past its upstream) is something to know, not a fault, so it counts as
/// ok in the chips, the filter and its group.
fn status(tone: Tone) -> Tone {
    match tone {
        Tone::Info => Tone::Good,
        tone => tone,
    }
}

/// How bad a tone is, for a group's worst row.
fn rank(tone: Tone) -> u8 {
    match tone {
        Tone::Crit | Tone::Died => 4,
        Tone::Warn => 3,
        Tone::Unknown => 2,
        _ => 1,
    }
}

impl ChangeStory {
    fn new(cx: &mut Context<Self>) -> Self {
        let column = |kind, label: &str, width| Column {
            kind,
            label: label.to_owned().into(),
            width,
        };
        let trail = trail::trail();
        let cells = (trail.hops.iter())
            .map(|hop| Cells {
                id: format!("{PREFIX}-hop-{}", hop.key).into(),
                name: hop.name.into(),
                detail: hop.detail.into(),
                from: hop.from.into(),
                time: hop.time.into(),
                link: hop.link,
            })
            .collect();
        let group_tones: Vec<Tone> = (0..trail.groups.len())
            .map(|group| {
                (trail.hops.iter())
                    .filter(|hop| hop.group == group)
                    .map(|hop| status(hop.tone))
                    .max_by_key(|tone| rank(*tone))
                    .unwrap_or(Tone::Good)
            })
            .collect();
        let anything_wrong = group_tones.iter().any(|tone| *tone != Tone::Good);
        let folded = (trail.groups.iter().enumerate())
            .filter(|(group, info)| {
                anything_wrong && info.stage.is_some() && group_tones[*group] == Tone::Good
            })
            .map(|(group, _)| group)
            .collect();
        let mut counts = [0; TONES.len()];
        for hop in &trail.hops {
            if let Some(ix) = TONES.iter().position(|(tone, _)| *tone == status(hop.tone)) {
                counts[ix] += 1;
            }
        }
        let mut story = Self {
            table: TableState::new(TABLE),
            columns: vec![
                column(Kind::Glyph, "", GLYPH_WIDTH),
                column(Kind::Hop, "Hop", 176.),
                column(Kind::Link, "Link", 96.),
                column(Kind::Detail, "Detail", 240.),
                column(Kind::Time, "Time", 64.),
                column(Kind::From, "Read from", 112.),
            ],
            trail,
            cells,
            group_tones,
            lines: Vec::new(),
            counts,
            folded,
            filter: None,
            selected: Some(FIRST),
            split: InspectorSplit::new("workbench-change", cx).stacked(STACKED),
            opened: None,
        };
        story.derive();
        story
    }

    pub fn selected(&self) -> Option<&'static str> {
        self.selected
    }

    /// The keys of the rows the table shows, in order.
    pub fn shown(&self) -> Vec<&'static str> {
        (self.lines.iter())
            .filter_map(|item| match item {
                Item::Row(ix) => Some(self.trail.hops[*ix].key),
                Item::Group(_) => None,
            })
            .collect()
    }

    pub fn opened(&self) -> Option<&SharedString> {
        self.opened.as_ref()
    }

    /// The lines, when the folds or the filter change; drawing only reads
    /// them. A filter shows its rows in every group, folded or not.
    fn derive(&mut self) {
        self.lines.clear();
        for group in 0..self.trail.groups.len() {
            let rows: Vec<usize> = (self.trail.hops.iter().enumerate())
                .filter(|(_, hop)| hop.group == group)
                .filter(|(_, hop)| self.filter.is_none_or(|tone| tone == status(hop.tone)))
                .map(|(ix, _)| ix)
                .collect();
            if rows.is_empty() {
                continue;
            }
            self.lines.push(Item::Group(group));
            if self.filter.is_none() && self.folded.contains(&group) {
                continue;
            }
            self.lines.extend(rows.into_iter().map(Item::Row));
        }
    }

    fn toggle_fold(&mut self, group: usize, cx: &mut Context<Self>) {
        if !self.folded.remove(&group) {
            self.folded.insert(group);
        }
        self.derive();
        cx.notify();
    }

    fn unfold_all(&mut self, cx: &mut Context<Self>) {
        self.folded.clear();
        self.derive();
        cx.notify();
    }

    fn toggle_filter(&mut self, tone: Tone, cx: &mut Context<Self>) {
        self.filter = (self.filter != Some(tone)).then_some(tone);
        self.derive();
        cx.notify();
    }

    fn press(&mut self, would: String, cx: &mut Context<Self>) {
        self.opened = Some(would.into());
        cx.notify();
    }

    fn render_header(&self, window: &mut Window, cx: &mut Context<Self>) -> Div {
        let header =
            PageHeader::new(PREFIX, "wonky-otter").parent("parent", "checkout", |_, _, _| {});
        let chips = table::status_chips(
            header.id("tally"),
            TONES.iter().enumerate().map(|(ix, (tone, what))| {
                let tone = *tone;
                let id = match tone {
                    Tone::Crit => "failing",
                    Tone::Warn => "warning",
                    Tone::Unknown => "waiting",
                    _ => "ok",
                };
                table::status_chip(
                    header.id(&format!("tally-{id}")),
                    tone,
                    self.counts[ix],
                    what,
                    self.filter == Some(tone),
                    cx,
                )
                .on_click(cx.listener(move |this, _, _, cx| this.toggle_filter(tone, cx)))
            }),
            cx,
        );
        let kargo = page::handler(cx, |this: &mut Self, _, cx| {
            this.press("Would open the Freight in Kargo.".into(), cx)
        });
        let copy = page::handler(cx, |this: &mut Self, _, cx| {
            this.press("Would copy a link to this change.".into(), cx)
        });
        let (kargo_id, copy_id) = (header.id("kargo"), header.id("copy"));
        header
            .chips(Some(chips))
            .foldable(
                control(kargo_id, "Open in Kargo ↗", kargo.clone()),
                page::item("Open in Kargo ↗", kargo),
            )
            .foldable(
                control(copy_id, "Copy link", copy.clone()),
                page::item("Copy link", copy),
            )
            .render(window, cx)
    }

    /// The table's width in dp: the page's, less the Inspector's beside it.
    fn table_width(&self, window: &Window) -> f32 {
        let page = page::page_width(window);
        if self.selected.is_some() && page >= inspector::SPLIT_WIDTH {
            page - self.split.width()
        } else {
            page
        }
    }

    /// The footer's count while Stages are folded.
    fn shown_rows(&self) -> usize {
        (self.lines.iter())
            .filter(|item| matches!(item, Item::Row(_)))
            .count()
    }
}

/// A header control: an outline button that runs `handler`, as its item in
/// the "…" menu does.
fn control(id: SharedString, label: &'static str, handler: page::Handler) -> Button {
    Button::new(id)
        .outline()
        .small()
        .h(dp(ui::CONTROL_HEIGHT))
        .label(label)
        .on_click(move |_, window, cx| handler(window, cx))
}

impl TableSource for ChangeStory {
    type Key = &'static str;
    type Sort = ();
    type Column = Column;
    type Row<'a> = &'a Cells;

    fn table_state(&self) -> &TableState {
        &self.table
    }

    fn columns(&self) -> &[Column] {
        &self.columns
    }

    fn width(&self) -> f32 {
        self.columns.iter().map(TableColumn::width).sum()
    }

    fn list_label(&self) -> String {
        "The change's hops, in the order it travels".into()
    }

    /// The trail keeps the order the change travels in, so no column
    /// sorts it.
    fn sorting(&self, _: &Column) -> Option<((), Option<SortOrder>)> {
        None
    }

    fn sort(&mut self, _: (), _: &mut Context<Self>) {}

    fn line_count(&self) -> usize {
        self.lines.len()
    }

    fn line(&self, line: usize, _: &App) -> Option<Line<&'static str, &Cells>> {
        Some(match self.lines.get(line)? {
            Item::Group(group) => Line::Group(*group),
            Item::Row(ix) => {
                let hop = &self.trail.hops[*ix];
                let cells = &self.cells[*ix];
                Line::Row(TableRow {
                    key: hop.key,
                    id: ElementId::from(cells.id.clone()),
                    label: cells.name.clone(),
                    tooltip: None,
                    marked: false,
                    muted: false,
                    data: cells,
                })
            }
        })
    }

    fn cell(
        &self,
        row: &TableRow<&'static str, &Cells>,
        style: &RowStyle,
        column: &Column,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let cells = row.data;
        let p = &style.p;
        let text = |text: &SharedString| table::cell(column).child(text.clone());
        match column.kind {
            Kind::Glyph => {
                let tone = (self.trail.hops.iter())
                    .find(|hop| hop.key == row.key)
                    .map_or(Tone::Unknown, |hop| hop.tone);
                table::glyph_cell(column)
                    .children(ui::status_glyph(tone, cx))
                    .into_any_element()
            }
            Kind::Hop => text(&cells.name).into_any_element(),
            // Cut at a word, its whole text in the tooltip (#522).
            Kind::Detail => table::word_cell(
                column,
                format!("change-hop-{}-detail", row.key),
                cells.detail.clone(),
            )
            .text_color(p.ink_2)
            .into_any_element(),
            Kind::From => text(&cells.from)
                .font_family(MONO_FONT)
                .text_color(p.ink_2)
                .into_any_element(),
            Kind::Link => match &cells.link {
                None => table::cell(column).into_any_element(),
                Some(confidence) => table::cell(column)
                    .flex()
                    .items_center()
                    .gap(dp(5.))
                    .children(
                        confidence
                            .tone()
                            .and_then(|tone| ui::status_glyph(tone, cx)),
                    )
                    .child(
                        div()
                            .flex_none()
                            .text_color(match confidence {
                                Confidence::Confirmed => p.muted,
                                _ => p.ink,
                            })
                            .child(confidence.word()),
                    )
                    .into_any_element(),
            },
            Kind::Time => text(&cells.time)
                .font_family(MONO_FONT)
                .text_color(p.muted)
                .into_any_element(),
        }
    }

    fn group(&self, group: usize, cx: &mut Context<Self>) -> Option<AnyElement> {
        let info = self.trail.groups.get(group)?;
        let tone = self.group_tones[group];
        let rows = (self.trail.hops.iter())
            .filter(|hop| hop.group == group)
            .count();
        let detail = match info.stage {
            Some(stage) => {
                let stage = &self.trail.stages[stage];
                vec![info.detail.to_owned(), stage.state.to_lowercase()]
            }
            None => vec![info.detail.to_owned()],
        };
        let mut row = GroupRow::new(
            SharedString::from(format!("{PREFIX}-group-{group}")),
            tone,
            info.label,
            ROW_HEIGHT,
        )
        .detail(detail);
        if info.stage.is_some() && self.filter.is_none() {
            let folded = self.folded.contains(&group);
            let what = if folded { "Expand" } else { "Collapse" };
            row = row
                .chevron(
                    Button::new(SharedString::from(format!("{PREFIX}-fold-{group}")))
                        .ghost()
                        .xsmall()
                        .icon(if folded {
                            IconName::ChevronRight
                        } else {
                            IconName::ChevronDown
                        })
                        .tooltip(format!("{what} {}", info.label))
                        .accessibility_label(format!("{what} {}", info.label))
                        .on_click(cx.listener(move |this, _, _, cx| {
                            cx.stop_propagation();
                            this.toggle_fold(group, cx);
                        })),
                )
                .after(if folded {
                    vec![format!("{rows} rows folded")]
                } else {
                    Vec::new()
                });
        }
        Some(row.render(cx).into_any_element())
    }

    fn empty(&self, _: &mut Context<Self>) -> Option<AnyElement> {
        self.lines
            .is_empty()
            .then(|| "No hops match this filter.".into_any_element())
    }

    fn selected_key(&self) -> Option<&&'static str> {
        self.selected.as_ref()
    }

    fn line_of(&self, key: &&'static str) -> Option<usize> {
        self.lines
            .iter()
            .position(|item| matches!(item, Item::Row(ix) if self.trail.hops[*ix].key == *key))
    }

    fn click(
        &mut self,
        key: &&'static str,
        _: &ClickEvent,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.selected = Some(*key);
        self.opened = None;
        cx.notify();
    }

    fn counts(&self, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let total = self.trail.hops.len();
        let shown = self.shown_rows();
        if shown == total || self.filter.is_some() {
            return Vec::new();
        }
        vec![
            table::showing(
                self.table.id("showing"),
                shown,
                total,
                Button::new(self.table.id("show-all"))
                    .ghost()
                    .xsmall()
                    .label(format!("Show all {total}"))
                    .on_click(cx.listener(|this, _, _, cx| this.unfold_all(cx))),
                cx,
            )
            .into_any_element(),
        ]
    }

    fn legend(&self, window: &Window, cx: &mut Context<Self>) -> Option<AnyElement> {
        const WHOLE: &str = "Link: how a hop joins the one before it. Confirmed: the key matches on both sides. Claimed: only a label, annotation or attestation says so. Unknown: a side can't be read.";
        if self.table_width(window) < 900. {
            return Some(
                table::legend_line(
                    self.table.id("legend"),
                    "Confirmed · Claimed · Unknown ⓘ",
                    WHOLE,
                    cx,
                )
                .into_any_element(),
            );
        }
        let item = |confidence: Confidence, text: &str| {
            let mark = h_flex()
                .gap(dp(5.))
                .children(
                    confidence
                        .tone()
                        .and_then(|tone| ui::status_glyph(tone, cx)),
                )
                .child(div().text_color(palette(cx).ink_2).child(confidence.word()));
            table::legend_item(mark, text.to_owned()).into_any_element()
        };
        Some(
            table::legend(
                [
                    item(Confidence::Confirmed, "the key matches on both sides"),
                    item(Confidence::Claimed, "only a label or attestation says so"),
                    item(Confidence::Unknown, "a side can't be read"),
                ],
                cx,
            )
            .into_any_element(),
        )
    }
}

impl Render for ChangeStory {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let header = self.render_header(window, cx);
        let beside = page::page_width(window) >= inspector::SPLIT_WIDTH;
        let table = table::data_table(self, window, cx)
            .size_full()
            .min_h_0()
            .into_any_element();
        let detail = self.render_detail(cx);
        page::page(SharedString::from(format!("{PREFIX}-page")))
            .child(page::toolbar(cx).child(header))
            .child(inspector::split(
                format!("{PREFIX}-split"),
                &self.split,
                beside,
                table,
                detail,
                window,
            ))
    }
}
