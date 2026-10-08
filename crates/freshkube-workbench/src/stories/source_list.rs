//! Source list: the column beside the rail as the app draws it
//! (`freshkube_ui::source_list`), on invented kinds, API groups and
//! namespaces. Its rows open with a click or the keyboard; the states
//! switch what an API group shows while it is read.

use super::motion::{option, segments};
use freshkube_ui::page::{self, PageHeader};
use freshkube_ui::palette::palette;
use freshkube_ui::source_list::{self, Line, Menu, Note, Row, Section, SourceList, SourceListHost};
use freshkube_ui::ui::{Tone, dp};
use gpui_kit::assets::IconName;
use gpui_kit::component::{h_flex, v_flex};
use gpui_kit::prelude::*;
use gpui_kit::{AnyView, App, Context, SharedString, TestSupportExt, Window, div};

/// The id prefix of everything the story draws.
const PREFIX: &str = "source-list";

pub fn build(_: &mut Window, cx: &mut App) -> AnyView {
    cx.new(|cx| {
        let mut story = SourceListStory {
            list: SourceList::new(format!("{PREFIX}-list"), cx),
            current: Key::Kind("pods"),
            group_open: true,
            state: State::Loaded,
        };
        story.derive();
        story
    })
    .into()
}

/// What a row of the story opens.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Key {
    Kind(&'static str),
    Group,
    Retry,
    Namespace(Option<&'static str>),
}

/// What the open API group shows.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum State {
    Loaded,
    Reading,
    Failed,
}

pub struct SourceListStory {
    list: SourceList<Key>,
    current: Key,
    group_open: bool,
    state: State,
}

impl SourceListStory {
    /// The row shown.
    pub fn current(&self) -> Key {
        self.current
    }

    pub fn list(&self) -> &SourceList<Key> {
        &self.list
    }

    fn set_state(&mut self, state: State, cx: &mut Context<Self>) {
        self.state = state;
        self.derive();
        cx.notify();
    }

    /// The lines, from the row shown and the group's state.
    fn derive(&mut self) {
        let kind = |key: &'static str, label: &'static str, icon: IconName| {
            Row::new(Key::Kind(key), format!("{PREFIX}-{key}"), label)
                .icon(icon)
                .current(self.current == Key::Kind(key))
        };
        let mut lines: Vec<Line<Key>> = vec![
            Section::new(format!("{PREFIX}-workloads"), "Workloads").into(),
            kind("health", "Health", IconName::HeartPulse)
                .shortcut("⌘5")
                .mark(Tone::Warn, "Workloads: 2 of 9 degraded")
                .into(),
            kind("pods", "Pods", IconName::Box)
                .count("48", None)
                .mark(Tone::Crit, "Pods: 3 failing · 2 crash-looping")
                .into(),
            kind("deployments", "Deployments", IconName::Layers).into(),
            kind(
                "bindings",
                "ValidatingAdmissionPolicyBindings",
                IconName::ShieldCheck,
            )
            .tooltip("admissionregistration.k8s.io/v1")
            .into(),
            Section::new(format!("{PREFIX}-custom"), "Custom resources").into(),
            Row::new(Key::Group, format!("{PREFIX}-group"), "widgets.example.io")
                .disclosure(self.group_open)
                .current(!self.group_open && self.current == Key::Kind("widgets"))
                .tooltip("v1, v1beta1")
                .into(),
        ];
        if self.group_open {
            lines.push(match self.state {
                State::Loaded => kind("widgets", "Widgets", IconName::Box).depth(1).into(),
                State::Reading => Note::new(format!("{PREFIX}-group-status"), "Discovering…")
                    .depth(1)
                    .into(),
                State::Failed => Note::new(format!("{PREFIX}-group-status"), "Timed out")
                    .depth(1)
                    .tooltip("Discovery of widgets.example.io timed out after 5 s")
                    .retry(Key::Retry)
                    .into(),
            });
        }
        lines.push(Section::new(format!("{PREFIX}-namespaces"), "Namespaces").into());
        lines.push(
            Row::new(
                Key::Namespace(None),
                format!("{PREFIX}-all-namespaces"),
                "All namespaces",
            )
            .icon(IconName::Folders)
            .count("112", None)
            .into(),
        );
        for (name, count) in [("shop", "24"), ("billing", "12"), ("observability", "9")] {
            lines.push(
                Row::new(
                    Key::Namespace(Some(name)),
                    format!("{PREFIX}-namespace-{name}"),
                    name,
                )
                .icon(IconName::Folder)
                .mono()
                .current(self.current == Key::Namespace(Some(name)))
                .count(count, None)
                .into(),
            );
        }
        // The namespaces past those listed, as the app's column offers them.
        lines.push(
            Menu::new(
                format!("{PREFIX}-namespaces-more"),
                "2 more namespaces…",
                [
                    ("All namespaces", None),
                    ("payments", Some("payments")),
                    ("search", Some("search")),
                ]
                .map(|(label, key)| (label.into(), Key::Namespace(key)))
                .to_vec(),
            )
            .tooltip("Choose any namespace")
            .into(),
        );
        self.list.set_lines(lines);
    }
}

