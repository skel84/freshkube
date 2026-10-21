use super::*;
use freshkube_ui::ui::field;

impl DetailPane {
    pub(in crate::pane) fn external_secret_section(
        &self,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        self.external.secret.as_ref()?;
        let view = &self.external;
        let line = |line: &Line, cx: &App| {
            v_flex()
                .id(line.id.clone())
                .test_support()
                .role(Role::Status)
                .aria_label(line.accessible.clone())
                .gap_1()
                .child(h_flex().gap_2().child(line.label.clone()).child(ui::tag(
                    line.tone,
                    None,
                    line.status.clone(),
                    cx,
                )))
                .child(div().text_size(ui::dp(12.)).child(line.detail.clone()))
        };
        Some(
            v_flex()
                .id("external-secret-summary")
                .test_support()
                .gap_3()
                .child(ui::caption("Sync status", cx))
                .when(view.conditions.is_empty(), |this| {
                    this.child(
                        div()
                            .id("external-unknown")
                            .test_support()
                            .child("No Ready or SecretSynced condition reported"),
                    )
                })
                .children(view.conditions.iter().map(|condition| line(condition, cx)))
                .child(field("Last refresh", view.refreshed.clone(), cx))
                .child(field("Refresh interval", view.interval.clone(), cx))
                .child(field("Refresh policy", view.policy.clone(), cx))
                .child(ui::caption("Store health", cx))
                .when(view.stores.is_empty(), |this| {
                    this.child("No supported store reference; store health unknown")
                })
                .children(view.stores.iter().map(|store| line(store, cx)))
                .children(view.note.clone())
                .into_any_element(),
        )
    }
}
