//! The pane's frame: header, notices, tabs, and what replaces a tab while
//! there is no document.

use gpui_kit::assets::IconName;
use gpui_kit::component::{
    Sizable,
    button::{Button, ButtonVariants},
    h_flex,
    tooltip::Tooltip,
    v_flex,
};
use gpui_kit::prelude::*;
use gpui_kit::*;

use super::{
    CONTEXT, CopyLines, DetailEvent, DetailPane, Dismiss, FindInYaml, FindNextMatch,
    FindPreviousMatch, NextTab, PreviousTab, SelectAllLines, TABS_CONTEXT, Tab,
};
use crate::palette::palette;
use crate::resources::detail::{Detail, DocumentRead, EventsRead};
use crate::screens::panel;
use crate::ui::{self, MONO_FONT, Tone, dp};
use freshkube_ui::inspector::{self, Inspector};

impl DetailPane {
    fn header(&self, detail: &Detail, cx: &mut Context<Self>) -> Div {
        let state = match &detail.read {
            DocumentRead::Loading => Some((Tone::Unknown, "Reading")),
            DocumentRead::Loaded => None,
            DocumentRead::Refused(_) => Some((Tone::Crit, "Not permitted")),
            DocumentRead::Failed(_) => Some((Tone::Crit, "Failed")),
            DocumentRead::Stale(_) => Some((Tone::Warn, "Stale")),
            DocumentRead::Deleted => Some((Tone::Crit, "Deleted")),
        };
        h_flex()
            .flex_1()
            .min_w_0()
            .items_start()
            .gap_2()
            .child(
                v_flex()
                    .flex_1()
                    .min_w_0()
                    .gap_0p5()
                    .child(ui::caption(&detail.target.kind.kind, cx))
                    .child(
                        div()
                            .id("detail-title")
                            .test_support()
                            .aria_label(self.title.clone())
                            .font_family(MONO_FONT)
                            .text_size(dp(13.5))
                            .truncate()
                            .child(self.title.clone()),
                    ),
            )
            .children(state.map(|(tone, text)| {
                div()
                    .id("detail-state")
                    .test_support()
                    .role(Role::Status)
                    .aria_label(text)
                    .mt(dp(14.))
                    .child(ui::tag(tone, None, text, cx))
            }))
            .when(detail.view.is_some(), |this| {
                this.child(
                    div().flex_none().mt(dp(10.)).child(
                        Button::new("detail-copy-yaml")
                            .outline()
                            .xsmall()
                            .icon(IconName::Copy)
                            .label("Copy YAML")
                            .tooltip("Copies the whole document as read; Secret values stay hidden")
                            .on_click(cx.listener(|pane, _, _, cx| pane.copy_document(cx))),
                    ),
                )
            })
            .child(
                div().flex_none().mt(dp(10.)).child(
                    Button::new("detail-close")
                        .ghost()
                        .xsmall()
                        .icon(IconName::X)
                        .tooltip("Close (Escape from the list)")
                        .accessibility_label("Close details")
                        .on_click(cx.listener(|_, _, _, cx| cx.emit(DetailEvent::Closed))),
                ),
            )
    }

    /// Deleted and stale documents say so above the tabs.
    fn notice(&self, detail: &Detail, cx: &mut Context<Self>) -> Option<AnyElement> {
        let (id, banner) = match (&detail.read, &detail.recreated) {
            (DocumentRead::Deleted, Some(successor)) => {
                let successor = successor.clone();
                (
                    "detail-deleted",
                    ui::warning_banner(
                        Some("Deleted and created again.".into()),
                        "An object with this name exists again, with a new UID. This pane still shows the one that was deleted.",
                        Some(
                            Button::new("detail-open-recreated")
                                .outline()
                                .small()
                                .label("Open the new one")
                                .on_click(cx.listener(move |_, _, _, cx| {
                                    cx.emit(DetailEvent::Open(successor.clone()))
                                }))
                                .into_any_element(),
                        ),
                        cx,
                    ),
                )
            }
            (DocumentRead::Deleted, None) if detail.view.is_some() => (
                "detail-deleted",
                ui::warning_banner(
                    Some("This object was deleted.".into()),
                    "Showing it as last read.",
                    None,
                    cx,
                ),
            ),
            (DocumentRead::Stale(reason), _) => (
                "detail-stale",
                ui::warning_banner(
                    Some("Couldn't read it again.".into()),
                    format!("Showing it as last read. {reason}"),
                    Some(
                        Button::new("detail-stale-retry")
                            .outline()
                            .small()
                            .icon(IconName::RefreshCw)
                            .label("Retry")
                            .on_click(cx.listener(|pane, _, _, cx| pane.refresh(cx)))
                            .into_any_element(),
                    ),
                    cx,
                ),
            ),
            _ => return None,
        };
        Some(
            div()
                .id(id)
                .test_support()
                .role(Role::Status)
                .child(banner)
                .into_any_element(),
        )
    }

