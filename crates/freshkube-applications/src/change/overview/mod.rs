//! The Overview (docs/DESIGN.md, "The Overview"): the change on one
//! screen. A start strip of three cards (the pull request, the Build and
//! the Freight, each join with its key and its word), then a matrix with a
//! row per Stage in promotion order and a glyph and a word per step. The
//! Inspector is shared with All hops, and so is the selection: a cell is
//! the hop it stands for. Missing evidence is never drawn as a failure; an
//! unknown cell says its cause.
use super::*;

pub(super) mod cells;
mod matrix;
mod strip;

pub(super) use cells::StageRow;
pub(super) use matrix::Matrix;
pub(super) use strip::Strip;

use crate::ui::dp;
use gpui_kit::component::{ActiveTheme, h_flex, v_flex};

/// Where the selection is in the Overview, for the arrow keys.
enum At {
    Card(usize),
    Cell(usize, usize),
}

impl ChangePage {
    /// The strip and the matrix, derived when the change, its Coroot marks
    /// or the rows change.
    pub(super) fn derive_overview(&mut self) {
        self.overview = cells::derive(&self.change, &self.rows);
        self.strip = strip::derive(&self.change, &self.rows);
        self.matrix_stale.set(true);
    }

    /// The width the strip and the matrix have, in dp: the page's, less the
    /// Inspector's beside them when it is open.
    pub(super) fn lead_width(&self, window: &Window, cx: &App) -> f32 {
        let page = freshkube_ui::page::page_width(window);
        if page >= inspector::SPLIT_WIDTH && self.selected.is_some() {
            page - self.overview_split.live_width(cx)
        } else {
            page
        }
    }

    /// Hands the matrix its rows and the selection, which it draws.
    ///
    /// Only when something it shows moved: the change, its marks, the
    /// selection, the filter or the loading state. A frame that moved none
    /// of them does nothing here.
    fn sync_matrix(&self, cx: &mut Context<Self>) {
        let loading_now = self.loading_rows().is_some();
        if !self.matrix_stale.replace(false)
            && self.matrix_loading.replace(loading_now) == loading_now
        {
            return;
        }
        self.matrix_loading.set(loading_now);
        let selected = self
            .selected
            .as_ref()
            .and_then(|key| cells::locate(&self.overview, key))
            .map(|(row, column)| (self.overview[row].name.clone(), column));
        let loading = self.loading_rows().cloned();
        // A status chip's filter shows the Stages it counts.
        let rows: Vec<&StageRow> = self
            .overview
            .iter()
            .filter(|row| {
                self.filter.is_none_or(|tone| {
                    self.groups
                        .get(row.group)
                        .is_some_and(|g| g.counts_as == tone)
                })
            })
            .collect();
        let freight = self.change.freight.as_str();
        self.matrix.update(cx, |matrix, _| {
            matrix.set(freight, &rows, selected, loading)
        });
    }

