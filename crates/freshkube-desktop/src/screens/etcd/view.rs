//! The page: its header, the quorum and alarm banners, the members table and
//! the selected member's details beside or below it.
use super::*;
use crate::palette::palette;
use crate::screens::{Scope, content_width, failure_banner, field, gate, mono, partial_notice};
use crate::ui::{MONO_FONT, dp};
use freshkube_ui::page::{self, PageHeader};
use freshkube_ui::table::{self, DataTable};
use gpui_kit::component::{
    Disableable, Sizable,
    button::{Button, ButtonVariants},
    h_flex,
    tooltip::Tooltip,
    v_flex,
};

impl Render for EtcdScreen {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.derive_if_changed();
        let header = self.render_header(window, cx);
        let state = gate(
            self.source.as_ref(),
            &self.loader,
            Scope::Cluster,
            "the etcd status",
            cx,
        );
        // The table runs edge to edge under the toolbar; the banners and a
        // state in the table's place sit in an inset between them.
        let page = page::page("etcd-page")
            .h_auto()
            .flex_none()
            .child(page::toolbar(cx).child(header));
        let page = match state {
            Some(state) => page.child(page::inset().child(state)),
            None => {
                let banners: Vec<AnyElement> = failure_banner(&self.loader, cx)
                    .map(IntoElement::into_any_element)
                    .into_iter()
                    .chain(partial_notice(self.derived.missing.clone(), cx))
                    .chain(self.render_quorum_banner(cx))
                    .chain(self.render_alarm_banner(cx))
                    .collect();
                page.when(!banners.is_empty(), |page| {
                    page.child(
                        page::inset()
                            .flex()
                            .flex_col()
                            .gap(dp(page::PANE_PADDING_Y))
                            .children(banners),
                    )
                })
                .child(self.render_members(window, cx))
            }
        };
        // The keys live on a wrapper drawn in every state, so they work
        // while the table is replaced by a state.
        div()
            .id("etcd-scroll")
            .key_context(CONTEXT)
            .track_focus(&self.focus)
            .size_full()
            .min_h_0()
            .overflow_y_scroll()
            .on_action(cx.listener(|view, _: &NextMember, _, cx| view.step(1, cx)))
            .on_action(cx.listener(|view, _: &PreviousMember, _, cx| view.step(-1, cx)))
            .on_action(cx.listener(|view, _: &FirstMember, _, cx| view.step(isize::MIN, cx)))
            .on_action(cx.listener(|view, _: &LastMember, _, cx| view.step(isize::MAX, cx)))
            .on_action(cx.listener(|view, _: &ClearSelection, _, cx| {
                view.selected = None;
                cx.notify();
            }))
            .child(page)
    }
}

impl EtcdScreen {
    fn render_header(&self, window: &Window, cx: &mut Context<Self>) -> Div {
        let narrow = content_width(window) < page::HEADER_NARROW;
        let header = PageHeader::new(PREFIX, "etcd", narrow);
        let chips = (!self.derived.rows.is_empty()).then(|| {
            table::status_chips(
                header.id("tally"),
                MemberState::ALL.into_iter().map(|state| {
                    table::status_chip(
                        header.id(&format!("tally-{}", state.what().replace(' ', "-"))),
                        state.tone(),
                        self.derived.counts[state.index()],
                        state.what(),
                        self.state == Some(state),
                        cx,
                    )
                    .on_click(cx.listener(move |view, _, _, cx| view.toggle_state(state, cx)))
                }),
                cx,
            )
        });
        let loading = self.loader.is_loading();
        let refresh = Button::new(header.id("refresh"))
            .ghost()
            .small()
            .size(dp(ui::CONTROL_HEIGHT))
            .icon(IconName::RefreshCw)
            .accessibility_label("Refresh etcd")
            .tooltip("Refresh etcd")
            .loading(loading)
            .disabled(loading || self.source.is_none())
            .on_click(cx.listener(|view, _, window, cx| view.manual_refresh(window, cx)));
        header
            .chips(chips)
            .control(refresh)
            .meta(self.render_meta(cx))
            .render(cx)
    }

    /// Context and members, the quorum, then leader, alarms and time.
    fn render_meta(&self, cx: &App) -> Vec<AnyElement> {
        let derived = &self.derived;
        let quorum = derived.quorum.as_ref().map(|quorum| {
            let detail = SharedString::from(quorum.detail.clone());
            h_flex()
                .id("etcd-quorum")
                .test_support()
                .role(Role::Status)
                .aria_label(format!("{} · {}", quorum.label, quorum.detail))
                .gap(dp(4.))
                .children(ui::status_glyph(quorum.tone, cx))
                .child(quorum.label)
                .tooltip(move |window, cx| Tooltip::new(detail.clone()).build(window, cx))
                .into_any_element()
        });
        let parts = derived
            .meta_before
            .iter()
            .map(|part| part.clone().into_any_element())
            .chain(quorum)
            .chain(
                derived
                    .meta_after
                    .iter()
                    .map(|part| part.clone().into_any_element()),
            );
        let mut meta = Vec::new();
        for part in parts {
            if !meta.is_empty() {
                meta.push(" · ".into_any_element());
            }
            meta.push(part);
        }
        meta
    }

