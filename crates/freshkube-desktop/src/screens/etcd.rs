//! etcd: the cluster's members, quorum, leader, database sizes and alarms.
//!
//! The target node's client asks for the member roster, then for each
//! member's status and the alarm list. Members that don't answer are shown as
//! "not reported", never as failed: only an error the API reports is one.
use freshkube_core::format_bytes_signed;
use freshkube_core::indicators::QuorumState;
use freshkube_core::inspection::{
    EtcdHealthSnapshot, EtcdInspectionRequest, EtcdMemberSnapshot, InspectionSource,
    InspectionUnavailable, assemble_etcd_health, collect_etcd_health,
};
use gpui_kit::assets::IconName;
use gpui_kit::component::{
    Sizable,
    button::{Button, ButtonVariants},
    h_flex, v_flex,
};
use gpui_kit::prelude::*;
use gpui_kit::*;
use talos_rs::{EtcdAlarm, EtcdMemberInfo, EtcdMemberStatus};
use tokio::runtime::Handle;

use super::{
    Column, Loader, Scope, ScreenEvent, ScreenPanel, ScreenSource, cell, content_width,
    failure_banner, field, gated_page, header, mono, panel, partial_notice, stat, table_width,
};
use crate::palette::palette;
use crate::presentation::{self, Health};
use crate::ui::{self, MONO_FONT, Tone, dp};

const CONTEXT: &str = "TalosEtcd";
const ROW_HEIGHT: f32 = 32.;
/// Below this content width the details pane moves under the list.
const SIDE_DETAILS: f32 = 960.;
/// Width of the side details pane in wide layouts.
const DETAILS_WIDTH: f32 = 380.;

const FULL_COLUMNS: [Column; 6] = [
    Column {
        label: "Member",
        width: None,
    },
    Column {
        label: "Role",
        width: Some(112.),
    },
    Column {
        label: "Endpoint",
        width: Some(168.),
    },
    Column {
        label: "DB size",
        width: Some(84.),
    },
    Column {
        label: "Raft index",
        width: Some(104.),
    },
    Column {
        label: "Issues",
        width: Some(84.),
    },
];

const COMPACT_COLUMNS: [Column; 4] = [
    FULL_COLUMNS[0],
    FULL_COLUMNS[1],
    FULL_COLUMNS[3],
    FULL_COLUMNS[5],
];

actions!(
    talos_etcd,
    [
        NextMember,
        PreviousMember,
        FirstMember,
        LastMember,
        ClearSelection
    ]
);

pub(crate) struct EtcdScreen {
    runtime: Handle,
    source: Option<ScreenSource>,
    loader: Loader<EtcdHealthSnapshot>,
    /// The selected member's etcd ID; survives refreshes.
    selected: Option<u64>,
    focus: FocusHandle,
}

impl EventEmitter<ScreenEvent> for EtcdScreen {}

impl ScreenPanel for EtcdScreen {
    fn new(runtime: Handle, _: &mut Window, cx: &mut Context<Self>) -> Self {
        cx.bind_keys([
            KeyBinding::new("down", NextMember, Some(CONTEXT)),
            KeyBinding::new("up", PreviousMember, Some(CONTEXT)),
            KeyBinding::new("home", FirstMember, Some(CONTEXT)),
            KeyBinding::new("end", LastMember, Some(CONTEXT)),
            KeyBinding::new("escape", ClearSelection, Some(CONTEXT)),
        ]);
        Self {
            runtime,
            source: None,
            loader: Loader::default(),
            selected: None,
            focus: cx.focus_handle(),
        }
    }

    fn set_source(&mut self, source: Option<ScreenSource>, _: &mut Window, cx: &mut Context<Self>) {
        let changed = self.source.as_ref().map(|source| &source.target)
            != source.as_ref().map(|source| &source.target);
        if changed {
            self.loader.reset();
            self.selected = None;
        }
        self.source = source;
        cx.notify();
    }

    fn activate(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.loader.data().is_none() && !self.loader.is_loading() {
            self.refresh(window, cx);
        }
    }

    fn focus(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        window.focus(&self.focus, cx);
    }

    fn refresh(&mut self, _: &mut Window, cx: &mut Context<Self>) {
        let Some(source) = self.source.clone() else {
            return;
        };
        if self.loader.is_loading() {
            return;
        }
        let Some(live) = source.live.clone() else {
            self.loader.resolve(source.target.clone(), example(&source));
            cx.notify();
            return;
        };
        let request = EtcdInspectionRequest::new(source.inspection_target());
        self.loader.load(
            source.target.clone(),
            &self.runtime,
            "the etcd status",
            async move {
                collect_etcd_health(live.client, request)
                    .await
                    .map_err(|error| error.to_string())
            },
            |screen: &mut Self| &mut screen.loader,
            cx,
        );
        cx.notify();
    }
}

