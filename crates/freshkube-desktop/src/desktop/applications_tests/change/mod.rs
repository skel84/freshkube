//! The change page in the real app, on example data: checkout's Freight
//! `wonky-otter`, followed from its application page's Stage.
use crate::applications::testing::change::ChangePage;
use crate::applications::{ApplicationsPage, Variant};
use crate::desktop::{Page, Pilot, layout_check, tests::fixture};

use gpui_kit::{
    AnyWindowHandle, AppContext, Entity, SharedString, TestAppContext, test::TestWindowExt,
};

mod layout;
mod links;
mod logs;
mod navigation;
mod reads;
mod revisions;
mod sources;
mod stages;
mod tools;

const CHECKOUT: &str = "application-kargo:checkout";

/// checkout's Stage `prod-ams`, on its application page.
const PROD_AMS: &str = "application-part-cicd-fra/0/checkout/prod-ams";

fn list(view: &Entity<Pilot>, cx: &gpui_kit::App) -> Entity<ApplicationsPage> {
    view.read(cx).applications().0
}

fn change(view: &Entity<Pilot>, cx: &gpui_kit::App) -> Option<Entity<ChangePage>> {
    list(view, cx).read(cx).change_page().cloned()
}

fn hop(key: &str) -> SharedString {
    format!("change-hop-{key}").into()
}

/// checkout's page with its Stage prod-ams selected, before following it.
fn on_stage(
    cx: &mut TestAppContext,
    width: f32,
    height: f32,
) -> (tokio::runtime::Runtime, AnyWindowHandle, Entity<Pilot>) {
    on_stage_of(cx, width, height, Variant::Acme)
}

/// [`on_stage`] in the example reshaped as `variant`.
fn on_stage_of(
    cx: &mut TestAppContext,
    width: f32,
    height: f32,
    variant: Variant,
) -> (tokio::runtime::Runtime, AnyWindowHandle, Entity<Pilot>) {
    let (runtime, handle, view) = fixture(cx, width, height);
    cx.update_window(handle, |_, window, cx| {
        list(&view, cx).update(cx, |page, _| page.set_variant(variant));
        window.render_frame(cx);
        window.click("nav-applications", cx);
        window.render_frame(cx);
        window.click(CHECKOUT, cx);
        window.press("enter", cx);
        window.render_frame(cx);
        window.click(PROD_AMS, cx);
        window.render_frame(cx);
    })
    .unwrap();
    cx.run_until_parked();
    (runtime, handle, view)
}

/// The change, followed from prod-ams with F.
fn open(
    cx: &mut TestAppContext,
    width: f32,
    height: f32,
) -> (tokio::runtime::Runtime, AnyWindowHandle, Entity<Pilot>) {
    let (runtime, handle, view) = on_stage(cx, width, height);
    cx.update_window(handle, |_, window, cx| {
        window.press("f", cx);
        window.render_frame(cx);
    })
    .unwrap();
    cx.run_until_parked();
    (runtime, handle, view)
}

/// The acme workspace's entries; `prod-lon` isn't one.
const ENTRIES: [&str; 5] = ["core-fra", "cicd-fra", "dev-fra", "stage-fra", "prod-ams"];

/// As [`open`], with `active` the workspace entry open in the window, which
/// in example data stands for the open example cluster.
fn open_on(
    cx: &mut TestAppContext,
    width: f32,
    height: f32,
    active: &str,
) -> (tokio::runtime::Runtime, AnyWindowHandle, Entity<Pilot>) {
    let (runtime, handle, view) = open(cx, width, height);
    cx.update(|cx| {
        list(&view, cx).update(cx, |page, cx| {
            page.set_clusters(
                crate::applications::Clusters {
                    entries: ENTRIES.map(str::to_owned).to_vec(),
                    active: Some(active.into()),
                    ..Default::default()
                },
                cx,
            )
        })
    });
    cx.run_until_parked();
    (runtime, handle, view)
}

/// The same, with the workspace listing `entries`, dev-fra open.
fn open_with(
    cx: &mut TestAppContext,
    entries: &[&str],
    after_read: bool,
) -> (tokio::runtime::Runtime, AnyWindowHandle, Entity<Pilot>) {
    let (runtime, handle, view) = fixture(cx, 1280., 880.);
    let entries: Vec<String> = entries.iter().map(|e| e.to_string()).collect();
    let switch = move |view: &Entity<Pilot>, cx: &mut gpui_kit::App| {
        list(view, cx).update(cx, |page, cx| {
            page.set_clusters(
                crate::applications::Clusters {
                    entries: entries.clone(),
                    active: Some("dev-fra".into()),
                    mapping: Default::default(),
                    ..Default::default()
                },
                cx,
            );
        });
    };
    cx.update_window(handle, |_, window, cx| {
        list(&view, cx).update(cx, |page, _| page.set_variant(Variant::Acme));
        if !after_read {
            switch(&view, cx);
        }
        window.render_frame(cx);
        window.click("nav-applications", cx);
        window.render_frame(cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        if after_read {
            switch(&view, cx);
        }
        window.render_frame(cx);
        window.click(CHECKOUT, cx);
        window.press("enter", cx);
        window.render_frame(cx);
        window.click(PROD_AMS, cx);
        window.render_frame(cx);
        window.press("f", cx);
        window.render_frame(cx);
    })
    .unwrap();
    cx.run_until_parked();
    (runtime, handle, view)
}

/// The index of the Inspector's button labelled `label`.
fn action(view: &Entity<Pilot>, label: &str, cx: &gpui_kit::App) -> SharedString {
    let labels = change(view, cx).unwrap().read(cx).action_labels();
    let ix = labels
        .iter()
        .position(|l| l == label)
        .unwrap_or_else(|| panic!("{label} in {labels:?}"));
    format!("change-action-{ix}").into()
}
