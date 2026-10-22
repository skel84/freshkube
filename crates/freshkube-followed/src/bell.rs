//! The status bar's "Notifying · n", hidden while nothing is followed and
//! nothing has been told, and its popover: New, Earlier and Following, each
//! followed item with what it reads and Stop.

use chrono::{DateTime, Utc};
use freshkube_ui::palette::palette;
use freshkube_ui::ui::{self, Tone, dp};
use gpui_kit::component::{
    Sizable,
    button::{Button, ButtonVariants},
    h_flex,
    popover::Popover,
    v_flex,
};
use gpui_kit::prelude::*;
use gpui_kit::*;

use crate::notice::Notice;
use crate::owner::{Followed, Item};
use crate::rules::Status;

pub struct FollowedBell {
    followed: Entity<Followed>,
    panel: Entity<BellPanel>,
    open: bool,
    label: SharedString,
    _subscription: Subscription,
}

impl FollowedBell {
    pub fn new(followed: Entity<Followed>, cx: &mut Context<Self>) -> Self {
        let panel = cx.new(|cx| BellPanel::new(followed.clone(), cx));
        let subscription = cx.observe(&followed, |this, _, cx| {
            this.derive(cx);
            cx.notify();
        });
        let mut bell = Self {
            followed,
            panel,
            open: false,
            label: SharedString::default(),
            _subscription: subscription,
        };
        bell.derive(cx);
        bell
    }

    /// "Notifying · 3", and how many are new.
    fn derive(&mut self, cx: &App) {
        let followed = self.followed.read(cx);
        let new = followed.inbox().new.len();
        self.label = match (followed.count(), new) {
            (n, 0) => format!("Notifying · {n}"),
            (n, new) => format!("Notifying · {n} · {new} new"),
        }
        .into();
    }
}

impl Render for FollowedBell {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let followed = self.followed.read(cx);
        if followed.count() == 0
            && followed.inbox().new.is_empty()
            && followed.inbox().earlier.is_empty()
        {
            return div().into_any_element();
        }
        let this = cx.entity().downgrade();
        let panel = self.panel.clone();
        let list = self.followed.clone();
        Popover::new("followed-popover")
            .anchor(Anchor::BottomRight)
            .open(self.open)
            .on_open_change(move |open, _, cx| {
                let open = *open;
                _ = this.update(cx, |this, cx| {
                    this.open = open;
                    cx.notify();
                });
                // Closing the bell: what was new is earlier.
                if !open {
                    list.update(cx, |followed, cx| followed.mark_seen(cx));
                }
            })
            .trigger(
                Button::new("followed")
                    .ghost()
                    .xsmall()
                    .label(self.label.clone())
                    .tooltip("What you follow, and what it told you"),
            )
            .content(move |_, _, _| panel.clone())
            .into_any_element()
    }
}

/// The popover's lists. An entity, so it redraws as notices arrive while
/// it is open.
pub struct BellPanel {
    followed: Entity<Followed>,
    _subscription: Subscription,
}

impl BellPanel {
    fn new(followed: Entity<Followed>, cx: &mut Context<Self>) -> Self {
        let subscription = cx.observe(&followed, |_, _, cx| cx.notify());
        Self {
            followed,
            _subscription: subscription,
        }
    }
}

impl Render for BellPanel {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let followed = self.followed.read(cx);
        let settings = followed.settings();
        let now = Utc::now();
        let new: Vec<AnyElement> = followed
            .inbox()
            .new
            .iter()
            .map(|n| notice_row("new", n, &settings, now, cx))
            .collect();
        let earlier: Vec<AnyElement> = followed
            .inbox()
            .earlier
            .iter()
            .map(|n| notice_row("earlier", n, &settings, now, cx))
            .collect();
        let following: Vec<AnyElement> = followed
            .items()
            .iter()
            .map(|item| following_row(&self.followed, item, cx))
            .collect();
        let section = |id: &'static str, title: &str, empty: &str, rows: Vec<AnyElement>| {
            v_flex()
                .id(id)
                .test_support()
                .gap_1()
                .child(ui::caption(title, cx))
                .when(rows.is_empty(), |this| {
                    this.child(
                        div()
                            .px_2()
                            .text_size(dp(12.))
                            .text_color(palette(cx).muted)
                            .child(empty.to_owned()),
                    )
                })
                .children(rows)
        };
        v_flex()
            .id("followed-panel")
            .test_support()
            .aria_label("Notifications")
            .w(dp(420.))
            .max_h(window.viewport_size().height - ui::dp_px(96., window))
            .overflow_y_scroll()
            .restrict_scroll_to_axis()
            .p_1()
            .gap_3()
            .child(section("followed-new", "New", "Nothing new.", new))
            .child(section(
                "followed-earlier",
                "Earlier",
                "Nothing earlier.",
                earlier,
            ))
            .child(section(
                "followed-following",
                "Following",
                "You follow nothing.",
                following,
            ))
    }
}

