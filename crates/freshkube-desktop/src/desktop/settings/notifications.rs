//! Settings › Notifications: how updates are grouped, quiet hours, Hide
//! details, and every followed item with what it reads and when it ends
//! (docs/features/settings.md). The settings and the follows are saved in
//! `follows.json` as they change; the owner is the app's `Followed`.

use freshkube_followed::notice::{Quiet, Settings};
use freshkube_followed::owner::followed;
use freshkube_followed::{Followed, Status};
use freshkube_ui::{form, page};
use gpui_kit::component::{
    Sizable,
    button::{Button, ButtonGroup},
    h_flex,
    switch::Switch,
    v_flex,
};
use gpui_kit::prelude::*;
use gpui_kit::*;

use super::SettingsPage;
use crate::palette::palette;
use crate::ui::{self, Tone, dp};

/// A quiet hour moves by this many minutes.
const STEP: i32 = 30;

/// A followed item as the list shows it.
pub(super) struct FollowRow {
    id: u64,
    kind: &'static str,
    title: String,
    /// "dev-fra · Watches pod shop/api".
    reads: String,
    ends: &'static str,
    status: String,
    tone: Tone,
    reason: Option<String>,
}

impl SettingsPage {
    /// Words the followed items; the list draws these.
    pub(super) fn derive_follows(&mut self, cx: &App) {
        let Some(owner) = followed(cx) else {
            self.follow_rows.clear();
            return;
        };
        self.follow_rows = owner
            .read(cx)
            .items()
            .iter()
            .map(|item| {
                let (tone, reason) = match &item.status {
                    Status::Watching => (Tone::Good, None),
                    Status::Starting => (Tone::Waiting, None),
                    Status::NotWatching(why) => (Tone::Unknown, Some(why.clone())),
                };
                FollowRow {
                    id: item.follow.id,
                    kind: item.follow.subject.kind(),
                    title: item.follow.subject.title(),
                    reads: format!("{} · {}", item.follow.context, item.follow.subject.reads()),
                    ends: item.follow.subject.ends(),
                    status: item.status.label().to_owned(),
                    tone,
                    reason,
                }
            })
            .collect();
    }

    pub(super) fn render_notifications(
        &self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let narrow = page::content_width(window) < form::NARROW;
        let header =
            page::PageHeader::new("settings-notifications", "Notifications").render(window, cx);
        let body = match followed(cx) {
            Some(followed) => self
                .notifications_form(followed, narrow, cx)
                .into_any_element(),
            None => div()
                .child("Nothing is followed in this window.")
                .into_any_element(),
        };
        page::padded("settings-page")
            .track_focus(&self.focus)
            .overflow_y_scroll()
            .restrict_scroll_to_axis()
            .child(header)
            .child(body)
            .into_any_element()
    }