fn endpoint(info: &EtcdMemberInfo) -> String {
    info.client_urls
        .first()
        .map(|url| url.replace("https://", "").replace("http://", ""))
        .unwrap_or_else(|| "not reported".into())
}

/// How a member shows in the list: what we know, never a guess.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum MemberRole {
    Leader,
    Follower,
    Learner,
    NotReported,
}

impl MemberRole {
    fn of(member: &EtcdMemberSnapshot) -> Self {
        match &member.status {
            None => Self::NotReported,
            Some(_) if member.is_leader() => Self::Leader,
            Some(status) if status.is_learner || member.info.is_learner => Self::Learner,
            Some(_) => Self::Follower,
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::Leader => "Leader",
            Self::Follower => "Follower",
            Self::Learner => "Learner",
            Self::NotReported => "Not reported",
        }
    }
}

fn role_tag(role: MemberRole, cx: &App) -> Div {
    match role {
        MemberRole::Leader => ui::tag(Tone::Accent, Some(IconName::Crosshair), "Leader", cx),
        MemberRole::Follower => ui::tag(Tone::Good, None, "Follower", cx),
        MemberRole::Learner => ui::tag(Tone::Outline, None, "Learner", cx),
        MemberRole::NotReported => ui::tag(Tone::Unknown, None, "Not reported", cx),
    }
}

fn member_health(member: &EtcdMemberSnapshot) -> Health {
    if member.has_problems() {
        Health::Unhealthy
    } else if member.is_reachable() {
        Health::Healthy
    } else {
        Health::Unknown
    }
}

fn error_count(member: &EtcdMemberSnapshot) -> Option<usize> {
    member.status.as_ref().map(|status| status.errors.len())
}

/// The quorum verdict, worded so silence is never reported as failure.
struct QuorumView {
    tone: Tone,
    label: &'static str,
    detail: String,
}

fn quorum_view(snapshot: &EtcdHealthSnapshot) -> QuorumView {
    let voting = snapshot.voting_members;
    let answered = snapshot.responding_voting_members;
    let status_missing = snapshot
        .unavailable
        .iter()
        .any(|missing| missing.source == InspectionSource::EtcdStatus);
    let quorum = freshkube_core::indicators::quorum(answered, voting);
    let tolerance = quorum.remaining_tolerance;
    if voting == 0 {
        return QuorumView {
            tone: Tone::Unknown,
            label: "Quorum unknown",
            detail: "The member list is empty.".into(),
        };
    }
    if status_missing || answered == 0 {
        return QuorumView {
            tone: Tone::Unknown,
            label: "Quorum not reported",
            detail: "No member statuses were available, so quorum can't be confirmed.".into(),
        };
    }
    let tolerates = if voting <= 1 {
        "single member, so no failure tolerance".to_owned()
    } else {
        format!(
            "tolerates {tolerance} additional member {}",
            if tolerance == 1 {
                "failure"
            } else {
                "failures"
            }
        )
    };
    match quorum.state {
        QuorumState::Healthy => QuorumView {
            tone: if tolerance > 0 {
                Tone::Good
            } else {
                Tone::Warn
            },
            label: "Quorum",
            detail: format!("All {voting} voting members answered · {tolerates}"),
        },
        QuorumState::Degraded { .. } => QuorumView {
            tone: Tone::Warn,
            label: "Degraded",
            detail: format!(
                "{answered} of {voting} voting members answered, which is still a quorum · {tolerates}"
            ),
        },
        _ => QuorumView {
            tone: Tone::Warn,
            label: "Quorum unconfirmed",
            detail: format!(
                "Only {answered} of {voting} voting members answered; a quorum needs {}. Members that didn't answer are not reported, not failed.",
                quorum.required
            ),
        },
    }
}

impl EtcdScreen {
    fn members(&self) -> &[EtcdMemberSnapshot] {
        self.loader
            .data()
            .map(|snapshot| snapshot.members.as_slice())
            .unwrap_or_default()
    }

    fn selected_member(&self) -> Option<&EtcdMemberSnapshot> {
        let id = self.selected?;
        self.members().iter().find(|member| member.info.id == id)
    }

    fn step(&mut self, delta: isize, cx: &mut Context<Self>) {
        let members = self.members();
        if members.is_empty() {
            return;
        }
        let current = members
            .iter()
            .position(|member| Some(member.info.id) == self.selected);
        let next = match current {
            Some(ix) => ix.saturating_add_signed(delta).min(members.len() - 1),
            None if delta < 0 => members.len() - 1,
            None => 0,
        };
        self.selected = Some(members[next].info.id);
        cx.notify();
    }

    fn name_of(&self, member_id: u64) -> String {
        self.members()
            .iter()
            .find(|member| member.info.id == member_id)
            .map(|member| member.info.hostname.clone())
            .unwrap_or_else(|| format!("{member_id:x}"))
    }