    fn render_quorum_banner(&self, cx: &App) -> Option<AnyElement> {
        let banner = self.derived.banner.as_ref()?;
        Some(
            div()
                .id("etcd-quorum-banner")
                .test_support()
                .role(Role::Alert)
                .aria_label(format!("{} · {}", banner.lead, banner.body))
                .child(ui::banner(
                    banner.tone,
                    Some(banner.lead.into()),
                    SharedString::from(banner.body.clone()),
                    None,
                    cx,
                ))
                .into_any_element(),
        )
    }

    /// The first alarms, and how many more the members' details hold.
    fn render_alarm_banner(&self, cx: &App) -> Option<AnyElement> {
        let alarms = &self.derived.alarms;
        if alarms.is_empty() {
            return None;
        }
        let more = alarms.len().saturating_sub(BANNER_ALARMS);
        let body = v_flex()
            .gap(dp(2.))
            .children(
                alarms
                    .iter()
                    .take(BANNER_ALARMS)
                    .enumerate()
                    .map(|(ix, alarm)| {
                        div()
                            .id(("etcd-alarm", ix))
                            .test_support()
                            .aria_label(alarm.label.clone())
                            .font_family(MONO_FONT)
                            .text_size(dp(12.))
                            .truncate()
                            .child(alarm.text.clone())
                    }),
            )
            .when(more > 0, |this| {
                this.child(
                    div()
                        .id("etcd-alarm-more")
                        .test_support()
                        .text_color(palette(cx).muted)
                        .child(format!(
                            "and {more} more; each member's details list its own"
                        )),
                )
            });
        Some(
            div()
                .id("etcd-alarms")
                .test_support()
                .aria_label("etcd alarms")
                .child(ui::banner(
                    self.derived.alarm_tone,
                    Some(plural(alarms.len(), "etcd alarm", "etcd alarms").into()),
                    body,
                    None,
                    cx,
                ))
                .into_any_element(),
        )
    }

    /// The table, with the selected member's details beside it on a wide
    /// page and below it on a narrow one.
    fn render_members(&self, window: &Window, cx: &mut Context<Self>) -> AnyElement {
        let beside = crate::screens::beside(window);
        let table = DataTable::new()
            .fit(self.derived.lines.len().max(1))
            .render(self, window, cx)
            .w_full()
            .flex_none()
            .into_any_element();
        // The details open with a selection, as Pods' pane does, so at rest
        // the table has the page's width.
        // The card keeps the inset on its outer edges until change 9.
        let details = self.selected.is_some().then(|| {
            div()
                .when_else(
                    beside,
                    |this| this.pr(dp(page::PANE_PADDING)).py(dp(page::PANE_PADDING_Y)),
                    |this| this.px(dp(page::PANE_PADDING)).pb(dp(page::PANE_PADDING_Y)),
                )
                .child(self.render_details(cx))
                .into_any_element()
        });
        crate::screens::split("etcd-split", beside, table, details)
    }

    /// The detail pane's heading: kind caption, monospace title and role.
    fn details_heading(&self, title: SharedString, role: Option<MemberRole>, cx: &App) -> Div {
        h_flex()
            .items_start()
            .gap(dp(8.))
            .px(dp(14.))
            .pt(dp(12.))
            .pb(dp(8.))
            .child(
                v_flex()
                    .flex_1()
                    .min_w_0()
                    .gap(dp(2.))
                    .child(ui::caption("etcd member", cx))
                    .child(
                        div()
                            .id("etcd-detail-title")
                            .test_support()
                            .font_family(MONO_FONT)
                            .text_size(dp(13.5))
                            .truncate()
                            .child(title),
                    ),
            )
            .children(role.map(|role| div().mt(dp(14.)).child(role_tag(role, cx))))
    }

    fn render_details(&self, cx: &mut Context<Self>) -> AnyElement {
        let p = palette(cx);
        let card = page::card(cx).id("etcd-details").test_support();
        let Some(member) = self.selected_member() else {
            return card
                .child(self.details_heading("".into(), None, cx))
                .child(
                    div()
                        .px(dp(14.))
                        .pb(dp(14.))
                        .text_color(p.muted)
                        .text_size(dp(12.5))
                        .child("The selected member is no longer in the roster."),
                )
                .into_any_element();
        };
        let info = &member.info;
        let role = MemberRole::of(member);
        let card = card
            .aria_label(format!("{} · {}", info.hostname, role.label()))
            .child(self.details_heading(info.hostname.clone().into(), Some(role), cx));
        let mut body = v_flex()
            .px(dp(14.))
            .pb(dp(14.))
            .gap_2p5()
            .min_w_0()
            .children(self.details_actions(member, cx));
        body = self.details_fields(body, member, cx);
        card.child(body).into_any_element()
    }