impl SourceListHost for SourceListStory {
    type Key = Key;

    fn source_list(&mut self) -> &mut SourceList<Key> {
        &mut self.list
    }

    fn open(&mut self, key: &Key, _: &mut Window, cx: &mut Context<Self>) {
        match *key {
            Key::Group => self.group_open = !self.group_open,
            Key::Retry => self.state = State::Loaded,
            Key::Namespace(None) => self.current = Key::Kind("pods"),
            key => self.current = key,
        }
        self.derive();
        cx.notify();
    }
}

impl Render for SourceListStory {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        freshkube_probe::probe::hit("workbench.source-list");
        let state = self.state;
        let choices = segments(
            [
                (State::Loaded, "loaded", "Loaded"),
                (State::Reading, "reading", "Discovering"),
                (State::Failed, "failed", "Failed"),
            ]
            .map(|(choice, id, label)| {
                option(format!("{PREFIX}-{id}"), label, state == choice, cx)
                    .on_click(cx.listener(move |this, _, _, cx| this.set_state(choice, cx)))
            }),
            cx,
        );
        let p = palette(cx);
        let header = PageHeader::new(PREFIX, "Source list")
            .control(choices)
            .meta([div()
                .child("Invented kinds and namespaces")
                .into_any_element()])
            .render(window, cx);
        let shown: SharedString = match self.current {
            Key::Kind(key) => key.into(),
            Key::Namespace(Some(name)) => format!("namespace {name}").into(),
            _ => "".into(),
        };
        let column = v_flex()
            .id(SharedString::from(format!("{PREFIX}-column")))
            .test_support()
            .w(dp(source_list::WIDTH))
            .h(dp(560.))
            .flex_none()
            .border_1()
            .border_color(p.line)
            .rounded(gpui_kit::px(8.))
            .child(source_list::title("Workloads", div(), cx))
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .child(source_list::list(&mut self.list, window, cx)),
            );
        page::page(SharedString::from(format!("{PREFIX}-page")))
            .child(page::toolbar(cx).child(header))
            .child(
                page::inset().child(
                    h_flex()
                        .items_start()
                        .gap(dp(24.))
                        .child(column)
                        .child(
                            v_flex()
                                .gap(dp(6.))
                                .text_color(p.muted)
                                .child(
                                    div()
                                        .id(SharedString::from(format!("{PREFIX}-shown")))
                                        .test_support()
                                        .text_color(p.ink)
                                        .child(format!("Shown: {shown}")),
                                )
                                .child("Click a row, or Tab to the list and use ↑ ↓ Home End and Enter.")
                                .child("A row's mark comes from the overview's cards; its tooltip says why."),
                        ),
                ),
            )
    }
}
