use super::stories::data_table::{DataTableStory, Sort, State};
use super::stories::dock::{self as dock_story, DockStory};
use super::stories::drawer::DrawerStory;
use super::stories::graph::{GraphStory, Shape};
use super::stories::motion::flash::{BURST, FlashStory, PODS, Rate};
use super::stories::motion::loading::LoadingStory;
use super::{Options, stories};
use super::{STORIES, Workbench, window_size};
use freshkube_probe::probe;
use freshkube_ui::motion::{self, Choice};
use freshkube_ui::table::SortOrder;
use freshkube_ui::table::{Look, Reduced};
use freshkube_ui::text_size;
use gpui_kit::component::{ActiveTheme, Root, Theme, ThemeMode};
use gpui_kit::test::TestWindowExt;
use gpui_kit::{AnyWindowHandle, AppContext, Entity, ScrollDelta, TestAppContext, point, px, size};
use std::time::Duration;

/// The workbench on `story` in a 1280 × 880 window, light, at text 13.
fn open(cx: &mut TestAppContext, story: usize) -> (AnyWindowHandle, Entity<Workbench>) {
    cx.update(|cx| {
        gpui_kit::init(cx);
        freshkube_ui::theme::install(cx);
        text_size::install(None, cx);
        freshkube_ui::source_list::bind_keys(cx);
        freshkube_ui::graph::bind_keys(cx);
        Theme::change(ThemeMode::Light, None, cx);
        // Dialogs animate on the real clock, which the test clock can't
        // advance; reduced motion opens and closes them at once.
        cx.set_reduce_motion(true);
    });
    let mut view = None;
    let window = cx.open_window(size(px(1280.), px(880.)), |window, cx| {
        let workbench = cx.new(|cx| Workbench::new(story, window, cx));
        view = Some(workbench.clone());
        Root::new(workbench, window, cx)
    });
    cx.run_until_parked();
    (window.into(), view.unwrap())
}

fn story<T: 'static>(workbench: &Entity<Workbench>, cx: &gpui_kit::App) -> Entity<T> {
    workbench.read(cx).view().clone().downcast::<T>().unwrap()
}

/// A frame's worth of time on the test clock.
const FRAME: Duration = Duration::from_millis(16);

/// Moves the clock on a frame, delivers the next frame and draws it;
/// returns how many frame requests it answered.
fn next_frame(handle: AnyWindowHandle, cx: &mut TestAppContext) -> usize {
    cx.background_executor.advance_clock(FRAME);
    let asked = cx
        .update_window(handle, |_, window, cx| window.simulate_next_frame(cx))
        .unwrap();
    cx.run_until_parked();
    asked
}

mod change;
mod chrome;
mod dock;
mod drawer;
mod fix_bar;
mod flash;
mod form;
mod graph;
mod loading;
mod options;
mod source_list;
mod squares;
mod table;