/// "just now", "5 min ago", "2 h ago".
pub fn ago(now: DateTime<Utc>, at: DateTime<Utc>) -> String {
    let seconds = (now - at).num_seconds().max(0);
    match seconds {
        0..=59 => "just now".to_owned(),
        60..=3599 => format!("{} min ago", seconds / 60),
        3600..=86_399 => format!("{} h ago", seconds / 3600),
        _ => format!("{} d ago", seconds / 86_400),
    }
}

fn notice_row(
    prefix: &'static str,
    notice: &Notice,
    settings: &crate::notice::Settings,
    now: DateTime<Utc>,
    cx: &App,
) -> AnyElement {
    let p = palette(cx);
    // The bell is in the app: it shows what a banner would hide.
    let lines: Vec<String> = notice
        .updates
        .iter()
        .map(|u| format!("{} · {}", u.headline, u.detail))
        .collect();
    let _ = settings;
    let tone = if notice.has_failure() {
        Tone::Crit
    } else {
        Tone::Good
    };
    let tag = if notice.held {
        "Held"
    } else if notice.has_failure() {
        "Failed"
    } else {
        "Update"
    };
    v_flex()
        .id(SharedString::from(format!("notice-{prefix}-{}", notice.id)))
        .test_support()
        .aria_label(lines.join(". "))
        .gap_1()
        .px_2()
        .py_1p5()
        .rounded(px(8.))
        .border_1()
        .border_color(p.line)
        .text_size(dp(12.))
        .child(
            h_flex()
                .gap_2()
                .child(ui::tag(tone, None, tag, cx))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .font_weight(FontWeight::SEMIBOLD)
                        .child(notice.summary()),
                )
                .child(
                    div()
                        .flex_none()
                        .text_color(p.muted)
                        .child(ago(now, notice.at)),
                ),
        )
        .children(lines.into_iter().map(|line| div().child(line)))
        .when(notice.held, |this| {
            this.child(
                div()
                    .text_color(p.muted)
                    .child("Held by quiet hours; no toast was shown."),
            )
        })
        .into_any_element()
}

fn following_row(followed: &Entity<Followed>, item: &Item, cx: &App) -> AnyElement {
    let p = palette(cx);
    let id = item.follow.id;
    let (tone, reason) = match &item.status {
        Status::Watching => (Tone::Good, None),
        Status::Starting => (Tone::Waiting, None),
        Status::NotWatching(why) => (Tone::Unknown, Some(why.clone())),
    };
    let followed = followed.clone();
    v_flex()
        .id(SharedString::from(format!("following-{id}")))
        .test_support()
        .aria_label(format!(
            "{}. {}. {}",
            item.follow.subject.title(),
            item.status.label(),
            item.follow.context
        ))
        .gap_1()
        .px_2()
        .py_1p5()
        .rounded(px(8.))
        .border_1()
        .border_color(p.line)
        .text_size(dp(12.))
        .child(
            h_flex()
                .gap_2()
                .child(ui::tag(tone, None, item.status.label().to_owned(), cx))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .truncate()
                        .font_weight(FontWeight::SEMIBOLD)
                        .child(item.follow.subject.title()),
                )
                .child(
                    div()
                        .flex_none()
                        .text_color(p.muted)
                        .child(item.follow.context.clone()),
                ),
        )
        .child(div().text_color(p.muted).child(format!(
            "{} · {}",
            item.follow.subject.reads(),
            item.follow.subject.ends()
        )))
        .when_some(reason, |this, why| {
            this.child(div().text_color(p.muted).child(why))
        })
        .child(
            h_flex().child(
                Button::new(SharedString::from(format!("following-{id}-stop")))
                    .ghost()
                    .xsmall()
                    .label("Stop")
                    .tooltip("Stop following; its read stops with it")
                    .on_click(move |_, _, cx| {
                        followed.update(cx, |followed, cx| followed.stop(id, cx));
                    }),
            ),
        )
        .into_any_element()
}
