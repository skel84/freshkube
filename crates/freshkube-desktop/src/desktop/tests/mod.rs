mod chrome;
mod column;
mod custom;
mod debug_options;
mod header;
mod keys;
mod kubernetes_only;
mod layout;
mod map;
mod motion;
mod observability;
mod operations;
mod overview;
mod rail;
mod screens;
mod services;
mod settings;
mod shells;
mod status;
mod target;
mod visible;

use super::{Area, ColumnReveal, GpuiOptions, NodeView, Page, Pilot, probe};
use crate::logs::TalosPanel;
pub(crate) use freshkube_ui::layout_check::settle_header;
use gpui_kit::test::TestWindowExt;
use gpui_kit::{
    AnyWindowHandle, AppContext, Entity, SharedString, TestAppContext,
    component::{ActiveTheme, Theme, ThemeMode},
    point, px, size,
};
use std::cell::Cell;
use std::rc::Rc;

use super::testing::{area, root_pilot};
pub(super) use super::testing::{click_node, expand_healthy_nodes, mount};
pub(crate) use super::testing::{
    fixture, open_kind, open_node_tab, pick_target, screen, start_shell,
};

/// Opens the node's Services tab and expands the inspector, so a selected
/// service's details sit beside the list rather than under it.
fn open_services_with_room(window: &mut gpui_kit::Window, cx: &mut gpui_kit::App) {
    open_node_tab(window, cx, super::nodes::NodeTab::Services);
    window.render_frame(cx);
    if !root_pilot(window, cx)
        .read(cx)
        .node_workspace
        .read(cx)
        .expanded
    {
        window.click("node-expand", cx);
        window.render_frame(cx);
    }
}

fn contexts(window: &mut gpui_kit::Window, cx: &mut gpui_kit::App) {
    if !root_pilot(window, cx).read(cx).context_display.open {
        window.click("context-switcher", cx);
        window.render_frame(cx);
    }
}

const DEGRADED_NODE: &str = "talos-wk-fra1-02";

/// Scrolls the navigation column until `id` shows in full.
fn reveal(window: &mut gpui_kit::Window, cx: &mut gpui_kit::App, id: &str) {
    for _ in 0..60 {
        if shown_in_column(window, id) {
            return;
        }
        let area = column_area(window);
        let above = window
            .try_find(SharedString::from(id.to_owned()))
            .is_some_and(|element| element.bounds().top() < area.top());
        window.scroll(
            "nav-column",
            gpui_kit::ScrollDelta::Pixels(gpui_kit::point(
                px(0.),
                px(if above { 40. } else { -40. }),
            )),
            cx,
        );
        window.render_frame(cx);
    }
    panic!("{id} never scrolled into view");
}

/// Whether `id` shows in full inside the scrolled navigation column.
/// Where the column's rows show: the source list under the title, or the
/// collapsed column's strip.
fn column_area(window: &mut gpui_kit::Window) -> gpui_kit::Bounds<gpui_kit::Pixels> {
    window
        .try_find("nav-column-list")
        .unwrap_or_else(|| window.find("nav-column"))
        .bounds()
}

fn shown_in_column(window: &mut gpui_kit::Window, id: &str) -> bool {
    let area = column_area(window);
    window
        .try_find(SharedString::from(id.to_owned()))
        .is_some_and(|element| {
            let bounds = element.bounds();
            element.visible() && bounds.top() >= area.top() && bounds.bottom() <= area.bottom()
        })
}

/// Lets the loading motion run, as the platform would: one 16 ms frame.
/// Returns how many next-frame callbacks the frame delivered.
fn motion_frame(cx: &mut TestAppContext, handle: AnyWindowHandle) -> usize {
    cx.background_executor
        .advance_clock(std::time::Duration::from_millis(16));
    let asked = cx
        .update_window(handle, |_, window, cx| window.simulate_next_frame(cx))
        .unwrap();
    cx.run_until_parked();
    asked
}

/// The shell's cached parts, by their probes.
const CHROME: [&str; 3] = ["chrome.header", "chrome.rail", "chrome.column"];

fn chrome_counts() -> [usize; 3] {
    CHROME.map(probe::count)
}

/// Runs 60 frames, about a second, and returns how many of them redrew
/// a part of the chrome or `page`. The header's countdown ring and a
/// page's ages tick once a second; nothing else should draw them.
fn frames_redrawing(cx: &mut TestAppContext, handle: AnyWindowHandle, page: &'static str) -> usize {
    let counts = || (chrome_counts(), probe::count(page));
    (0..60)
        .filter(|_| {
            let before = counts();
            assert!(
                motion_frame(cx, handle) > 0,
                "the bars move while the page loads"
            );
            counts() != before
        })
        .count()
}

/// Opens Pods with example data that never answers, so it stays on its
/// loading rows, and lets the motion start.
fn loading_pods(
    cx: &mut TestAppContext,
) -> (tokio::runtime::Runtime, AnyWindowHandle, Entity<Pilot>) {
    let (runtime, handle, view) = fixture(cx, 1280., 880.);
    cx.update(|cx| cx.set_reduce_motion(false));
    cx.update_window(handle, |_, window, cx| {
        view.update(cx, |pilot, cx| {
            pilot.hold.lists = true;
            pilot.push_source(window, cx);
        });
        open_kind(window, cx, "pods");
    })
    .unwrap();
    cx.run_until_parked();
    for _ in 0..3 {
        motion_frame(cx, handle);
    }
    cx.update_window(handle, |_, window, _| {
        assert!(window.find("resource-loading").visible());
    })
    .unwrap();
    (runtime, handle, view)
}

/// Applies `change` and returns which parts of the chrome drew again.
pub(super) fn chrome_redrawn(
    cx: &mut TestAppContext,
    handle: AnyWindowHandle,
    change: impl FnOnce(&mut gpui_kit::Window, &mut gpui_kit::App),
) -> Vec<&'static str> {
    let before = chrome_counts();
    cx.update_window(handle, |_, window, cx| change(window, cx))
        .unwrap();
    cx.run_until_parked();
    let after = chrome_counts();
    CHROME
        .into_iter()
        .zip(before.into_iter().zip(after))
        .filter(|(_, (before, after))| after > before)
        .map(|(part, _)| part)
        .collect()
}

/// Draws what changed since the last frame. Unlike `render_frame` this keeps
/// GPUI's view cache, which is what these tests are about.
fn draw(window: &mut gpui_kit::Window, cx: &mut gpui_kit::App) {
    window.draw(cx).clear(cx);
}

fn count(name: &'static str) -> usize {
    super::probe::count(name)
}

fn scratch(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("freshkube-tests-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    dir.join(name)
}
