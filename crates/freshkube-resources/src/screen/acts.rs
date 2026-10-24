//! The everyday actions on the selected workload (#857): Scale, Restart, a
//! Deployment's Roll back and a CronJob's Suspend or Resume and Run now. The list only offers them, from the row menu,
//! its keys and the toolbar, and says which one on which object; the shell
//! describes and sends the write (`freshkube_core::write::everyday`). A row
//! never gets a button.
use super::*;
use freshkube_core::write::everyday::{Everyday, offered_for};
use freshkube_ui::menu::MenuAction;
use gpui_kit::component::Disableable;

/// A request to act on the selected workload, with the connection it was
/// listed from. The shell reads the object, describes the action and starts
/// it under the cluster's level.
#[derive(Clone, Debug)]
pub struct WorkloadAct {
    pub source: ClusterSource,
    pub identity: ResourceIdentity,
    pub kind: ResourceKind,
    pub act: Everyday,
}

impl EventEmitter<WorkloadAct> for ResourcesScreen {}

/// A request to review an edit of the open object's YAML (#858), with the
/// connection the pane read it from. `action` is core's description of the
/// edit; the shell takes the window's level and hands it to the write flow.
#[derive(Clone, Debug)]
pub struct EditYamlAct {
    pub source: ClusterSource,
    pub identity: ResourceIdentity,
    pub action: freshkube_core::write::ActionDescription,
}

impl EventEmitter<EditYamlAct> for ResourcesScreen {}

/// The words the menu, the toolbar and the tooltips use.
fn label(act: Everyday) -> &'static str {
    match act {
        Everyday::Scale => "Scale…",
        Everyday::Restart => "Restart",
        Everyday::Suspend => "Suspend or resume",
        Everyday::ArgoRefresh => "Refresh",
        Everyday::ArgoHardRefresh => "Hard refresh",
        Everyday::FluxReconcile => "Reconcile",
        Everyday::Renew => "Renew…",
        Everyday::Rollback => "Roll back…",
        Everyday::RunNow => "Run now",
    }
}

impl ResourcesScreen {
    /// Why writes are not allowed now, or `None`: the shell sets it from the
    /// cluster's level, so a read-only cluster shows the actions disabled
    /// with the reason instead of hiding them.
    pub fn set_write_block(&mut self, block: Option<&'static str>, cx: &mut Context<Self>) {
        self.detail
            .update(cx, |detail, cx| detail.set_write_block(block, cx));
        if self.write_block != block {
            self.write_block = block;
            cx.notify();
        }
    }

