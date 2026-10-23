use super::{Close, NAMESPACE_PLACEHOLDER, display};
use super::{DatabasesScreen, Mode, PageSource, ScreenPanel};
use crate::desktop::layout_check;
use chrono::{TimeZone, Utc};
use freshkube_core::{
    cluster_source::{ClusterAccess, ClusterSource},
    databases::{self, Coverage, Inventory},
};
use gpui_kit::component::Root;
use gpui_kit::test::TestWindowExt;
use gpui_kit::{AppContext, Entity, InputEvent, TestAppContext, WindowHandle, px, size};
mod coroot;
mod freshness;
mod inspector;
mod inventory;
mod layout;
mod navigation;
mod reads;

fn source(id: &str) -> PageSource {
    PageSource {
        kube: Some(ClusterSource {
            id: id.into(),
            context: "invented-db-context".into(),
            access: ClusterAccess::EXAMPLE,
        }),
        ..PageSource::default()
    }
}
fn at() -> chrono::DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 10, 20, 12, 0, 0).unwrap()
}
/// The id of the row for database `name` in the current view.
fn row_id(
    page: &Entity<DatabasesScreen>,
    name: &str,
    cx: &gpui_kit::App,
) -> gpui_kit::SharedString {
    page.read(cx)
        .display
        .rows
        .iter()
        .find(|row| row.identity.name == name)
        .unwrap_or_else(|| panic!("no row {name}"))
        .id
        .clone()
}
pub(super) fn mount(
    cx: &mut TestAppContext,
    width: f32,
) -> (
    tokio::runtime::Runtime,
    Entity<DatabasesScreen>,
    WindowHandle<Root>,
) {
    mount_sized(cx, width, 880.)
}
pub(super) fn mount_sized(
    cx: &mut TestAppContext,
    width: f32,
    height: f32,
) -> (
    tokio::runtime::Runtime,
    Entity<DatabasesScreen>,
    WindowHandle<Root>,
) {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(1)
        .enable_all()
        .build()
        .unwrap();
    cx.update(|cx| {
        gpui_kit::init(cx);
        cx.set_reduce_motion(true);
        crate::theme::install(cx)
    });
    let mut page = None;
    let window = cx.open_window(size(px(width), px(height)), |window, cx| {
        let view = cx.new(|cx| {
            let mut view = DatabasesScreen::new(runtime.handle().clone(), window, cx);
            view.set_page_source(source("invented-access"), window, cx);
            view.set_visible(true, window, cx);
            view.activate(window, cx);
            view
        });
        page = Some(view.clone());
        Root::new(view, window, cx)
    });
    cx.run_until_parked();
    (runtime, page.unwrap(), window)
}
fn display_of(inv: &Inventory, mode: Mode) -> super::display::Display {
    super::display::Display::new(
        Some(inv),
        &databases::coroot::Evidence::default(),
        mode,
        at(),
    )
}
fn field<'a>(
    display: &'a super::display::Display,
    name: &str,
    label: &str,
) -> &'a super::display::Field {
    display
        .rows
        .iter()
        .find(|row| row.identity.name == name)
        .unwrap()
        .sections
        .iter()
        .flat_map(|section| &section.fields)
        .find(|field| field.label == label)
        .unwrap_or_else(|| panic!("no field {label}"))
}
