//! The Stage Inspector's actions (#855): Refresh, Reverify, Promote and
//! Abort. The page only offers them and says which Stage and change they are
//! for; the shell describes the write through `delivery::acts`, looks up the
//! levels, and core's `write` module sends it. Nothing here builds a request.
//!
//! Approvals stay in Kargo, so a Promote that isn't offered keeps the
//! header's Open in Kargo.

use super::detail::{ActionLine, Leads};
use super::*;
use freshkube_core::workspace::{Chosen, Entry, Part};
use freshkube_core::write::AccessQuestion;
use freshkube_delivery::acts::Slot;

/// Which action a button asks for: delivery's, which also decides what each
/// one is held to (`delivery::acts::describe`).
pub use freshkube_delivery::acts::Kind as ActKind;

/// Where Kargo is, so the shell can connect to write there.
#[derive(Clone, Debug)]
pub enum KargoAt {
    /// Example data: there is no cluster, and nothing is ever sent.
    Example,
    /// The open connection (the workspace of one).
    Open,
    /// This workspace entry.
    Entry(Box<Entry>),
}

/// A Stage action's button was pressed: what the shell needs to describe it.
#[derive(Clone, Debug)]
pub struct StageAct {
    pub kind: ActKind,
    pub stage: String,
    /// The change as it was read, for the preview and the handles.
    pub change: Box<Change>,
    pub kargo: KargoAt,
    /// The workspace's kubeconfig, `None` for the automatic one.
    pub kubeconfig: Option<std::path::PathBuf>,
}

impl ChangePage {
    /// Where Kargo is read, or why there is nowhere to write to.
    fn kargo_at(&self) -> Result<KargoAt, SharedString> {
        match &self.fetch {
            Fetch::Example { .. } => Ok(KargoAt::Example),
            Fetch::Live { place, .. } => match place.sources.get(Part::Kargo) {
                Chosen::Open => Ok(KargoAt::Open),
                Chosen::Entry { entry, .. } => Ok(KargoAt::Entry(Box::new(entry.clone()))),
                other => Err(other
                    .missing()
                    .unwrap_or_else(|| "Kargo's cluster isn't known".into())
                    .into()),
            },
        }
    }

    /// The Stage the selected hop shows, if it shows one.
    fn selected_stage(&self) -> Option<&Stage> {
        match self.change.hop(self.selected.as_ref()?)?.shows {
            Shows::Stage(ix) => self.change.stages.get(ix),
            Shows::Hop(_) => None,
        }
    }

    /// What the gate asks before each of the selected Stage's actions: core's
    /// description of each action says it. Only where the reviews are made on
    /// the cluster the page reads: Kargo's cluster is the open one, or the
    /// example. A Kargo on another workspace entry's cluster is reached with
    /// that entry's own connection, so it is not asked here, and its actions
    /// stay on offer for the gate to decide.
    fn stage_questions(&self) -> Vec<(ActKind, Vec<AccessQuestion>)> {
        if !matches!(self.kargo_at(), Ok(KargoAt::Open | KargoAt::Example)) {
            return Vec::new();
        }
        let Some(stage) = self.selected_stage() else {
            return Vec::new();
        };
        let Some(acts) = &stage.acts else {
            return Vec::new();
        };
        [
            ActKind::RefreshStage,
            ActKind::RefreshWarehouse,
            ActKind::Reverify,
            ActKind::Promote,
            ActKind::Abort,
        ]
        .into_iter()
        .filter_map(|kind| {
            let action =
                freshkube_delivery::acts::describe(kind, acts, &self.change, None, "", "")?;
            Some((kind, action.access_questions()))
        })
        .collect()
    }

    /// Asks whether the selected Stage's actions may be offered, for what
    /// isn't known: only the selection's, never every Stage's. Called when
    /// the selection or the change changes, never while drawing.
    pub(super) fn want_stage_offers(&mut self, cx: &mut Context<Self>) {
        let Some(desk) = self.offers.clone() else {
            return;
        };
        let questions = self.stage_questions();
        if self.visible {
            // Also when nothing is asked of the selection (a hop with no
            // Stage): the call replaces what the last selection wanted.
            let all: Vec<AccessQuestion> = questions
                .iter()
                .flat_map(|(_, asked)| asked.iter().cloned())
                .collect();
            let owner = cx.entity_id();
            desk.update(cx, |desk, cx| desk.want(owner, &all, cx));
        }
        let offers: Vec<_> = questions
            .iter()
            .map(|(kind, asked)| (*kind, desk.read(cx).offer(asked, cx)))
            .collect();
        if offers != self.stage_offers {
            self.stage_offers = offers;
            self.detail = self.derive_detail();
        }
    }

