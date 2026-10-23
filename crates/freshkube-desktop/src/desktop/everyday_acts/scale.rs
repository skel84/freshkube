//! Scale asks one thing first, the count; everything after it is the write
//! flow's (`write_flow`), so the level decides whether anything more is asked.

use std::sync::Arc;

use freshkube_core::write::Runner;
use freshkube_core::write::everyday::{self, Subject};
use freshkube_ui::form::{self, Action};
use gpui_kit::component::{
    Sizable, WindowExt,
    input::{Input, InputState},
    v_flex,
};
use gpui_kit::prelude::*;
use gpui_kit::*;
use tokio::runtime::Handle;

use crate::palette::palette;
use crate::ui::dp;

/// The most a count may ask for here; a larger one is almost always a typo.
pub(super) const MOST: u64 = 10_000;

/// Reads the typed count: a whole number from 0 to [`MOST`].
pub(super) fn parse(text: &str) -> Result<u64, String> {
    if text.trim().starts_with('-') {
        return Err("A replica count can't be negative.".to_owned());
    }
    let count: u64 = text
        .trim()
        .parse()
        .map_err(|_| "Enter a whole number of replicas, such as 3.".to_owned())?;
    if count > MOST {
        return Err(format!("Scale to at most {MOST} replicas."));
    }
    Ok(count)
}

pub(super) struct ScaleDialog {
    runner: Arc<Runner>,
    runtime: Handle,
    subject: Subject,
    count: Entity<InputState>,
    error: Option<String>,
}

pub(super) fn open(
    runner: Arc<Runner>,
    runtime: Handle,
    subject: Subject,
    window: &mut Window,
    cx: &mut App,
) {
    let current = subject.replicas().to_string();
    let count = cx.new(|cx| InputState::new(window, cx).placeholder("Replicas"));
    count.update(cx, |input, cx| input.set_value(current, window, cx));
    let dialog = cx.new(|_| ScaleDialog {
        runner,
        runtime,
        subject,
        count,
        error: None,
    });
    window.open_dialog(cx, move |view, window, cx| {
        let title = format!("Scale {}", dialog.read(cx).subject.target().label());
        let weak = dialog.downgrade();
        let actions = form::actions(
            Action::new("scale-review", "Review", move |_, window, cx| {
                _ = weak.update(cx, |dialog, cx| dialog.review(window, cx));
            }),
            Action::cancel("scale-cancel"),
        );
        freshkube_ui::dialog::frame(view, 420., title, dialog.clone(), actions, window)
    });
    // The count is the first thing the dialog wants.
}

impl ScaleDialog {
    fn review(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let text = self.count.read(cx).value().to_string();
        let action = parse(&text).and_then(|count| {
            everyday::scale(&self.subject, count)
                .map_err(|everyday::NotOffered(why)| why.to_owned())
        });
        match action {
            Ok(action) => {
                window.close_dialog(cx);
                super::start(
                    self.runner.clone(),
                    self.runtime.clone(),
                    action,
                    window,
                    cx,
                );
            }
            Err(why) => {
                self.error = Some(why);
                cx.notify();
            }
        }
    }
}

impl Render for ScaleDialog {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let p = palette(cx);
        v_flex()
            .id("scale-dialog")
            .test_support()
            .gap(dp(8.))
            .child(format!(
                "{} has {} replicas now. Scale to:",
                self.subject.target().label(),
                self.subject.replicas()
            ))
            .child(Input::new(&self.count).id("scale-count").small())
            .when_some(self.error.clone(), |this, why| {
                this.child(
                    div()
                        .id("scale-error")
                        .test_support()
                        .text_color(p.crit_ink)
                        .child(why),
                )
            })
    }
}

#[cfg(test)]
mod tests {
    // Not `super::*`: gpui_kit's glob would shadow the built-in `#[test]`.
    use super::{MOST, parse};

    #[test]
    fn a_count_is_a_whole_number_up_to_the_most() {
        assert_eq!(parse(" 3 "), Ok(3));
        assert_eq!(parse("0"), Ok(0));
        assert_eq!(parse("10000"), Ok(MOST));
        for bad in ["", "-1", "2.5", "three", "10001"] {
            assert!(parse(bad).is_err(), "{bad}");
        }
        assert_eq!(
            parse("-2"),
            Err("A replica count can't be negative.".to_owned())
        );
    }
}
