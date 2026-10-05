//! `DataTable` on invented pods: group rows, status glyphs, a pinned name
//! that stays while the columns scroll sideways, sorting, and the loading,
//! empty and failed states, with 24 rows or 2,000.

use freshkube_ui::page::{self, PageHeader};
use freshkube_ui::palette::palette;
use freshkube_ui::table::{
    self, GLYPH_WIDTH, GroupRow, Line, ROW_HEIGHT, RowStyle, SortOrder, TableColumn, TableRow,
    TableSource, TableState,
};
use freshkube_ui::ui::{self, MONO_FONT, Tone, dp};
use gpui_kit::assets::IconName;
use gpui_kit::component::button::Button;
use gpui_kit::component::{Sizable, h_flex};
use gpui_kit::prelude::*;
use gpui_kit::{
    AnyElement, AnyView, App, ClickEvent, Context, Div, ElementId, Role, SharedString,
    TestSupportExt, Window, div, px, relative,
};

/// The id prefix of everything the story draws.
const PREFIX: &str = "data-table";
/// The table's own ids' prefix: `pods-sort`, `pods-list`, `pods-empty`.
const TABLE: &str = "pods";

pub fn build(_: &mut Window, cx: &mut App) -> AnyView {
    cx.new(|_| DataTableStory::new()).into()
}

/// What the story shows in place of its body.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum State {
    Rows,
    Loading,
    Empty,
    Failed,
}

impl State {
    const ALL: [(State, &'static str, &'static str); 4] = [
        (State::Rows, "rows", "Rows"),
        (State::Loading, "loading", "Loading"),
        (State::Empty, "empty", "Empty"),
        (State::Failed, "failed", "Failed"),
    ];
}

/// How many invented pods the rows hold.
pub const COUNTS: [usize; 2] = [24, 2_000];

/// What a header click sorts by.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Sort {
    Name,
    Restarts,
    Age,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    Glyph,
    Name,
    Namespace,
    Cluster,
    Status,
    Restarts,
    Cpu,
    Memory,
    Node,
    Age,
    Image,
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
        self.kind == Kind::Image
    }

    fn pinned(&self) -> bool {
        matches!(self.kind, Kind::Glyph | Kind::Name)
    }
}

/// One invented pod, with its cells' text made when it is.
pub struct Pod {
    id: usize,
    name: SharedString,
    namespace: SharedString,
    cluster: SharedString,
    tone: Tone,
    status: SharedString,
    restarts: u32,
    restarts_text: SharedString,
    cpu: SharedString,
    memory: SharedString,
    node: SharedString,
    minutes: u32,
    age: SharedString,
    image: SharedString,
}

const APPS: [&str; 8] = [
    "shop-api",
    "shop-web",
    "shop-redis",
    "checkout",
    "search",
    "ledger-sync",
    "mailer",
    "media-resize",
];
const NAMESPACES: [&str; 3] = ["app-a", "app-b", "shop"];

impl Pod {
    /// The pod numbered `id`, the same every time.
    fn invent(id: usize) -> Self {
        let app = APPS[id % APPS.len()];
        let hash = (id as u64).wrapping_mul(2_654_435_761) % 0xf_ffff;
        let (tone, status) = if id % 11 == 3 {
            (Tone::Crit, "CrashLoopBackOff")
        } else if id % 7 == 2 {
            (Tone::Warn, "Pending")
        } else if id % 13 == 5 {
            (Tone::Unknown, "Unknown")
        } else {
            (Tone::Good, "Running")
        };
        let restarts = if tone == Tone::Crit {
            14
        } else {
            (hash % 4) as u32
        };
        let minutes = (hash % 20_000) as u32 + 3;
        Self {
            id,
            name: format!("{app}-{hash:05x}").into(),
            namespace: NAMESPACES[id % NAMESPACES.len()].into(),
            cluster: if id.is_multiple_of(2) {
                "cluster-a"
            } else {
                "cluster-b"
            }
            .into(),
            tone,
            status: status.into(),
            restarts,
            restarts_text: restarts.to_string().into(),
            cpu: format!("{}m", hash % 900 + 5).into(),
            memory: format!("{}Mi", hash % 700 + 16).into(),
            node: format!("worker-{}", id % 6 + 1).into(),
            minutes,
            age: age(minutes).into(),
            image: format!("registry.example/{app}:1.{}.{}", hash % 9, hash % 31).into(),
        }
    }
}

