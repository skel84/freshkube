use super::*;
use crate::ui::{self, dp};
use freshkube_db::wire::Encryption;
use freshkube_ui::{page, palette::palette};
use gpui_kit::TestSupportExt as _;
use gpui_kit::assets::IconName;
use gpui_kit::component::{
    Disableable, Sizable,
    button::{Button, ButtonVariants},
    h_flex,
};

/// The page width from which the browser sits beside the rows, in dp.
const BESIDE_WIDTH: f32 = 900.;
const BROWSER_WIDTH: f32 = 400.;
/// Stacked, the browser's height and the rows' least.
const BROWSER_HEIGHT: f32 = 200.;
const DATA_LEAST: f32 = 280.;

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
        // A narrow window or a large text size may not fit the words and the
        // button: the state scrolls, so Try again can always be reached.
        div()
            .id("explore-state-scroll")
            .size_full()
            .overflow_y_scroll()
            .restrict_scroll_to_axis()
            .child(
                page::inset().child(
                    ui::empty_state(IconName::Database, title, words.clone(), None, actions, cx)
                        .h_auto()
                        .id(SharedString::from(id))
                        .aria_label(words)
                        .test_support(),
                ),
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
            .flex_wrap()
            .gap_x(dp(8.))
            .gap_y(dp(2.))
            .px(dp(page::PANE_PADDING))
            .py(dp(3.))
            .min_h(dp(34.))
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
            .children(self.encryption().map(|encryption| {
                let tone = match encryption {
                    Encryption::Verified => ui::Tone::Good,
                    Encryption::Unverified | Encryption::Plain => ui::Tone::Warn,
                };
                ui::tag(tone, None, encryption.label(), cx)
                    .id("explore-encryption")
                    .aria_label(encryption.label())
                    .test_support()
            }))
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
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // Below this the rows would have no room beside the browser.
        let beside = crate::screens::page_width(window) >= BESIDE_WIDTH;
        let connected = self.phase == Phase::Connected;
        let body = if connected {
            let p = palette(cx);
            let browser = div()
                .flex_none()
                .border_color(p.line)
                .child(self.browser.clone());
            let data = div().flex_1().min_w_0().min_h_0().child(self.data.clone());
            let panes = if beside {
                h_flex()
                    .flex_1()
                    .min_h_0()
                    .child(browser.w(dp(BROWSER_WIDTH)).h_full().border_r_1())
                    .child(data.h_full())
            } else {
                v_flex()
                    .flex_1()
                    .min_h_0()
                    .child(browser.w_full().h(dp(BROWSER_HEIGHT)).border_b_1())
                    .child(data.min_h(dp(DATA_LEAST)))
            };
            v_flex()
                .size_full()
                .child(self.strip(cx))
                .child(panes)
                .into_any_element()
        } else {
            self.step(cx)
        };
        // A short window scrolls the page; the stacked panes keep their room.
        let least = if connected && !beside {
            34. + BROWSER_HEIGHT + DATA_LEAST
        } else {
            0.
        };
        div()
            .id("explore")
            .test_support()
            .size_full()
            .flex()
            .flex_col()
            .min_h(dp(least))
            .child(body)
    }
}

use gpui_kit::component::v_flex;
