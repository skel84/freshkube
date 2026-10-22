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
use freshkube_delivery::acts::Slot;

/// Which action a button asks for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ActKind {
    RefreshStage,
    RefreshWarehouse,
    Reverify,
    Promote,
    Abort,
}

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

    /// The buttons for a Stage's actions, each greyed out with why where it
    /// can't be offered.
    pub(super) fn stage_actions(&self, stage: &Stage) -> Vec<ActionLine> {
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