fn age(minutes: u32) -> String {
    match minutes {
        m if m < 60 => format!("{m}m"),
        m if m < 60 * 24 => format!("{}h", m / 60),
        m => format!("{}d", m / (60 * 24)),
    }
}

/// The groups, in the order they show: problems first.
const GROUPS: [(Tone, &str); 4] = [
    (Tone::Crit, "Failing"),
    (Tone::Warn, "Warning"),
    (Tone::Unknown, "Unknown"),
    (Tone::Good, "Running"),
];

enum Item {
    Group(usize),
    Row(usize),
}

pub struct DataTableStory {
    table: TableState,
    columns: Vec<Column>,
    pods: Vec<Pod>,
    /// Derived from the pods, the sort and the filter: what each line is.
    lines: Vec<Item>,
    /// Each group's pods, after the filter.
    counts: [usize; GROUPS.len()],
    state: State,
    count: usize,
    sort: (Sort, SortOrder),
    filter: Option<Tone>,
    selected: Option<usize>,
}

impl DataTableStory {
    pub fn new() -> Self {
        let column = |kind, label: &str, width| Column {
            kind,
            label: label.to_owned().into(),
            width,
        };
        let mut story = Self {
            table: TableState::new(TABLE),
            columns: vec![
                column(Kind::Glyph, "", GLYPH_WIDTH),
                column(Kind::Name, "Name", 210.),
                column(Kind::Namespace, "Namespace", 110.),
                column(Kind::Cluster, "Cluster", 100.),
                column(Kind::Status, "Status", 150.),
                column(Kind::Restarts, "Restarts", 84.),
                column(Kind::Cpu, "CPU", 70.),
                column(Kind::Memory, "Memory", 80.),
                column(Kind::Node, "Node", 100.),
                column(Kind::Age, "Age", 64.),
                column(Kind::Image, "Image", 260.),
            ],
            pods: Vec::new(),
            lines: Vec::new(),
            counts: [0; GROUPS.len()],
            state: State::Rows,
            count: COUNTS[0],
            sort: (Sort::Name, SortOrder::Ascending),
            filter: None,
            selected: None,
        };
        story.invent();
        story
    }

    pub fn state(&self) -> State {
        self.state
    }

    pub fn rows(&self) -> usize {
        self.pods.len()
    }

    pub fn sorted_by(&self) -> (Sort, SortOrder) {
        self.sort
    }

    /// The pods' names in the order the lines show them.
    pub fn names(&self) -> Vec<SharedString> {
        self.lines
            .iter()
            .filter_map(|item| match item {
                Item::Row(ix) => Some(self.pods[*ix].name.clone()),
                Item::Group(_) => None,
            })
            .collect()
    }

    fn set_state(&mut self, state: State, cx: &mut Context<Self>) {
        self.state = state;
        cx.notify();
    }

    fn set_count(&mut self, count: usize, cx: &mut Context<Self>) {
        if count != self.count {
            self.count = count;
            self.selected = None;
            self.invent();
            cx.notify();
        }
    }

    fn toggle_filter(&mut self, tone: Tone, cx: &mut Context<Self>) {
        self.filter = (self.filter != Some(tone)).then_some(tone);
        self.derive();
        cx.notify();
    }

    fn invent(&mut self) {
        self.pods = (0..self.count).map(Pod::invent).collect();
        self.derive();
    }

    /// The lines and the counts, when the pods, the sort or the filter
    /// change; drawing only reads them.
    fn derive(&mut self) {
        let (sort, order) = self.sort;
        let mut order_of: Vec<usize> = (0..self.pods.len()).collect();
        order_of.sort_by(|a, b| {
            let (a, b) = (&self.pods[*a], &self.pods[*b]);
            let by = match sort {
                Sort::Name => a.name.cmp(&b.name),
                Sort::Restarts => a.restarts.cmp(&b.restarts),
                Sort::Age => a.minutes.cmp(&b.minutes),
            }
            .then(a.id.cmp(&b.id));
            if order == SortOrder::Descending {
                by.reverse()
            } else {
                by
            }
        });
        self.lines.clear();
        for (group, (tone, _)) in GROUPS.iter().enumerate() {
            let members: Vec<usize> = order_of
                .iter()
                .copied()
                .filter(|ix| self.pods[*ix].tone == *tone)
                .collect();
            self.counts[group] = members.len();
            if members.is_empty() || self.filter.is_some_and(|filter| filter != *tone) {
                continue;
            }
            self.lines.push(Item::Group(group));
            self.lines.extend(members.into_iter().map(Item::Row));
        }
    }