    fn tabs(&self, detail: &Detail, cx: &mut Context<Self>) -> Div {
        let events = &detail.events;
        let tab = |id: &'static str,
                   tab: Tab,
                   label: SharedString,
                   extra: Option<AnyElement>,
                   tip: Option<SharedString>| {
            inspector::tab(id, label, self.tab == tab, cx)
                .track_focus(&self.tab_focus[tab.index()])
                .children(extra)
                .tooltip(move |window, cx| {
                    let m = ui::modifier();
                    let keys = format!("{m}⇧[ and {m}⇧] switch tabs; ← and → move between them");
                    Tooltip::new(match &tip {
                        Some(tip) => format!("{tip}\n{keys}"),
                        None => keys,
                    })
                    .build(window, cx)
                })
                .on_click(cx.listener(move |pane, _, _, cx| pane.set_tab(tab, cx)))
                .into_any_element()
        };
        let count = match events.read() {
            EventsRead::Loaded | EventsRead::Stale(_) => Some(events.len()),
            _ => None,
        };
        let warnings = events.warnings();
        h_flex()
            .key_context(TABS_CONTEXT)
            .on_action(cx.listener(|pane, _: &NextTab, window, cx| pane.move_tab(1, window, cx)))
            .on_action(
                cx.listener(|pane, _: &PreviousTab, window, cx| pane.move_tab(-1, window, cx)),
            )
            .gap_1()
            .child(tab(
                "detail-tab-overview",
                Tab::Overview,
                "Overview".into(),
                None,
                None,
            ))
            .child(tab("detail-tab-yaml", Tab::Yaml, "YAML".into(), None, None))
            .child(tab(
                "detail-tab-events",
                Tab::Events,
                match count {
                    Some(count) => format!("Events {count}").into(),
                    None => "Events".into(),
                },
                (warnings > 0).then(|| {
                    ui::tag(
                        Tone::Warn,
                        None,
                        match warnings {
                            1 => "1 warning".to_owned(),
                            count => format!("{count} warnings"),
                        },
                        cx,
                    )
                    .into_any_element()
                }),
                None,
            ))
            .when(detail.target.kind.is_pod(), |this| {
                let shell = self.shell.read(cx);
                let running = shell.running().then(|| {
                    ui::status_mark("detail-shell-running", Tone::Good, "A shell runs", cx)
                });
                let title = shell.title().cloned();
                this.child(tab("detail-tab-logs", Tab::Logs, "Logs".into(), None, None))
                    .child(tab(
                        "detail-tab-shell",
                        Tab::Shell,
                        "Shell".into(),
                        running,
                        title,
                    ))
            })
            .when(Tab::of(&detail.target.kind).contains(&Tab::Ports), |this| {
                this.child(tab(
                    "detail-tab-ports",
                    Tab::Ports,
                    "Ports".into(),
                    None,
                    None,
                ))
            })
    }

