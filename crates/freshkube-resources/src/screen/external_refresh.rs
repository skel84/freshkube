//! All selected-row Refresh entry points share one confirmation in the pane.
use super::*;

impl ResourcesScreen {
    pub(super) fn external_secret_selected(&self) -> bool {
        !self.embedded
            && self.kind.group == "external-secrets.io"
            && self.kind.plural == "externalsecrets"
            && self
                .projection
                .selected()
                .is_some_and(|id| !id.uid.is_empty() && self.store.get(id).is_some())
    }

    pub(super) fn refresh_external_secret(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        // The filter input shares no mutation command with the list.
        if !self.external_secret_selected()
            || self.query.read(cx).focus_handle(cx).is_focused(window)
        {
            return;
        }
        let identity = self.projection.selected().unwrap().clone();
        self.open_detail(identity, Duration::ZERO, cx);
        self.detail
            .update(cx, |pane, cx| pane.request_external_refresh(window, cx));
    }
}
