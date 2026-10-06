//! etcd: the cluster's members, quorum, leader, database sizes and alarms,
//! as a table page with the selected member's details beside it.
//!
//! The target node's client asks for the member roster, then for each
//! member's status and the alarm list. Members that don't answer are shown as
//! "not reported", never as failed: only an error the API reports is one.
mod source;
#[cfg(test)]
mod tests;
mod view;

use freshkube_core::format_bytes_signed;
use freshkube_core::indicators::QuorumState;
use freshkube_core::inspection::{
    EtcdHealthSnapshot, EtcdInspectionRequest, EtcdMemberSnapshot, InspectionSource,
    InspectionUnavailable, assemble_etcd_health, collect_etcd_health,
};
use freshkube_ui::status::{Part, Segment};
use freshkube_ui::table::TableState;
use gpui_kit::assets::IconName;
use gpui_kit::prelude::*;
use gpui_kit::*;
use talos_rs::{EtcdAlarm, EtcdMemberInfo, EtcdMemberStatus};
use tokio::runtime::Handle;

use super::{Loader, ScreenEvent, ScreenPanel, ScreenSource};
use crate::presentation;
use crate::ui::{self, Tone, clock};

const CONTEXT: &str = "TalosEtcd";
/// The page's id prefix: `etcd-title`, `etcd-list`, `etcd-tally-…`.
const PREFIX: &str = "etcd";
/// The alarm banner lists this many; the members' details hold the rest.
const BANNER_ALARMS: usize = 3;

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
    table: TableState,
    /// The state whose chip filters the table.
    state: Option<MemberState>,
    derived: Derived,
}

/// One member as the table shows it.
pub(crate) struct MemberRow {
    id: u64,
    /// Derived from the member, so a click that lands after the roster
    /// changed can't select another one.
    element_id: SharedString,
    name: SharedString,
    role: MemberRole,
    endpoint: SharedString,
    db: SharedString,
    raft: SharedString,
    issues: SharedString,
    issue_count: usize,
    state: MemberState,
    label: SharedString,
}

/// A member's state, which its glyph shows and the chips count and filter
/// by, worst first.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum MemberState {
    /// An alarm or a status-reported error.
    Issues,
    /// It answers but follows no leader, so it can't serve writes. Warn,
    /// not Crit: the member itself is up, and the lost quorum is the
    /// banner's and the summary's to state; never Good under that banner.
    NoLeader,
    NotReported,
    Healthy,
}

impl MemberState {
    const ALL: [Self; 4] = [
        Self::Issues,
        Self::NoLeader,
        Self::NotReported,
        Self::Healthy,
    ];

    fn of(member: &EtcdMemberSnapshot) -> Self {
        if member.has_problems() {
            Self::Issues
        } else if member
            .status
            .as_ref()
            .is_some_and(|status| status.leader_id == 0)
        {
            Self::NoLeader
        } else if member.is_reachable() {
            Self::Healthy
        } else {
            Self::NotReported
        }
    }

    fn tone(self) -> Tone {
        match self {
            Self::Issues => Tone::Crit,
            Self::NoLeader => Tone::Warn,
            Self::NotReported => Tone::Unknown,
            Self::Healthy => Tone::Good,
        }
    }

    /// What its chip counts.
    fn what(self) -> &'static str {
        match self {
            Self::Issues => "with issues",
            Self::NoLeader => "without a leader",
            Self::NotReported => "not reported",
            Self::Healthy => "healthy",
        }
    }

    /// Its glyph's tooltip and accessibility label.
    fn label(self) -> &'static str {
        match self {
            Self::Issues => "Has issues",
            Self::NoLeader => "No leader",
            Self::NotReported => "Not reported",
            Self::Healthy => "Healthy",
        }
    }

    fn index(self) -> usize {
        self as usize
    }
}

/// An alarm's tone. etcd's alarms stop writes (NOSPACE) or flag corruption
/// (CORRUPT), so each is critical; an alarm kind this app doesn't know is
/// one etcd raised all the same, and its member already counts as having
/// issues. The match is exhaustive, so a new kind needs a decision here.
fn alarm_tone(alarm: &talos_rs::EtcdAlarmType) -> Tone {
    match alarm {
        talos_rs::EtcdAlarmType::NoSpace
        | talos_rs::EtcdAlarmType::Corrupt
        | talos_rs::EtcdAlarmType::Unknown(_) => Tone::Crit,
        talos_rs::EtcdAlarmType::None => Tone::Unknown,
    }
}

/// One alarm in the banner.
struct AlarmLine {
    /// `NOSPACE on talos-cp-1`, its accessibility label.
    label: SharedString,
    text: SharedString,
}