    /// What replaces the overview and YAML while there is no document.
    fn document_state(&self, detail: &Detail, cx: &mut Context<Self>) -> AnyElement {
        let kind = detail.target.kind.kind.to_lowercase();
        let state = |id: &'static str, element: Div| {
            element
                .id(id)
                .test_support()
                .role(Role::Status)
                .into_any_element()
        };
        match &detail.read {
            DocumentRead::Refused(reason) => state(
                "detail-refused",
                ui::empty_state(
                    IconName::ShieldX,
                    format!("Not permitted to read this {kind}"),
                    "The identity may list these objects but not read this one in full. Its events may still be readable.",
                    Some(reason.clone()),
                    Vec::new(),
                    cx,
                ),
            ),
            DocumentRead::Failed(reason) => state(
                "detail-failed",
                ui::empty_state(
                    IconName::CircleDashed,
                    format!("Couldn't read this {kind}"),
                    "Nothing was read, so nothing is shown.",
                    Some(reason.clone()),
                    vec![
                        Button::new("detail-retry")
                            .primary()
                            .icon(IconName::RefreshCw)
                            .label("Retry")
                            .on_click(cx.listener(|pane, _, _, cx| pane.refresh(cx)))
                            .into_any_element(),
                    ],
                    cx,
                ),
            ),
            DocumentRead::Deleted => state(
                "detail-gone",
                ui::empty_state(
                    IconName::Trash,
                    format!("This {kind} was deleted"),
                    "It was deleted before it could be read.",
                    None,
                    Vec::new(),
                    cx,
                ),
            ),
            DocumentRead::Loading | DocumentRead::Loaded | DocumentRead::Stale(_) => state(
                "detail-loading",
                v_flex()
                    .p_4()
                    .gap_3()
                    .children((0..8).map(|_| ui::skeleton(relative(0.7), dp(12.)))),
            ),
        }
    }
}

impl Render for DetailPane {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        crate::desktop::probe::hit("resource-detail");
        let Some(detail) = self.detail.as_ref() else {
            return div().into_any_element();
        };
        let body = match (self.tab, &detail.view, &self.summary) {
            (Tab::Overview, Some(_), Some(summary)) => self.overview(detail, summary, cx),
            (Tab::Yaml, Some(view), _) => self.yaml(view, cx),
            (Tab::Events, ..) => self.events(detail, cx),
            // The log view draws to its edges; the card used to frame it.
            (Tab::Logs, ..) => v_flex()
                .size_full()
                .min_h_0()
                .px(dp(freshkube_ui::page::PANE_PADDING))
                .pb(dp(freshkube_ui::page::PANE_PADDING))
                .child(
                    v_flex()
                        .id("detail-logs")
                        .test_support()
                        .flex_1()
                        .min_h_0()
                        .w_full()
                        .child(self.logs.clone()),
                )
                .into_any_element(),
            (Tab::Shell, ..) => self.shell.clone().into_any_element(),
            (Tab::Ports, ..) => self.ports.clone().into_any_element(),
            _ => self.document_state(detail, cx),
        };
        let notice = self.notice(detail, cx);
        let feedback = self.feedback.clone().map(|feedback| {
            div()
                .id("detail-feedback")
                .test_support()
                .role(Role::Status)
                .aria_label(feedback.clone())
                .text_size(dp(12.))
                .text_color(palette(cx).muted)
                .child(feedback)
        });
        let frame = if self.embedded_node {
            // The node pane keeps its card, without the heading and tabs.
            let line = palette(cx).line;
            panel(cx)
                .children(notice.map(|notice| div().px_4().pb_2().child(notice)))
                .child(div().flex_1().min_h_0().child(body))
                .children(
                    feedback
                        .map(|feedback| feedback.px_4().py_1p5().border_t_1().border_color(line)),
                )
        } else {
            div().flex().flex_col().child(
                Inspector::new("detail-inspector")
                    .heading(self.header(detail, cx))
                    .banner(notice)
                    .tabs(self.tabs(detail, cx))
                    .content(body)
                    .footer(feedback)
                    .render(cx),
            )
        };
        frame
            .id("resource-detail")
            .test_support()
            .key_context(if self.embedded_node {
                "NodeDocument"
            } else {
                CONTEXT
            })
            .track_focus(&self.focus)
            .on_action(cx.listener(|pane, _: &FindInYaml, window, cx| pane.focus_find(window, cx)))
            .on_action(cx.listener(|pane, _: &SelectAllLines, _, cx| pane.select_all(cx)))
            .on_action(cx.listener(|pane, _: &CopyLines, _, cx| pane.copy_lines(cx)))
            .on_action(cx.listener(|pane, _: &Dismiss, window, cx| pane.dismiss(window, cx)))
            .on_action(cx.listener(|pane, _: &FindNextMatch, _, cx| pane.find_match(true, cx)))
            .on_action(cx.listener(|pane, _: &FindPreviousMatch, _, cx| pane.find_match(false, cx)))
            .on_action(cx.listener(|pane, _: &NextTab, window, cx| pane.switch_tab(1, window, cx)))
            .on_action(
                cx.listener(|pane, _: &PreviousTab, window, cx| pane.switch_tab(-1, window, cx)),
            )
            .size_full()
            .overflow_hidden()
            .into_any_element()
    }
}