    /// An answer landed: the selected Stage's buttons are derived again if
    /// what they say changed.
    pub(super) fn offers_landed(&mut self, cx: &mut Context<Self>) {
        let before = self.stage_offers.clone();
        let Some(desk) = self.offers.clone() else {
            return;
        };
        let questions = self.stage_questions();
        self.stage_offers = questions
            .iter()
            .map(|(kind, asked)| (*kind, desk.read(cx).offer(asked, cx)))
            .collect();
        if self.stage_offers != before {
            self.detail = self.derive_detail();
            cx.notify();
        }
    }

    /// The buttons for a Stage's actions, each greyed out with why where it
    /// can't be offered.
    pub(super) fn stage_actions(&self, stage: &Stage) -> Vec<ActionLine> {
        self.with_offers(self.stage_buttons(stage))
    }

    /// What the cluster said, on top of what the page knows: a refused
    /// action is greyed out with its reason (any reason the page already had
    /// outranks it), and one whose review didn't answer says so and stays on.
    fn with_offers(&self, mut buttons: Vec<ActionLine>) -> Vec<ActionLine> {
        for button in &mut buttons {
            let Some(Leads::Act(kind, _)) = &button.leads else {
                continue;
            };
            let Some((_, offer)) = self.stage_offers.iter().find(|(asked, _)| asked == kind) else {
                continue;
            };
            if let Some(reason) = offer.refusal() {
                button.why = Some(reason.to_owned().into());
                button.leads = None;
            } else if let Some(note) = offer.note() {
                let tooltip = button.tooltip.take().unwrap_or_default();
                button.tooltip = Some(format!("{tooltip} {note}.").trim().to_owned().into());
            }
        }
        buttons
    }

    fn stage_buttons(&self, stage: &Stage) -> Vec<ActionLine> {
        let name = &stage.name;
        let line = |label: String, tooltip: &str, kind, why: Option<SharedString>| ActionLine {
            label: label.into(),
            why: why.clone(),
            tooltip: Some(tooltip.to_owned().into()),
            leads: why.is_none().then_some(Leads::Act(kind, name.clone())),
        };
        let unread = || -> SharedString {
            "The Stage's UID and version weren't read, so a change to it couldn't be checked."
                .into()
        };
        let no_kargo = self.kargo_at().err();
        let Some(acts) = &stage.acts else {
            return vec![
                line(
                    format!("Refresh {name}"),
                    "Adds the refresh annotation",
                    ActKind::RefreshStage,
                    Some(no_kargo.clone().unwrap_or_else(unread)),
                ),
                line(
                    format!("Promote to {name}"),
                    "Creates a Promotion of this Freight",
                    ActKind::Promote,
                    Some(no_kargo.unwrap_or_else(unread)),
                ),
            ];
        };
        let mut buttons = vec![line(
            format!("Refresh {name}"),
            "Adds kargo.akuity.io/refresh to the Stage. Needs no confirmation.",
            ActKind::RefreshStage,
            no_kargo.clone(),
        )];
        if acts.warehouse.is_some() {
            buttons.push(line(
                "Refresh Warehouse".to_owned(),
                "Adds kargo.akuity.io/refresh to the Freight's Warehouse. Needs no confirmation.",
                ActKind::RefreshWarehouse,
                no_kargo.clone(),
            ));
        }
        buttons.push(line(
            "Reverify".to_owned(),
            "Adds kargo.akuity.io/reverify to the Stage. Needs no confirmation.",
            ActKind::Reverify,
            no_kargo.clone().or_else(|| {
                acts.verification
                    .is_none()
                    .then(|| "No verification of this Freight was read to repeat.".into())
            }),
        ));
        // Promote becomes Abort while any promotion runs, someone else's too.
        buttons.push(match acts.slot() {
            Slot::Promote => line(
                format!("Promote to {name}"),
                "Shows what would change first. Approvals stay in Kargo.",
                ActKind::Promote,
                no_kargo,
            ),
            Slot::Abort(running) => line(
                "Abort promotion".to_owned(),
                &format!(
                    "Promotion {} is {}",
                    running.promotion.name,
                    running.phase.to_lowercase()
                ),
                ActKind::Abort,
                no_kargo,
            ),
            Slot::Not(why) => line(
                format!("Promote to {name}"),
                "Approvals stay in Kargo.",
                ActKind::Promote,
                Some(why.into()),
            ),
        });
        buttons
    }

    /// Asks the shell to describe and run the action for `stage`.
    pub(super) fn act(&mut self, kind: ActKind, stage: &str, cx: &mut Context<Self>) {
        let Ok(kargo) = self.kargo_at() else {
            return;
        };
        let kubeconfig = match &self.fetch {
            Fetch::Live { kubeconfig, .. } => kubeconfig.clone(),
            Fetch::Example { .. } => None,
        };
        cx.emit(ChangeEvent::Act(Box::new(StageAct {
            kind,
            stage: stage.to_owned(),
            change: Box::new(self.change.clone()),
            kargo,
            kubeconfig,
        })));
    }
}