    fn render_header(&self, window: &mut Window, cx: &mut Context<Self>) -> Div {
        let header = PageHeader::new(PREFIX, "Pods");
        let chips = table::status_chips(
            header.id("tally"),
            GROUPS.iter().enumerate().map(|(group, (tone, label))| {
                let tone = *tone;
                table::status_chip(
                    header.id(&format!("tally-{}", label.to_lowercase())),
                    tone,
                    self.counts[group],
                    &label.to_lowercase(),
                    self.filter == Some(tone),
                    cx,
                )
                .on_click(cx.listener(move |this, _, _, cx| this.toggle_filter(tone, cx)))
            }),
            cx,
        );
        let states = segments(
            State::ALL.map(|(state, id, label)| {
                ui::segment(
                    Button::new(SharedString::from(format!("{PREFIX}-state-{id}"))),
                    self.state == state,
                    cx,
                )
                .small()
                .label(label)
                .on_click(cx.listener(move |this, _, _, cx| this.set_state(state, cx)))
            }),
            cx,
        );
        let counts = segments(
            COUNTS.map(|count| {
                ui::segment(
                    Button::new(SharedString::from(format!("{PREFIX}-count-{count}"))),
                    self.count == count,
                    cx,
                )
                .small()
                .label(rows(count))
                .on_click(cx.listener(move |this, _, _, cx| this.set_count(count, cx)))
            }),
            cx,
        );
        let p = palette(cx);
        header
            .chips(Some(chips))
            .control(counts)
            .secondary(states)
            .meta([
                div().child("Invented pods").into_any_element(),
                div().text_color(p.faint).child(" · ").into_any_element(),
                div().child(rows(self.pods.len())).into_any_element(),
            ])
            .render(window, cx)
    }

    fn render_body(&self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        match self.state {
            State::Rows => {
                #[cfg(debug_assertions)]
                {
                    static STORY: freshkube_probe::first_frame::FirstFrame =
                        freshkube_probe::first_frame::FirstFrame::new("story");
                    if STORY.pending() {
                        window.on_next_frame(|_, _| STORY.mark());
                    }
                }
                table::data_table(self, window, cx)
                    .flex_1()
                    .min_h_0()
                    .into_any_element()
            }
            State::Loading => state(
                "loading",
                page::inset().child(
                    page::card(cx)
                        .p_3()
                        .gap_3()
                        .children((0..9).map(|_| ui::skeleton(relative(0.7), dp(12.)))),
                ),
            ),
            State::Empty => state(
                "empty",
                ui::empty_state(
                    IconName::SearchX,
                    "No pods in app-a",
                    "Nothing in this namespace on cluster-a matches. Rows show here when a pod is created.",
                    None,
                    Vec::new(),
                    cx,
                ),
            ),
            State::Failed => state(
                "failed",
                ui::empty_state(
                    IconName::CircleDashed,
                    "Couldn't list pods",
                    "Nothing is shown as missing: the list is read again on Retry.",
                    Some("the server is currently unable to handle the request (get pods)".into()),
                    vec![
                        Button::new("data-table-retry")
                            .outline()
                            .small()
                            .label("Retry")
                            .on_click(cx.listener(|this, _, _, cx| this.set_state(State::Rows, cx)))
                            .into_any_element(),
                    ],
                    cx,
                ),
            ),
        }
    }
}

impl Default for DataTableStory {
    fn default() -> Self {
        Self::new()
    }
}

/// `24 rows`, `2,000 rows`.
fn rows(count: usize) -> String {
    if count >= 1000 {
        format!("{},{:03} rows", count / 1000, count % 1000)
    } else {
        format!("{count} rows")
    }
}

/// A state's element: role status, id `data-table-<state>`.
fn state(id: &str, body: impl IntoElement) -> AnyElement {
    div()
        .id(SharedString::from(format!("{PREFIX}-{id}")))
        .test_support()
        .role(Role::Status)
        .flex_1()
        .min_h_0()
        .child(body)
        .into_any_element()
}

/// Segments on a track, as a segmented control.
fn segments<const N: usize>(options: [Button; N], cx: &App) -> Div {
    h_flex()
        .gap(dp(2.))
        .p(dp(3.))
        .rounded(px(8.))
        .bg(palette(cx).surface_2)
        .children(options)
}

