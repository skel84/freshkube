//! The fix bar (`freshkube_ui::table::fix_bar`): what a page can't confirm,
//! counted and listed as chips. Width shows how chips that don't fit fold
//! into `+N`; Few and None show a short list and the empty one, which draws
//! nothing. A chip, or a menu item, selects what it names, and the story
//! says which.

use super::motion::{option, segments};
use freshkube_ui::page::{self, PageHeader};
use freshkube_ui::palette::palette;
use freshkube_ui::table::{self, FixItem};
use freshkube_ui::ui::dp;
use gpui_kit::component::v_flex;
use gpui_kit::prelude::*;
use gpui_kit::{AnyView, App, Context, SharedString, TestSupportExt, Window, div};

/// The id prefix of everything the story draws.
const PREFIX: &str = "fix-bar";

pub fn build(_: &mut Window, cx: &mut App) -> AnyView {
    cx.new(|_| FixBarStory {
        case: Case::Wide,
        picked: None,
    })
    .into()
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Case {
    /// Room for every chip.
    Wide,
    /// Room for a chip or two: the rest folds into `+N`.
    Narrow,
    /// A short list.
    Few,
    /// Nothing to fix: no bar.
    None,
}

impl Case {
    const ALL: [(Self, &'static str, &'static str); 4] = [
        (Self::Wide, "wide", "Wide"),
        (Self::Narrow, "narrow", "Narrow"),
        (Self::Few, "few", "Few"),
        (Self::None, "none", "None"),
    ];

    /// The room the bar has, in dp.
    fn room(self) -> f32 {
        match self {
            Self::Narrow => 450.,
            _ => 1100.,
        }
    }

    fn items(self) -> Vec<FixItem> {
        let all = [
            ("prod-a-pods", "Can't read: prod-a pods"),
            ("build-run", "Not set up: Build run"),
            ("policy", "Not found: policy result"),
            ("signature", "Not found: signature"),
        ];
        let shown = match self {
            Self::Wide | Self::Narrow => 4,
            Self::Few => 1,
            Self::None => 0,
        };
        all.into_iter()
            .take(shown)
            .map(|(key, label)| FixItem::new(key, label))
            .collect()
    }
}

pub struct FixBarStory {
    case: Case,
    picked: Option<SharedString>,
}

impl FixBarStory {
    pub fn case(&self) -> Case {
        self.case
    }

    pub fn picked(&self) -> Option<&SharedString> {
        self.picked.as_ref()
    }

    fn set(&mut self, case: Case, cx: &mut Context<Self>) {
        self.case = case;
        self.picked = None;
        cx.notify();
    }
}

impl Render for FixBarStory {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        freshkube_probe::probe::hit("workbench.fix_bar");
        let case = self.case;
        let choices = segments(
            Case::ALL.map(|(choice, id, label)| {
                option(format!("{PREFIX}-{id}"), label, case == choice, cx)
                    .on_click(cx.listener(move |this, _, _, cx| this.set(choice, cx)))
            }),
            cx,
        );
        let header = PageHeader::new(PREFIX, "Fix bar")
            .control(choices)
            .meta([div().child("Invented fixes").into_any_element()])
            .render(window, cx);
        let p = palette(cx);
        let story = cx.entity().downgrade();
        let bar = table::fix_bar(
            "fix-bar-bar",
            &case.items(),
            case.room(),
            move |key, _, cx| {
                _ = story.update(cx, |this, cx| {
                    this.picked = Some(key.clone());
                    cx.notify();
                });
            },
            cx,
        );
        let picked = self
            .picked
            .as_ref()
            .map_or("Nothing selected".to_owned(), |key| {
                format!("Selected {key}")
            });
        page::page(SharedString::from(format!("{PREFIX}-page")))
            .child(page::toolbar(cx).child(header))
            .child(
                page::inset().child(
                    v_flex()
                        .gap(dp(10.))
                        .child(div().w(dp(case.room())).child(
                            bar.map(IntoElement::into_any_element).unwrap_or_else(|| {
                                div()
                                    .text_color(p.muted)
                                    .child("Nothing to fix: no bar")
                                    .into_any_element()
                            }),
                        ))
                        .child(
                            div()
                                .id("fix-bar-picked")
                                .test_support()
                                .aria_label(SharedString::from(picked.clone()))
                                .text_color(p.muted)
                                .child(picked),
                        ),
                ),
            )
    }
}
