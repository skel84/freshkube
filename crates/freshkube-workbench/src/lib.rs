//! The workbench: Freshkube's shared components on invented data, one story
//! at a time, beside a strip that changes the theme, the text size and the
//! window's width (docs/WORKBENCH.md). It depends on the shared crates only,
//! never on the app, so a story is what a page can build from them.

use freshkube_ui::motion::{self, Choice};
use freshkube_ui::palette::palette;
use freshkube_ui::source_list::{self, Line, Row, SourceList, SourceListHost};
use freshkube_ui::text_size;
use freshkube_ui::theme;
use freshkube_ui::ui::{self, dp};
use gpui_kit::component::button::Button;
use gpui_kit::component::{ActiveTheme, Sizable, Theme, ThemeMode, h_flex, v_flex};
use gpui_kit::prelude::*;
use gpui_kit::{
    AnyView, App, Context, Div, KeyBinding, Pixels, SharedString, Size, TestSupportExt,
    TitlebarOptions, Window, WindowBounds, WindowOptions, div, px, size,
};

pub mod stories;

pub use stories::{STORIES, Story};

gpui_kit::actions!(workbench, [Quit]);

/// The window widths the strip offers: a roomy window and the narrowest
/// the app allows.
pub const WIDTHS: [f32; 2] = [1280., 760.];
/// The window's height when it opens.
pub const HEIGHT: f32 = 880.;
/// The story list's width, as the app's column.
pub(crate) const LIST_WIDTH: f32 = source_list::WIDTH;

/// Opens the workbench window on the first story, or in a debug build on
/// `FRESHKUBE_STORY`, at `FRESHKUBE_THEME` and `FRESHKUBE_WINDOW_SIZE` as the
/// app takes them (`FRESHKUBE_TEXT_SIZE` is read by `text_size::install`).
/// Nothing is saved.
pub fn run() {
    gpui_kit::application()
        .with_assets(theme::AppAssets)
        .run(|cx| {
            gpui_kit::init(cx);
            theme::install(cx);
            text_size::install(None, cx);
            motion::follow_system(cx);
            cx.bind_keys([KeyBinding::new("secondary-q", Quit, None)]);
            source_list::bind_keys(cx);
            cx.on_action(|_: &Quit, cx| cx.quit());
            cx.on_window_closed(|cx, _| {
                if cx.windows().is_empty() {
                    cx.quit();
                }
            })
            .detach();
            let start = Start::from_env();
            match start.theme {
                Some(true) => Theme::change(ThemeMode::Dark, None, cx),
                Some(false) => Theme::change(ThemeMode::Light, None, cx),
                None => {}
            }
            let options = WindowOptions {
                window_bounds: Some(WindowBounds::centered(start.size, cx)),
                window_min_size: Some(size(px(WIDTHS[1]), px(560.))),
                titlebar: Some(TitlebarOptions {
                    title: Some("Freshkube workbench".into()),
                    ..Default::default()
                }),
                ..Default::default()
            };
            let opened = gpui_kit::open_window(options, cx, |window, cx| {
                cx.new(|cx| Workbench::new(start.story, window, cx))
            });
            if opened.is_err() {
                cx.quit();
            }
            cx.activate(true);
        });
}

/// What the window opens on.
struct Start {
    story: usize,
    /// Dark, light, or the system's.
    theme: Option<bool>,
    size: Size<Pixels>,
}

impl Start {
    #[cfg(debug_assertions)]
    fn from_env() -> Self {
        let var = |name| std::env::var(name).ok();
        Self {
            story: var("FRESHKUBE_STORY")
                .and_then(|slug| stories::find(&slug))
                .unwrap_or(0),
            theme: match var("FRESHKUBE_THEME").as_deref() {
                Some("dark") => Some(true),
                Some("light") => Some(false),
                _ => None,
            },
            size: var("FRESHKUBE_WINDOW_SIZE")
                .and_then(|value| window_size(&value))
                .unwrap_or(size(px(WIDTHS[0]), px(HEIGHT))),
        }
    }

    #[cfg(not(debug_assertions))]
    fn from_env() -> Self {
        Self {
            story: 0,
            theme: None,
            size: size(px(WIDTHS[0]), px(HEIGHT)),
        }
    }
}

/// `1280x880`, within the sizes the app allows.
#[cfg(any(debug_assertions, test))]
fn window_size(value: &str) -> Option<Size<Pixels>> {
    let (width, height) = value.split_once('x')?;
    let (width, height) = (width.parse::<f32>().ok()?, height.parse::<f32>().ok()?);
    ((760. ..=8192.).contains(&width) && (560. ..=8192.).contains(&height))
        .then(|| size(px(width), px(height)))
}

/// The window's view: the story list, the strip and the story.
pub struct Workbench {
    story: usize,
    view: AnyView,
    list: SourceList<usize>,
}

impl Workbench {
    pub fn new(story: usize, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let story = story.min(STORIES.len() - 1);
        let mut list = SourceList::new("workbench-stories", cx);
        list.set_lines(lines(story));
        Self {
            story,
            view: (STORIES[story].build)(window, cx),
            list,
        }
    }

