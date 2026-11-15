//! Drawing the pull request card: the compact one in the strip's first card
//! (an avatar and one line) and the full one in the Inspector. Both only lay
//! out what `pull_card::derive` worked out with the read.
use freshkube_delivery::github::AvatarUrl;
use freshkube_ui::palette::palette;
use freshkube_ui::platform::OpenSourcesSettings;
use freshkube_ui::tooltip::FollowTooltip as _;
use gpui_kit::component::avatar::Avatar;
use gpui_kit::component::{Sizable as _, button::Button, h_flex, v_flex};

use super::detail::field;
use super::pull_card::{Check, Person, PullCard};
use super::*;
use crate::ui::{self, dp};

/// An avatar's drawn size in dp; the file is fetched at twice that, in
/// pixels, as the design says.
pub(super) const AVATAR_DP: f32 = 18.;
pub(super) const AVATAR_PX: u32 = 2 * AVATAR_DP as u32;

impl ChangePage {
    /// A person's avatar: the picture when it is on disk, else the
    /// initials of their login. Never a fetch from here.
    pub(super) fn avatar(&self, login: &SharedString, url: Option<AvatarUrl>) -> Avatar {
        let avatar = Avatar::new().name(login.clone());
        let avatar = match url.and_then(|url| self.pull.avatar_path(&url)) {
            Some(path) => avatar.src(path),
            None => avatar,
        };
        avatar.size(dp(AVATAR_DP))
    }

    fn render_person(&self, person: &Person, cx: &App) -> impl IntoElement {
        let p = palette(cx);
        h_flex()
            .id(SharedString::from(format!(
                "change-pull-person-{}",
                person.login
            )))
            .test_support()
            .aria_label(format!("{}: {}", person.login, person.role))
            .gap(dp(6.))
            .items_center()
            .min_w_0()
            .child(self.avatar(&person.login, person.avatar))
            .child(div().min_w_0().truncate().child(person.login.clone()))
            .child(
                div()
                    .flex_none()
                    .text_size(dp(11.))
                    .text_color(p.muted)
                    .child(person.role.clone()),
            )
    }

    /// One bar segment for each check, in its tone's colour.
    fn render_checks(&self, card: &PullCard, cx: &App) -> Option<Div> {
        let checks = card.checks.as_ref()?;
        let p = palette(cx);
        let colour = |tone: Tone| match tone {
            Tone::Good => p.good,
            Tone::Crit => p.crit,
            Tone::Waiting => p.accent,
            _ => p.line,
        };
        let bar = h_flex()
            .id("change-pull-checks-bar")
            .test_support()
            .aria_label(checks.words.clone())
            .gap(dp(2.))
            .h(dp(6.))
            .children(checks.list.iter().map(|check| {
                div()
                    .flex_1()
                    .h_full()
                    .min_w(dp(2.))
                    .rounded(gpui_kit::px(3.))
                    .bg(colour(check.tone))
            }));
        let list = checks
            .list
            .iter()
            .enumerate()
            .map(|(ix, check)| self.render_check(ix, check, cx));
        Some(
            v_flex()
                .gap(dp(5.))
                .child(
                    div()
                        .id("change-pull-checks")
                        .test_support()
                        .aria_label(format!("Checks: {}", checks.words))
                        .child(checks.words.clone()),
                )
                .child(bar)
                .child(v_flex().gap(dp(2.)).children(list)),
        )
    }

    fn render_check(&self, ix: usize, check: &Check, cx: &App) -> impl IntoElement {
        let p = palette(cx);
        let address = check.page.as_ref().map(|page| page.as_str().to_owned());
        h_flex()
            .id(SharedString::from(format!("change-pull-check-{ix}")))
            .test_support()
            .aria_label(format!("{}: {}", check.name, check.word))
            .gap(dp(6.))
            .items_center()
            .text_size(dp(11.5))
            .when_some(address, |this, address| {
                this.cursor_pointer()
                    .follow_tooltip(address.clone())
                    .on_click(move |_, _, cx| cx.open_url(&address))
            })
            .child(div().flex_none().children(ui::status_glyph(check.tone, cx)))
            .child(
                div()
                    .min_w_0()
                    .truncate()
                    .flex_1()
                    .child(check.name.clone()),
            )
            .child(div().flex_none().text_color(p.muted).child(check.word))
    }

