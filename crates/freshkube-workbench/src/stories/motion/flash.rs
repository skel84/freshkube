//! The change flash on invented pods: a row whose status or restarts change
//! is tinted and fades over 1.5 s in steps. A batch past the burst cap
//! doesn't flash at all. The story changes pods on a timer, one at a time or
//! as a burst, and shows what each fade step costs: the layer draws, the cached table
//! doesn't, and the views above both do, which a heavy ancestor makes plain.

use std::cell::Cell;
use std::rc::Rc;
use std::time::{Duration, Instant};

use super::{APPS, Column, Kind, STATES, columns, option, segments};
use freshkube_ui::motion::FLASH_BURST;
use freshkube_ui::page::{self, PageHeader};
use freshkube_ui::palette::palette;
use freshkube_ui::table::{
    self, FlashLayer, Line, Reduced, RowStyle, SortOrder, TableColumn, TableRow, TableSource,
    TableState,
};
use freshkube_ui::ui::{self, MONO_FONT, Tone, dp};
use gpui_kit::component::button::Button;
use gpui_kit::component::{Sizable, h_flex};
use gpui_kit::prelude::*;
use gpui_kit::{
    AnyElement, AnyView, App, Context, ElementId, Entity, SharedString, StyleRefinement, Task,
    TestSupportExt, Window, div,
};

/// The id prefix of everything the story draws.
const PREFIX: &str = "change-flash";
/// The table's own ids' prefix.
const TABLE: &str = "flash-pods";
/// How many invented pods the table holds.
pub const PODS: usize = 40;
/// How many pods the Burst button changes at once.
pub const BURST: usize = 40;
/// The cells the heavy ancestor draws on every frame it renders.
const HEAVY_CELLS: usize = 1_200;

pub fn build(_: &mut Window, cx: &mut App) -> AnyView {
    cx.new(FlashStory::new).into()
}

/// How often the timer changes a pod.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Rate {
    Off,
    PerSecond(u32),
}

impl Rate {
    const ALL: [(Rate, &'static str, &'static str); 3] = [
        (Rate::Off, "off", "Off"),
        (Rate::PerSecond(1), "1", "1 / s"),
        (Rate::PerSecond(4), "4", "4 / s"),
    ];
}

/// The story's root: the header, the cached table and the layer over it.
pub struct FlashStory {
    table: Entity<FlashTable>,
    layer: Entity<FlashLayer<usize>>,
    rate: Rate,
    ticker: Option<Task<()>>,
    /// Whether the root draws a block of cells as heavy as a page.
    heavy: bool,
    /// Its renders, and the table's, shown in the header.
    renders: usize,
    table_renders: Rc<Cell<usize>>,
    /// How long the root's last render took to build, in µs.
    last_render: u128,
    /// The next pod the timer changes.
    next: usize,
    /// What the last batch did: flashed or held back as a burst.
    last: Option<(usize, bool)>,
}

impl FlashStory {
    fn new(cx: &mut Context<Self>) -> Self {
        let table_renders = Rc::new(Cell::new(0));
        let table = cx.new(|_| FlashTable::new(table_renders.clone()));
        let rows = table.read(cx).table.rows_at();
        let lines = table.downgrade();
        let layer = cx.new(|_| {
            FlashLayer::new(TABLE, rows, move |key: &usize, cx: &App| {
                lines.upgrade()?.read(cx).line_of(key)
            })
        });
        let mut story = Self {
            table,
            layer,
            rate: Rate::Off,
            ticker: None,
            heavy: false,
            renders: 0,
            table_renders,
            last_render: 0,
            next: 0,
            last: None,
        };
        story.set_rate(Rate::PerSecond(1), cx);
        story
    }

    pub fn table(&self) -> &Entity<FlashTable> {
        &self.table
    }

    pub fn layer(&self) -> &Entity<FlashLayer<usize>> {
        &self.layer
    }

    pub fn rate(&self) -> Rate {
        self.rate
    }

    pub fn heavy(&self) -> bool {
        self.heavy
    }

    /// Changes `count` pods at once, as one batch, from the next in turn.
    pub fn change(&mut self, count: usize, cx: &mut Context<Self>) {
        let pods: Vec<usize> = (0..count).map(|n| (self.next + n * 7) % PODS).collect();
        self.next = (self.next + 3) % PODS;
        let revision = self.table.update(cx, |table, cx| table.change(&pods, cx));
        // A page tells the layer when its rows change, so it finds the
        // flashing rows' lines then rather than on every frame.
        let flashed = self.layer.update(cx, |layer, cx| {
            layer.rows_changed(revision, cx);
            layer.changed(pods.iter().copied(), cx)
        });
        self.last = Some((count, flashed));
        cx.notify();
    }

