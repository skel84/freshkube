//! The fix bar: what a page can't confirm, counted ("3 to fix") and listed
//! as one chip each, under the page header (docs/DESIGN.md, "The change
//! page"). A chip selects what it names. The chips that don't fit the room
//! fold into a `+N` menu of the rest, so the bar stays one line.
//!
//! How many chips fit is worked out from their text, in dp, as [`fit`]
//! sizes a column, so drawing measures nothing.

use std::rc::Rc;

use gpui_kit::base::ObservedElement as Observed;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::menu::{DropdownMenu, PopupMenuItem};
use gpui_kit::component::{Sizable, h_flex};
use gpui_kit::prelude::*;
use gpui_kit::{Anchor, App, Div, ElementId, SharedString, Stateful, TestSupportExt, Window, div};

use crate::palette::palette;
use crate::ui::{self, Tone, dp};

/// A character of a chip's text, in dp.
const CHAR: f32 = 5.9;
/// A chip's glyph, padding and gap, in dp, beside its text.
const CHIP_PAD: f32 = 30.;
/// The gap between the bar's parts, in dp.
const GAP: f32 = 6.;

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

    fn width(&self) -> f32 {
        self.label.chars().count() as f32 * CHAR + CHIP_PAD
    }
}

/// The count's words: `3 to fix`.
pub fn count_words(count: usize) -> String {
    format!("{count} to fix")
}

/// The width of the count, in dp.
fn lead_width(count: usize) -> f32 {
    count_words(count).chars().count() as f32 * CHAR
}

/// The width of the `+N` button, in dp.
fn more_width(hidden: usize) -> f32 {
    format!("+{hidden}").chars().count() as f32 * CHAR + CHIP_PAD / 2.
}

/// How many of `items` fit one line `room` dp wide, with the count before
/// them and `+N` after the last, whenever some are left out. At least one
/// shows: a bar of nothing but `+N` says nothing.
pub fn visible(items: &[FixItem], room: f32) -> usize {
    let mut used = lead_width(items.len()) + GAP;
    let mut shown = 0;
    for (ix, item) in items.iter().enumerate() {
        let left = items.len() - ix - 1;
        let more = if left > 0 { more_width(left) + GAP } else { 0. };
        used += item.width() + GAP;
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
    cx: &App,
) -> Option<Observed<Stateful<Div>>> {
    if items.is_empty() {
        return None;
    }
    let id: ElementId = id.into();
    let name = id.to_string();
    let p = palette(cx);
    let select = Rc::new(select);
    let shown = visible(items, room);
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

    #[test]
    fn every_chip_shows_when_the_room_holds_them() {
        let list = items(&["Not found: policy", "Can't read: prod pods"]);
        assert_eq!(visible(&list, 1200.), 2);
    }

    #[test]
    fn chips_that_do_not_fit_fold_into_the_rest() {
        let list = items(&[
            "Can't read: prod-ams pods",
            "Not set up: Build run",
            "Not found: policy result",
            "Not found: signature",
        ]);
        let wide = visible(&list, 1200.);
        let narrow = visible(&list, 480.);
        assert_eq!(wide, 4);
        assert!(narrow < wide && narrow >= 1, "{narrow}");
    }

    #[test]
    fn one_chip_shows_however_narrow_the_room() {
        let list = items(&["Can't read: prod-ams pods", "Not found: policy result"]);
        assert_eq!(visible(&list, 10.), 1);
    }

    /// `n` chips of 20 characters each: 148 dp wide.
    fn same(n: usize) -> Vec<FixItem> {
        (0..n)
            .map(|ix| FixItem::new(format!("k{ix}"), "Not found: signature"))
            .collect()
    }

    #[test]
    fn a_known_room_shows_an_exact_number_of_chips() {
        // "12 to fix" 53.1 and a gap, then 154 (148 and a gap) for each
        // chip, and the "+7" button (26.8 and a gap) while any are left
        // out: five chips take 829.1 and the button 32.8, so 861.9.
        assert_eq!(visible(&same(12), 1000.), 5);
        assert_eq!(visible(&same(12), 862.), 5);
        assert_eq!(visible(&same(12), 861.), 4);
        // Room for everything needs no button.
        assert_eq!(visible(&same(3), 600.), 3);
    }

    #[test]
    fn a_two_digit_rest_takes_the_wider_button() {
        // Eleven left out is "+11" (38.7 with its gap, against 32.8 for
        // "+7"): five chips and it take 867.8, which 865 does not hold.
        assert_eq!(visible(&same(16), 1000.), 5);
        assert_eq!(visible(&same(16), 868.), 5);
        assert_eq!(visible(&same(16), 865.), 4);
        assert_eq!(visible(&same(12), 865.), 5);
        assert_eq!(more_width(11), more_width(10));
        assert!(more_width(11) > more_width(9));
    }

    #[test]
    fn the_count_is_in_words() {
        assert_eq!(count_words(4), "4 to fix");
    }
}
