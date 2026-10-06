//! The loading state every table shares: skeleton rows at the row height,
//! one bar per column, under the table's real header. A source returns it
//! from [`TableSource::loading`](super::TableSource::loading) until its
//! first answer.
//!
//! The rows are their own small view, so their animation asks frames for
//! them and their ancestors only. The bars pulse as Kit's skeleton does, or
//! a band sweeps across them; under reduced motion they stand still.

use gpui_kit::component::{ActiveTheme, h_flex, v_flex};
use gpui_kit::prelude::*;
use gpui_kit::{
    Context, Div, ElementId, Hsla, Role, SharedString, TestSupportExt, Window, div,
    linear_color_stop, linear_gradient, px, white,
};

use super::{CELL_PAD, GLYPH_WIDTH, ROW_HEIGHT, TableColumn};
use crate::motion;
use crate::ui::dp;

/// How the bars move.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Look {
    /// Every bar dims and comes back together.
    Pulse,
    /// A light band crosses the rows from the left.
    Shimmer,
}

/// The rows a loading table shows; the list clips what doesn't fit.
pub const LOADING_ROWS: usize = 14;
/// A bar's height in a row.
const BAR: f32 = 8.;
/// The shimmer's band, in dp.
const BAND: f32 = 120.;
/// How far each row's band trails the one above, so the sweep leans.
const SLANT: f32 = 6.;
/// How much of a cell each row's bar fills, by row and column, so the
/// rows read as text rather than a grid.
const FILL: [f32; 7] = [0.72, 0.48, 0.86, 0.6, 0.78, 0.54, 0.66];

/// One column's bar: where it starts in the row and how wide its cell is.
struct Slot {
    x: f32,
    width: f32,
    glyph: bool,
    flexible: bool,
}

/// Skeleton rows in a table's columns.
pub struct LoadingRows {
    id: SharedString,
    slots: Vec<Slot>,
    /// The row's width in dp: where the band leaves.
    sweep: f32,
    look: Look,
}

impl LoadingRows {
    /// Rows in `columns`, with the id `<prefix>-loading`. A glyph column
    /// (unlabelled, [`GLYPH_WIDTH`] wide) shows a dot, the others a bar.
    pub fn new<C: TableColumn>(prefix: &str, columns: &[C], look: Look) -> Self {
        let mut x = 0.;
        let slots = columns
            .iter()
            .map(|column| {
                let slot = Slot {
                    x,
                    width: column.width(),
                    glyph: column.label().is_empty() && column.width() == GLYPH_WIDTH,
                    flexible: column.flexible(),
                };
                x += column.width();
                slot
            })
            .collect();
        Self {
            id: format!("{prefix}-loading").into(),
            slots,
            sweep: x,
            look,
        }
    }

    pub fn look(&self) -> Look {
        self.look
    }

    pub fn set_look(&mut self, look: Look, cx: &mut Context<Self>) {
        if look != self.look {
            self.look = look;
            cx.notify();
        }
    }

    fn render_row(&self, row: usize, colors: &Colors) -> Div {
        h_flex()
            .w_full()
            .h(dp(ROW_HEIGHT))
            .flex_none()
            .border_1()
            .border_color(gpui_kit::transparent_black())
            .children(
                self.slots
                    .iter()
                    .enumerate()
                    .map(|(column, slot)| self.render_cell(row, column, slot, colors)),
            )
    }

    fn render_cell(&self, row: usize, column: usize, slot: &Slot, colors: &Colors) -> Div {
        let cell = div().h_full().flex().items_center();
        let cell = if slot.flexible {
            cell.flex_1().min_w(dp(slot.width))
        } else {
            cell.flex_none().w(dp(slot.width))
        };
        let (offset, width) = if slot.glyph {
            let dot = BAR + 2.;
            ((slot.width - dot) / 2., dot)
        } else {
            let fill = FILL[(row * 3 + column) % FILL.len()];
            (CELL_PAD, ((slot.width - 2. * CELL_PAD) * fill).max(BAR))
        };
        let bar = div()
            .ml(dp(offset))
            .w(dp(width))
            .h(dp(if slot.glyph { width } else { BAR }))
            .rounded(px(if slot.glyph { width } else { 5. }))
            .bg(colors.bar);
        let id = ElementId::from((self.id.clone(), row * self.slots.len() + column));
        cell.child(match self.look {
            Look::Pulse => motion::pulse(id, bar).into_any_element(),
            Look::Shimmer => {
                // The band's place in the bar: where the sweep is, less the
                // bar's start in the row and this row's lag.
                let start = slot.x + offset + row as f32 * SLANT;
                let travel = self.sweep + BAND + LOADING_ROWS as f32 * SLANT;
                let band = band(colors);
                bar.relative()
                    .overflow_hidden()
                    .child(motion::shimmer(id, band, move |band, phase| {
                        band.left(dp(phase * travel - BAND - start))
                    }))
                    .into_any_element()
            }
        })
    }
}

/// The band: clear, light at its middle, clear again.
fn band(colors: &Colors) -> Div {
    let half = |from: Hsla, to: Hsla| {
        div().h_full().w(dp(BAND / 2.)).bg(linear_gradient(
            90.,
            linear_color_stop(from, 0.),
            linear_color_stop(to, 1.),
        ))
    };
    let clear = colors.band.opacity(0.);
    h_flex()
        .absolute()
        .top_0()
        .bottom_0()
        .left(dp(-BAND))
        .w(dp(BAND))
        .child(half(clear, colors.band))
        .child(half(colors.band, clear))
}

struct Colors {
    bar: Hsla,
    band: Hsla,
}

impl Render for LoadingRows {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        freshkube_probe::probe::hit("table.loading-rows");
        let theme = cx.theme();
        let colors = Colors {
            bar: theme.skeleton,
            band: white().opacity(if theme.mode.is_dark() { 0.09 } else { 0.6 }),
        };
        v_flex()
            .id(self.id.clone())
            .test_support()
            .role(Role::Status)
            .aria_label("Loading")
            .size_full()
            .overflow_hidden()
            .children((0..LOADING_ROWS).map(|row| self.render_row(row, &colors)))
    }
}