    /// The actions the listed kind has. None in the node pane's list.
    fn kind_acts(&self) -> &'static [Everyday] {
        if self.embedded {
            return &[];
        }
        offered_for(&self.kind.group, &self.kind.kind)
    }

    /// The actions on offer for the selected row: its kind's, except Scale
    /// for a ReplicaSet the pane shows a Deployment owning. The list does not
    /// know an owner; only a row whose pane is open can say, and core refuses
    /// an owned ReplicaSet after it reads the object whatever was offered.
    pub(super) fn acts(&self, cx: &App) -> Vec<Everyday> {
        let owned = self.projection.selected().is_some_and(|identity| {
            self.detail.read(cx).controller_of(identity) == Some("Deployment")
        });
        self.kind_acts()
            .iter()
            .copied()
            .filter(|act| !(owned && *act == Everyday::Scale))
            .collect()
    }

    /// The selected row's actions as menu lines, after a separator.
    pub(super) fn act_menu(&self, cx: &App) -> Vec<MenuAction> {
        let acts = self.acts(cx);
        if acts.is_empty() || self.projection.selected().is_none() {
            return Vec::new();
        }
        let mut lines = vec![MenuAction::Separator];
        for act in acts {
            let line = match act {
                Everyday::Scale => MenuAction::new(label(act), ScaleSelected),
                Everyday::Restart => MenuAction::new(label(act), RestartSelected),
                Everyday::Suspend => MenuAction::new(label(act), SuspendSelected),
                Everyday::ArgoRefresh => MenuAction::new(label(act), RefreshSelected),
                Everyday::ArgoHardRefresh => MenuAction::new(label(act), HardRefreshSelected),
                Everyday::FluxReconcile => MenuAction::new(label(act), ReconcileSelected),
                Everyday::Renew => MenuAction::new(label(act), RenewSelected),
                Everyday::Rollback => MenuAction::new(label(act), RollbackSelected),
                Everyday::RunNow => MenuAction::new(label(act), RunNowSelected),
            };
            lines.push(line.enabled(self.write_block.is_none()));
        }
        lines
    }

    /// Asks the shell to act on the selected row. Nothing happens for a
    /// kind that lacks the action, a blocked cluster or no selection: the
    /// controls are disabled then, and a key must not do what they cannot.
    pub(super) fn run_act(&mut self, act: Everyday, cx: &mut Context<Self>) {
        // The kind's actions, not the menu's: Scale is left out of the menu
        // for an owned ReplicaSet, but its key still asks, so the shell can say
        // why it does nothing ("scale its Deployment") instead of staying mute.
        if self.write_block.is_some() || !self.kind_acts().contains(&act) {
            return;
        }
        let (Some(identity), Some(source)) =
            (self.projection.selected().cloned(), self.source.clone())
        else {
            return;
        };
        cx.emit(WorkloadAct {
            source,
            identity,
            kind: self.kind.clone(),
            act,
        });
    }

    /// The toolbar's buttons for the selected row's actions, each with its
    /// folded form: disabled with the reason while writes are blocked.
    pub(super) fn act_buttons(&self, header: page::PageHeader, cx: &App) -> page::PageHeader {
        if self.projection.selected().is_none() {
            return header;
        }
        let mut header = header;
        for act in self.acts(cx) {
            let id = header.id(act.key());
            let button = Button::new(id)
                .ghost()
                .small()
                .h(dp(ui::CONTROL_HEIGHT))
                .label(label(act));
            let (button, fold) = match self.write_block {
                Some(reason) => (
                    button.disabled(true).tooltip(reason),
                    page::disabled_item(label(act)),
                ),
                None => {
                    let focus = &self.focus;
                    let fold = match act {
                        Everyday::Scale => page::action_item(label(act), ScaleSelected, focus),
                        Everyday::Restart => page::action_item(label(act), RestartSelected, focus),
                        Everyday::Suspend => page::action_item(label(act), SuspendSelected, focus),
                        Everyday::ArgoRefresh => {
                            page::action_item(label(act), RefreshSelected, focus)
                        }
                        Everyday::ArgoHardRefresh => {
                            page::action_item(label(act), HardRefreshSelected, focus)
                        }
                        Everyday::FluxReconcile => {
                            page::action_item(label(act), ReconcileSelected, focus)
                        }
                        Everyday::Renew => page::action_item(label(act), RenewSelected, focus),
                        Everyday::Rollback => {
                            page::action_item(label(act), RollbackSelected, focus)
                        }
                        Everyday::RunNow => page::action_item(label(act), RunNowSelected, focus),
                    };
                    let button = match act {
                        Everyday::Scale => button.on_click(page::dispatch(ScaleSelected, focus)),
                        Everyday::Restart => {
                            button.on_click(page::dispatch(RestartSelected, focus))
                        }
                        Everyday::Suspend => {
                            button.on_click(page::dispatch(SuspendSelected, focus))
                        }
                        Everyday::ArgoRefresh => {
                            button.on_click(page::dispatch(RefreshSelected, focus))
                        }
                        Everyday::ArgoHardRefresh => {
                            button.on_click(page::dispatch(HardRefreshSelected, focus))
                        }
                        Everyday::FluxReconcile => {
                            button.on_click(page::dispatch(ReconcileSelected, focus))
                        }
                        Everyday::Renew => button.on_click(page::dispatch(RenewSelected, focus)),
                        Everyday::Rollback => {
                            button.on_click(page::dispatch(RollbackSelected, focus))
                        }
                        Everyday::RunNow => button.on_click(page::dispatch(RunNowSelected, focus)),
                    };
                    (button, fold)
                }
            };
            header = header.foldable(button, fold);
        }
        header
    }
}
