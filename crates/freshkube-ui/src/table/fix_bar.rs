//! The fix bar: what a page can't confirm, counted ("3 to fix") and listed
//! as one chip each, under the page header (docs/DESIGN.md, "The change
//! page"). A chip selects what it names. The chips that don't fit the room
//! fold into a `+N` menu of the rest, so the bar stays one line.
//!
//! How many chips fit is worked out from their text, measured with the
//! text system at the chip's size, in dp, plus each chip's fixed icon and
//! padding.

use std::rc::Rc;

use gpui_kit::base::ObservedElement as Observed;
use gpui_kit::component::ActiveTheme;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::menu::{DropdownMenu, PopupMenuItem};
use gpui_kit::component::{Sizable, h_flex};
use gpui_kit::prelude::*;
use gpui_kit::{
    Anchor, App, Div, ElementId, Font, SharedString, Stateful, TestSupportExt, TextRun, Window, div,
};

use crate::palette::palette;
use crate::ui::{self, Tone, dp};

/// A chip's glyph, padding and gap, in dp, beside its text.
const CHIP_PAD: f32 = 20.;
/// The `+N` button's padding, in dp, beside its text.
const MORE_PAD: f32 = 10.;
/// The gap between the bar's parts, in dp.
const GAP: f32 = 6.;
/// The count's text size in the bar, in dp.
const TEXT: f32 = 12.;
/// A chip's and the `+N` button's text size in dp: Kit's extra small
/// button text, 0.75 rem.
const BUTTON_TEXT: f32 = 9.75;

/// One thing to fix: what a chip selects, and its words.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FixItem {
    /// What the page selects; the chip's id is `<bar>-<key>`.
    pub key: SharedString,
    /// The chip's words, `⟨cause⟩: ⟨where⟩`.
    pub label: SharedString,
}

impl FixItem {
    pub fn new(key: impl Into<SharedString>, label: impl Into<SharedString>) -> Self {
        Self {
            key: key.into(),
            label: label.into(),
        }
    }
}

/// The count's words: `3 to fix`.
pub fn count_words(count: usize) -> String {
    format!("{count} to fix")
}

/// The widths of a bar's parts in dp, worked out from their text.
#[derive(Clone, Debug, PartialEq)]
pub struct Measured {
    /// The count's words.
    lead: f32,
    /// Each chip, with its glyph and padding.
    chips: Vec<f32>,
    /// The `+N` button for `n` left out, at index `n`.
    more: Vec<f32>,
}

impl Measured {
    /// Measures the bar's text at its size with the window's text system,
    /// in the font the chips draw in: the theme's, not the window's default.
    pub fn of(items: &[FixItem], window: &mut Window, cx: &App) -> Self {
        let unit = ui::dp_px(1., window).as_f32();
        let font = Font {
            family: cx.theme().font_family.clone(),
            ..Default::default()
        };
        let mut width = |text: String, dp: f32| {
            let size = ui::dp_px(dp, window);
            let text = SharedString::from(text);
            let run = TextRun {
                len: text.len(),
                font: font.clone(),
                color: Default::default(),
                background_color: None,
                underline: None,
                strikethrough: None,
            };
            let line = window.text_system().shape_line(text, size, &[run], None);
            line.width.as_f32() / unit
        };
        Self {
            lead: width(count_words(items.len()), TEXT),
            chips: items
                .iter()
                .map(|item| width(item.label.to_string(), BUTTON_TEXT) + CHIP_PAD)
                .collect(),
            more: (0..=items.len())
                .map(|hidden| width(format!("+{hidden}"), BUTTON_TEXT) + MORE_PAD)
                .collect(),
        }
    }

    /// The width of the `ix`th chip in dp, with its glyph and padding.
    pub fn chip(&self, ix: usize) -> f32 {
        self.chips[ix]
    }

    /// Widths of `char` dp a character, for tests that need exact rooms.
    #[cfg(test)]
    fn of_chars(items: &[FixItem], char: f32) -> Self {
        let width = |text: &str| text.chars().count() as f32 * char;
        Self {
            lead: width(&count_words(items.len())),
            chips: items
                .iter()
                .map(|item| width(&item.label) + CHIP_PAD)
                .collect(),
            more: (0..=items.len())
                .map(|hidden| width(&format!("+{hidden}")) + MORE_PAD)
                .collect(),
        }
    }
}

/// How many chips fit one line `room` dp wide, with the count before them
/// and `+N` after the last, whenever some are left out. At least one shows:
/// a bar of nothing but `+N` says nothing.
pub fn visible(sizes: &Measured, room: f32) -> usize {
    let total = sizes.chips.len();
    let mut used = sizes.lead + GAP;
    let mut shown = 0;
    for (ix, chip) in sizes.chips.iter().enumerate() {
        let left = total - ix - 1;
        let more = if left > 0 { sizes.more[left] + GAP } else { 0. };
        used += chip + GAP;
        if used + more > room && shown >= 1 {
            break;
        }
        shown += 1;
    }
    shown
}

