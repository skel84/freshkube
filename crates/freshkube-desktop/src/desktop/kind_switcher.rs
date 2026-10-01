//! Command-K, "Go to kind": a palette of every Kubernetes kind the sidebar
//! offers, built-in and discovered, that opens the chosen one on the
//! Resources page from any page.

use std::rc::Rc;

use freshkube_core::resources::{ResourceKind, builtin};
use gpui_kit::component::WindowExt;
use gpui_kit::component::command::{Command, CommandGroup, CommandItem};
use gpui_kit::*;

use super::Pilot;
use crate::resources::custom::Discovery;
use crate::resources::navigation::NAVIGATION;

const WIDTH: f32 = 480.;

/// The palette's groups and, at the same positions, the kinds they open.
#[derive(Default)]
struct Catalogue {
    groups: Vec<CommandGroup>,
    kinds: Vec<Vec<ResourceKind>>,
}

impl Catalogue {
    fn push(&mut self, label: &'static str, entries: Vec<(CommandItem, ResourceKind)>) {
        if entries.is_empty() {
            return;
        }
        let (items, kinds): (Vec<_>, Vec<_>) = entries.into_iter().unzip();
        self.groups
            .push(CommandGroup::new().label(label).items(items));
        self.kinds.push(kinds);
    }
}

impl Pilot {
    /// The kinds as the sidebar groups them, with the custom kinds found so
    /// far. Built when the palette opens, so its render only clones them.
    fn kind_catalogue(&self, cx: &App) -> Catalogue {
        let mut catalogue = Catalogue::default();
        for group in &NAVIGATION {
            let entries = group
                .items
                .iter()
                .filter_map(|(label, key)| {
                    let kind = builtin(key)?;
                    let item = CommandItem::new()
                        .label(*label)
                        .keywords([key.to_string(), kind.kind.clone()]);
                    Some((item, kind))
                })
                .collect();
            catalogue.push(group.label, entries);
        }
        if let Some(Discovery::Loaded(groups)) = self.custom.read(cx).groups() {
            let entries = groups
                .iter()
                .filter_map(|group| match &group.kinds {
                    Some(Discovery::Loaded(rows)) => Some((group, &rows.kinds)),
                    _ => None,
                })
                .flat_map(|(group, rows)| {
                    rows.iter().map(move |row| {
                        let item = CommandItem::new()
                            .label(format!("{} · {}", row.label, group.name))
                            .keywords([row.key.clone()]);
                        (item, row.kind.clone())
                    })
                })
                .collect();
            catalogue.push("Custom Resources", entries);
        }
        catalogue
    }

    pub(super) fn open_kind_switcher(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if window.has_active_dialog(cx) {
            return;
        }
        let Catalogue { groups, kinds } = self.kind_catalogue(cx);
        let kinds = Rc::new(kinds);
        let state = self.kind_switcher.clone();
        state.update(cx, |state, cx| state.set_query("", window, cx));
        let pilot = cx.entity().downgrade();
        window.open_dialog(cx, move |dialog, _, _| {
            let (kinds, pilot) = (kinds.clone(), pilot.clone());
            let command = groups
                .iter()
                .cloned()
                .fold(Command::new(&state), Command::group);
            dialog.p_0().w(px(WIDTH)).close_button(false).child(
                command
                    .bordered(false)
                    .placeholder("Go to kind…")
                    .on_confirm(move |ix, window, cx| {
                        let Some(kind) = kinds
                            .get(ix.section)
                            .and_then(|kinds| kinds.get(ix.row))
                            .cloned()
                        else {
                            return;
                        };
                        window.close_dialog(cx);
                        _ = pilot.update(cx, |pilot, cx| pilot.open_kind(kind, window, cx));
                    }),
            )
        });
        // After opening: the dialog takes focus and remembers what had it,
        // to hand it back on Escape.
        self.kind_switcher
            .update(cx, |state, cx| state.focus(window, cx));
    }
}
