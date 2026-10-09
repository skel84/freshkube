//! Settings › Workspace's second list: the workspace's Argo CD destinations,
//! each mapped to one of its clusters (`workspace.json`'s `destinations`).
//! It keeps its own selection, keys and toolbar under the clusters, and
//! saves through the page, as the clusters do. It derives its rows when the
//! page's workspace changes, so `render` only reads them.
//!
//! A row is selected by what it matches (its `Key`), found again whenever
//! the rows are derived, so a save or a Reload never moves the selection to
//! another row. A change names the row by its place in the file and what it
//! matched, and the page checks both before it saves.
mod form;
mod source;
#[cfg(test)]
mod tests;

use super::SettingsPage;
use crate::ui::{self, dp};
use freshkube_core::workspace::Key;
use freshkube_ui::{menu, page, table, table::TableSource as _};
use gpui_kit::component::{
    Disableable, Sizable, WindowExt,
    button::{Button, ButtonVariants},
    h_flex, v_flex,
};
use gpui_kit::prelude::*;
use gpui_kit::*;
use source::Column;

/// The list's id prefix: `settings-destinations-list`, `-title`, `-map`.
const PREFIX: &str = "settings-destinations";
/// The list's key context, around the table.
pub(in crate::desktop) const CONTEXT: &str = "SettingsDestinations";

gpui_kit::actions!(
    settings,
    [
        /// Selects the next destination.
        NextDestination,
        /// Selects the previous destination.
        PreviousDestination,
        /// Clears the destination selection.
        ClearDestination,
        /// Opens the form for a new destination mapping.
        MapDestination,
        /// Opens the form for the selected mapping.
        EditDestination,
        /// Asks to remove the selected mapping.
        RemoveDestination
    ]
);

/// One mapping, as the table shows it.
pub(crate) struct DestinationRow {
    key: Key,
    destination: SharedString,
    by: SharedString,
    entry: SharedString,
    /// Whether the workspace lists the entry; a row whose cluster is gone
    /// is kept, marked, and maps nothing.
    listed: bool,
    tooltip: SharedString,
    /// The row's id, `settings-destination-{place}`, and its mark's.
    id: SharedString,
    mark_id: SharedString,
    /// The mark's words, for a row whose cluster is gone.
    mark: SharedString,
}

pub(crate) struct Destinations {
    page: WeakEntity<SettingsPage>,
    rows: Vec<DestinationRow>,
    columns: Vec<Column>,
    width: f32,
    table: table::TableState,
    /// The page's workspace revision the rows were derived from.
    revision: u64,
    editable: bool,
    why_not: &'static str,
    /// The workspace's cluster ids, for the form's picker.
    entries: Vec<SharedString>,
    selected: Option<Key>,
    focus: FocusHandle,
    /// The form last opened, for tests to pick its cluster.
    #[cfg(test)]
    form: Option<WeakEntity<form::DestinationForm>>,
    _observe: Subscription,
}

impl Destinations {
    pub(super) fn new(page: &Entity<SettingsPage>, cx: &mut Context<Self>) -> Self {
        let (columns, width) = source::columns(&[]);
        Self {
            page: page.downgrade(),
            rows: Vec::new(),
            columns,
            width,
            table: table::TableState::new(PREFIX),
            revision: 0,
            editable: false,
            why_not: "",
            entries: Vec::new(),
            selected: None,
            focus: cx.focus_handle(),
            #[cfg(test)]
            form: None,
            _observe: cx.observe(page, |this, page, cx| this.follow(&page, cx)),
        }
    }

    /// Takes what changed on the page: its workspace, rederived only when
    /// it is a new one, and whether a change can be saved now.
    fn follow(&mut self, page: &Entity<SettingsPage>, cx: &mut Context<Self>) {
        let page = page.read(cx);
        let (editable, why_not) = (page.editable(), page.why_not_editable());
        let changed = page.revision() != self.revision;
        if changed {
            self.revision = page.revision();
            let workspace = page.workspace();
            self.entries = workspace
                .clusters
                .iter()
                .map(|entry| entry.id.clone().into())
                .collect();
            self.rows = workspace
                .destinations
                .iter()
                .enumerate()
                .map(|(at, row)| {
                    let key = row.key();
                    let listed = workspace.clusters.iter().any(|e| e.id == row.entry);
                    let by = match key {
                        Key::Server(_) => "Server URL",
                        Key::Name(_) => "Cluster name",
                    };
                    let mut tooltip = format!("{} → {}", key.describe(), row.entry);
                    if !listed {
                        tooltip.push_str(&format!(
                            "\n{} isn’t in the workspace, so this maps nothing; edit it to \
                             pick another cluster",
                            row.entry
                        ));
                    }
                    DestinationRow {
                        destination: key.value().to_owned().into(),
                        by: by.into(),
                        entry: row.entry.clone().into(),
                        listed,
                        tooltip: tooltip.into(),
                        id: format!("settings-destination-{at}").into(),
                        mark_id: format!("settings-destination-{at}-mark").into(),
                        mark: format!("{} isn’t in the workspace", row.entry).into(),
                        key,
                    }
                })
                .collect();
            (self.columns, self.width) = source::columns(&self.rows);
            if self
                .selected
                .as_ref()
                .is_some_and(|key| self.line_of(key).is_none())
            {
                self.selected = None;
            }
        }
        if changed || (editable, why_not) != (self.editable, self.why_not) {
            (self.editable, self.why_not) = (editable, why_not);
            cx.notify();
        }
    }