    /// The story shown, by its index in [`STORIES`].
    pub fn story(&self) -> usize {
        self.story
    }

    /// The story's view.
    pub fn view(&self) -> &AnyView {
        &self.view
    }

    /// Shows another story, built afresh.
    pub fn select(&mut self, story: usize, window: &mut Window, cx: &mut Context<Self>) {
        if story != self.story && story < STORIES.len() {
            self.story = story;
            self.view = (STORIES[story].build)(window, cx);
            self.list.set_lines(lines(story));
            cx.notify();
        }
    }

    /// The stories, as the app's column lists its pages.
    fn render_list(&mut self, window: &Window, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let p = palette(cx);
        v_flex()
            .flex_none()
            .w(dp(LIST_WIDTH))
            .h_full()
            .border_r_1()
            .border_color(p.line)
            .child(source_list::title("Stories", div(), cx))
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .child(source_list::list(&mut self.list, window, cx)),
            )
    }

    /// Theme, text size and width: what every story is checked at.
    fn render_strip(&self, window: &Window, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let dark = cx.theme().mode.is_dark();
        let text = text_size::current(cx);
        let width = window.viewport_size().width / px(1.);
        let themes =
            [("light", "Light", false), ("dark", "Dark", true)].map(|(id, label, mode)| {
                choice(format!("workbench-theme-{id}"), label, mode == dark, cx).on_click(
                    move |_, window, cx| {
                        let mode = if mode {
                            ThemeMode::Dark
                        } else {
                            ThemeMode::Light
                        };
                        Theme::change(mode, Some(window), cx);
                    },
                )
            });
        let sizes = text_size::STEPS.map(|step| {
            choice(
                format!("workbench-text-{step}"),
                step.to_string(),
                step == text,
                cx,
            )
            .on_click(move |_, _, cx| text_size::set(step, cx))
        });
        let widths = WIDTHS.map(|preset| {
            choice(
                format!("workbench-width-{preset}"),
                preset.to_string(),
                (width - preset).abs() < 0.5,
                cx,
            )
            .on_click(move |_, window, _| {
                let height = window.viewport_size().height;
                window.resize(size(px(preset), height));
            })
        });
        let chosen = motion::choice(cx);
        let motions = [
            (Choice::System, "system", self.system_label(cx)),
            (Choice::Reduced, "reduced", "Reduced".into()),
            (Choice::Full, "full", "Full".into()),
        ]
        .map(|(choice, id, label)| {
            self::choice(
                format!("workbench-motion-{id}"),
                label,
                choice == chosen,
                cx,
            )
            .on_click(move |_, _, cx| motion::choose(choice, cx))
        });
        h_flex()
            .id("workbench-strip")
            .test_support()
            .flex_none()
            .flex_wrap()
            .gap(dp(16.))
            .px(dp(12.))
            .py(dp(8.))
            .border_b_1()
            .border_color(palette(cx).line)
            .child(group("Theme", themes, cx))
            .child(group("Text", sizes, cx))
            .child(group("Width", widths, cx))
            .child(group("Motion", motions, cx))
    }

    /// The System option, with what the OS last said.
    fn system_label(&self, cx: &App) -> SharedString {
        match motion::system(cx) {
            Some(true) => "System (reduced)".into(),
            Some(false) => "System (full)".into(),
            None => "System".into(),
        }
    }
}

/// The story list's lines, with `shown` selected.
fn lines(shown: usize) -> Vec<Line<usize>> {
    STORIES
        .iter()
        .enumerate()
        .map(|(ix, story)| {
            Row::new(ix, format!("story-{}", story.slug), story.title)
                .icon((story.icon)())
                .current(ix == shown)
                .into()
        })
        .collect()
}

impl SourceListHost for Workbench {
    type Key = usize;

    fn source_list(&mut self) -> &mut SourceList<usize> {
        &mut self.list
    }

    fn open(&mut self, story: &usize, window: &mut Window, cx: &mut Context<Self>) {
        self.select(*story, window, cx);
    }
}

/// One option of a strip group.
fn choice(id: String, label: impl Into<SharedString>, selected: bool, cx: &App) -> Button {
    ui::segment(Button::new(SharedString::from(id)), selected, cx)
        .small()
        .label(label.into())
}

/// A labelled segmented control on a track.
fn group<const N: usize>(label: &str, options: [Button; N], cx: &App) -> Div {
    let p = palette(cx);
    h_flex()
        .gap(dp(8.))
        .child(ui::toolbar_label(label.to_owned(), cx))
        .child(
            h_flex()
                .gap(dp(2.))
                .p(dp(3.))
                .rounded(px(8.))
                .bg(p.surface_2)
                .children(options),
        )
}

impl Render for Workbench {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        h_flex()
            .id("workbench")
            .size_full()
            .bg(cx.theme().background)
            .text_color(palette(cx).ink)
            .child(self.render_list(window, cx))
            .child(
                v_flex()
                    .flex_1()
                    .min_w_0()
                    .h_full()
                    .child(self.render_strip(window, cx))
                    .child(div().flex_1().min_h_0().child(self.view.clone())),
            )
    }
}

#[cfg(test)]
mod tests;
