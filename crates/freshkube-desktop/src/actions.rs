//! Pieces shared by the actions that change a node from the desktop: the
//! Services restart and the Diagnostics fixes. Both run the core's
//! confirmed-fix executor, so the node is always pinned explicitly and the
//! cancellation check runs before each network step.
use std::hash::{DefaultHasher, Hash, Hasher};

use freshkube_core::diagnostic_runner::{
    DiagnosticFix, DiagnosticFixAction, DiagnosticFixRequest, DiagnosticTarget, FixExecution,
    execute_confirmed_fix,
};
use talos_rs::TalosClient;

/// Services whose restart can cut the node off from the cluster or from this
/// tool. A restart is still allowed, but only after an explicit warning.
const CRITICAL_SERVICES: [&str; 4] = ["etcd", "apid", "trustd", "machined"];

pub(crate) fn is_critical_service(id: &str) -> bool {
    CRITICAL_SERVICES.contains(&id)
}

/// What losing this service means, for the confirmation's warning.
pub(crate) fn critical_warning(id: &str) -> Option<&'static str> {
    match id {
        "etcd" => Some(
            "etcd holds the cluster's state. Restarting it can briefly break quorum and the Kubernetes API.",
        ),
        "apid" => Some(
            "apid is the Talos API this tool talks through. The node stops answering until it is back.",
        ),
        "trustd" => Some(
            "trustd hands out certificates to joining nodes. Nodes that need one during the restart will fail to join.",
        ),
        "machined" => Some(
            "machined supervises every other Talos service. Restarting it can take the whole node's services down.",
        ),
        _ => None,
    }
}

/// A stable-within-the-process identity for a preview.
pub(crate) fn fingerprint(parts: impl Hash) -> u64 {
    let mut hasher = DefaultHasher::new();
    parts.hash(&mut hasher);
    hasher.finish()
}

/// Runs a confirmed fix on the node in `target` and describes the outcome.
/// `Err` is a failure to show; a cancellation before the request was sent is
/// an `Ok` that says nothing changed.
pub(crate) async fn apply_fix(
    client: TalosClient,
    target: DiagnosticTarget,
    check_id: String,
    fix: DiagnosticFix,
    cancelled: impl Fn() -> bool + Send + Sync + 'static,
) -> Result<String, String> {
    let confirmed = DiagnosticFixRequest::new(check_id, target, fix).confirm();
    let mut is_cancelled = move || cancelled();
    match execute_confirmed_fix(&client, confirmed, &mut is_cancelled).await {
        Ok(FixExecution::Applied {
            results,
            requires_reboot,
        }) => {
            let mut text = String::from("Talos accepted the request");
            let answers: Vec<_> = results
                .iter()
                .map(|result| result.message.trim())
                .filter(|message| !message.is_empty())
                .collect();
            if !answers.is_empty() {
                text.push_str(": ");
                text.push_str(&answers.join("; "));
            }
            text.push('.');
            if requires_reboot {
                text.push_str(" The node is rebooting to apply it.");
            }
            Ok(text)
        }
        Ok(FixExecution::Cancelled { .. }) => {
            Ok("Cancelled before anything was sent; nothing changed.".to_owned())
        }
        Ok(FixExecution::CopyOnly { .. }) => {
            Err("This fix is guidance only and can't be applied.".to_owned())
        }
        Err(error) => Err(error.to_string()),
    }
}

/// The offline stand-in for [`apply_fix`]: says what would have happened and
/// never touches the network.
pub(crate) fn simulated(fix: &DiagnosticFix) -> String {
    match &fix.action {
        DiagnosticFixAction::RestartService { service_id } => {
            format!("Example data: pretended to restart {service_id}. Nothing was sent.")
        }
        _ => "Example data: pretended to apply the fix. Nothing was sent.".to_owned(),
    }
}

/// Whether the desktop may apply this fix, as opposed to showing it only.
pub(crate) fn is_applicable(fix: &DiagnosticFix) -> bool {
    matches!(
        fix.action,
        DiagnosticFixAction::RestartService { .. } | DiagnosticFixAction::ApplyConfigPatch { .. }
    )
}
