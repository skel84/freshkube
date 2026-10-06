//! Loading: today's skeleton bars, as each page draws its own, beside the
//! one loading state every table would share, skeleton rows at the real
//! row height under the real header, pulsing or shimmering. The table draws
//! the bars still and its `LoadingMotion` moves them from beside it, so the
//! table isn't drawn again for each frame.

use super::{Column, Kind, columns, option, segments};
use freshkube_ui::page::{self, PageHeader};
use freshkube_ui::palette::palette;
use freshkube_ui::table::{
    self, Line, LoadingMotion, LoadingRows, Look, RowStyle, SortOrder, TableColumn, TableRow,
    TableSource, TableState,
};
use freshkube_ui::ui::{self, dp};
use gpui_kit::component::v_flex;
use gpui_kit::prelude::*;
use gpui_kit::{
    AnyElement, AnyView, App, Context, Entity, SharedString, StyleRefinement, Window, div, relative,
};

/// The id prefix of everything the story draws.
const PREFIX: &str = "loading";

pub fn build(_: &mut Window, cx: &mut App) -> AnyView {
    cx.new(LoadingStory::new).into()
}

/// The story's frame: the header and the two loading states. Today's bars
/// are their own view; the shared rows are a cached table and the motion
/// over it, so a frame draws the motion and the views above it only.
pub struct LoadingStory {
    today: Entity<TodayBars>,
    table: Entity<LoadingTable>,
    motion: Entity<LoadingMotion>,
}

impl LoadingStory {
    fn new(cx: &mut Context<Self>) -> Self {
        let table = cx.new(|_| LoadingTable::new());
        let motion = cx.new(|cx| table.read(cx).rows.motion(Look::Pulse));
        Self {
            today: cx.new(|_| TodayBars),
            table,
            motion,
        }
    }

    pub fn table(&self) -> &Entity<LoadingTable> {
        &self.table
    }

    pub fn motion(&self) -> &Entity<LoadingMotion> {
        &self.motion
    }

    fn set_look(&mut self, look: Look, cx: &mut Context<Self>) {
        self.motion
            .update(cx, |motion, cx| motion.set_look(look, cx));
        cx.notify();
    }
}

impl Render for LoadingStory {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        freshkube_probe::probe::hit("workbench.loading");
        let look = self.motion.read(cx).look();
        let looks = segments(
            [
                (Look::Pulse, "pulse", "Pulse"),
                (Look::Shimmer, "shimmer", "Shimmer"),
            ]
            .map(|(choice, id, label)| {
                option(format!("{PREFIX}-look-{id}"), label, look == choice, cx)
                    .on_click(cx.listener(move |this, _, _, cx| this.set_look(choice, cx)))
            }),
            cx,
        );
        let p = palette(cx);
        let header = PageHeader::new(PREFIX, "Loading")
            .control(looks)
            .meta([
                div().child("Invented pods").into_any_element(),
                div().text_color(p.faint).child(" · ").into_any_element(),
                div()
                    .child(if cx.reduce_motion() {
                        "Reduced motion: the bars stand still"
                    } else {
                        "Full motion"
                    })
                    .into_any_element(),
            ])
            .render(window, cx);
        page::page(SharedString::from(format!("{PREFIX}-page")))
            .child(page::toolbar(cx).child(header))
            .child(
                v_flex()
                    .flex_none()
                    .gap(dp(8.))
                    .child(
                        page::inset().child(ui::caption("Today: each page draws its own bars", cx)),
                    )
                    .child(self.today.clone()),
            )
            .child(page::inset().child(ui::caption(
                "Shared: the table's loading rows under its header",
                cx,
            )))
            .child(
                div()
                    .relative()
                    .flex_1()
                    .min_h_0()
                    .child(
                        AnyView::from(self.table.clone())
                            .cached(StyleRefinement::default().size_full()),
                    )
                    .child(self.motion.clone()),
            )
    }
}

/// Today's loading card: Kit's skeleton through `ui::skeleton`, as the
/// Resources list draws it.
pub struct TodayBars;

impl Render for TodayBars {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        freshkube_probe::probe::hit("workbench.loading-today");
        page::inset()
            .id(SharedString::from(format!("{PREFIX}-today")))
            .child(
                page::card(cx)
                    .p_3()
                    .gap_3()
                    .children((0..4).map(|_| ui::skeleton(relative(0.7), dp(12.)))),
            )
    }
}

/// A table on its first read: the real header, and the shared loading rows
/// in place of its own.
pub struct LoadingTable {
    table: TableState,
    columns: Vec<Column>,
    rows: LoadingRows,
}

impl LoadingTable {
    fn new() -> Self {
        Self {
            table: TableState::new("loading-pods"),
            columns: columns(),
            rows: LoadingRows::new("loading-pods"),
        }
    }
}

impl Render for LoadingTable {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        freshkube_probe::probe::hit("workbench.loading-table");
        table::data_table(self, window, cx).size_full()
    }
}

impl TableSource for LoadingTable {
    type Key = usize;
    type Sort = ();
    type Column = Column;
    type Row<'a> = ();

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
        "Invented pods, loading".into()
    }

    fn sorting(&self, column: &Column) -> Option<((), Option<SortOrder>)> {
        (column.kind == Kind::Name).then_some(((), Some(SortOrder::Ascending)))
    }

    fn sort(&mut self, _: (), _: &mut Context<Self>) {}

    fn line_count(&self) -> usize {
        0
    }

    fn line(&self, _: usize, _: &App) -> Option<Line<usize, ()>> {
        None
    }

    fn cell(
        &self,
        _: &TableRow<usize, ()>,
        _: &RowStyle,
        _: &Column,
        _: &mut Context<Self>,
    ) -> AnyElement {
        div().into_any_element()
    }

    fn group(&self, _: usize, _: &mut Context<Self>) -> Option<AnyElement> {
        None
    }

    fn loading(&self) -> Option<&LoadingRows> {
        Some(&self.rows)
    }

    fn empty(&self, _: &mut Context<Self>) -> Option<AnyElement> {
        None
    }
}
