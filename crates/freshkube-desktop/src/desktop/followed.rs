//! The shell's side of the app's follows (`freshkube-followed`): it offers
//! the follows the connection of the cluster the window is on, and shows
//! what they tell as a toast. The follows themselves belong to the app, not
//! to this window (docs/STATE_LIFETIMES.md, "User-started sessions").

use freshkube_core::write::Protection;
use freshkube_followed::live::Coroot;
use freshkube_followed::model::Subject;
use freshkube_followed::owner::{Connection, Event, followed};
use freshkube_nav::{KindKey, LinkPage, Name, Namespace, ViewState, ViewTab};
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
                |this, _, event: &Event, window, cx| {
                    let (words, failure) = match event {
                        Event::Toast { words, failure } => (words, failure),
                        Event::Open { subject, context } => {
                            this.open_from_notice(subject, context, window, cx);
                            return;
                        }
                    };
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
        // Banners show only while the app is not the front app.
        if let Some(followed) = followed(cx) {
            followed.update(cx, |f, _| f.set_in_front(window.is_window_active()));
            subscriptions.push(cx.observe_window_activation(window, move |_, window, cx| {
                let front = window.is_window_active();
                followed.update(cx, |f, _| f.set_in_front(front));
            }));
        }
        let observability = self.pages.observability.clone();
        subscriptions.push(cx.observe(&observability, |this, _, cx| this.offer_followed(cx)));
        subscriptions
    }
}

/// Where a banner's click goes: the pod's Events, or the page that holds
/// the item. Identities only, so it goes through the same guarded open as
/// a deep link.
pub(super) fn view_for(subject: &Subject) -> Option<ViewState> {
    let page = |page| Some(ViewState::page(page));
    match subject {
        Subject::CrashLoop { namespace, pod } => {
            let mut view = ViewState::page(LinkPage::Resources);
            view.kind = Some(KindKey::parse("pods").ok()?);
            view.ns = Some(Namespace::parse(namespace).ok()?);
            view.object = Some(freshkube_nav::ViewObject {
                name: Name::parse(pod).ok()?,
                uid: None,
            });
            // The cause: what the pod's events say.
            view.tab = ViewTab::Events;
            view.check().ok()?;
            Some(view)
        }
        Subject::Landing { .. } | Subject::Verification { .. } => page(LinkPage::Applications),
        Subject::Incident { .. } => page(LinkPage::Observability),
    }
}

impl Pilot {
    /// A banner was clicked. Opens what it was about in the cluster that is
    /// open; another cluster is named, never switched to unasked.
    pub(super) fn open_from_notice(
        &mut self,
        subject: &Subject,
        context: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        window.activate_window();
        let here = self
            .source()
            .kube
            .is_some_and(|kube| kube.context == context);
        if !here {
            self.tell(
                format!("Open {context} to see this: {}", subject.title()),
                window,
                cx,
            );
            return;
        }
        let Some(view) = view_for(subject) else {
            return;
        };
        self.apply_view(&view, window, cx);
        self.tell("Opened from a notification".into(), window, cx);
    }
}