    /// Quorum verdict and the figures that matter at a glance.
    fn summary(&self, snapshot: &EtcdHealthSnapshot, cx: &App) -> Stateful<Div> {
        let p = palette(cx);
        let quorum = quorum_view(snapshot);
        let alarm_count: usize = snapshot
            .members
            .iter()
            .map(|member| member.alarms.len())
            .sum::<usize>()
            + snapshot.unmatched_alarms.len();
        let alarms_unknown = snapshot
            .unavailable
            .iter()
            .any(|missing| missing.source == InspectionSource::EtcdAlarms);
        let leader = match (snapshot.leader_id(), snapshot.reported_leader_ids.len()) {
            (Some(id), _) => self.name_of(id),
            (None, 0) => "not reported".into(),
            (None, _) => "members disagree".into(),
        };
        let reported = snapshot.members.iter().filter(|m| m.is_reachable()).count();
        let largest = if reported == 0 {
            "not reported".into()
        } else {
            snapshot.largest_database_size_display()
        };
        let revision = if reported == 0 {
            "not reported".into()
        } else {
            snapshot.revision.to_string()
        };
        v_flex()
            .id("etcd-summary")
            .gap_3()
            .child(
                h_flex()
                    .id("etcd-quorum")
                    .test_support()
                    .role(Role::Status)
                    .aria_label(format!("{} · {}", quorum.label, quorum.detail))
                    .gap_2p5()
                    .flex_wrap()
                    .child(ui::tag(quorum.tone, None, quorum.label, cx))
                    .child(
                        div()
                            .text_size(dp(12.5))
                            .text_color(p.muted)
                            .child(quorum.detail),
                    ),
            )
            .child(
                h_flex()
                    .gap_2p5()
                    .flex_wrap()
                    .child(stat(
                        "Members",
                        format!("{reported} / {} reported", snapshot.members.len()),
                        cx,
                    ))
                    .child(stat("Leader", leader, cx))
                    .child(stat("Largest DB", largest, cx))
                    .child(stat("Raft index", revision, cx))
                    .child(stat(
                        "Alarms",
                        if alarms_unknown {
                            "not reported".to_owned()
                        } else if alarm_count == 0 {
                            "None".to_owned()
                        } else {
                            alarm_count.to_string()
                        },
                        cx,
                    )),
            )
    }

    fn head(&self, columns: &[Column], cx: &App) -> Div {
        super::table_head(columns, cx)
    }

    fn render_row(
        &self,
        ix: usize,
        member: &EtcdMemberSnapshot,
        compact: bool,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        let p = palette(cx);
        let id = member.info.id;
        let selected = self.selected == Some(id);
        let role = MemberRole::of(member);
        let db = member
            .status
            .as_ref()
            .map(|status| format_bytes_signed(status.db_size))
            .unwrap_or_else(|| "—".into());
        let raft = member
            .status
            .as_ref()
            .map(|status| status.raft_index.to_string())
            .unwrap_or_else(|| "—".into());
        let issues_count = error_count(member).unwrap_or(0) + member.alarms.len();
        let issues = match (member.status.is_some(), issues_count) {
            (false, 0) => "—".to_owned(),
            (_, 0) => "none".to_owned(),
            (_, count) => count.to_string(),
        };
        let columns: &[Column] = if compact {
            &COMPACT_COLUMNS
        } else {
            &FULL_COLUMNS
        };
        let name = div()
            .flex()
            .items_center()
            .gap_2()
            .child(ui::health_mark(
                SharedString::from(format!("etcd-member-health-{}", member.info.id)),
                member_health(member),
                cx,
            ))
            .child(
                div()
                    .truncate()
                    .font_weight(FontWeight::MEDIUM)
                    .child(member.info.hostname.clone()),
            );
        let role_cell = div()
            .id(("etcd-role", ix))
            .test_support()
            .aria_label(role.label())
            .child(role_tag(role, cx));
        let leader_mark = (role == MemberRole::Leader).then(|| {
            div()
                .id(("etcd-leader", ix))
                .test_support()
                .aria_label("Leader")
                .size_0()
        });
        let mut values: Vec<AnyElement> = vec![
            name.into_any_element(),
            role_cell.into_any_element(),
            div().child(endpoint(&member.info)).into_any_element(),
            div().text_right().child(db.clone()).into_any_element(),
            div().text_right().child(raft).into_any_element(),
            div()
                .text_right()
                .when(issues_count > 0 && !selected, |this| {
                    this.text_color(p.crit_ink)
                })
                .child(issues)
                .into_any_element(),
        ];
        if compact {
            values = vec![
                values.remove(0),
                values.remove(0),
                values.remove(1),
                values.remove(2),
            ];
        }
        h_flex()
            .id(("etcd-member", ix))
            .test_support()
            .role(Role::ListBoxOption)
            .aria_selected(selected)
            .aria_label(format!(
                "{} · {} · DB {db} · {} issue(s)",
                member.info.hostname,
                role.label(),
                issues_count
            ))
            .w_full()
            .h(dp(ROW_HEIGHT))
            .font_family(MONO_FONT)
            .text_size(dp(12.))
            .cursor_pointer()
            .when(selected, |this| this.bg(p.accent_soft).text_color(p.accent))
            .when(!selected, |this| this.hover(|style| style.bg(p.hover)))
            .children(
                values
                    .into_iter()
                    .zip(columns.iter().copied())
                    .map(|(value, column)| cell(column).child(value)),
            )
            .children(leader_mark)
            .on_click(cx.listener(move |view, _, window, cx| {
                view.selected = Some(id);
                window.focus(&view.focus, cx);
                cx.notify();
            }))
    }

