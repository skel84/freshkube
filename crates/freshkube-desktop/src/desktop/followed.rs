//! The shell's side of the app's follows (`freshkube-followed`): it offers
//! the follows the connection of the cluster the window is on, and shows
//! what they tell as a toast. The follows themselves belong to the app, not
//! to this window (docs/STATE_LIFETIMES.md, "User-started sessions").

use freshkube_core::write::Protection;
use freshkube_followed::live::Coroot;
use freshkube_followed::owner::{Connection, Event, followed};
use gpui_kit::component::{
    WindowExt,
    notification::{Notification, NotificationType},
};
use gpui_kit::*;

use super::Pilot;

/// What the follows were last offered, to offer again only on a change.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Offered {
    id: String,
    sensitive: bool,
    coroot: Option<freshkube_core::coroot::Source>,
}

/// Whether a level hides details in banners by default: a protected or
/// read-only cluster's banners can show on a lock screen.
pub(super) fn hides_by_default(level: Protection) -> bool {
    matches!(level, Protection::ReadOnly | Protection::Protected)
}

impl Pilot {
    /// Offers the app's follows this window's connection and the level it
    /// works under, as one pair read at the same moment, when either or the
    /// Coroot project has changed. The cheap keys are compared first; the
    /// provider is cloned only for an offer.
    pub(super) fn offer_followed(&mut self, cx: &mut App) {
        let Some(followed) = followed(cx) else {
            return;
        };
        let Some(kube) = self.source().kube else {
            return;
        };
        let sensitive = hides_by_default(self.current_level(cx));
        let observability = self.pages.observability.read(cx);
        let unchanged = self.followed_offer.as_ref().is_some_and(|offered| {
            offered.id == kube.id
                && offered.sensitive == sensitive
                && offered.coroot.as_ref() == observability.follow_source()
        });
        if unchanged {
            return;
        }
        let reader = observability.follow_reader();
        self.followed_offer = Some(Offered {
            id: kube.id.clone(),
            sensitive,
            coroot: reader.as_ref().map(|(_, source)| source.clone()),
        });
        let connection = Connection {
            id: kube.id.clone(),
            context: kube.context.clone(),
            access: kube.access.clone(),
            sensitive,
            coroot: reader.map(|(provider, source)| Coroot { provider, source }),
        };
        followed.update(cx, |followed, cx| {
            followed.offer(connection.clone(), cx);
            followed.seed_example(&connection, cx);
        });
    }

    /// Shows what the follows tell as a toast, and offers them the
    /// connection again as Observability's Coroot project changes.
    pub(super) fn watch_followed(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Vec<Subscription> {
        let mut subscriptions = Vec::new();
        if let Some(followed) = followed(cx) {
            subscriptions.push(cx.subscribe_in(
                &followed,
                window,
                |_, _, event: &Event, window, cx| {
                    let Event::Toast { words, failure } = event;
                    let mut note = Notification::new().title(words.title.clone());
                    // A failure wears the bell's failure glyph and colour.
                    note = if *failure {
                        note.with_type(NotificationType::Error).icon(
                            gpui_kit::component::Icon::new(gpui_kit::assets::IconName::CircleX)
                                .text_color(freshkube_ui::palette::palette(cx).crit_ink),
                        )
                    } else {
                        note.with_type(NotificationType::Info)
                    };
                    if let Some(body) = &words.body {
                        note = note.message(body.clone());
                    }
                    window.push_notification(note, cx);
                },
            ));
        }
        let observability = self.pages.observability.clone();
        subscriptions.push(cx.observe(&observability, |this, _, cx| this.offer_followed(cx)));
        subscriptions
    }
}
