//! Changes' lifecycle and explicit commands; the shared state is in Session.
use super::*;
use crate::results_forward::{Binding, Target};
use gpui_kit::component::{
    Disableable, Sizable,
    button::{Button, ButtonVariants},
};

impl ChangesPage {
    pub(super) fn configure_results(&mut self, cx: &mut Context<Self>) {
        let Some(source) = &self.source else {
            self.results = Binding::default();
            return;
        };
        let target = Target::of(
            &self.across.sources,
            self.across.mapping.open.as_deref(),
            source,
        );
        let changed = self.results.configure(
            target,
            source,
            self.runtime.clone(),
            Self::results_changed,
            cx,
        );
        if changed {
            self.loader.reset();
            self.stale = None;
            self.show_read(cx);
        }
    }

    fn results_changed(&mut self, cx: &mut Context<Self>) {
        self.results.sync(cx);
        self.loader.reset();
        self.stale = None;
        self.show_read(cx);
        if self.visible {
            self.read(cx);
        }
        cx.notify();
    }

    pub(super) fn forward_results(&mut self, cx: &mut Context<Self>) {
        let Some(session) = self.results.session.clone() else {
            return;
        };
        if let Some((entry, link)) = session.update(cx, |session, cx| session.act(cx)) {
            cx.emit(ChangesEvent::Switch(entry, Box::new(link)));
        }
    }

    pub(super) fn results_button(&self, cx: &mut Context<Self>) -> Option<Button> {
        let session = self.results.session.as_ref()?.read(cx);
        Some(
            Button::new("changes-forward-results")
                .ghost()
                .small()
                .label(session.label.clone())
                .tooltip(session.words.clone())
                .disabled(!session.enabled)
                .on_click(cx.listener(|this, _, _, cx| this.forward_results(cx))),
        )
    }
}
