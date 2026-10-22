//! A link that could not be followed (#873): the drawer's slot says why, in
//! place of the object, with what to do next. Nothing here takes the
//! keyboard: opening a link never moves focus onto a button.

use super::*;
use freshkube_nav::LinkOutcome;

/// What the drawer shows for a link that opened nothing.
#[derive(Clone, Debug, PartialEq)]
pub struct LinkOutcomeCard {
    pub outcome: LinkOutcome,
    /// What the link named, as the reader sees it: `deployments.apps
    /// payments/checkout`.
    pub subject: String,
    /// For `Replaced`: the object that has the name now, which the card
    /// offers to open.
    pub current: Option<ResourceIdentity>,
}

impl ResourcesScreen {
    /// Shows `card` in the drawer's slot. The pane closes first: the card
    /// stands where the object would have.
    pub fn show_link_outcome(&mut self, card: LinkOutcomeCard, cx: &mut Context<Self>) {
        self.detail.update(cx, |detail, cx| detail.close(cx));
        self.link_outcome = Some(card);
        cx.notify();
    }

    /// Whether the list holds the keyboard: a link that opens nothing leaves
    /// it there, never on the card's buttons.
    pub fn list_focused(&self, window: &Window) -> bool {
        self.focus.is_focused(window)
    }

    /// The card the drawer shows, if any.
    pub fn link_outcome(&self) -> Option<&LinkOutcomeCard> {
        self.link_outcome.as_ref()
    }

    pub(super) fn render_link_outcome(
        &self,
        card: &LinkOutcomeCard,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let outcome = card.outcome;
        let icon = match outcome {
            LinkOutcome::CantRead => IconName::ShieldX,
            LinkOutcome::NotFound | LinkOutcome::NotServed => IconName::SearchX,
            LinkOutcome::Replaced => IconName::RefreshCw,
        };
        let close = Button::new("link-outcome-close")
            .small()
            .h(dp(ui::CONTROL_HEIGHT))
            .label("Close")
            .on_click(cx.listener(|view, _, window, cx| {
                view.hide_detail(cx);
                window.focus(&view.focus, cx);
            }));
        let current = card.current.clone().map(|identity| {
            Button::new("link-outcome-open-current")
                .small()
                .h(dp(ui::CONTROL_HEIGHT))
                .label("Open the current one")
                .on_click(cx.listener(move |view, _, window, cx| {
                    view.open_identity_on(identity.clone(), crate::Tab::Overview, window, cx)
                }))
        });
        let actions = current
            .into_iter()
            .map(IntoElement::into_any_element)
            .chain([close.into_any_element()])
            .collect();
        ui::empty_state(
            icon,
            outcome.title(),
            format!(
                "{}\n\nWhy: {}\nNext: {}",
                card.subject,
                outcome.why(),
                outcome.next()
            ),
            None,
            actions,
            cx,
        )
        .id(SharedString::from(format!(
            "link-outcome-{}",
            outcome.slug()
        )))
        .test_support()
        .role(Role::Status)
        .aria_label(SharedString::from(format!(
            "{}: {}",
            outcome.title(),
            card.subject
        )))
        .into_any_element()
    }
}
