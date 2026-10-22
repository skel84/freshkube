//! The status bar's "Notifying · n", hidden while nothing is followed and
//! nothing has been told, and its popover: New, Earlier and Following, each
//! followed item with what it reads and Stop.

use std::time::Duration;

use chrono::{DateTime, Utc};
use freshkube_ui::palette::palette;
use freshkube_ui::ui::{self, Tone, dp};
use gpui_kit::assets::IconName;
use gpui_kit::component::{
    Sizable,
    button::{Button, ButtonVariants},
    h_flex,
    popover::Popover,
    v_flex,
};
use gpui_kit::prelude::*;
use gpui_kit::*;

use crate::model::MAX_FOLLOWS;
use crate::notice::{Notice, Settings};
use crate::owner::{Followed, Item};
use crate::rules::Status;

pub struct FollowedBell {
    followed: Entity<Followed>,
    panel: Entity<BellPanel>,
    open: bool,
    label: SharedString,
    /// The new count alone (the follows' without any), for a window too
    /// narrow for the bar; the icon says the rest.
    short: SharedString,
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
            short: SharedString::default(),
            _subscription: subscription,
        };
        bell.derive(cx);
        bell
    }

    /// "Notifying · 3", and how many are new; without anything followed,
    /// only what is new.
    fn derive(&mut self, cx: &App) {
        let followed = self.followed.read(cx);
        let new = followed.inbox().new.len();
        let count = followed.count();
        // At the cap, the bell says so before the next follow is refused.
        let of = match count == MAX_FOLLOWS {
            true => format!("{count} of {MAX_FOLLOWS}"),
            false => count.to_string(),
        };
        let (label, short) = match (count, new) {
            (0, 0) => ("Notifications".to_owned(), "0".to_owned()),
            (0, new) => (format!("{new} new"), new.to_string()),
            (_, 0) => (format!("Notifying · {of}"), count.to_string()),
            (_, new) => (format!("Notifying · {of} · {new} new"), new.to_string()),
        };
        self.label = label.into();
        self.short = short.into();
    }
}

impl Render for FollowedBell {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
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
        // The status bar's right side keeps its room, so a narrow bar gets
        // the bell's icon and count, not its word.
        let narrow = window.viewport_size().width < ui::dp_px(1080., window);
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
                    .when(narrow, |this| this.icon(IconName::BellRing))
                    .label(if narrow {
                        self.short.clone()
                    } else {
                        self.label.clone()
                    })
                    .tooltip("What you follow, and what it told you"),
            )
            .content(move |_, _, _| panel.clone())
            .into_any_element()
    }
}

/// A notice, worded when the notices change.
struct NoticeLine {
    id: u64,
    tone: Tone,
    tag: &'static str,
    summary: String,
    lines: Vec<String>,
    held: bool,
    age: Entity<Age>,
}

/// A followed item, worded when the follows change.
struct FollowLine {
    id: u64,
    tone: Tone,
    status: String,
    title: String,
    context: String,
    reads: String,
    reason: Option<String>,
    label: String,
}

/// How long ago a notice was: the one thing in the bell that changes with
/// the clock, so it is the one small view that ticks.
struct Age {
    at: DateTime<Utc>,
    _tick: Task<()>,
}

impl Age {
    fn new(at: DateTime<Utc>, cx: &mut Context<Self>) -> Self {
        Self {
            at,
            _tick: cx.spawn(async move |this, cx| {
                loop {
                    cx.background_executor()
                        .timer(Duration::from_secs(30))
                        .await;
                    if this.update(cx, |_, cx| cx.notify()).is_err() {
                        return;
                    }
                }
            }),
        }
    }
}

impl Render for Age {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .flex_none()
            .text_color(palette(cx).muted)
            .child(ago(Utc::now(), self.at))
    }
}

/// The popover's lists. An entity, so it redraws as notices arrive while
/// it is open. It words its lines when the owner changes, not when it
/// draws.
pub struct BellPanel {
    followed: Entity<Followed>,
    new: Vec<NoticeLine>,
    earlier: Vec<NoticeLine>,
    following: Vec<FollowLine>,
    _subscription: Subscription,
}

impl BellPanel {
    fn new(followed: Entity<Followed>, cx: &mut Context<Self>) -> Self {
        let subscription = cx.observe(&followed, |this, _, cx| {
            this.derive(cx);
            cx.notify();
        });
        let mut panel = Self {
            followed,
            new: Vec::new(),
            earlier: Vec::new(),
            following: Vec::new(),
            _subscription: subscription,
        };
        panel.derive(cx);
        panel
    }

