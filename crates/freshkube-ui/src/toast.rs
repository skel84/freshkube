//! Where toasts show (#927): at the bottom right, above the status bar and
//! the footer row over it, where a table keeps its counts and an Inspector
//! its actions. At Kit's default, the top right under the title bar, they
//! covered the page header's buttons until they closed.

use gpui_kit::component::Theme;
use gpui_kit::{Anchor, App, Pixels, px};

use crate::page::STATUS_BAR_HEIGHT;
use crate::ui::BASE_TEXT;

/// The footer row a toast stays above, in dp. The change page's wrapped
/// actions measure 38 at text size 13; `Inspector::render` (inspector/mod.rs)
/// adds 6 above and below plus a 1 px hairline: 51 in all. The footer takes
/// variable-height content, so it has no fixed height to share. A table's
/// footer is 26.
pub const FOOTER_ROW: f32 = 51.;
/// Between a toast and that row, in dp.
pub const GAP: f32 = 8.;
/// Between a toast and the window's right edge, in dp: Kit's 16 px at the
/// default text size.
pub const EDGE: f32 = 16.;

/// Places Kit's notification layer for text of `size` px. Its margins are
/// pixels, so they are set again whenever the text size changes, and a
/// theme change keeps them.
pub fn place(size: f32, cx: &mut App) {
    let dp = |n: f32| -> Pixels { px(n * size / BASE_TEXT) };
    let notification = &mut Theme::global_mut(cx).notification;
    notification.placement = Anchor::BottomRight;
    notification.margins.bottom = dp(STATUS_BAR_HEIGHT + FOOTER_ROW + GAP);
    notification.margins.right = dp(EDGE);
}