    fn list(&self, snapshot: &EtcdHealthSnapshot, compact: bool, cx: &mut Context<Self>) -> Div {
        let p = palette(cx);
        let columns: &[Column] = if compact {
            &COMPACT_COLUMNS
        } else {
            &FULL_COLUMNS
        };
        let rows = snapshot
            .members
            .iter()
            .enumerate()
            .map(|(ix, member)| self.render_row(ix, member, compact, cx))
            .collect::<Vec<_>>();
        panel(cx)
            .overflow_hidden()
            .child(self.head(columns, cx))
            .child(
                div()
                    .id("etcd-list")
                    .test_support()
                    .role(Role::ListBox)
                    .aria_label("etcd members; arrows select a member")
                    .key_context(CONTEXT)
                    .track_focus(&self.focus)
                    .on_action(cx.listener(|view, _: &NextMember, _, cx| view.step(1, cx)))
                    .on_action(cx.listener(|view, _: &PreviousMember, _, cx| view.step(-1, cx)))
                    .on_action(
                        cx.listener(|view, _: &FirstMember, _, cx| view.step(isize::MIN, cx)),
                    )
                    .on_action(cx.listener(|view, _: &LastMember, _, cx| view.step(isize::MAX, cx)))
                    .on_action(cx.listener(|view, _: &ClearSelection, _, cx| {
                        view.selected = None;
                        cx.notify();
                    }))
                    .map(|this| {
                        if rows.is_empty() {
                            this.child(
                                div()
                                    .px_3()
                                    .py_3p5()
                                    .text_size(dp(12.5))
                                    .text_color(p.muted)
                                    .child("etcd reported no members."),
                            )
                        } else {
                            this.children(rows)
                        }
                    }),
            )
    }