    fn derive(&mut self, cx: &mut Context<Self>) {
        let followed = self.followed.read(cx);
        let new: Vec<Notice> = followed.inbox().new.iter().cloned().collect();
        let earlier: Vec<Notice> = followed.inbox().earlier.iter().cloned().collect();
        self.following = followed.items().iter().map(follow_line).collect();
        let settings = followed.settings();
        self.new = new.iter().map(|n| notice_line(n, &settings, cx)).collect();
        self.earlier = earlier
            .iter()
            .map(|n| notice_line(n, &settings, cx))
            .collect();
    }
}

fn notice_line(notice: &Notice, settings: &Settings, cx: &mut Context<BellPanel>) -> NoticeLine {
    // The bell is in the app: it lists what a banner would hide.
    let lines = notice
        .updates
        .iter()
        .map(|u| format!("{} · {}", u.headline, u.detail))
        .collect();
    let at = notice.at;
    NoticeLine {
        id: notice.id,
        tone: if notice.has_failure() {
            Tone::Crit
        } else {
            Tone::Good
        },
        tag: if notice.held {
            "Held"
        } else if notice.has_failure() {
            "Failed"
        } else {
            "Update"
        },
        // The headline when its details show, the summary when they hide.
        summary: notice.words(settings).title,
        lines,
        held: notice.held,
        age: cx.new(|cx| Age::new(at, cx)),
    }
}

fn follow_line(item: &Item) -> FollowLine {
    let (tone, reason) = match &item.status {
        Status::Watching => (Tone::Good, None),
        Status::Starting => (Tone::Waiting, None),
        Status::NotWatching(why) => (Tone::Unknown, Some(why.clone())),
    };
    FollowLine {
        id: item.follow.id,
        tone,
        status: item.status.label().to_owned(),
        title: item.follow.subject.title(),
        context: item.follow.context.clone(),
        reads: format!(
            "{} · {}",
            item.follow.subject.reads(),
            item.follow.subject.ends()
        ),
        reason,
        label: format!(
            "{}. {}. {}",
            item.follow.subject.title(),
            item.status.label(),
            item.follow.context
        ),
    }
}

impl Render for BellPanel {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let rows = |prefix: &'static str, lines: &[NoticeLine]| -> Vec<AnyElement> {
            lines
                .iter()
                .map(|line| notice_row(prefix, line, cx))
                .collect()
        };
        let new = rows("new", &self.new);
        let earlier = rows("earlier", &self.earlier);
        let following: Vec<AnyElement> = self
            .following
            .iter()
            .map(|line| following_row(&self.followed, line, cx))
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

fn notice_row(prefix: &'static str, line: &NoticeLine, cx: &App) -> AnyElement {
    let p = palette(cx);
    v_flex()
        .id(SharedString::from(format!("notice-{prefix}-{}", line.id)))
        .test_support()
        .aria_label(format!("{}. {}", line.summary, line.lines.join(". ")))
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
                .child(ui::tag(line.tone, None, line.tag, cx))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .font_weight(FontWeight::SEMIBOLD)
                        .child(line.summary.clone()),
                )
                .child(line.age.clone()),
        )
        .children(line.lines.iter().cloned().map(|text| div().child(text)))
        .when(line.held, |this| {
            this.child(
                div()
                    .text_color(p.muted)
                    .child("Held by quiet hours; no toast was shown."),
            )
        })
        .into_any_element()
}

fn following_row(followed: &Entity<Followed>, line: &FollowLine, cx: &App) -> AnyElement {
    let p = palette(cx);
    let id = line.id;
    let followed = followed.clone();
    v_flex()
        .id(SharedString::from(format!("following-{id}")))
        .test_support()
        .aria_label(line.label.clone())
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
                .child(ui::tag(line.tone, None, line.status.clone(), cx))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .truncate()
                        .font_weight(FontWeight::SEMIBOLD)
                        .child(line.title.clone()),
                )
                .child(
                    div()
                        .flex_none()
                        .text_color(p.muted)
                        .child(line.context.clone()),
                ),
        )
        .child(div().text_color(p.muted).child(line.reads.clone()))
        .when_some(line.reason.clone(), |this, why| {
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