    fn notifications_form(
        &self,
        owner: Entity<Followed>,
        narrow: bool,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let settings = owner.read(cx).settings();
        let default_hides = owner.read(cx).hides_by_default();
        // Said once, on the first group, and only where something is saved.
        let saves = owner.read(cx).saves();
        let saved_once = move |group: form::Group| match saves {
            true => group.saved_in(form::SavedIn::file("follows.json")),
            false => group,
        };
        let change = |owner: &Entity<Followed>, edit: fn(&mut Settings, i32), value: i32| {
            let owner = owner.clone();
            move |_: &ClickEvent, _: &mut Window, cx: &mut App| {
                owner.update(cx, |followed, cx| {
                    let mut settings = followed.settings();
                    edit(&mut settings, value);
                    followed.set_settings(settings, cx);
                });
            }
        };
        let grouping = {
            let owner = owner.clone();
            Switch::new("notifications-grouping")
                .small()
                .checked(settings.grouping)
                .accessibility_label("Group updates within 2 minutes")
                .on_click(move |checked, _, cx| {
                    owner.update(cx, |followed, cx| {
                        let mut settings = followed.settings();
                        settings.grouping = *checked;
                        followed.set_settings(settings, cx);
                    });
                })
        };
        let quiet_switch = {
            let owner = owner.clone();
            Switch::new("notifications-quiet")
                .small()
                .checked(settings.quiet.enabled)
                .accessibility_label("Quiet hours")
                .on_click(move |checked, _, cx| {
                    owner.update(cx, |followed, cx| {
                        let mut settings = followed.settings();
                        settings.quiet.enabled = *checked;
                        followed.set_settings(settings, cx);
                    });
                })
        };
        let time = |id: &'static str, minute: u16, edit: fn(&mut Settings, i32)| {
            h_flex()
                .gap(dp(6.))
                .items_center()
                .child(
                    div()
                        .id(SharedString::from(format!("{id}-value")))
                        .w(dp(48.))
                        .child(Quiet::clock(minute)),
                )
                .child(
                    Button::new(SharedString::from(format!("{id}-earlier")))
                        .outline()
                        .small()
                        .label("−")
                        .on_click(change(&owner, edit, -STEP)),
                )
                .child(
                    Button::new(SharedString::from(format!("{id}-later")))
                        .outline()
                        .small()
                        .label("+")
                        .on_click(change(&owner, edit, STEP)),
                )
        };
        let hide = {
            let choice = |id: &'static str, label: &'static str, selected: bool| {
                ui::choice(
                    Button::new(id).label(label).h(dp(ui::CONTROL_HEIGHT)),
                    selected,
                )
            };
            let owner = owner.clone();
            ButtonGroup::new("notifications-hide")
                .outline()
                .small()
                .child(choice(
                    "notifications-hide-default",
                    match default_hides {
                        Some(true) => "Default (On)",
                        Some(false) => "Default (Off)",
                        None => "Default",
                    },
                    settings.hide_details.is_none(),
                ))
                .child(choice(
                    "notifications-hide-on",
                    "On",
                    settings.hide_details == Some(true),
                ))
                .child(choice(
                    "notifications-hide-off",
                    "Off",
                    settings.hide_details == Some(false),
                ))
                .on_click(move |selected: &Vec<usize>, _, cx| {
                    let hide = match selected.first() {
                        Some(0) => None,
                        Some(1) => Some(true),
                        Some(_) => Some(false),
                        None => return,
                    };
                    owner.update(cx, |followed, cx| {
                        let mut settings = followed.settings();
                        settings.hide_details = hide;
                        followed.set_settings(settings, cx);
                    });
                })
        };
        let p = palette(cx);
        let last = self.follow_rows.last().map(|row| row.id);
        let items: Vec<form::Row> = self
            .follow_rows
            .iter()
            .map(|row| {
                let id = row.id;
                let stop = {
                    let owner = owner.clone();
                    Button::new(SharedString::from(format!("notifications-stop-{id}")))
                        .outline()
                        .small()
                        .label("Stop")
                        .on_click(move |_, _, cx| {
                            owner.update(cx, |followed, cx| followed.stop(id, cx));
                        })
                };
                let divided = Some(row.id) != last;
                let control = v_flex()
                    .min_w_0()
                    .gap(dp(4.))
                    .when(divided, |this| {
                        this.pb(dp(10.)).border_b_1().border_color(p.line)
                    })
                    .child(div().truncate().child(row.title.clone()))
                    .child(
                        div()
                            .truncate()
                            .text_color(p.muted)
                            .child(row.reads.clone()),
                    )
                    .child(div().text_color(p.muted).child(row.ends))
                    .when_some(row.reason.clone(), |this, why| {
                        this.child(div().text_color(p.muted).child(why))
                    })
                    .child(
                        h_flex()
                            .gap(dp(8.))
                            .items_center()
                            .child(ui::tag(row.tone, None, row.status.clone(), cx))
                            .child(stop),
                    );
                form::row(format!("notifications-follow-{id}"), row.kind, control)
            })
            .collect();
        let mut followed_group = form::group("notifications-followed", "Followed").narrow(narrow);
        if items.is_empty() {
            followed_group = followed_group.row(form::row(
                "notifications-none",
                "Nothing followed",
                div().text_color(p.muted).child(
                    "Follow a pod from its menu, or a change from its page. Nothing notifies until you do.",
                ),
            ));
        }
        for row in items {
            followed_group = followed_group.row(row);
        }
        v_flex()
            .id("settings-notifications-form")
            .test_support()
            .w_full()
            .gap(dp(20.))
            .flex_none()
            .child(
                saved_once(
                form::group("notifications-grouping-group", "Grouping")
                    .narrow(narrow)
                    .row(
                        form::row(
                            "notifications-grouping-row",
                            "Group updates",
                            div()
                                .h(dp(ui::CONTROL_HEIGHT))
                                .flex()
                                .items_center()
                                .child(grouping),
                        )
                        .hint("The first update shows at once; the ones after it within 2 minutes join one notice. The same state is never told twice."),
                    )
                )
                .render(cx),
            )
            .child(
                form::group("notifications-quiet-group", "Quiet hours")
                    .narrow(narrow)
                    .row(
                        form::row(
                            "notifications-quiet-row",
                            "Hold updates",
                            div()
                                .h(dp(ui::CONTROL_HEIGHT))
                                .flex()
                                .items_center()
                                .child(quiet_switch),
                        )
                        .hint("Held updates wait in the bell. Failures still come through."),
                    )
                    .row(form::row(
                        "notifications-from-row",
                        "From",
                        time("notifications-from", settings.quiet.from, |s, by| {
                            s.quiet.from = shifted(s.quiet.from, by);
                        }),
                    ))
                    .row(form::row(
                        "notifications-until-row",
                        "Until",
                        time("notifications-until", settings.quiet.until, |s, by| {
                            s.quiet.until = shifted(s.quiet.until, by);
                        }),
                    ))
                    .render(cx),
            )
            .child(
                form::group("notifications-details-group", "Details")
                    .narrow(narrow)
                    .row(
                        form::row("notifications-hide-row", "Hide details", hide).hint(
                            "With it on, a notice says only \"Freshkube · 1 update\". Default is on for protected and read-only clusters, whose notices can show on a lock screen.",
                        ),
                    )
                    .render(cx),
            )
            .child(followed_group.render(cx))
    }
}

/// A time of day moved by `by` minutes, around the clock.
fn shifted(minute: u16, by: i32) -> u16 {
    (minute as i32 + by).rem_euclid(24 * 60) as u16
}

#[cfg(test)]
mod tests {
    use super::shifted;

    #[test]
    fn a_time_moves_around_the_clock() {
        assert_eq!(shifted(23 * 60 + 30, 30), 0);
        assert_eq!(shifted(0, -30), 23 * 60 + 30);
        assert_eq!(shifted(12 * 60, 30), 12 * 60 + 30);
    }
}
