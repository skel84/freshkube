//! The app's log sources. The view they feed, `LogView<S>`, is the
//! `freshkube-logs` crate; a source reaches it only through that crate's
//! source contract.
//!
//! - `talos`: Talos service logs: the node's catalog, collection and
//!   delivery. The Logs page is `LogView<TalosLogs>`, named [`LogPanel`].
//! - `pod`: one pod container's log, for the pane's Logs tab.
//! - `workload`: every container of a workload's pods, interleaved by
//!   time, for a workload pane's Logs tab.
//! - `coroot`: an application's messages from Coroot, for the
//!   Application page's Logs report.

mod coroot;
mod pod;
mod talos;
mod workload;

#[cfg(test)]
mod tests;

use gpui_kit::{
    Pixels, Window,
    assets::IconName,
    component::{Selectable, Sizable, button::Button},
};

use crate::ui;

pub(crate) use coroot::{CorootLogView, CorootPanel};
pub(crate) use freshkube_logs::{ClearSelection, Columns, DownloadLines, LogSource, LogView};
pub(crate) use pod::{PodLogPanel, PodLogView, choice_label, role_heading};
pub(crate) use talos::{TalosLogs, TalosPanel};
pub(crate) use workload::{WorkloadLogPanel, WorkloadLogView};

/// The Talos Logs page.
pub(crate) type LogPanel = LogView<TalosLogs>;

/// The Timestamps toggle of a pod's and a workload's toolbar, on while
/// `time` shows; the caller adds what a click does.
fn timestamps_button(id: &'static str, time: bool) -> Button {
    Button::new(id)
        .outline()
        .small()
        .icon(IconName::Clock)
        .toggled(time)
        .selected(time)
        .accessibility_label("Timestamps")
        .tooltip("Show each line's time. Copy copies what shows.")
}

// A source's chips, as the workload logs' containers: they take the rows
// the panel has room for, and "+N" lists the rest.

/// Chip rows by the panel's height: what's left of a short panel goes to
/// the filters, the search and the log.
fn chip_rows(panel: Option<Pixels>, window: &Window) -> usize {
    let Some(panel) = panel else {
        return 2;
    };
    if panel >= ui::dp_px(340., window) {
        2
    } else if panel >= ui::dp_px(290., window) {
        1
    } else {
        0
    }
}

/// How many chips fit in `rows` rows of `room`, leaving the last row room
/// for the "+N" chip when some don't. Every chip fits when they all do.
fn chips_that_fit(
    widths: &[Pixels],
    more: Pixels,
    gap: Pixels,
    room: Pixels,
    rows: usize,
) -> usize {
    // Where each chip lands, packed left to right and row by row: its row
    // and where it ends.
    let mut placed: Vec<(usize, Pixels)> = Vec::with_capacity(widths.len());
    for &width in widths {
        let (row, end) = match placed.last() {
            None => (0, width),
            Some(&(row, end)) if end + gap + width <= room => (row, end + gap + width),
            Some(&(row, _)) => (row + 1, width),
        };
        if row >= rows {
            break;
        }
        placed.push((row, end));
    }
    if placed.len() == widths.len() {
        return widths.len();
    }
    // Some don't fit: take chips off the end until "+N" fits after the
    // last one, or alone at the start of its row.
    while let Some(&(row, end)) = placed.last() {
        if end + gap + more <= room {
            break;
        }
        placed.pop();
        if placed.last().is_none_or(|&(previous, _)| previous < row) {
            break;
        }
    }
    placed.len()
}