/// What the page shows of the last answer, derived when the loader's
/// revision moves, never while drawing.
#[derive(Default)]
struct Derived {
    /// The loader revision these were derived from.
    revision: Option<u64>,
    rows: Vec<MemberRow>,
    /// The rows shown under the state filter, in roster order.
    lines: Vec<usize>,
    counts: [usize; 4],
    columns: Vec<source::Column>,
    width: f32,
    quorum: Option<QuorumView>,
    banner: Option<QuorumBanner>,
    alarms: Vec<AlarmLine>,
    /// The alarm banner's tone: its worst alarm's.
    alarm_tone: Tone,
    /// Gaps in the answer, for the partial notice.
    missing: Vec<String>,
    /// The status bar's line: the context, members, the quorum, then
    /// leader, alarms, time and example data; the quorum's detail is its
    /// note. None until etcd answers.
    status: Option<Segment>,
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
            table: TableState::new(PREFIX),
            state: None,
            derived: Derived::default(),
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
        // The meta names the source's context and whether it is example data.
        self.derived.revision = None;
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

    fn status(&mut self) -> Option<&Segment> {
        self.derive_if_changed();
        self.derived.status.as_ref()
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
    /// It answered, but reports no leader to follow.
    NoLeader,
    NotReported,
}

impl MemberRole {
    fn of(member: &EtcdMemberSnapshot) -> Self {
        match &member.status {
            None => Self::NotReported,
            Some(_) if member.is_leader() => Self::Leader,
            Some(status) if status.leader_id == 0 => Self::NoLeader,
            Some(status) if status.is_learner || member.info.is_learner => Self::Learner,
            Some(_) => Self::Follower,
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::Leader => "Leader",
            Self::Follower => "Follower",
            Self::Learner => "Learner",
            Self::NoLeader => "No leader",
            Self::NotReported => "Not reported",
        }
    }
}

/// A role is identity, so it carries no health colour; the row's glyph does.
/// The leader keeps the accent to stand out, and "No leader" warns because
/// it's a state rather than a role.
fn role_tag(role: MemberRole, cx: &App) -> Div {
    match role {
        MemberRole::Leader => ui::tag(Tone::Accent, Some(IconName::Crosshair), "Leader", cx),
        MemberRole::Follower => ui::tag(Tone::Outline, None, "Follower", cx),
        MemberRole::Learner => ui::tag(Tone::Outline, None, "Learner", cx),
        MemberRole::NoLeader => ui::tag(Tone::Warn, None, "No leader", cx),
        MemberRole::NotReported => ui::tag(Tone::Unknown, None, "Not reported", cx),
    }
}

/// How bad a tone is, for picking the worst.
fn tone_rank(tone: Tone) -> u8 {
    match tone {
        Tone::Crit | Tone::Died => 3,
        Tone::Warn => 2,
        Tone::Unknown => 1,
        _ => 0,
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
    // The members' own raft status: answered, and none follows a leader.
    // The banner says so with the arithmetic; the summary must agree.
    if snapshot.reported_leader_ids.is_empty() {
        return QuorumView {
            tone: Tone::Crit,
            label: "No quorum",
            detail: format!(
                "{answered} of {voting} voting members answered; none reports a leader"
            ),
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
            // A single member is a configuration, not a state: it stays
            // calm, with a note in the summary.
            tone: if tolerance > 0 || voting == 1 {
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

/// A quorum that is lost or has no failure left to spare, worded from the
/// members' own answers: silence alone never makes one critical. A single
/// member has no tolerance by design, so it gets no banner.
struct QuorumBanner {
    tone: Tone,
    lead: &'static str,
    body: String,
}

fn quorum_banner(snapshot: &EtcdHealthSnapshot) -> Option<QuorumBanner> {
    let voting = snapshot.voting_members;
    let answered = snapshot.responding_voting_members;
    let status_missing = snapshot
        .unavailable
        .iter()
        .any(|missing| missing.source == InspectionSource::EtcdStatus);
    if voting == 0 || answered == 0 || status_missing {
        return None;
    }
    let quorum = freshkube_core::indicators::quorum(answered, voting);
    let required = quorum.required;
    if snapshot.reported_leader_ids.is_empty() {
        return Some(QuorumBanner {
            tone: Tone::Crit,
            lead: "No quorum",
            body: format!(
                "{answered} of {voting} voting members answered and none reports a leader. A quorum needs {required} members behind one leader; until one is elected, etcd accepts no writes."
            ),
        });
    }
    if voting == 1 {
        return None;
    }
    match quorum.state {
        QuorumState::NoQuorum { .. } => Some(QuorumBanner {
            tone: Tone::Warn,
            lead: "Quorum unconfirmed",
            body: format!(
                "Only {answered} of {voting} voting members answered; a quorum needs {required}. A leader is reported, and members that didn't answer are not reported, not failed."
            ),
        }),
        _ if quorum.remaining_tolerance == 0 => Some(QuorumBanner {
            tone: Tone::Warn,
            lead: "Quorum at risk",
            body: format!(
                "{answered} of {voting} voting members answered; a quorum needs {required}, so one more failure loses it."
            ),
        }),
        _ => None,
    }
}

fn plural(count: usize, one: &str, many: &str) -> String {
    format!("{count} {}", if count == 1 { one } else { many })
}

fn member_row(member: &EtcdMemberSnapshot) -> MemberRow {
    let id = member.info.id;
    let role = MemberRole::of(member);
    let status = member.status.as_ref();
    let db = status.map_or_else(|| "—".into(), |status| format_bytes_signed(status.db_size));
    let raft = status.map_or_else(|| "—".into(), |status| status.raft_index.to_string());
    let issue_count = error_count(member).unwrap_or(0) + member.alarms.len();
    let issues = match (status.is_some(), issue_count) {
        (false, 0) => "—".to_owned(),
        (_, 0) => "none".to_owned(),
        (_, count) => count.to_string(),
    };
    MemberRow {
        id,
        element_id: format!("etcd-member-{id:x}").into(),
        label: format!(
            "{} · {} · DB {db} · {issue_count} issue(s)",
            member.info.hostname,
            role.label()
        )
        .into(),
        name: member.info.hostname.clone().into(),
        role,
        endpoint: endpoint(&member.info).into(),
        db: db.into(),
        raft: raft.into(),
        issues: issues.into(),
        issue_count,
        state: MemberState::of(member),
    }
}

/// Gaps in the answer: sources that didn't answer, silent members, leaders
/// that disagree and statuses from outside the roster.
fn missing(snapshot: &EtcdHealthSnapshot) -> Vec<String> {
    let mut missing: Vec<String> = snapshot
        .unavailable
        .iter()
        .map(|InspectionUnavailable { source, message }| format!("{}: {message}", source.label()))
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
    missing
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

    fn name_of(&self, member_id: u64) -> String {
        name_in(self.members(), member_id)
    }

    /// Derives the rows, banners and meta from the loader's answer, when its
    /// revision moved since the last time.
    fn derive_if_changed(&mut self) {
        let revision = self.loader.revision();
        if self.derived.revision == Some(revision) {
            return;
        }
        self.derived = Derived {
            revision: Some(revision),
            ..Derived::default()
        };
        let Some(snapshot) = self.loader.data() else {
            (self.derived.columns, self.derived.width) = source::columns(&[]);
            return;
        };
        let rows: Vec<MemberRow> = snapshot.members.iter().map(member_row).collect();
        let alarms: Vec<AlarmLine> = snapshot
            .members
            .iter()
            .flat_map(|member| member.alarms.iter())
            .chain(snapshot.unmatched_alarms.iter())
            .map(|alarm| AlarmLine {
                label: alarm_text(alarm).into(),
                text: format!(
                    "{} on member {} · reported by {}",
                    alarm.alarm_type.as_str(),
                    name_in(&snapshot.members, alarm.member_id),
                    alarm.node
                )
                .into(),
            })
            .collect();
        let alarms_unknown = snapshot
            .unavailable
            .iter()
            .any(|missing| missing.source == InspectionSource::EtcdAlarms);
        let reported = snapshot.members.iter().filter(|m| m.is_reachable()).count();
        let members = if reported < snapshot.members.len() {
            format!("{reported} of {} reported", snapshot.members.len())
        } else {
            plural(snapshot.members.len(), "member", "members")
        };
        let leader = match (snapshot.leader_id(), snapshot.reported_leader_ids.len()) {
            (Some(id), _) => format!("leader {}", name_in(&snapshot.members, id)),
            (None, 0) => "leader not reported".into(),
            (None, _) => "members disagree on the leader".into(),
        };
        let alarm_tone = snapshot
            .members
            .iter()
            .flat_map(|member| member.alarms.iter())
            .chain(snapshot.unmatched_alarms.iter())
            .map(|alarm| alarm_tone(&alarm.alarm_type))
            .max_by_key(|tone| tone_rank(*tone))
            .unwrap_or(Tone::Unknown);
        let quorum = quorum_view(snapshot);
        // Only a part that warns is toned; a calm one keeps the bar's text.
        let toned = |text: String, tone: Tone| {
            let part = Part::new(text);
            if matches!(tone, Tone::Warn | Tone::Crit) {
                part.tone(tone)
            } else {
                part
            }
        };
        // Quorum and leader come first, so a narrow bar keeps them in view.
        let mut parts = vec![
            Part::new(members),
            toned(quorum.label.into(), quorum.tone),
            Part::new(leader),
        ];
        if snapshot.voting_members == 1 {
            parts.push(Part::new("single member · no failure tolerance"));
        }
        parts.push(if alarms_unknown {
            Part::new("alarms not reported")
        } else if alarms.is_empty() {
            Part::new("no alarms")
        } else {
            toned(plural(alarms.len(), "alarm", "alarms"), alarm_tone)
        });
        if let Some(time) = self.loader.last_successful() {
            parts.push(Part::new(format!("updated {}", clock(time))).minor());
        }
        if self.source.as_ref().is_some_and(ScreenSource::is_example) {
            parts.push(Part::new("example data").minor());
        }
        let context = self
            .source
            .as_ref()
            .map(|source| source.target.context.clone());
        self.derived.status = Some(Segment::new(context, parts).note(quorum.detail.clone()));
        (self.derived.columns, self.derived.width) = source::columns(&rows);
        self.derived.quorum = Some(quorum);
        self.derived.banner = quorum_banner(snapshot);
        self.derived.missing = missing(snapshot);
        self.derived.rows = rows;
        self.derived.alarm_tone = alarm_tone;
        self.derived.alarms = alarms;
        self.refilter();
    }

    /// The lines under the state filter, and the counts its chips show.
    fn refilter(&mut self) {
        let derived = &mut self.derived;
        derived.counts = [0; 4];
        for row in &derived.rows {
            derived.counts[row.state.index()] += 1;
        }
        derived.lines = derived
            .rows
            .iter()
            .enumerate()
            .filter(|(_, row)| self.state.is_none_or(|state| state == row.state))
            .map(|(ix, _)| ix)
            .collect();
    }

    fn toggle_state(&mut self, state: MemberState, cx: &mut Context<Self>) {
        self.state = (self.state != Some(state)).then_some(state);
        self.refilter();
        self.table.reveal(0, ScrollStrategy::Top);
        cx.notify();
    }

    fn step(&mut self, delta: isize, cx: &mut Context<Self>) {
        if let Some(id) = freshkube_ui::table::step(self, delta, cx) {
            self.selected = Some(id);
            freshkube_ui::table::reveal(self, ScrollStrategy::Nearest);
            cx.notify();
        }
    }
}

fn name_in(members: &[EtcdMemberSnapshot], member_id: u64) -> String {
    members
        .iter()
        .find(|member| member.info.id == member_id)
        .map(|member| member.info.hostname.clone())
        .unwrap_or_else(|| format!("{member_id:x}"))
}

fn alarm_text(alarm: &EtcdAlarm) -> String {
    format!("{} on {}", alarm.alarm_type.as_str(), alarm.node)
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
    let (members, statuses, alarms) = example_state(members, statuses);
    Ok(assemble_etcd_health(
        source.inspection_target(),
        members,
        statuses,
        alarms,
        Vec::new(),
    ))
}

/// Debug builds reshape the example for captures with `FRESHKUBE_ETCD`:
/// `lost` (every member answers, none follows a leader), `single` (one
/// member) or `alarms` (five, more than the banner lists).
fn example_state(
    mut members: Vec<EtcdMemberInfo>,
    mut statuses: Vec<EtcdMemberStatus>,
) -> (Vec<EtcdMemberInfo>, Vec<EtcdMemberStatus>, Vec<EtcdAlarm>) {
    let state = if cfg!(debug_assertions) {
        std::env::var("FRESHKUBE_ETCD").ok()
    } else {
        None
    };
    let mut alarms = Vec::new();
    match state.as_deref() {
        Some("lost") => statuses.iter_mut().for_each(|status| status.leader_id = 0),
        Some("single") => {
            members.truncate(1);
            statuses.truncate(1);
            if let Some(status) = statuses.first_mut() {
                status.leader_id = status.member_id;
            }
        }
        Some("alarms") => {
            alarms = statuses
                .iter()
                .map(|status| (status, talos_rs::EtcdAlarmType::NoSpace))
                .chain(
                    statuses
                        .iter()
                        .take(2)
                        .map(|status| (status, talos_rs::EtcdAlarmType::Corrupt)),
                )
                .map(|(status, alarm_type)| EtcdAlarm {
                    node: status.node.clone(),
                    member_id: status.member_id,
                    alarm_type,
                })
                .collect();
        }
        _ => {}
    }
    (members, statuses, alarms)
}