/// The bar: `N to fix`, the chips that fit `room` dp, and `+N` for the
/// rest. `select` runs with the chip's key, from a chip or the menu.
/// `None` for nothing to fix: the bar shows only when something can't be
/// confirmed.
pub fn fix_bar(
    id: impl Into<ElementId>,
    items: &[FixItem],
    room: f32,
    select: impl Fn(&SharedString, &mut Window, &mut App) + 'static,
    window: &mut Window,
    cx: &App,
) -> Option<Observed<Stateful<Div>>> {
    if items.is_empty() {
        return None;
    }
    let id: ElementId = id.into();
    let name = id.to_string();
    let p = palette(cx);
    let select = Rc::new(select);
    let shown = visible(&Measured::of(items, window, cx), room);
    let chips = items[..shown].iter().map(|item| {
        let select = select.clone();
        let key = item.key.clone();
        Button::new(SharedString::from(format!("{name}-{}", item.key)))
            .ghost()
            .xsmall()
            .children(ui::status_glyph(Tone::Unknown, cx))
            .child(div().child(item.label.clone()))
            .text_color(p.ink_2)
            .accessibility_label(item.label.clone())
            .tooltip(format!("{} · click to select", item.label))
            .on_click(move |_, window, cx| select(&key, window, cx))
    });
    let rest: Vec<FixItem> = items[shown..].to_vec();
    let more = (!rest.is_empty()).then(|| {
        let hidden = rest.len();
        let menu_select = select.clone();
        Button::new(SharedString::from(format!("{name}-more")))
            .ghost()
            .xsmall()
            .label(format!("+{hidden}"))
            .text_color(p.ink_2)
            .accessibility_label(format!("{hidden} more to fix"))
            .tooltip(format!("{hidden} more to fix"))
            .dropdown_menu_with_anchor(Anchor::TopLeft, move |mut menu, _, _| {
                for item in &rest {
                    let select = menu_select.clone();
                    let key = item.key.clone();
                    menu = menu.item(
                        PopupMenuItem::new(item.label.clone())
                            .on_click(move |_, window, cx| select(&key, window, cx)),
                    );
                }
                menu
            })
    });
    Some(
        h_flex()
            .id(id)
            .test_support()
            .flex_none()
            .gap(dp(GAP))
            .overflow_hidden()
            .text_size(dp(12.))
            .child(
                div()
                    .flex_none()
                    .text_color(p.muted)
                    .child(count_words(items.len())),
            )
            .children(chips)
            .children(more),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn items(labels: &[&str]) -> Vec<FixItem> {
        labels
            .iter()
            .enumerate()
            .map(|(ix, label)| FixItem::new(format!("k{ix}"), *label))
            .collect()
    }

    /// The bar's widths at 5 dp a character.
    fn sized(list: &[FixItem]) -> Measured {
        Measured::of_chars(list, 5.)
    }

    #[test]
    fn every_chip_shows_when_the_room_holds_them() {
        let list = items(&["Not found: policy", "Can't read: prod pods"]);
        assert_eq!(visible(&sized(&list), 1200.), 2);
    }

    #[test]
    fn chips_that_do_not_fit_fold_into_the_rest() {
        let list = items(&[
            "Can't read: prod-ams pods",
            "Not set up: Build run",
            "Not found: policy result",
            "Not found: signature",
        ]);
        let wide = visible(&sized(&list), 1200.);
        let narrow = visible(&sized(&list), 400.);
        assert_eq!(wide, 4);
        assert!(narrow < wide && narrow >= 1, "{narrow}");
    }

    #[test]
    fn one_chip_shows_however_narrow_the_room() {
        let list = items(&["Can't read: prod-ams pods", "Not found: policy result"]);
        assert_eq!(visible(&sized(&list), 10.), 1);
    }

    /// `n` chips of 20 characters each: 120 dp wide with their padding.
    fn same(n: usize) -> Measured {
        let list: Vec<FixItem> = (0..n)
            .map(|ix| FixItem::new(format!("k{ix}"), "Not found: signature"))
            .collect();
        sized(&list)
    }

    #[test]
    fn a_known_room_shows_an_exact_number_of_chips() {
        // "12 to fix" 45 and a gap, then 126 (120 and a gap) for each
        // chip, and the "+7" button (20 and a gap) while any are left
        // out: five chips take 681 and the button 26, so 707.
        assert_eq!(visible(&same(12), 1000.), 7);
        assert_eq!(visible(&same(12), 707.), 5);
        assert_eq!(visible(&same(12), 706.), 4);
        // Room for everything needs no button.
        assert_eq!(visible(&same(3), 600.), 3);
    }

    #[test]
    fn a_two_digit_rest_takes_the_wider_button() {
        // Eleven left out is "+11" (31 with its gap, against 26 for "+7"):
        // five chips and it take 712, which 711 does not hold.
        assert_eq!(visible(&same(16), 712.), 5);
        assert_eq!(visible(&same(16), 711.), 4);
        assert_eq!(visible(&same(12), 711.), 5);
        let sizes = same(16);
        assert_eq!(sizes.more[11], sizes.more[10]);
        assert!(sizes.more[11] > sizes.more[9]);
    }

    #[test]
    fn the_count_is_in_words() {
        assert_eq!(count_words(4), "4 to fix");
    }
}