    fn set_rate(&mut self, rate: Rate, cx: &mut Context<Self>) {
        self.rate = rate;
        self.ticker = match rate {
            Rate::Off => None,
            Rate::PerSecond(n) => {
                let every = Duration::from_millis(1000 / u64::from(n));
                Some(cx.spawn(async move |this, cx| {
                    loop {
                        cx.background_executor().timer(every).await;
                        if this.update(cx, |this, cx| this.change(1, cx)).is_err() {
                            break;
                        }
                    }
                }))
            }
        };
        cx.notify();
    }

    fn set_reduced(&mut self, reduced: Reduced, cx: &mut Context<Self>) {
        self.layer
            .update(cx, |layer, cx| layer.set_reduced(reduced, cx));
        cx.notify();
    }

    fn render_header(&self, window: &mut Window, cx: &mut Context<Self>) -> gpui_kit::Div {
        let rates = segments(
            Rate::ALL.map(|(rate, id, label)| {
                option(format!("{PREFIX}-rate-{id}"), label, self.rate == rate, cx)
                    .on_click(cx.listener(move |this, _, _, cx| this.set_rate(rate, cx)))
            }),
            cx,
        );
        let reduced = self.layer.read(cx).reduced();
        let reduced = segments(
            [
                (Reduced::Nothing, "nothing", "No tint"),
                (Reduced::Held, "held", "Held tint"),
            ]
            .map(|(choice, id, label)| {
                option(
                    format!("{PREFIX}-reduced-{id}"),
                    label,
                    reduced == choice,
                    cx,
                )
                .on_click(cx.listener(move |this, _, _, cx| this.set_reduced(choice, cx)))
            }),
            cx,
        );
        let heavy = segments(
            [
                (false, "light", "Light root"),
                (true, "heavy", "Heavy root"),
            ]
            .map(|(choice, id, label)| {
                option(
                    format!("{PREFIX}-root-{id}"),
                    label,
                    self.heavy == choice,
                    cx,
                )
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.heavy = choice;
                    cx.notify();
                }))
            }),
            cx,
        );
        let one = Button::new(SharedString::from(format!("{PREFIX}-one")))
            .outline()
            .small()
            .label("Change one")
            .on_click(cx.listener(|this, _, _, cx| this.change(1, cx)));
        let burst = Button::new(SharedString::from(format!("{PREFIX}-burst")))
            .outline()
            .small()
            .label(format!("Burst of {BURST}"))
            .on_click(cx.listener(|this, _, _, cx| this.change(BURST, cx)));
        let p = palette(cx);
        let last = match self.last {
            None => "No change yet".to_owned(),
            Some((count, true)) => format!("{count} changed, flashed"),
            Some((count, false)) => {
                format!("{count} changed, past {FLASH_BURST} in 1.5 s: no flash")
            }
        };
        let costs = format!(
            "Root renders {} ({} µs) · table renders {}",
            self.renders,
            self.last_render,
            self.table_renders.get()
        );
        PageHeader::new(PREFIX, "Change flash")
            .control(rates)
            .control(one)
            .control(burst)
            .secondary(
                h_flex()
                    .gap(dp(16.))
                    .child(ui::toolbar_label("Reduced motion shows", cx))
                    .child(reduced)
                    .child(heavy),
            )
            .meta([
                div().child(last).into_any_element(),
                div().text_color(p.faint).child(" · ").into_any_element(),
                div()
                    .id(SharedString::from(format!("{PREFIX}-costs")))
                    .font_family(MONO_FONT)
                    .child(costs)
                    .into_any_element(),
            ])
            .render(window, cx)
    }

    /// A block of cells as costly to build as a page's own, drawn by the
    /// root on every frame it renders.
    fn render_heavy(&self, cx: &App) -> gpui_kit::Div {
        let p = palette(cx);
        page::inset().child(
            div()
                .id(SharedString::from(format!("{PREFIX}-heavy")))
                .test_support()
                .h(dp(96.))
                .overflow_hidden()
                .flex()
                .flex_wrap()
                .gap(dp(2.))
                .text_size(dp(9.))
                .text_color(p.faint)
                .children((0..HEAVY_CELLS).map(|n| {
                    div()
                        .px(dp(2.))
                        .bg(p.surface_2)
                        .child(SharedString::from(format!("{n:04}")))
                })),
        )
    }
}

impl Render for FlashStory {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        freshkube_probe::probe::hit("workbench.change-flash");
        let started = Instant::now();
        self.renders += 1;
        let header = self.render_header(window, cx);
        let heavy = self.heavy.then(|| self.render_heavy(cx));
        let root = page::page(SharedString::from(format!("{PREFIX}-page")))
            .child(page::toolbar(cx).child(header))
            .children(heavy)
            .child(
                div()
                    .relative()
                    .flex_1()
                    .min_h_0()
                    .child(
                        AnyView::from(self.table.clone())
                            .cached(StyleRefinement::default().size_full()),
                    )
                    .child(self.layer.clone()),
            );
        self.last_render = started.elapsed().as_micros();
        root
    }
}

/// One invented pod.
pub struct Pod {
    id: usize,
    name: SharedString,
    namespace: SharedString,
    state: usize,
    restarts: u32,
    restarts_text: SharedString,
    age: SharedString,
    image: SharedString,
}

