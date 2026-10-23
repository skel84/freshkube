use super::*;
use crate::ui::{self, dp};
use freshkube_ui::{page, palette::palette};
use gpui_kit::TestSupportExt as _;
use gpui_kit::assets::IconName;
use gpui_kit::component::{
    Disableable, Sizable,
    button::{Button, ButtonVariants},
    h_flex,
};

impl ExploreView {
    /// The connect step while it isn't connected: where it is, or why it stopped.
    fn step(&self, cx: &mut Context<Self>) -> AnyElement {
        let name = self.database.identity.name.clone();
        let (id, title, words, retry): (String, String, String, bool) = match &self.phase {
            Phase::Failed(fail) => (
                format!("explore-state-{}", fail.slug()),
                fail.title().into(),
                fail.words(),
                fail.retry(),
            ),
            Phase::Picking => (
                "explore-state-connecting".into(),
                format!("Connecting to {name}"),
                "Choosing a ready replica…".into(),
                false,
            ),
            Phase::Forwarding => (
                "explore-state-connecting".into(),
                format!("Connecting to {name}"),
                "Forwarding to the pod…".into(),
                false,
            ),
            Phase::Connecting => (
                "explore-state-connecting".into(),
                format!("Connecting to {name}"),
                "Connecting read-only…".into(),
                false,
            ),
            Phase::Connected => unreachable!("a connected tab draws its tables"),
        };
        let actions = if retry {
            vec![
                Button::new("explore-retry")
                    .icon(IconName::RefreshCw)
                    .label("Try again")
                    .on_click(cx.listener(|this, _, _, cx| this.connect(cx)))
                    .into_any_element(),
            ]
        } else {
            Vec::new()
        };
        page::inset()
            .child(
                ui::empty_state(IconName::Database, title, words.clone(), None, actions, cx)
                    .h_auto()
                    .id(SharedString::from(id))
                    .aria_label(words)
                    .test_support(),
            )
            .into_any_element()
    }

    /// The session's line: how it reads, who it is, and the writes gate.
    fn strip(&self, cx: &mut Context<Self>) -> AnyElement {
        let p = palette(cx);
        let summary = self.data.read(cx).summary();
        let capped = self.data.read(cx).capped();
        h_flex()
            .id("explore-strip")
            .test_support()
            .gap(dp(8.))
            .px(dp(page::PANE_PADDING))
            .h(dp(34.))
            .flex_none()
            .items_center()
            .border_b_1()
            .border_color(p.line)
            .text_size(dp(12.))
            .child(
                ui::tag(ui::Tone::Good, None, "Read-only session", cx)
                    .id("explore-read-only")
                    .aria_label("Read-only session")
                    .test_support(),
            )
            .children(self.who().map(|who| {
                div()
                    .id("explore-who")
                    .aria_label(who.clone())
                    .test_support()
                    .text_color(p.ink_2)
                    .child(who)
            }))
            .children(self.route.clone().map(|route| {
                div()
                    .id("explore-route")
                    .aria_label(route.clone())
                    .test_support()
                    .text_color(p.muted)
                    .truncate()
                    .child(route)
            }))
            .child(div().flex_1())
            .children(summary.map(|summary| {
                div()
                    .id("explore-summary")
                    .aria_label(summary.clone())
                    .test_support()
                    .text_color(p.muted)
                    .truncate()
                    .child(summary)
            }))
            .children(capped.then(|| {
                ui::tag(ui::Tone::Warn, None, "Capped", cx)
                    .id("explore-capped")
                    .aria_label("The table has more rows than the cap")
                    .test_support()
            }))
            .child(
                Button::new("explore-reload")
                    .ghost()
                    .small()
                    .h(dp(ui::CONTROL_HEIGHT))
                    .icon(ui::refresh_icon(
                        self.browser.read(cx).is_loading() || self.data.read(cx).is_loading(),
                        cx,
                    ))
                    .accessibility_label("Read the schema and the table again")
                    .tooltip("Read again")
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.browser.update(cx, |b, cx| b.reload(cx));
                        this.data.update(cx, |d, cx| d.reload(cx));
                    })),
            )
            .child(
                h_flex()
                    .id("explore-writes")
                    .test_support()
                    .aria_label("Writes: later slice")
                    .gap(dp(6.))
                    .child(
                        Button::new("explore-writes-gate")
                            .ghost()
                            .small()
                            .h(dp(ui::CONTROL_HEIGHT))
                            .label("Edit data")
                            .disabled(true),
                    )
                    .child(ui::tag(ui::Tone::Outline, None, "later slice", cx)),
            )
            .into_any_element()
    }
}

impl Render for ExploreView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let body = if self.phase == Phase::Connected {
            let p = palette(cx);
            v_flex()
                .size_full()
                .child(self.strip(cx))
                .child(
                    h_flex()
                        .flex_1()
                        .min_h_0()
                        .child(
                            div()
                                .w(dp(380.))
                                .flex_none()
                                .h_full()
                                .border_r_1()
                                .border_color(p.line)
                                .child(self.browser.clone()),
                        )
                        .child(div().flex_1().min_w_0().h_full().child(self.data.clone())),
                )
                .into_any_element()
        } else {
            self.step(cx)
        };
        div()
            .id("explore")
            .test_support()
            .size_full()
            .flex()
            .flex_col()
            .min_h_0()
            .child(body)
    }
}

use gpui_kit::component::v_flex;