    fn details(&self, cx: &mut Context<Self>) -> AnyElement {
        let p = palette(cx);
        let Some(member) = self.selected_member() else {
            return panel(cx)
                .p_4()
                .text_color(p.muted)
                .text_size(dp(12.5))
                .child(if self.selected.is_some() {
                    "The selected member is no longer in the roster."
                } else {
                    "Select a member to see its details."
                })
                .into_any_element();
        };
        let info = &member.info;
        let role = MemberRole::of(member);
        let urls = |urls: &[String]| {
            if urls.is_empty() {
                mono("not reported").into_any_element()
            } else {
                v_flex()
                    .children(urls.iter().map(|url| mono(url.clone())))
                    .into_any_element()
            }
        };
        // Targeting only makes sense for members the roster can map to a node.
        let node_known = self
            .source
            .as_ref()
            .is_some_and(|source| source.nodes.iter().any(|node| node.name == info.hostname));
        let is_target = self
            .source
            .as_ref()
            .is_some_and(|source| source.target.node == info.hostname);
        let hostname = info.hostname.clone();
        let logs_node = info.hostname.clone();
        let mut pane = panel(cx)
            .id("etcd-details")
            .test_support()
            .aria_label(format!("{} · {}", info.hostname, role.label()))
            .p_4()
            .gap_2p5()
            .child(
                h_flex()
                    .gap_2()
                    .flex_wrap()
                    .child(
                        div()
                            .font_family(MONO_FONT)
                            .text_size(dp(14.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .truncate()
                            .child(info.hostname.clone()),
                    )
                    .child(role_tag(role, cx))
                    .child(div().flex_1())
                    .when(node_known, |this| {
                        this.child(
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
                    })
                    .when(node_known && !is_target, |this| {
                        this.child(
                            Button::new("etcd-select-node")
                                .ghost()
                                .xsmall()
                                .icon(IconName::Crosshair)
                                .label("Target this node")
                                .on_click(cx.listener(move |_, _, _, cx| {
                                    cx.emit(ScreenEvent::SelectNode(hostname.clone()))
                                })),
                        )
                    }),
            )
            .child(field("Member ID", mono(format!("{:x}", info.id)), cx))
            .child(field("Peer URLs", urls(&info.peer_urls), cx))
            .child(field("Client URLs", urls(&info.client_urls), cx))
            .child(field(
                "Learner",
                mono(if info.is_learner { "Yes" } else { "No" }),
                cx,
            ));
        let Some(status) = &member.status else {
            return pane
                .child(field(
                    "Status",
                    div()
                        .text_color(p.unk_ink)
                        .child("Not reported. This member didn't answer the status request, which doesn't mean it is down."),
                    cx,
                ))
                .child(self.alarm_field(&member.alarms, cx))
                .into_any_element();
        };
        let in_use_percent = if status.db_size > 0 {
            status.db_size_in_use as f64 / status.db_size as f64 * 100.
        } else {
            0.
        };
        let lag = status.raft_index.saturating_sub(status.raft_applied_index);
        pane = pane
            .child(field("Protocol", mono(status.protocol_version.clone()), cx))
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
            ));
        pane.child(self.alarm_field(&member.alarms, cx))
            .into_any_element()
    }

    fn alarm_field(&self, alarms: &[EtcdAlarm], cx: &App) -> Div {
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

    fn alarms_panel(&self, snapshot: &EtcdHealthSnapshot, cx: &App) -> impl IntoElement + use<> {
        let p = palette(cx);
        let unknown = snapshot
            .unavailable
            .iter()
            .any(|missing| missing.source == InspectionSource::EtcdAlarms);
        let alarms: Vec<&EtcdAlarm> = snapshot
            .members
            .iter()
            .flat_map(|member| member.alarms.iter())
            .chain(snapshot.unmatched_alarms.iter())
            .collect();
        let body = if unknown {
            h_flex()
                .gap_2()
                .child(ui::tag(Tone::Unknown, None, "Not reported", cx))
                .child(
                    div()
                        .text_color(p.muted)
                        .child("The alarm list didn't answer."),
                )
                .into_any_element()
        } else if alarms.is_empty() {
            h_flex()
                .gap_2()
                .child(ui::tag(Tone::Good, None, "No alarms", cx))
                .into_any_element()
        } else {
            v_flex()
                .gap_1p5()
                .children(alarms.into_iter().enumerate().map(|(ix, alarm)| {
                    h_flex()
                        .id(("etcd-alarm", ix))
                        .test_support()
                        .aria_label(alarm_text(alarm))
                        .gap_2()
                        .child(ui::tag(Tone::Warn, None, alarm.alarm_type.as_str(), cx))
                        .child(mono(format!(
                            "member {} · reported by {}",
                            self.name_of(alarm.member_id),
                            alarm.node
                        )))
                }))
                .into_any_element()
        };
        panel(cx)
            .id("etcd-alarms")
            .test_support()
            .aria_label("etcd alarms")
            .p_4()
            .gap_2p5()
            .child(ui::caption("Alarms", cx))
            .child(body)
    }
}

fn alarm_text(alarm: &EtcdAlarm) -> String {
    format!("{} on {}", alarm.alarm_type.as_str(), alarm.node)
}

impl Render for EtcdScreen {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if let Some(page) = gated_page(
            "etcd-page",
            "etcd",
            Scope::Cluster,
            self.source.as_ref(),
            &self.loader,
            "the etcd status",
            cx,
        ) {
            return page;
        }
        let (Some(source), Some(snapshot)) = (self.source.clone(), self.loader.data().cloned())
        else {
            return div().into_any_element();
        };
        let width = content_width(window);
        let wide = width >= SIDE_DETAILS;
        // The table gets what the side details leave; drop columns before
        // they'd be clipped.
        let list_width = if wide {
            width - (DETAILS_WIDTH + 14.)
        } else {
            width
        };
        let compact = list_width < table_width(&FULL_COLUMNS);
        let mut missing: Vec<String> = snapshot
            .unavailable
            .iter()
            .map(|InspectionUnavailable { source, message }| {
                format!("{}: {message}", source.label())
            })
            .collect();
        let silent: Vec<&str> = snapshot
            .members
            .iter()
            .filter(|member| !member.is_reachable())
            .map(|member| member.info.hostname.as_str())
            .collect();
        if !silent.is_empty() && snapshot.unavailable.is_empty() {
            missing.push(format!(
                "member status: {} didn't answer",
                silent.join(", ")
            ));
        }
        if snapshot.reported_leader_ids.len() > 1 {
            missing.push("leader: members report different leaders".into());
        }
        if !snapshot.unmatched_statuses.is_empty() {
            missing.push(format!(
                "roster: {} status record(s) came from members not in the roster",
                snapshot.unmatched_statuses.len()
            ));
        }
        let summary = self.summary(&snapshot, cx);
        let list = self.list(&snapshot, compact, cx);
        let alarms = self.alarms_panel(&snapshot, cx);
        let details = self.details(cx);
        let body = if wide {
            h_flex()
                .items_start()
                .gap(dp(14.))
                .child(
                    v_flex()
                        .flex_1()
                        .min_w_0()
                        .gap(dp(14.))
                        .child(list)
                        .child(alarms),
                )
                .child(div().w(dp(DETAILS_WIDTH)).flex_none().child(details))
                .into_any_element()
        } else {
            v_flex()
                .gap(dp(14.))
                .child(list)
                .child(details)
                .child(alarms)
                .into_any_element()
        };
        v_flex()
            .id("etcd-page")
            .size_full()
            .min_h_0()
            .overflow_y_scroll()
            .px(dp(crate::desktop::PAGE_PADDING))
            .pt(dp(22.))
            .pb(dp(18.))
            .gap(dp(14.))
            .child(header("etcd", &source, Scope::Cluster, &self.loader, cx))
            .children(failure_banner(&self.loader, cx))
            .children(partial_notice(missing, cx))
            .child(summary)
            .child(body)
            .into_any_element()
    }
}

/// Example members for `--fixture`: the control planes of the example cluster,
/// healthy, with one member whose database has grown and which trails the
/// leader a little.
fn example(source: &ScreenSource) -> Result<EtcdHealthSnapshot, String> {
    let Some(node) = source.node() else {
        return Err("Example data has no such node".into());
    };
    if !node.responding {
        return Err(format!(
            "{} didn't answer the Talos API within 10 s (example)",
            node.name
        ));
    }
    const MIB: i64 = 1024 * 1024;
    let control_planes: Vec<_> = source
        .nodes
        .iter()
        .filter(|node| node.role == presentation::Role::ControlPlane)
        .collect();
    let member_id = |ix: usize| 0x4c1d_9e07_a3b5_2f60_u64 + ix as u64 * 0x1111;
    let leader_id = member_id(0);
    let members: Vec<EtcdMemberInfo> = control_planes
        .iter()
        .enumerate()
        .map(|(ix, node)| EtcdMemberInfo {
            id: member_id(ix),
            hostname: node.name.clone(),
            peer_urls: vec![format!("https://{}:2380", node.address)],
            client_urls: vec![format!("https://{}:2379", node.address)],
            is_learner: false,
        })
        .collect();
    let statuses: Vec<EtcdMemberStatus> = control_planes
        .iter()
        .enumerate()
        .map(|(ix, node)| {
            // The last member carries a larger, more fragmented database and
            // applies entries a few behind the others.
            let heavy = ix > 0 && ix + 1 == control_planes.len();
            let raft_index = 4_812_377_u64;
            EtcdMemberStatus {
                node: node.name.clone(),
                member_id: member_id(ix),
                protocol_version: "3.6.0".into(),
                db_size: if heavy { 412 * MIB } else { 96 * MIB },
                db_size_in_use: if heavy { 188 * MIB } else { 71 * MIB },
                leader_id,
                raft_index,
                raft_term: 7,
                raft_applied_index: if heavy { raft_index - 36 } else { raft_index },
                errors: Vec::new(),
                is_learner: false,
            }
        })
        .collect();
    Ok(assemble_etcd_health(
        source.inspection_target(),
        members,
        statuses,
        Vec::new(),
        Vec::new(),
    ))
}

#[cfg(test)]
mod ui_tests {
    use std::sync::Arc;

