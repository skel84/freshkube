//! The pane's frame: header, notices, tabs, and what replaces a tab while
//! there is no document.

use gpui_kit::assets::IconName;
use gpui_kit::base::ObservedElement as Observed;
use gpui_kit::component::{
    Disableable, Selectable, Sizable,
    button::{Button, ButtonVariants},
    h_flex,
    menu::{DropdownMenu, PopupMenuItem},
    tooltip::Tooltip,
    v_flex,
};
use gpui_kit::prelude::*;
use gpui_kit::*;

use super::{
    CONTEXT, CopyLines, DetailEvent, DetailPane, Dismiss, FindInYaml, FindNextMatch,
    FindPreviousMatch, NextTab, PreviousTab, Section, SelectAllLines, TABS_CONTEXT, Tab,
};
use crate::logs::role_heading;
use crate::palette::palette;
use crate::resources::detail::{Detail, DocumentRead, EventsRead};
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
                        h_flex()
                            .min_w_0()
                            .gap_1()
                            .child(
                                div()
                                    .id("detail-title")
                                    .test_support()
                                    .aria_label(self.title.clone())
                                    .min_w_0()
                                    .font_family(MONO_FONT)
                                    .text_size(dp(13.5))
                                    .truncate()
                                    .child(self.title.clone()),
                            )
                            .child(
                                Button::new("detail-copy-name")
                                    .ghost()
                                    .xsmall()
                                    .flex_none()
                                    .icon(IconName::Copy)
                                    .tooltip("Copy the name")
                                    .on_click(cx.listener(|pane, _, _, cx| pane.copy_name(cx))),
                            ),
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
            .when(self.has_logs(), |this| {
                this.child(
                    div().flex_none().mt(dp(10.)).child(
                        Button::new("detail-open-logs")
                            .outline()
                            .xsmall()
                            .icon(IconName::ScrollText)
                            .label("Logs")
                            .tooltip(self.cross_links.logs_tip.clone().unwrap_or_else(|| {
                                "Opens the logs in the dock (L from the list)".into()
                            }))
                            .on_click(cx.listener(|pane, _, _, cx| pane.open_logs(cx))),
                    ),
                )
            })
            .when(detail.target.kind.is_pod(), |this| {
                this.child(
                    div()
                        .flex_none()
                        .mt(dp(10.))
                        .child(self.render_shell_menu(cx)),
                )
            })
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

    /// The Shell menu: "Start shell in ⟨container⟩" for each container,
    /// enabled for running ones only. Picking one starts it in a dock tab; opening the
    /// menu runs nothing.
    fn render_shell_menu(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let pane = cx.entity().downgrade();
        let choices = self.shell_choices.clone();
        Button::new("detail-open-shell")
            .outline()
            .xsmall()
            .icon(IconName::SquareTerminal)
            .label("Shell")
            .dropdown_caret(true)
            .tooltip("Starts a shell in a container, in the dock. Whatever you type there runs in the pod.")
            .disabled(choices.is_empty())
            .dropdown_menu(move |mut menu, _, _| {
                // Headings only when there's more than one kind to tell apart.
                let mixed = choices.iter().any(|choice| choice.role != choices[0].role);
                let mut role = None;
                for choice in choices.iter() {
                    if mixed && role != Some(choice.role) {
                        if role.is_some() {
                            menu = menu.separator();
                        }
                        menu = menu.label(role_heading(choice.role));
                        role = Some(choice.role);
                    }
                    let (pane, name) = (pane.clone(), choice.name.clone());
                    menu = menu.item(
                        PopupMenuItem::new(choice.label.clone())
                            .disabled(!choice.enabled)
                            .on_click(move |_, _, cx| {
                                let name = name.clone();
                                _ = pane.update(cx, |pane, cx| pane.request_shell(name, cx));
                            }),
                    );
                }
                menu
            })
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

    fn tabs(&self, cx: &mut Context<Self>) -> Observed<Stateful<Div>> {
        let tab = |id: &'static str, tab: Tab, label: SharedString, tip: Option<SharedString>| {
            inspector::tab(id, label, self.tab == tab, cx)
                .track_focus(&self.tab_focus[tab.index()])
                .tooltip(move |window, cx| {
                    let m = ui::modifier();
                    let keys = format!("{m}⇧[ and {m}⇧] switch tabs; ← and → move between them");
                    Tooltip::new(match &tip {
                        Some(tip) => format!("{tip}\n{keys}"),
                        None => keys,
                    })
                    .build(window, cx)
                })
                .on_click(cx.listener(move |pane, _, _, cx| pane.show_page(tab, cx)))
                .into_any_element()
        };
        self.tab_strip
            .row("detail-tabs", self.tab.index())
            .key_context(TABS_CONTEXT)
            .on_action(cx.listener(|pane, _: &NextTab, window, cx| pane.move_tab(1, window, cx)))
            .on_action(
                cx.listener(|pane, _: &PreviousTab, window, cx| pane.move_tab(-1, window, cx)),
            )
            .gap_1()
            .child(tab(
                "detail-tab-details",
                Tab::Overview,
                "Details".into(),
                Some("The overview, ports and events, on one page".into()),
            ))
            .child(tab("detail-tab-yaml", Tab::Yaml, "YAML".into(), None))
    }

    /// Details: the overview, then the ports, then the events, on one
    /// scrolling page under an index that stays at its top.
    fn details(&self, detail: &Detail, cx: &mut Context<Self>) -> AnyElement {
        let sections = Section::of(&detail.target.kind);
        let line = palette(cx).line;
        let pad = dp(freshkube_ui::page::PANE_PADDING);
        let body = |section: Section, this: &Self, cx: &mut Context<Self>| match section {
            Section::Overview => match (&detail.view, &this.summary) {
                (Some(_), Some(summary)) => this.overview(detail, summary, cx),
                _ => this.document_state(detail, cx),
            },
            Section::Ports => this.ports.clone().into_any_element(),
            Section::Events => this.events(detail, cx),
        };
        let parts: Vec<AnyElement> = sections
            .iter()
            .enumerate()
            .map(|(ix, section)| {
                v_flex()
                    .id(SharedString::from(format!(
                        "detail-section-{}",
                        section.slug()
                    )))
                    .test_support()
                    .when(ix > 0, |this| {
                        this.mt(dp(18.))
                            .pt(dp(14.))
                            .border_t_1()
                            .border_color(line)
                            .child(self.section_heading(*section, detail, cx))
                    })
                    .child(body(*section, self, cx))
                    .into_any_element()
            })
            .collect();
        v_flex()
            .size_full()
            .child(self.section_index(sections, detail, cx))
            .child(
                v_flex()
                    .id("detail-details")
                    .test_support()
                    .track_scroll(&self.details_scroll)
                    .on_scroll_wheel({
                        let pinned = self.section_pinned.clone();
                        move |_, _, _| pinned.set(false)
                    })
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .restrict_scroll_to_axis()
                    .px(pad)
                    .py_3()
                    .children(parts),
            )
            .child(self.watch_sections(sections, cx))
            .into_any_element()
    }

    /// Learns which section is at the top of Details as it draws, and draws
    /// the index again when that changed.
    fn watch_sections(
        &self,
        sections: &'static [Section],
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let scroll = self.details_scroll.clone();
        let shown = self.shown_section.clone();
        let pinned = self.section_pinned.clone();
        let this = cx.entity().downgrade();
        canvas(
            move |_, window, _| {
                if pinned.get() {
                    return;
                }
                let at_end = scroll.max_offset().y > px(0.)
                    && -scroll.offset().y >= scroll.max_offset().y - px(1.);
                let ix = if at_end {
                    sections.len() - 1
                } else {
                    scroll.top_item()
                };
                let Some(section) = sections.get(ix).copied() else {
                    return;
                };
                if shown.get() == section {
                    return;
                }
                shown.set(section);
                let this = this.clone();
                window.on_next_frame(move |_, cx| {
                    _ = this.update(cx, |_, cx| cx.notify());
                });
            },
            |_, _, _, _| {},
        )
        .absolute()
        .size_0()
    }

    /// The index over Details: a button per section, the one at the top
    /// marked.
    fn section_index(
        &self,
        sections: &'static [Section],
        detail: &Detail,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let shown = self.shown_section.get();
        let warnings = detail.events.warnings();
        h_flex()
            .id("detail-index")
            .test_support()
            .flex_none()
            .flex_wrap()
            .gap_1()
            .px(dp(freshkube_ui::page::PANE_PADDING - 6.))
            .py_1()
            .border_b_1()
            .border_color(palette(cx).line)
            .children(sections.iter().map(|section| {
                let section = *section;
                let label = self.section_label(section, detail);
                // The wrapper tells tests and assistive tools which section
                // is marked; a Kit button can't.
                div()
                    .id(SharedString::from(format!(
                        "detail-jump-{}",
                        section.slug()
                    )))
                    .test_support()
                    .role(Role::Tab)
                    .aria_selected(section == shown)
                    .aria_label(label.clone())
                    .child(
                        Button::new(SharedString::from(format!(
                            "detail-jump-{}-button",
                            section.slug()
                        )))
                        .ghost()
                        .xsmall()
                        .selected(section == shown)
                        .label(label)
                        .when(section == Section::Events && warnings > 0, |this| {
                            this.child(ui::tag(Tone::Warn, None, warnings.to_string(), cx))
                        })
                        .on_click(cx.listener(move |pane, _, _, cx| {
                            pane.show_section(section);
                            cx.notify();
                        })),
                    )
            }))
    }

    fn section_label(&self, section: Section, detail: &Detail) -> SharedString {
        match section {
            Section::Overview => "Overview".into(),
            Section::Ports => "Ports".into(),
            Section::Events => match detail.events.read() {
                EventsRead::Loaded | EventsRead::Stale(_) => {
                    format!("Events {}", detail.events.len()).into()
                }
                _ => "Events".into(),
            },
        }
    }

    /// The heading over each section after the overview.
    fn section_heading(
        &self,
        section: Section,
        detail: &Detail,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let warnings = detail.events.warnings();
        h_flex()
            .gap_2()
            .pb_2()
            .child(
                div()
                    .text_size(dp(13.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(self.section_label(section, detail)),
            )
            .when(section == Section::Events && warnings > 0, |this| {
                this.child(ui::tag(
                    Tone::Warn,
                    None,
                    match warnings {
                        1 => "1 warning".to_owned(),
                        count => format!("{count} warnings"),
                    },
                    cx,
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
        let body = match (self.tab, &detail.view) {
            (Tab::Yaml, Some(view)) => self.yaml(view, cx),
            (Tab::Yaml, None) => self.document_state(detail, cx),
            // The node's inspector shows its events alone.
            (Tab::Events, _) if self.embedded_node => div()
                .id("detail-events-page")
                .size_full()
                .overflow_y_scroll()
                .restrict_scroll_to_axis()
                .px(dp(freshkube_ui::page::PANE_PADDING))
                .py_3()
                .child(self.events(detail, cx))
                .into_any_element(),
            _ => self.details(detail, cx),
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
        let frame =
            if self.embedded_node {
                // In the node's inspector, which draws the heading and tabs.
                let line = palette(cx).line;
                let pad = dp(freshkube_ui::page::PANE_PADDING);
                v_flex()
                    .children(notice.map(|notice| div().px(pad).pt(pad).child(notice)))
                    .child(div().flex_1().min_h_0().child(body))
                    .children(feedback.map(|feedback| {
                        feedback.px(pad).py(dp(6.)).border_t_1().border_color(line)
                    }))
            } else {
                div().flex().flex_col().child(
                    Inspector::new("detail-inspector")
                        .heading(self.header(detail, cx))
                        .banner(notice)
                        .tabs(&self.tab_strip, self.tabs(cx))
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
