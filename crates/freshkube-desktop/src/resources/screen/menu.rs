//! The list's context menus (DESIGN.md change 10): a row's and a group's.
//! Each item is one of the list's actions, so it does what its key does,
//! and acts on the selection, which the right-click sets first.
use super::super::projection::{Cause, Group};
use super::*;
use table::RowAction;

impl ResourcesScreen {
    /// A right-click on a row selects it, as an arrow key would: an open
    /// drawer follows, a closed one stays closed.
    pub(super) fn select_for_menu(
        &mut self,
        identity: &ResourceIdentity,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.projection.selected() != Some(identity) {
            self.select_identity(identity, window, cx);
            if self.detail.read(cx).target_identity().is_some() {
                self.open_detail(identity.clone(), KEYBOARD_PAUSE, cx);
            }
        }
        window.focus(&self.focus, cx);
    }

    /// A group's menu selects its first row in sight, so the group's
    /// actions, which act on the selected row's group, act on it; they
    /// come with that row, for the menu to check it is still there. A
    /// folded group with no row in sight offers its fold alone.
    pub(super) fn group_menu(
        &mut self,
        group: &Group,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> (Vec<RowAction>, Option<ResourceIdentity>) {
        let first = (group.shown > 0)
            .then(|| self.projection.row(&self.store, group.start))
            .flatten()
            .map(|row| row.identity.clone());
        match first {
            Some(identity) => {
                self.select_for_menu(&identity, window, cx);
                (self.menu_actions(cx), Some(identity))
            }
            None => {
                window.focus(&self.focus, cx);
                (self.fold_action().into_iter().collect(), None)
            }
        }
    }

    /// The selected row's actions, with the list's keys.
    pub(super) fn menu_actions(&self, _cx: &App) -> Vec<RowAction> {
        let Some(identity) = self.projection.selected() else {
            return Vec::new();
        };
        let mut actions = vec![RowAction::new("Open", OpenSelected)];
        if self.lists_pods() {
            actions.push(RowAction::new("Logs", OpenLogs));
        }
        actions.push(RowAction::Separator);
        actions.push(RowAction::new(
            if self.marked.contains(identity) {
                "Unmark"
            } else {
                "Mark"
            },
            ToggleMark,
        ));
        if let Some(group) = self
            .selected_group()
            .filter(|group| group.cause != Cause::Healthy)
        {
            actions.push(RowAction::new(
                format!("Select all {} in group", group.total),
                SelectGroup,
            ));
        }
        if let Some(node) = self.selected_node() {
            actions.push(RowAction::Separator);
            actions.push(RowAction::new(format!("Open node {node}"), OpenNode));
        }
        if let Some(fold) = self.fold_action() {
            actions.push(RowAction::Separator);
            actions.push(fold);
        }
        actions
    }

    /// Expand or collapse the healthy pods, while they may fold.
    fn fold_action(&self) -> Option<RowAction> {
        self.projection.folds_healthy().then(|| {
            RowAction::new(
                if self.healthy_open {
                    "Collapse healthy pods"
                } else {
                    "Expand healthy pods"
                },
                ToggleHealthy,
            )
        })
    }
}
