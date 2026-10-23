//! The destinations list in the real app: mapping, changing and removing a
//! destination, saved to the file, and the list beside the clusters.
use super::super::SettingsSection;
use super::super::edit_tests::{open_section, read_workspace, saved, write};
use super::super::tests::launch;
use super::{Destinations, form};
use crate::desktop::{layout_check, tests::fixture};
use freshkube_core::workspace::{self, Destination, Key, Loaded};
use gpui_kit::{AppContext, TestAppContext, test::TestWindowExt};
use std::path::Path;

mod clusters;
mod dialogs;
mod find;
mod in_cluster;
mod layout;
mod mapping;
mod selection;
mod validation;

const PAGE: layout_check::TablePage = layout_check::TablePage {
    page: "settings-page",
    title: "settings-destinations-title",
    title_text: "Argo CD destinations",
    table: "settings-destinations-table-scroll",
    list: "settings-destinations-list",
};

/// One cluster, so the form picks it; and a mapping by name.
const ONE: &str = r#"{"version":1,"clusters":[
    {"id":"prod","role":"environment","context":"acme-prod"}],
  "destinations":[{"name":"prod-lon","entry":"prod","note":"kept"}]}"#;

fn destinations(directory: &Path) -> Vec<Destination> {
    read_workspace(directory).destinations
}

fn server(server: &str, entry: &str) -> Destination {
    Destination::new(Key::Server(server.into()), entry)
}

fn list<'a>(
    view: &gpui_kit::Entity<crate::desktop::Pilot>,
    cx: &'a gpui_kit::App,
) -> &'a super::Destinations {
    view.read(cx).settings_page.read(cx).destinations.read(cx)
}

fn name(name: &str) -> Key {
    Key::Name(name.into())
}

const TWO: &str = r#"{"version":1,"clusters":[
    {"id":"mgmt","role":"core","context":"acme-mgmt"},
    {"id":"prod","role":"environment","context":"acme-prod"}],
  "destinations":[
    {"server":"https://a.example.test","entry":"mgmt"},
    {"name":"prod-lon","entry":"prod"}]}"#;

fn assert_field_error(
    window: &gpui_kit::Window,
    view: &gpui_kit::Entity<crate::desktop::Pilot>,
    field: &str,
    why: &str,
    cx: &gpui_kit::App,
) {
    let form = list(view, cx).form().unwrap();
    assert_eq!(form.read(cx).field_error(field), Some(why));
    let error = window.find(format!("settings-destination-{field}-field-error"));
    assert!(error.visible());
    assert_eq!(error.role(), Some(gpui_kit::Role::Alert));
    assert!(window.try_find("settings-destination-error").is_none());
}