    pub(super) fn render_overview(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        self.sync_matrix(cx);
        let strip = if self.loading_rows().is_some() {
            self.render_strip_loading(window, cx)
        } else {
            self.render_strip(window, cx)
        };
        let lead = v_flex()
            .id("change-overview")
            .test_support()
            .size_full()
            .min_w_0()
            .min_h_0()
            .child(strip)
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .min_w_0()
                    .child(self.matrix.clone()),
            )
            .into_any_element();
        let beside = freshkube_ui::page::page_width(window) >= inspector::SPLIT_WIDTH;
        let details = self.render_detail(cx);
        inspector::split(
            "change-split",
            &self.overview_split,
            beside,
            lead,
            details,
            window,
        )
    }

    /// Three skeleton cards in the strip's place, until the first answer.
    fn render_strip_loading(&self, window: &Window, cx: &mut Context<Self>) -> AnyElement {
        let stacked = self.lead_width(window, cx) < strip::STACKS_BELOW;
        let bar = cx.theme().skeleton;
        let card = |ix: usize| {
            freshkube_ui::page::card(cx)
                .id(SharedString::from(format!("change-strip-loading-{ix}")))
                .test_support()
                .flex_1()
                .min_w(dp(120.))
                .h(dp(44.))
                .p(dp(10.))
                .gap(dp(6.))
                .child(div().h(dp(8.)).w(dp(90.)).rounded(px(3.)).bg(bar))
                .child(div().h(dp(8.)).w(dp(60.)).rounded(px(3.)).bg(bar))
        };
        let strip = if stacked { v_flex() } else { h_flex() };
        strip
            .id("change-strip")
            .test_support()
            .role(Role::Status)
            .aria_label("Loading")
            .flex_none()
            .w_full()
            .gap(dp(24.))
            .px(dp(freshkube_ui::page::PANE_PADDING))
            .py(dp(freshkube_ui::page::PANE_PADDING_Y))
            .children((0..3).map(card))
            .into_any_element()
    }

    /// Where the selection is: on a card of the strip, or in the matrix.
    fn overview_at(&self) -> Option<At> {
        let key = self.selected.as_ref()?;
        if let Some(ix) = self
            .strip
            .cards
            .iter()
            .position(|card| card.hops.contains(key))
        {
            return Some(At::Card(ix));
        }
        let (row, column) = cells::locate(&self.overview, key)?;
        Some(At::Cell(row, column.map_or(0, |column| column as usize)))
    }

    /// The arrow keys in the Overview: ← → along a row or the strip, ↑ ↓
    /// between Stages, and ↑ from the top row to the strip.
    pub(super) fn step_overview(
        &mut self,
        dy: isize,
        dx: isize,
        window: &Window,
        cx: &mut Context<Self>,
    ) {
        let cards = self.strip.cards.len();
        let target = match self.overview_at() {
            None if self.overview.is_empty() && cards == 0 => None,
            None if dy < 0 && cards > 0 => self.card_hop(cards - 1),
            None => self.cell_hop(0, 0, 1),
            Some(At::Card(ix)) => {
                if dy > 0 {
                    self.cell_hop(0, 0, 1)
                } else if dx != 0 {
                    let next = (ix as isize + dx).clamp(0, cards as isize - 1) as usize;
                    self.card_hop(next)
                } else {
                    None
                }
            }
            Some(At::Cell(row, column)) => {
                if dy < 0 && row == 0 {
                    (cards > 0).then(|| self.card_hop(cards - 1)).flatten()
                } else if dy != 0 {
                    let row = (row as isize + dy).clamp(0, self.overview.len() as isize - 1);
                    self.cell_hop(row as usize, column, 1)
                } else {
                    self.cell_hop(row, (column as isize + dx).clamp(0, 5) as usize, dx)
                }
            }
        };
        if let Some(key) = target {
            self.select(&key, window, cx);
        }
    }

    /// Selects a hop with the link into it open in the Inspector.
    pub(super) fn select_link(&mut self, key: &str, window: &Window, cx: &mut Context<Self>) {
        self.select(key, window, cx);
        if self.selected.as_deref() == Some(key) {
            self.link_selected = true;
            self.detail_scroll.set_offset(point(px(0.), px(0.)));
            self.detail = self.derive_detail();
            cx.notify();
        }
    }

    fn card_hop(&self, ix: usize) -> Option<SharedString> {
        self.strip.cards.get(ix).map(|card| card.select.clone())
    }

    /// The hop of the cell at `row`, `column`; or, when the change has none
    /// there, the nearest one along `toward`.
    fn cell_hop(&self, row: usize, column: usize, toward: isize) -> Option<SharedString> {
        let cells = &self.overview.get(row)?.cells;
        let order: Vec<usize> = if toward >= 0 {
            (column..cells.len()).chain((0..column).rev()).collect()
        } else {
            (0..=column).rev().chain(column + 1..cells.len()).collect()
        };
        order.into_iter().find_map(|ix| cells[ix].hop.clone())
    }
}
