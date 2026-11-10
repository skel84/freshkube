//! The everyday actions on the selected workload (#857): Scale, Restart, a
//! Deployment's Roll back and a CronJob's Suspend or Resume and Run now. The list only offers them, from the row menu,
//! its keys and the toolbar, and says which one on which object; the shell
//! describes and sends the write (`freshkube_core::write::everyday`). A row
//! never gets a button.
use super::*;
use freshkube_core::access::Offer;
use freshkube_core::write::AccessQuestion;
use freshkube_core::write::delete;
use freshkube_core::write::everyday::{Everyday, Row, acts_for};
use freshkube_ui::menu::MenuAction;
use freshkube_ui::tooltip::FollowTooltip as _;
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
        Everyday::Renew => "Renew",
        Everyday::ExternalSecretRefresh => "Refresh ExternalSecret",
        Everyday::Rollback => "Roll back…",
        Everyday::RunNow => "Run now",
        Everyday::Delete => "Delete…",
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

    /// The selected row, as core names what an action is about.
    fn act_row(&self) -> Option<Row> {
        let identity = self.projection.selected()?;
        Some(Row {
            group: self.kind.group.clone(),
            version: self.kind.version.clone(),
            kind: self.kind.kind.clone(),
            resource: self.kind.plural.clone(),
            namespace: (!identity.namespace.is_empty()).then(|| identity.namespace.clone()),
            name: identity.name.clone(),
            uid: identity.uid.clone(),
        })
    }

    /// What the gate asks before `act` on the selected row, which core
    /// declares (`Everyday::needs`); none with no selection.
    fn act_needs(&self, act: Everyday) -> Vec<AccessQuestion> {
        self.act_row()
            .map(|row| act.needs(&row))
            .unwrap_or_default()
    }

    /// Asks the cluster whether the actions the selection shows may be
    /// offered, for what isn't known: only the selected row's, and the open
    /// pane's Edit, never every row's, in one call that replaces what the
    /// last one wanted. Called when the selection changes or the page shows,
    /// and never while drawing. The answers only guide; the gate asks afresh.
    pub(super) fn want_offers(&mut self, cx: &mut Context<Self>) {
        if !self.visible || self.embedded {
            return;
        }
        // With nothing selected the call is empty, and still replaces what
        // the row just left wanted, so it is not asked about.
        let mut questions: Vec<AccessQuestion> = Vec::new();
        if self.projection.selected().is_some() {
            questions = self
                .kind_acts()
                .into_iter()
                .flat_map(|act| self.act_needs(act))
                .collect();
            questions.extend(self.detail.read(cx).edit_needs());
        }
        let owner = cx.entity_id();
        self.offers
            .update(cx, |offers, cx| offers.want(owner, &questions, cx));
        self.derive_offers(cx);
    }

    /// What the cluster has said of each action on the selected row so far,
    /// worked out here and read while drawing.
    pub(super) fn derive_offers(&mut self, cx: &mut Context<Self>) {
        let selected = self.projection.selected().cloned();
        let derived: Vec<(Everyday, Offer)> = match (&selected, self.embedded) {
            (Some(_), false) => self
                .kind_acts()
                .into_iter()
                .map(|act| {
                    let needs = self.act_needs(act);
                    let offer = if needs.is_empty() {
                        Offer::Pending
                    } else {
                        self.offers.read(cx).offer(&needs, cx)
                    };
                    (act, offer)
                })
                .collect(),
            _ => Vec::new(),
        };
        self.act_offers = derived;
        self.act_offers_for = selected;
    }

    /// What the cluster said of `act` on the selected row: nothing is said of
    /// another row than the one it was derived for.
    pub(super) fn act_offer(&self, act: Everyday) -> Offer {
        if self.act_offers_for.as_ref() != self.projection.selected() {
            return Offer::Pending;
        }
        self.act_offers
            .iter()
            .find(|(asked, _)| *asked == act)
            .map_or(Offer::Pending, |(_, offer)| offer.clone())
    }

    /// The actions the listed kind has. None in the node pane's list.
    fn kind_acts(&self) -> Vec<Everyday> {
        if self.embedded {
            return Vec::new();
        }
        acts_for(&self.kind.group, &self.kind.kind)
    }

    /// Why this action is unavailable now, or `None`: the cluster's level,
    /// or, for Delete, a kind that is not deleted from here.
    /// The level's reason wins over a kind's, and a kind's over the cluster's
    /// ("Needs patch on deployments in shop"): a review that didn't answer
    /// blocks nothing here.
    pub(super) fn act_block(&self, act: Everyday) -> Option<SharedString> {
        self.write_block
            .or_else(|| {
                (act == Everyday::Delete)
                    .then(|| delete::refusal(&self.kind.group, &self.kind.kind))
                    .flatten()
            })
            .map(SharedString::from)
            .or_else(|| {
                self.act_offer(act)
                    .refusal()
                    .map(|reason| SharedString::from(reason.to_owned()))
            })
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
            .into_iter()
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
                Everyday::ExternalSecretRefresh => {
                    MenuAction::new(label(act), RefreshExternalSecret)
                }
                Everyday::Rollback => MenuAction::new(label(act), RollbackSelected),
                Everyday::RunNow => MenuAction::new(label(act), RunNowSelected),
                Everyday::Delete => MenuAction::new(label(act), DeleteSelected),
            };
            lines.push(match self.act_block(act) {
                Some(reason) => line.refused(reason),
                None => line,
            });
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
        if self.write_block.is_some()
            || !self.kind_acts().contains(&act)
            || self.act_offer(act).refusal().is_some()
        {
            return;
        }
        // A kind Delete is refused on still asks: the shell says why, once,
        // instead of the key staying mute.
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
            let (button, fold) = match self.act_block(act) {
                // The reason belongs to a wrapper that draws it while the
                // pointer is over the button, so it doesn't depend on the
                // disabled button's own hover.
                Some(reason) => (
                    div()
                        .id(header.id(&format!("{}-why", act.key())))
                        .child(button.disabled(true))
                        .follow_tooltip(reason.clone())
                        .into_any_element(),
                    page::Fold::from(page::refused_item(label(act), reason.clone()))
                        .stamped(Some(reason)),
                ),
                None => {
                    let focus = &self.focus;
                    // On offer, but the review didn't answer: say so.
                    let button = match self.act_offer(act).note() {
                        Some(note) => button.tooltip(note),
                        None => button,
                    };
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
                        Everyday::ExternalSecretRefresh => {
                            page::action_item(label(act), RefreshExternalSecret, focus)
                        }
                        Everyday::Rollback => {
                            page::action_item(label(act), RollbackSelected, focus)
                        }
                        Everyday::RunNow => page::action_item(label(act), RunNowSelected, focus),
                        Everyday::Delete => page::action_item(label(act), DeleteSelected, focus),
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
                        Everyday::ExternalSecretRefresh => {
                            button.on_click(page::dispatch(RefreshExternalSecret, focus))
                        }
                        Everyday::Rollback => {
                            button.on_click(page::dispatch(RollbackSelected, focus))
                        }
                        Everyday::RunNow => button.on_click(page::dispatch(RunNowSelected, focus)),
                        Everyday::Delete => button.on_click(page::dispatch(DeleteSelected, focus)),
                    };
                    let fold = page::Fold::from(fold).stamped(None::<SharedString>);
                    (button.into_any_element(), fold)
                }
            };
            header = header.foldable(button, fold);
        }
        header
    }
}