    /// The full card, for the Inspector.
    pub(super) fn render_pull_card(
        &self,
        card: &PullCard,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let p = palette(cx);
        // An old answer's tag is a warning too, never a green current one.
        let state = card.state.map(|(word, tone)| {
            let tone = if card.old.is_some() { Tone::Warn } else { tone };
            ui::tag(tone, None, word, cx)
        });
        let lack = card.lack.as_ref().map(|lack| {
            let connect = lack.connect.then(|| {
                Button::new("change-pull-connect")
                    .outline()
                    .xsmall()
                    .label("Connect GitHub in Settings")
                    .tooltip("Opens Settings; the token is set in the setup file (delivery.github.token).")
                    .on_click(cx.listener(|_, _, window, cx| {
                        window.dispatch_action(Box::new(OpenSourcesSettings), cx)
                    }))
            });
            v_flex()
                .gap(dp(6.))
                .child(
                    ui::banner(lack.tone, Some(lack.lead.into()), lack.text.clone(), None, cx)
                        .id("change-pull-lack")
                        .aria_label(format!("{}: {}", lack.lead, lack.text))
                        .test_support(),
                )
                .children(connect)
        });
        let links = (!card.links.is_empty()).then(|| {
            h_flex()
                .flex_wrap()
                .gap_x(dp(8.))
                .gap_y(dp(4.))
                .children(card.links.iter().map(|link| {
                    let address = link.address.as_str().to_owned();
                    Button::new(SharedString::from(format!("change-pull-{}", link.id)))
                        .outline()
                        .xsmall()
                        .label(format!("{} ↗", link.label))
                        .tooltip(address.clone())
                        .on_click(move |_, _, cx| cx.open_url(&address))
                }))
        });
        let facts = [
            ("Merged", card.merged.clone()),
            ("Changes", card.changed.clone()),
        ]
        .into_iter()
        .filter_map(|(label, value)| value.map(|value| (label, value)))
        .map(|(label, value)| field(label, div().child(value), cx));
        let labels = card
            .labels
            .clone()
            .map(|labels| field("Labels", div().child(labels), cx));
        // The Inspector's own heading names the section; no second caption.
        v_flex()
            .gap(dp(7.))
            .text_size(dp(12.))
            .id("change-pull")
            .test_support()
            .aria_label(format!("{} · {}", card.title, card.compact))
            .child(
                h_flex()
                    .gap(dp(8.))
                    .items_start()
                    .child(
                        div()
                            .min_w_0()
                            .flex_1()
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(p.ink)
                            .child(card.title.clone()),
                    )
                    .children(state.map(|tag| div().flex_none().child(tag))),
            )
            .children(
                card.repo
                    .clone()
                    .map(|repo| div().text_color(p.muted).child(repo)),
            )
            .children(lack)
            .child(
                v_flex().gap(dp(4.)).children(
                    card.people
                        .iter()
                        .map(|person| self.render_person(person, cx)),
                ),
            )
            .children(self.render_checks(card, cx))
            .children(facts)
            .children(labels)
            .children(card.notes.iter().enumerate().map(|(ix, note)| {
                div()
                    .id(SharedString::from(format!("change-pull-note-{ix}")))
                    .test_support()
                    .text_size(dp(11.))
                    .text_color(if card.old.as_ref() == Some(note) {
                        p.warn_ink
                    } else {
                        p.muted
                    })
                    .child(note.clone())
            }))
            .children(links)
    }
}