    use freshkube_core::inspection::{
        InspectionSource, InspectionUnavailable, assemble_etcd_health,
    };
    use gpui_kit::component::Root;
    use gpui_kit::test::TestWindowExt;
    use gpui_kit::{AppContext, Entity, TestAppContext, WindowHandle, px, size};
    use talos_rs::{EtcdAlarm, EtcdAlarmType, EtcdMemberInfo, EtcdMemberStatus};
    use tokio::runtime::{Builder, Runtime};

    // Not `super::*`: gpui_kit's glob would shadow the built-in `#[test]`.
    use super::{EtcdScreen, MemberRole, ScreenPanel, ScreenSource, quorum_view};
    use crate::backend::Target;
    use crate::{fixture, presentation};

    fn source(node: &str) -> ScreenSource {
        let nodes = presentation::node_summaries(&fixture::cluster("prod-fra", 1));
        let summary = nodes.iter().find(|summary| summary.name == node).unwrap();
        ScreenSource {
            target: Target {
                epoch: 1,
                context: "prod-fra".into(),
                node: summary.name.clone(),
                address: summary.address.clone(),
            },
            nodes: Arc::new(nodes),
            live: None,
        }
    }

    fn mount(
        cx: &mut TestAppContext,
        node: &str,
    ) -> (Runtime, Entity<EtcdScreen>, WindowHandle<Root>) {
        let runtime = Builder::new_multi_thread()
            .worker_threads(1)
            .enable_all()
            .build()
            .unwrap();
        cx.update(|cx| {
            gpui_kit::init(cx);
            crate::theme::install(cx);
        });
        let source = source(node);
        let mut screen = None;
        let handle = cx.open_window(size(px(1100.), px(760.)), |window, cx| {
            let view = cx.new(|cx| {
                let mut view = EtcdScreen::new(runtime.handle().clone(), window, cx);
                view.set_source(Some(source), window, cx);
                view.activate(window, cx);
                view
            });
            screen = Some(view.clone());
            Root::new(view, window, cx)
        });
        cx.run_until_parked();
        (runtime, screen.unwrap(), handle)
    }