impl Pod {
    fn invent(id: usize) -> Self {
        let app = APPS[id % APPS.len()];
        let hash = (id as u64).wrapping_mul(2_654_435_761) % 0xf_ffff;
        Self {
            id,
            name: format!("{app}-{hash:05x}").into(),
            namespace: ["app-a", "app-b", "shop"][id % 3].into(),
            state: 0,
            restarts: 0,
            restarts_text: "0".into(),
            age: format!("{}h", hash % 48 + 1).into(),
            image: format!("registry.example/{app}:1.{}.{}", hash % 9, hash % 31).into(),
        }
    }

    fn tone(&self) -> Tone {
        STATES[self.state].0
    }

    /// Its next state; a pod that leaves Running restarts.
    fn change(&mut self) {
        self.state = (self.state + 1) % STATES.len();
        if self.state != 0 {
            self.restarts += 1;
            self.restarts_text = self.restarts.to_string().into();
        }
    }
}

/// The table: invented pods by name, in a cached view of their own.
pub struct FlashTable {
    table: TableState,
    columns: Vec<Column>,
    /// By name, which a change doesn't move.
    pods: Vec<Pod>,
    /// Counts the rows' changes, for the flash layer.
    revision: u64,
    renders: Rc<Cell<usize>>,
}

impl FlashTable {
    fn new(renders: Rc<Cell<usize>>) -> Self {
        let mut pods: Vec<Pod> = (0..PODS).map(Pod::invent).collect();
        pods.sort_by(|a, b| a.name.cmp(&b.name));
        Self {
            table: TableState::new(TABLE),
            columns: columns(),
            pods,
            revision: 0,
            renders,
        }
    }

    /// Where the table's rows are, as the layer reads them.
    pub fn rows_at(&self) -> table::RowsAt {
        self.table.rows_at()
    }

    /// The pod on `line`.
    pub fn pod_at(&self, line: usize) -> Option<usize> {
        self.pods.get(line).map(|pod| pod.id)
    }

    /// The pods' states, by id.
    pub fn states(&self) -> Vec<(usize, &'static str)> {
        let mut states: Vec<_> = self
            .pods
            .iter()
            .map(|pod| (pod.id, STATES[pod.state].1))
            .collect();
        states.sort();
        states
    }

    /// Changes the pods `ids` and returns the rows' new revision.
    fn change(&mut self, ids: &[usize], cx: &mut Context<Self>) -> u64 {
        for pod in self.pods.iter_mut().filter(|pod| ids.contains(&pod.id)) {
            pod.change();
        }
        self.revision += 1;
        cx.notify();
        self.revision
    }
}

impl Render for FlashTable {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        freshkube_probe::probe::hit("workbench.change-flash-table");
        self.renders.set(self.renders.get() + 1);
        table::data_table(self, window, cx).size_full()
    }
}

impl TableSource for FlashTable {
    type Key = usize;
    type Sort = ();
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

    fn sorting(&self, column: &Column) -> Option<((), Option<SortOrder>)> {
        (column.kind == Kind::Name).then_some(((), Some(SortOrder::Ascending)))
    }

    fn sort(&mut self, _: (), _: &mut Context<Self>) {}

    fn line_count(&self) -> usize {
        self.pods.len()
    }

    fn line(&self, line: usize, _: &App) -> Option<Line<usize, &Pod>> {
        let pod = self.pods.get(line)?;
        Some(Line::Row(TableRow {
            key: pod.id,
            id: ElementId::from((SharedString::from("flash-pods-row"), pod.id)),
            label: pod.name.clone(),
            tooltip: None,
            marked: false,
            muted: false,
            data: pod,
        }))
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
        match column.kind {
            Kind::Glyph => table::glyph_cell(column)
                .children(ui::status_glyph(pod.tone(), cx))
                .into_any_element(),
            Kind::Name => text(&pod.name).into_any_element(),
            Kind::Namespace => text(&pod.namespace).text_color(p.ink_2).into_any_element(),
            Kind::Status => table::cell(column)
                .text_color(match pod.tone() {
                    Tone::Crit => p.crit_ink,
                    Tone::Warn => p.warn_ink,
                    _ => p.ink_2,
                })
                .child(STATES[pod.state].1)
                .into_any_element(),
            Kind::Restarts => text(&pod.restarts_text).text_right().into_any_element(),
            Kind::Age => text(&pod.age).text_right().into_any_element(),
            Kind::Image => text(&pod.image).text_color(p.muted).into_any_element(),
        }
    }

    fn group(&self, _: usize, _: &mut Context<Self>) -> Option<AnyElement> {
        None
    }

    fn empty(&self, _: &mut Context<Self>) -> Option<AnyElement> {
        None
    }

    fn line_of(&self, key: &usize) -> Option<usize> {
        self.pods.iter().position(|pod| pod.id == *key)
    }

    fn clickable(&self) -> bool {
        false
    }
}
