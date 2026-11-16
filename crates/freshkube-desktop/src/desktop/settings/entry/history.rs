//! The Build history group: what the selected entry reads for finished
//! builds, Tekton Results and Tekton Chains, and what the last Check sources
//! found. Read only: where Results is reached is edited in Delivery sources
//! (the Results row), and the verifying key in its Supply chain row. The
//! page reads nothing here; a check runs there, and only on the entry Builds
//! is read from.
use super::SettingsPage;
use crate::desktop::settings::history;
use freshkube_ui::form;
use gpui_kit::prelude::*;
use gpui_kit::*;

impl SettingsPage {
    pub(super) fn render_build_history(&self, id: &str, cx: &mut Context<Self>) -> AnyElement {
        let results = self
            .workspace
            .clusters
            .iter()
            .find(|entry| entry.id == id)
            .and_then(|entry| entry.results.as_deref().cloned());
        let (kind, what) = history::route(results.as_ref());
        let forward = results.as_ref().and_then(|results| results.forward.clone());
        let found = self.sources.read(cx).history_for(id).cloned();
        let (state, hint) = match &found {
            Some(found) => {
                let mut state = format!("Chains {}", history::state(&found.chains).to_lowercase());
                let mut hint = history::chains_words(id, &found.chains);
                if let (Some(reach), Some(forward)) = (&found.results, &forward) {
                    state.push_str(&format!(
                        ", Results {}",
                        history::state(reach).to_lowercase()
                    ));
                    hint.push(' ');
                    hint.push_str(&history::results_words(id, &forward.service, reach));
                }
                (state, hint)
            }
            None => (
                "Not checked".to_owned(),
                "Check sources, in Delivery sources, looks for these on the entry Builds is read from, and on no other.".to_owned(),
            ),
        };
        form::group("settings-history", "Build history")
            .narrow(true)
            .row(form::row("settings-history-results-row", "Results", kind).hint(what))
            .row(
                form::row("settings-history-chains-row", "Chains", "Attestations").hint(
                    "Tekton Chains’ signed attestations, looked for where it runs. The verifying key is chosen in Delivery sources.",
                ),
            )
            .row(form::row("settings-history-state-row", "State", state).hint(hint))
            .saved_in(form::SavedIn::file("freshkube.yaml").and(" · results, in this entry"))
            .render(cx)
            .into_any_element()
    }
}