    fn info(id: u64, name: &str) -> EtcdMemberInfo {
        EtcdMemberInfo {
            id,
            hostname: name.into(),
            peer_urls: vec![format!("https://10.0.0.{id}:2380")],
            client_urls: vec![format!("https://10.0.0.{id}:2379")],
            is_learner: false,
        }
    }

    fn status(id: u64, leader: u64) -> EtcdMemberStatus {
        EtcdMemberStatus {
            node: format!("n{id}"),
            member_id: id,
            protocol_version: "3.6.0".into(),
            db_size: 1 << 20,
            db_size_in_use: 1 << 19,
            leader_id: leader,
            raft_index: 100,
            raft_term: 2,
            raft_applied_index: 100,
            errors: Vec::new(),
            is_learner: false,
        }
    }

    #[gpui_kit::test]
    fn keyboard_selects_members_and_updates_details(cx: &mut TestAppContext) {
        let (_runtime, screen, handle) = mount(cx, "talos-cp-fra1-01");
        cx.update_window(handle.into(), |_, window, cx| {
            window.render_frame(cx);
            assert_eq!(screen.read(cx).members().len(), 3);
            window.click(("etcd-member", 0usize), cx);
            assert_eq!(window.find(("etcd-member", 0usize)).selected(), Some(true));
            window.press("down", cx);
            assert_eq!(window.find(("etcd-member", 1usize)).selected(), Some(true));
            assert_eq!(window.find(("etcd-member", 0usize)).selected(), Some(false));
            let name = screen
                .read(cx)
                .selected_member()
                .unwrap()
                .info
                .hostname
                .clone();
            assert_eq!(name, "talos-cp-fra1-02");
            assert!(
                window
                    .find("etcd-details")
                    .label()
                    .is_some_and(|label| label.contains(&name))
            );
            window.press("end", cx);
            assert_eq!(window.find(("etcd-member", 2usize)).selected(), Some(true));
            assert!(
                window
                    .find("etcd-details")
                    .label()
                    .is_some_and(|label| label.contains("talos-cp-fra1-03-baremetal-rack-b7"))
            );
            window.press("home", cx);
            assert_eq!(window.find(("etcd-member", 0usize)).selected(), Some(true));
        })
        .unwrap();
    }

    #[gpui_kit::test]
    fn leader_is_marked_and_quorum_is_healthy(cx: &mut TestAppContext) {
        let (_runtime, screen, handle) = mount(cx, "talos-cp-fra1-02");
        cx.update_window(handle.into(), |_, window, cx| {
            window.render_frame(cx);
            window.find(("etcd-leader", 0usize));
            assert!(window.try_find(("etcd-leader", 1usize)).is_none());
            assert_eq!(window.find(("etcd-role", 0usize)).label(), Some("Leader"));
            assert_eq!(window.find(("etcd-role", 1usize)).label(), Some("Follower"));
            let label = window.find("etcd-quorum").label().unwrap().to_owned();
            assert!(
                label.contains("tolerates 1 additional member failure"),
                "{label}"
            );
            assert!(window.try_find("partial-notice").is_none());
            let snapshot = screen.read(cx).loader.data().unwrap().clone();
            assert!(snapshot.quorum.has_quorum());
            window.find("etcd-alarms");
        })
        .unwrap();
    }