    /// etcd logs and Target this node, for a member the roster maps to a
    /// node.
    fn details_actions(&self, member: &EtcdMemberSnapshot, cx: &mut Context<Self>) -> Option<Div> {
        let hostname = member.info.hostname.clone();
        // Targeting only makes sense for members the roster can map to a node.
        let node_known = self
            .source
            .as_ref()
            .is_some_and(|source| source.nodes.iter().any(|node| node.name == hostname));
        if !node_known {
            return None;
        }
        let is_target = self
            .source
            .as_ref()
            .is_some_and(|source| source.target.node == hostname);
        let logs_node = hostname.clone();
        Some(
            h_flex()
                .gap(dp(8.))
                .flex_wrap()
                .child(
                    Button::new("etcd-logs")
                        .outline()
                        .xsmall()
                        .icon(IconName::ScrollText)
                        .label("etcd logs")
                        .tooltip("Target this member's node and show its etcd logs")
                        .on_click(cx.listener(move |_, _, _, cx| {
                            cx.emit(ScreenEvent::OpenLogsOn {
                                node: logs_node.clone(),
                                service: "etcd".into(),
                            })
                        })),
                )
                .when(!is_target, |this| {
                    this.child(
                        Button::new("etcd-select-node")
                            .outline()
                            .xsmall()
                            .icon(IconName::Crosshair)
                            .label("Target this node")
                            .on_click(cx.listener(move |_, _, _, cx| {
                                cx.emit(ScreenEvent::SelectNode(hostname.clone()))
                            })),
                    )
                }),
        )
    }

    fn details_fields(&self, body: Div, member: &EtcdMemberSnapshot, cx: &App) -> Div {
        let p = palette(cx);
        let info = &member.info;
        let urls = |urls: &[String]| {
            if urls.is_empty() {
                mono("not reported").into_any_element()
            } else {
                v_flex()
                    .children(urls.iter().map(|url| mono(url.clone())))
                    .into_any_element()
            }
        };
        let body = body
            .child(field("Member ID", mono(format!("{:x}", info.id)), cx))
            .child(field("Peer URLs", urls(&info.peer_urls), cx))
            .child(field("Client URLs", urls(&info.client_urls), cx))
            .child(field(
                "Learner",
                mono(if info.is_learner { "Yes" } else { "No" }),
                cx,
            ));
        let Some(status) = &member.status else {
            return body
                .child(field(
                    "Status",
                    div()
                        .text_color(p.unk_ink)
                        .child("Not reported. This member didn't answer the status request, which doesn't mean it is down."),
                    cx,
                ))
                .child(alarm_field(&member.alarms, cx));
        };
        let in_use_percent = if status.db_size > 0 {
            status.db_size_in_use as f64 / status.db_size as f64 * 100.
        } else {
            0.
        };
        let lag = status.raft_index.saturating_sub(status.raft_applied_index);
        body.child(field("Protocol", mono(status.protocol_version.clone()), cx))
            .child(field(
                "Leader",
                mono(if status.leader_id == 0 {
                    "none reported".to_owned()
                } else {
                    format!(
                        "{} ({:x})",
                        self.name_of(status.leader_id),
                        status.leader_id
                    )
                }),
                cx,
            ))
            .child(field("Raft term", mono(status.raft_term.to_string()), cx))
            .child(field(
                "Raft index",
                mono(format!(
                    "{} · applied {}{}",
                    status.raft_index,
                    status.raft_applied_index,
                    if lag > 0 {
                        format!(" ({lag} behind)")
                    } else {
                        String::new()
                    }
                )),
                cx,
            ))
            .child(field(
                "DB size",
                mono(format_bytes_signed(status.db_size)),
                cx,
            ))
            .child(field(
                "DB in use",
                mono(format!(
                    "{} ({in_use_percent:.0}%)",
                    format_bytes_signed(status.db_size_in_use)
                )),
                cx,
            ))
            .child(field(
                "Errors",
                if status.errors.is_empty() {
                    mono("None").into_any_element()
                } else {
                    v_flex()
                        .text_color(p.crit_ink)
                        .children(status.errors.iter().map(|error| mono(error.clone())))
                        .into_any_element()
                },
                cx,
            ))
            .child(alarm_field(&member.alarms, cx))
    }
}

fn alarm_field(alarms: &[EtcdAlarm], cx: &App) -> Div {
    field(
        "Alarms",
        if alarms.is_empty() {
            mono("None").into_any_element()
        } else {
            v_flex()
                .text_color(palette(cx).warn_ink)
                .children(alarms.iter().map(|alarm| mono(alarm_text(alarm))))
                .into_any_element()
        },
        cx,
    )
}
