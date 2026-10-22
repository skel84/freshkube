//! "Notify me": the control a page offers its follows with (a change page's
//! three, for example). Off until picked; each choice says what it keeps
//! reading, and turning one off stops its read when no other follow needs
//! it.

use freshkube_ui::palette::palette;
use freshkube_ui::ui::{self, dp};
use gpui_kit::component::{
    Sizable,
    button::{Button, ButtonVariants},
    h_flex,
    popover::Popover,
    switch::Switch,
    v_flex,
};
use gpui_kit::prelude::*;
use gpui_kit::*;

use crate::model::Subject;
use crate::owner::Followed;

/// One thing the page offers to follow.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Offer {
    pub subject: Subject,
    /// The cluster it is about, when it isn't the one the window is on: a
    /// Kargo project is followed in the cluster Kargo runs in. A cluster
    /// that isn't open shows "Not watching" until it is.
    pub context: Option<String>,
    /// What it asks, as a sentence: "Freight wonky-otter reaches prod-ams".
    pub label: String,
}

/// The button and its popover.
pub struct FollowMenu {
    prefix: SharedString,
    panel: Entity<Panel>,
    open: bool,
}

impl FollowMenu {
    /// The menu, once the app's follows are made; a window without them
    /// has none.
    pub fn create(prefix: impl Into<SharedString>, cx: &mut App) -> Option<Entity<Self>> {
        let followed = crate::owner::followed(cx)?;
        let prefix = prefix.into();
        Some(cx.new(|cx| {
            let menu = cx.entity().downgrade();
            let panel = cx.new(|cx| Panel::new(prefix.clone(), followed, menu, cx));
            Self {
                prefix,
                panel,
                open: false,
            }
        }))
    }

    /// What is offered now.
    pub fn offers(&self, cx: &App) -> Vec<Offer> {
        self.panel.read(cx).offers.clone()
    }

    /// What is offered now; nothing redraws unless it changed.
    pub fn offer(&mut self, offers: Vec<Offer>, cx: &mut Context<Self>) {
        self.panel.update(cx, |panel, cx| {
            if panel.offers != offers {
                panel.offers = offers;
                cx.notify();
            }
        });
    }
}

impl Render for FollowMenu {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let this = cx.entity().downgrade();
        let panel = self.panel.clone();
        Popover::new(SharedString::from(format!("{}-popover", self.prefix)))
            .anchor(Anchor::TopRight)
            .open(self.open)
            .on_open_change(move |open, _, cx| {
                let open = *open;
                _ = this.update(cx, |this, cx| {
                    this.open = open;
                    cx.notify();
                });
            })
            .trigger(
                Button::new(SharedString::from(format!("{}-trigger", self.prefix)))
                    .ghost()
                    .small()
                    .h(dp(ui::CONTROL_HEIGHT))
                    .label("Notify me")
                    .tooltip(
                        "Off until you pick: what to be told about, and what it keeps reading",
                    ),
            )
            .content(move |_, _, _| panel.clone())
    }
}

struct Panel {
    prefix: SharedString,
    followed: Entity<Followed>,
    /// The menu whose popover this is, closed once a choice is made so its
    /// confirmation toast isn't drawn under it.
    menu: WeakEntity<FollowMenu>,
    offers: Vec<Offer>,
    /// Why the last choice was refused.
    refused: Option<String>,
    _subscription: Subscription,
}

impl Panel {
    fn new(
        prefix: SharedString,
        followed: Entity<Followed>,
        menu: WeakEntity<FollowMenu>,
        cx: &mut Context<Self>,
    ) -> Self {
        let subscription = cx.observe(&followed, |_, _, cx| cx.notify());
        Self {
            prefix,
            followed,
            menu,
            offers: Vec::new(),
            refused: None,
            _subscription: subscription,
        }
    }
}

impl Render for Panel {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let p = palette(cx);
        let rows: Vec<AnyElement> = self
            .offers
            .iter()
            .enumerate()
            .map(|(ix, offer)| {
                let on = self
                    .followed
                    .read(cx)
                    .id_for(&offer.subject, offer.context.as_deref())
                    .is_some();
                let subject = offer.subject.clone();
                let context = offer.context.clone();
                let this = cx.entity().downgrade();
                let followed = self.followed.clone();
                let menu = self.menu.clone();
                let label = offer.label.clone();
                h_flex()
                    .id(SharedString::from(format!("{}-offer-{ix}", self.prefix)))
                    .test_support()
                    .aria_label(format!("{}. {}", offer.label, offer.subject.reads()))
                    .gap_2()
                    .items_start()
                    .px_2()
                    .py_1p5()
                    .child(
                        Switch::new(SharedString::from(format!("{}-switch-{ix}", self.prefix)))
                            .small()
                            .checked(on)
                            .accessibility_label(offer.label.clone())
                            .on_click(move |_, window, cx| {
                                let label = label.clone();
                                let (refused, started) = followed.update(cx, |followed, cx| {
                                    match followed.id_for(&subject, context.as_deref()) {
                                        Some(id) => {
                                            followed.stop(id, cx);
                                            (None, None)
                                        }
                                        None => match followed.follow_for(
                                            subject.clone(),
                                            context.as_deref(),
                                            cx,
                                        ) {
                                            Ok(_) => (
                                                None,
                                                Some(match followed
                                                    .hides_details_in(context.as_deref())
                                                {
                                                    true => "Notifying you about this. The bell lists it with Stop.".to_owned(),
                                                    false => format!("Notifying you: {label}. The bell lists it with Stop."),
                                                }),
                                            ),
                                            Err(refused) => (Some(refused.words()), None),
                                        },
                                    }
                                });
                                if let Some(words) = started {
                                    // The popover goes first: a toast is drawn under it.
                                    _ = menu.update(cx, |menu, cx| {
                                        menu.open = false;
                                        cx.notify();
                                    });
                                    gpui_kit::component::WindowExt::push_notification(window, words, cx);
                                }
                                _ = this.update(cx, |panel, cx| {
                                    panel.refused = refused;
                                    cx.notify();
                                });
                            }),
                    )
                    .child(
                        v_flex()
                            .flex_1()
                            .min_w_0()
                            .child(div().text_size(dp(12.)).child(offer.label.clone()))
                            .child(
                                div()
                                    .text_size(dp(11.))
                                    .text_color(p.muted)
                                    .child(offer.subject.reads()),
                            ),
                    )
                    .into_any_element()
            })
            .collect();
        v_flex()
            .id(SharedString::from(format!("{}-panel", self.prefix)))
            .test_support()
            .aria_label("Notify me")
            .w(dp(360.))
            .p_1()
            .gap_1()
            .child(ui::caption("Notify me when", cx))
            .children(rows)
            .when_some(self.refused.clone(), |this, why| {
                this.child(
                    div()
                        .px_2()
                        .text_size(dp(12.))
                        .text_color(p.warn_ink)
                        .child(why),
                )
            })
    }
}