    #[gpui_kit::test]
    fn silent_member_is_not_reported_rather_than_failed(cx: &mut TestAppContext) {
        let (_runtime, screen, handle) = mount(cx, "talos-cp-fra1-01");
        cx.update_window(handle.into(), |_, window, cx| {
            let target = source("talos-cp-fra1-01").target;
            screen.update(cx, |screen, cx| {
                let snapshot = assemble_etcd_health(
                    source("talos-cp-fra1-01").inspection_target(),
                    vec![info(1, "cp-a"), info(2, "cp-b"), info(3, "cp-c")],
                    vec![status(1, 1), status(2, 1)],
                    Vec::new(),
                    Vec::new(),
                );
                screen.loader.resolve(target, Ok(snapshot));
                cx.notify();
            });
            window.render_frame(cx);
            assert_eq!(
                window.find(("etcd-role", 2usize)).label(),
                Some("Not reported")
            );
            let row = window
                .find(("etcd-member", 2usize))
                .label()
                .unwrap()
                .to_owned();
            assert!(!row.to_lowercase().contains("fail"), "{row}");
            assert!(!row.to_lowercase().contains("down"), "{row}");
            window.find("partial-notice");
            // Two of three still answer: degraded, never "no quorum".
            let quorum = window.find("etcd-quorum").label().unwrap().to_owned();
            assert!(quorum.starts_with("Degraded"), "{quorum}");
            assert!(
                quorum.contains("tolerates 0 additional member failures"),
                "{quorum}"
            );
            window.click(("etcd-member", 2usize), cx);
            window.render_frame(cx);
            assert_eq!(
                screen.read(cx).selected_member().map(MemberRole::of),
                Some(MemberRole::NotReported)
            );
        })
        .unwrap();
    }

    #[test]
    fn quorum_display_has_no_spare_capacity_at_two_of_three_or_three_of_five() {
        for (answered, total) in [(2, 3), (3, 5)] {
            let snapshot = assemble_etcd_health(
                freshkube_core::inspection::InspectionTarget::new("cp-a", "10.0.0.1"),
                (1..=total)
                    .map(|id| info(id, &format!("cp-{id}")))
                    .collect(),
                (1..=answered).map(|id| status(id, 1)).collect(),
                Vec::new(),
                Vec::new(),
            );
            let view = quorum_view(&snapshot);
            assert_eq!(view.label, "Degraded");
            assert_eq!(view.tone, crate::ui::Tone::Warn);
            assert!(
                view.detail
                    .contains("tolerates 0 additional member failures")
            );
        }
    }

    #[test]
    fn missing_statuses_make_quorum_unknown_not_lost() {
        let snapshot = assemble_etcd_health(
            presentation::node_summaries(&fixture::cluster("prod-fra", 1))
                .first()
                .map(|node| {
                    freshkube_core::inspection::InspectionTarget::new(&node.name, &node.address)
                })
                .unwrap(),
            vec![info(1, "a"), info(2, "b"), info(3, "c")],
            Vec::new(),
            Vec::new(),
            vec![InspectionUnavailable {
                source: InspectionSource::EtcdStatus,
                message: "timed out".into(),
            }],
        );
        let view = quorum_view(&snapshot);
        assert_eq!(view.label, "Quorum not reported");
    }

    #[gpui_kit::test]
    fn alarms_and_errors_are_shown_when_reported(cx: &mut TestAppContext) {
        let (_runtime, screen, handle) = mount(cx, "talos-cp-fra1-01");
        cx.update_window(handle.into(), |_, window, cx| {
            let target = source("talos-cp-fra1-01").target;
            screen.update(cx, |screen, cx| {
                let mut broken = status(2, 1);
                broken.errors = vec!["etcdserver: no space".into()];
                let snapshot = assemble_etcd_health(
                    source("talos-cp-fra1-01").inspection_target(),
                    vec![info(1, "cp-a"), info(2, "cp-b")],
                    vec![status(1, 1), broken],
                    vec![EtcdAlarm {
                        node: "cp-b".into(),
                        member_id: 2,
                        alarm_type: EtcdAlarmType::NoSpace,
                    }],
                    Vec::new(),
                );
                screen.loader.resolve(target, Ok(snapshot));
                cx.notify();
            });
            window.render_frame(cx);
            assert_eq!(
                window.find(("etcd-alarm", 0usize)).label(),
                Some("NOSPACE on cp-b")
            );
        })
        .unwrap();
    }

    #[gpui_kit::test]
    fn changing_target_drops_old_data(cx: &mut TestAppContext) {
        let (_runtime, screen, handle) = mount(cx, "talos-cp-fra1-01");
        cx.update_window(handle.into(), |_, window, cx| {
            screen.update(cx, |screen, cx| {
                screen.selected = Some(1);
                assert!(screen.loader.data().is_some());
                screen.set_source(Some(source("talos-cp-fra1-02")), window, cx);
                assert!(screen.loader.data().is_none());
                assert!(screen.selected.is_none());
            });
        })
        .unwrap();
    }

    #[gpui_kit::test]
    fn silent_target_offers_retry_without_data(cx: &mut TestAppContext) {
        let (_runtime, screen, handle) = mount(cx, "talos-wk-fra1-03");
        cx.update_window(handle.into(), |_, window, cx| {
            window.render_frame(cx);
            assert!(screen.read(cx).loader.data().is_none());
            window.find("screen-retry");
            assert!(window.try_find("etcd-list").is_none());
        })
        .unwrap();
    }
}