    pub(super) fn select(&mut self, key: Key, cx: &mut Context<Self>) {
        self.selected = Some(key);
        table::reveal(self, ScrollStrategy::Nearest);
        cx.notify();
    }

    /// Up from the first mapping hands the keyboard back to the clusters,
    /// on their last row; the clusters' Down past their last comes here.
    fn step(&mut self, delta: isize, window: &mut Window, cx: &mut Context<Self>) {
        let first = self.rows.first().map(|row| &row.key);
        if delta < 0
            && self.selected.is_some()
            && self.selected.as_ref() == first
            && let Some(page) = self.page.upgrade()
        {
            page.update(cx, |page, cx| page.enter_from_below(window, cx));
            return;
        }
        if let Some(key) = table::step(self, delta, cx) {
            self.select(key, cx);
        }
    }

    /// Takes the keyboard from the clusters' last row, onto the first
    /// mapping. Returns false when there is none to go to.
    pub(super) fn enter_from_above(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        let Some(key) = self.rows.first().map(|row| row.key.clone()) else {
            return false;
        };
        self.select(key, cx);
        self.focus(window, cx);
        true
    }

    fn clear_selection(&mut self, cx: &mut Context<Self>) {
        if self.selected.take().is_some() {
            cx.notify();
        } else {
            cx.propagate();
        }
    }

    fn focus(&self, window: &mut Window, cx: &mut App) {
        window.focus(&self.focus, cx);
    }

    fn selected_row(&self) -> Option<(usize, &DestinationRow)> {
        let at = self.line_of(self.selected.as_ref()?)?;
        Some((at, &self.rows[at]))
    }

    /// Whether the form can open: a change can be saved, and there is a
    /// cluster to map to.
    fn mappable(&self) -> bool {
        self.editable && !self.entries.is_empty()
    }

    fn open_form(&mut self, editing: bool, window: &mut Window, cx: &mut Context<Self>) {
        if !self.mappable() || window.has_active_dialog(cx) {
            return;
        }
        let Some(page) = self.page.upgrade() else {
            return;
        };
        let editing = match editing {
            true => match self.selected_row() {
                Some((at, row)) => Some(form::Editing {
                    at,
                    key: row.key.clone(),
                    entry: row.entry.clone(),
                    listed: row.listed,
                }),
                None => return,
            },
            false => None,
        };
        // The dialog gives the keyboard back to what had it when it opened:
        // this list, whether a key or the toolbar opened it.
        self.focus(window, cx);
        let _form = form::open(
            page,
            cx.entity().downgrade(),
            editing,
            self.entries.clone(),
            window,
            cx,
        );
        #[cfg(test)]
        {
            self.form = Some(_form.downgrade());
        }
    }

    fn ask_remove_selected(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some((at, key)) = self.selected_row().map(|(at, row)| (at, row.key.clone())) else {
            return;
        };
        if !self.editable || window.has_active_dialog(cx) {
            return;
        }
        self.focus(window, cx);
        let page = self.page.clone();
        let what = key.describe();
        window.open_dialog(cx, move |dialog, window, _| {
            let confirm_page = page.clone();
            let confirm_key = key.clone();
            dialog
                .w(ui::dp_px(420., window))
                .title(format!("Remove the mapping for {what}?"))
                .child(
                    v_flex()
                        .id("settings-destination-remove-dialog")
                        .test_support()
                        .role(Role::Dialog)
                        .aria_label(format!("Remove the mapping for {what}"))
                        .gap_3()
                        .text_size(dp(13.))
                        .child(
                            "Argo CD Applications with this destination show it as Unknown \
                             again. Nothing on a cluster is touched.",
                        )
                        .child(
                            h_flex()
                                .gap_2()
                                .justify_end()
                                .child(
                                    Button::new("settings-destination-remove-cancel")
                                        .outline()
                                        .label("Cancel")
                                        .on_click(|_, window, cx| window.close_dialog(cx)),
                                )
                                .child(
                                    Button::new("settings-destination-remove-confirm")
                                        .danger()
                                        .label("Remove")
                                        .on_click(move |_, window, cx| {
                                            window.close_dialog(cx);
                                            _ = confirm_page.update(cx, |page, cx| {
                                                page.remove_destination(at, &confirm_key, cx)
                                            });
                                        }),
                                ),
                        ),
                )
        });
    }