impl TableSource for DataTableStory {
    type Key = usize;
    type Sort = Sort;
    type Column = Column;
    type Row<'a> = &'a Pod;

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
        "Invented pods, by name".into()
    }

    fn sorting(&self, column: &Column) -> Option<(Sort, Option<SortOrder>)> {
        let sort = match column.kind {
            Kind::Name => Sort::Name,
            Kind::Restarts => Sort::Restarts,
            Kind::Age => Sort::Age,
            _ => return None,
        };
        Some((sort, (self.sort.0 == sort).then_some(self.sort.1)))
    }

    fn sort(&mut self, sort: Sort, cx: &mut Context<Self>) {
        self.sort = match self.sort {
            (current, SortOrder::Ascending) if current == sort => (sort, SortOrder::Descending),
            _ => (sort, SortOrder::Ascending),
        };
        self.derive();
        cx.notify();
    }

    fn line_count(&self) -> usize {
        self.lines.len()
    }

    fn line(&self, line: usize, _: &App) -> Option<Line<usize, &Pod>> {
        Some(match self.lines.get(line)? {
            Item::Group(group) => Line::Group(*group),
            Item::Row(ix) => {
                let pod = &self.pods[*ix];
                Line::Row(TableRow {
                    key: pod.id,
                    id: ElementId::from((SharedString::from("data-table-row"), pod.id)),
                    label: pod.name.clone(),
                    tooltip: None,
                    marked: false,
                    muted: false,
                    data: pod,
                })
            }
        })
    }

    fn cell(
        &self,
        row: &TableRow<usize, &Pod>,
        style: &RowStyle,
        column: &Column,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let pod = row.data;
        let p = &style.p;
        let text = |text: &SharedString| table::cell(column).child(text.clone());
        let figure = |value: &SharedString| text(value).text_right().font_family(MONO_FONT);
        match column.kind {
            Kind::Glyph => table::glyph_cell(column)
                .children(ui::status_glyph(pod.tone, cx))
                .into_any_element(),
            Kind::Name => text(&pod.name).font_family(MONO_FONT).into_any_element(),
            Kind::Namespace => text(&pod.namespace).text_color(p.ink_2).into_any_element(),
            Kind::Cluster => text(&pod.cluster).text_color(p.ink_2).into_any_element(),
            Kind::Status => text(&pod.status)
                .text_color(match pod.tone {
                    Tone::Crit => p.crit_ink,
                    Tone::Warn => p.warn_ink,
                    _ => p.ink_2,
                })
                .into_any_element(),
            Kind::Restarts => figure(&pod.restarts_text).into_any_element(),
            Kind::Cpu => figure(&pod.cpu).into_any_element(),
            Kind::Memory => figure(&pod.memory).into_any_element(),
            Kind::Node => text(&pod.node).into_any_element(),
            Kind::Age => figure(&pod.age).into_any_element(),
            Kind::Image => text(&pod.image).text_color(p.muted).into_any_element(),
        }
    }

    fn group(&self, group: usize, cx: &mut Context<Self>) -> Option<AnyElement> {
        let (tone, label) = GROUPS.get(group)?;
        Some(
            GroupRow::new(
                (SharedString::from("data-table-group"), group),
                *tone,
                *label,
                ROW_HEIGHT,
            )
            .detail(vec![format!("{} pods", self.counts[group])])
            .render(cx)
            .into_any_element(),
        )
    }

    fn empty(&self, _: &mut Context<Self>) -> Option<AnyElement> {
        self.lines
            .is_empty()
            .then(|| "No pods match this filter.".into_any_element())
    }

    fn selected_key(&self) -> Option<&usize> {
        self.selected.as_ref()
    }

    fn line_of(&self, key: &usize) -> Option<usize> {
        self.lines
            .iter()
            .position(|item| matches!(item, Item::Row(ix) if self.pods[*ix].id == *key))
    }

    fn click(&mut self, key: &usize, _: &ClickEvent, _: &mut Window, cx: &mut Context<Self>) {
        self.selected = Some(*key);
        cx.notify();
    }
}

impl Render for DataTableStory {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let header = self.render_header(window, cx);
        let body = self.render_body(window, cx);
        page::page(SharedString::from(format!("{PREFIX}-page")))
            .child(page::toolbar(cx).child(header))
            .child(body)
    }
}
