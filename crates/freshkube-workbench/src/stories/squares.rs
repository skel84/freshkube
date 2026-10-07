//! Container squares: one small square per container, as Pods' Containers
//! column draws them (`freshkube_ui::squares`), on invented pods. Each state
//! has a row with its words beside it, as the cell's tooltip gives them;
//! Many shows a pod with more containers than the cell draws.

use super::motion::{option, segments};
use freshkube_ui::page::{self, PageHeader};
use freshkube_ui::palette::palette;
use freshkube_ui::squares::{self, Square, Squares};
use freshkube_ui::ui::{self, Tone, dp};
use gpui_kit::component::{h_flex, v_flex};
use gpui_kit::prelude::*;
use gpui_kit::{AnyView, App, Context, SharedString, TestSupportExt, Window, div};

/// The id prefix of everything the story draws.
const PREFIX: &str = "squares";

pub fn build(_: &mut Window, cx: &mut App) -> AnyView {
    cx.new(|_| SquaresStory { many: false }).into()
}

pub struct SquaresStory {
    many: bool,
}

impl SquaresStory {
    pub fn many(&self) -> bool {
        self.many
    }

    fn set_many(&mut self, many: bool, cx: &mut Context<Self>) {
        self.many = many;
        cx.notify();
    }
}

/// Each state's pod: its name, its squares and what the tooltip says.
fn cases() -> Vec<(&'static str, Vec<Square>, &'static str)> {
    let good = Square::new(Tone::Good);
    let warn = Square::new(Tone::Warn);
    let crit = Square::new(Tone::Crit);
    let done = Square::new(Tone::Unknown);
    vec![
        ("app-a", vec![good], "running"),
        (
            "app-b",
            vec![good, good.outlined(true)],
            "running; sidecar running · 3 restarts",
        ),
        ("app-c", vec![warn], "running, not ready"),
        ("app-d", vec![warn], "waiting · ContainerCreating"),
        (
            "app-e",
            vec![crit.outlined(true)],
            "waiting · CrashLoopBackOff · 7 restarts",
        ),
        ("app-f", vec![crit], "exited 137 · OOMKilled"),
        ("app-g", vec![done], "exited 0 · Completed"),
        ("app-h", vec![done.outlined(true)], "no state reported"),
        (
            "shop-api",
            vec![done.dim(true), good],
            "migrate (init): exited 0 · Completed; api running",
        ),
        (
            "report",
            vec![done, done],
            "ship-logs (sidecar): exited 143, stopped when the Job succeeded; report exited 0",
        ),
    ]
}

/// A pod with twelve containers: the cell draws eight and counts the rest.
fn crowded() -> Vec<(&'static str, Vec<Square>, &'static str)> {
    let mut squares = vec![Square::new(Tone::Good); 11];
    squares.insert(5, Square::new(Tone::Warn));
    vec![("shop-mesh", squares, "12 containers, one not ready")]
}

impl Render for SquaresStory {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        freshkube_probe::probe::hit("workbench.squares");
        let many = self.many;
        let choices = segments(
            [(false, "states", "States"), (true, "many", "Many")].map(|(choice, id, label)| {
                option(format!("{PREFIX}-{id}"), label, many == choice, cx)
                    .on_click(cx.listener(move |this, _, _, cx| this.set_many(choice, cx)))
            }),
            cx,
        );
        let p = palette(cx);
        let header = PageHeader::new(PREFIX, "Container squares")
            .control(choices)
            .meta([div().child("Invented pods").into_any_element()])
            .render(window, cx);
        let rows = if many { crowded() } else { cases() };
        page::page(SharedString::from(format!("{PREFIX}-page")))
            .child(page::toolbar(cx).child(header))
            .child(
                page::inset().child(v_flex().gap(dp(10.)).children(rows.into_iter().map(
                    |(name, drawn, words)| {
                        h_flex()
                            .id(SharedString::from(format!("{PREFIX}-{name}")))
                            .test_support()
                            .gap(dp(12.))
                            .child(div().w(dp(96.)).font_family(ui::MONO_FONT).child(name))
                            .child(
                                div()
                                    .w(dp(squares::width(drawn.len())))
                                    .flex_none()
                                    .child(squares::squares(&Squares::new(drawn), &p)),
                            )
                            .child(div().text_color(p.muted).child(words))
                    },
                ))),
            )
    }
}
