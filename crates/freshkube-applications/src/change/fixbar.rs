//! What the change can't confirm, under the header: its headline sentence
//! and the fix bar (docs/DESIGN.md, "The change page"). Both are derived
//! with the change (`show`); drawing only lays them out. A chip selects its
//! hop, as a click on its row does.
use super::*;
use crate::ui::dp;
use freshkube_ui::page;
use freshkube_ui::palette::palette;
use gpui_kit::component::v_flex;

/// A fix as a chip: its hop's key and its words.
pub(super) fn item(fix: &Fix) -> kit::FixItem {
    kit::FixItem::new(fix.hop.clone(), fix.words())
}

impl ChangePage {
    /// The headline and the bar, in one inset under the header; nothing
    /// while the change is unread.
    pub(super) fn render_summary(
        &self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        if self.headline.is_empty() || self.rows.is_empty() || self.loading_rows().is_some() {
            return None;
        }
        let p = palette(cx);
        let this = cx.entity().downgrade();
        let bar = kit::fix_bar(
            "change-fixbar",
            &self.fix_items,
            page::inset_width(window),
            move |key, window, cx| {
                _ = this.update(cx, |page, cx| page.select(key, window, cx));
            },
            cx,
        );
        Some(
            page::inset()
                .id("change-summary")
                .child(
                    v_flex()
                        .w_full()
                        .min_w_0()
                        .gap(dp(6.))
                        .child(
                            div()
                                .id("change-headline")
                                .test_support()
                                .aria_label(self.headline.clone())
                                .w_full()
                                .min_w_0()
                                // GPUI wraps every line but the last at the
                                // width; the ellipsis makes the last one
                                // wrap and end there too.
                                .text_ellipsis()
                                .line_clamp(2)
                                .text_size(dp(12.5))
                                .text_color(p.ink_2)
                                .child(self.headline.clone()),
                        )
                        .children(bar),
                )
                .into_any_element(),
        )
    }

    /// The headline sentence, for the page's tests.
    #[cfg(test)]
    pub(super) fn headline(&self) -> &str {
        &self.headline
    }
}