    /// The list's toolbar: its name, Map, and the actions on the selected
    /// mapping, each off with why in its tooltip.
    fn render_header(&self, window: &mut Window, cx: &mut Context<Self>) -> Div {
        let selected = self.selected_row().is_some();
        let map_why = if self.entries.is_empty() {
            "Map: add a cluster first"
        } else {
            "Map an Argo CD destination to a workspace cluster"
        };
        let (map, map_fold) = self.render_action(
            "map",
            "Map",
            self.mappable(),
            map_why,
            MapDestination,
            |this, window, cx| this.open_form(false, window, cx),
            cx,
        );
        let (edit, edit_fold) = self.render_action(
            "edit",
            "Edit",
            self.mappable() && selected,
            "Change the selected mapping",
            EditDestination,
            |this, window, cx| this.open_form(true, window, cx),
            cx,
        );
        let (remove, remove_fold) = self.render_action(
            "remove",
            "Remove",
            self.editable && selected,
            "Remove the selected mapping",
            RemoveDestination,
            |this, window, cx| this.ask_remove_selected(window, cx),
            cx,
        );
        page::PageHeader::new(PREFIX, "Argo CD destinations")
            .foldable(map, map_fold)
            .foldable(edit, edit_fold)
            .foldable(remove, remove_fold)
            .render(window, cx)
    }

    /// A toolbar button and its folded form, as the clusters' are.
    #[allow(clippy::too_many_arguments)]
    fn render_action(
        &self,
        id: &str,
        label: &'static str,
        enabled: bool,
        tooltip: &'static str,
        action: impl Action,
        run: impl Fn(&mut Self, &mut Window, &mut Context<Self>) + 'static,
        cx: &Context<Self>,
    ) -> (Button, page::MenuItems) {
        let handler = page::handler(cx, run);
        let tooltip = if !self.editable {
            self.why_not.to_owned()
        } else if enabled || id == "map" {
            tooltip.to_owned()
        } else {
            format!("{label}: select a mapping first")
        };
        let button = Button::new(SharedString::from(format!("{PREFIX}-{id}")))
            .outline()
            .small()
            .h(dp(ui::CONTROL_HEIGHT))
            .label(label)
            .disabled(!enabled)
            .tooltip_with_action(tooltip, &action, Some(CONTEXT))
            .on_click(move |_, window, cx| handler(window, cx));
        let fold = page::action_entry(
            menu::MenuAction::new(label, action).enabled(enabled),
            &self.focus,
        );
        (button, fold)
    }

    #[cfg(test)]
    pub(super) fn selected(&self) -> Option<&Key> {
        self.selected.as_ref()
    }

    #[cfg(test)]
    fn form(&self) -> Option<Entity<form::DestinationForm>> {
        self.form.as_ref()?.upgrade()
    }

    #[cfg(test)]
    pub(super) fn is_focused(&self, window: &Window) -> bool {
        self.focus.is_focused(window)
    }

    #[cfg(test)]
    pub(super) fn rows(&self) -> impl Iterator<Item = (&str, &str, bool)> {
        self.rows
            .iter()
            .map(|row| (row.destination.as_ref(), row.entry.as_ref(), row.listed))
    }
}

impl Render for Destinations {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let short = page::is_short(window);
        let header = self.render_header(window, cx);
        v_flex()
            .id("settings-destinations")
            .flex_1()
            .min_h_0()
            .child(
                page::toolbar(cx)
                    .border_t_1()
                    .border_color(crate::palette::palette(cx).line)
                    .child(header),
            )
            .child(
                div()
                    .key_context(CONTEXT)
                    .track_focus(&self.focus)
                    .on_action(
                        cx.listener(|this, _: &NextDestination, window, cx| {
                            this.step(1, window, cx)
                        }),
                    )
                    .on_action(cx.listener(|this, _: &PreviousDestination, window, cx| {
                        this.step(-1, window, cx)
                    }))
                    .on_action(
                        cx.listener(|this, _: &ClearDestination, _, cx| this.clear_selection(cx)),
                    )
                    .on_action(cx.listener(|this, _: &MapDestination, window, cx| {
                        this.open_form(false, window, cx)
                    }))
                    .on_action(cx.listener(|this, _: &EditDestination, window, cx| {
                        this.open_form(true, window, cx)
                    }))
                    .on_action(cx.listener(|this, _: &RemoveDestination, window, cx| {
                        this.ask_remove_selected(window, cx)
                    }))
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_h_0()
                    .when(short, |this| this.min_h(dp(page::SHORT_LIST_HEIGHT)))
                    .child(table::data_table(self, window, cx).flex_1().min_h_0()),
            )
    }
}

/// The list's keys, as the clusters' are: A maps, Enter or E edits,
/// Backspace removes.
pub(in crate::desktop) fn key_bindings() -> Vec<KeyBinding> {
    vec![
        KeyBinding::new("down", NextDestination, Some(CONTEXT)),
        KeyBinding::new("up", PreviousDestination, Some(CONTEXT)),
        KeyBinding::new("escape", ClearDestination, Some(CONTEXT)),
        KeyBinding::new("a", MapDestination, Some(CONTEXT)),
        KeyBinding::new("enter", EditDestination, Some(CONTEXT)),
        KeyBinding::new("e", EditDestination, Some(CONTEXT)),
        KeyBinding::new("backspace", RemoveDestination, Some(CONTEXT)),
    ]
}
