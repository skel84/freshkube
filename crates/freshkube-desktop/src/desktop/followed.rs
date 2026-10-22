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

impl Pilot {
    /// Offers the app's follows this window's connection, when it, the
    /// cluster's protection or the Coroot project it reads has changed.
    pub(super) fn offer_followed(&mut self, cx: &mut App) {
        let Some(followed) = followed(cx) else {
            return;
        };
        let Some(kube) = self.source().kube else {
            return;
        };
        // A protected or read-only cluster hides details by default: its
        // banners show on the lock screen.
        let sensitive = matches!(
            self.context_display.level,
            Protection::ReadOnly | Protection::Protected
        );
        let reader = self.pages.observability.read(cx).follow_reader();
        let offered = Offered {
            id: kube.id.clone(),
            sensitive,
            coroot: reader.as_ref().map(|(_, source)| source.clone()),
        };
        if self.followed_offer.as_ref() == Some(&offered) {
            return;
        }
        self.followed_offer = Some(offered);
        let connection = Connection {
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
                    let mut note =
                        Notification::new()
                            .title(words.title.clone())
                            .with_type(if *failure {
                                NotificationType::Warning
                            } else {
                                NotificationType::Info
                            });
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
