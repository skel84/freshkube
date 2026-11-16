//! The Overview matrix's rows and cells, derived from the change and the
//! same rows All hops draws, when they change. One word and a glyph per
//! cell; an unknown cell says its cause. Nothing here runs while drawing.
use freshkube_delivery::change::{Cause, Eligible, NextStep, Promotion};

use super::*;

/// The matrix's step columns, in the order the change travels.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Column {
    Eligible,
    Promoted,
    Verified,
    NextStep,
    Synced,
    Running,
    AfterDeploy,
}

impl Column {
    pub const ALL: [Column; 7] = [
        Self::Eligible,
        Self::Promoted,
        Self::Verified,
        Self::NextStep,
        Self::Synced,
        Self::Running,
        Self::AfterDeploy,
    ];

    /// The header's label: at most 14 characters, like every cell's word.
    pub fn label(self) -> &'static str {
        match self {
            Self::Eligible => "Eligible",
            Self::Promoted => "Promoted",
            Self::Verified => "Verified",
            Self::NextStep => "Next step",
            Self::Synced => "Synced",
            Self::Running => "Running",
            Self::AfterDeploy => "After deploy",
        }
    }

    /// The part of the cell's id after the Stage's.
    pub fn id(self) -> &'static str {
        match self {
            Self::Eligible => "eligible",
            Self::Promoted => "promoted",
            Self::Verified => "verified",
            Self::NextStep => "next-step",
            Self::Synced => "synced",
            Self::Running => "running",
            Self::AfterDeploy => "after-deploy",
        }
    }

    /// The cell's word when all is well.
    fn ok(self) -> &'static str {
        match self {
            Self::Eligible => "Eligible",
            Self::Promoted => "Promoted",
            Self::Verified => "Verified",
            Self::NextStep => "Done",
            Self::Synced => "Synced",
            Self::Running => "Running",
            Self::AfterDeploy => "Healthy",
        }
    }
}

/// One cell: its glyph's tone, its word, and the hop it stands for.
#[derive(Clone, Debug, PartialEq)]
pub struct Cell {
    pub tone: Tone,
    pub word: SharedString,
    /// The hop a click selects; `None` when the change has none for it.
    pub hop: Option<SharedString>,
    /// Whether the change got as far as this cell: an unreached one is a
    /// hairline track, not a problem.
    pub reached: bool,
    /// How sure the link into the hop is, when the hop has one (a gate and
    /// Coroot's verdict have none).
    pub link: Option<Confidence>,
    /// The Stage isn't expected to have this step (Not used here): dim, no
    /// track of its own, counted nowhere. A column where every Stage's cell
    /// is not used is hidden ([`visible`]).
    pub not_used: bool,
    pub tooltip: SharedString,
}

impl Cell {
    /// The style of the track into the cell: the confidence of the link into
    /// it (solid Confirmed, dashed Claimed, dotted Unknown), else, for a cell
    /// with no link of its own, its status; a hairline for a step the change
    /// has not reached. Status is the glyph's, confidence the track's.
    pub fn track(&self) -> kit::Track {
        if !self.reached {
            return kit::Track::Hairline;
        }
        match self.link {
            Some(Confidence::Confirmed) => kit::Track::Solid,
            Some(Confidence::Claimed) => kit::Track::Dashed,
            Some(Confidence::Unknown) => kit::Track::Dotted,
            None => kit::Track::into_tone(self.tone),
        }
    }
}

/// One Stage's row.
#[derive(Clone, Debug, PartialEq)]
pub struct StageRow {
    pub name: SharedString,
    pub cluster: SharedString,
    /// The muted second line: since when, or what it still runs.
    pub line2: SharedString,
    pub cells: [Cell; 7],
    /// The Stage's group in the change, where All hops folds it.
    pub group: usize,
    /// Every hop of the Stage, for the selection to find its row.
    pub hops: Vec<SharedString>,
    /// The rail's node: the weakest of its cells.
    pub tone: Tone,
    /// The Stage waits for promotion, so the rail into it is dotted.
    pub waiting: bool,
    /// Where the row sits in the rail, and the rail's segments into it
    /// and out of it.
    pub at: kit::RailAt,
    pub rail: [kit::Track; 2],
}

/// The rows, one per Stage in promotion order.
pub fn derive(change: &Change, rows: &[HopRow]) -> Vec<StageRow> {
    let count = change.stages.len();
    let mut stages: Vec<StageRow> = change
        .stages
        .iter()
        .enumerate()
        .map(|(ix, stage)| {
            let mut row = stage_row(change, rows, ix, stage);
            row.at = kit::RailAt::of(ix, count);
            row
        })
        .collect();
    // A segment of the rail takes the style of the Stage it runs into.
    let into: Vec<kit::Track> = stages
        .iter()
        .map(|row| {
            if row.waiting {
                kit::Track::Dotted
            } else {
                kit::Track::Solid
            }
        })
        .collect();
    for (ix, row) in stages.iter_mut().enumerate() {
        row.rail = [
            into[ix],
            into.get(ix + 1).copied().unwrap_or(kit::Track::None),
        ];
    }
    // A hidden column has no cell to select or ring: its hops belong to
    // their Stage's row.
    let shown = visible(&stages);
    for row in &mut stages {
        for (column, cell) in Column::ALL.into_iter().zip(&mut row.cells) {
            if !shown.contains(&column) {
                cell.hop = None;
            }
        }
    }
    stages
}

/// The columns the matrix draws: all of them, but one where every Stage's
/// cell is Not used here, which is hidden (with no Stage at all, nothing is
/// hidden, so the loading rows have every column).
pub fn visible<'a>(rows: impl IntoIterator<Item = &'a StageRow> + Clone) -> Vec<Column> {
    let none = rows.clone().into_iter().next().is_none();
    Column::ALL
        .into_iter()
        .zip(0..)
        .filter(|(_, at)| none || rows.clone().into_iter().any(|row| !row.cells[*at].not_used))
        .map(|(column, _)| column)
        .collect()
}

fn stage_row(change: &Change, rows: &[HopRow], ix: usize, stage: &Stage) -> StageRow {
    let group = change
        .groups
        .iter()
        .position(|group| group.phase == Phase::Stage(ix))
        .unwrap_or_default();
    let of = |key: String| rows.iter().find(|row| row.key.as_ref() == key);
    let kind = |kind: &str, last: bool| {
        let mut found = change
            .hops
            .iter()
            .zip(rows)
            .filter(|(hop, _)| {
                hop.group == group && matches!(&hop.shows, Shows::Hop(d) if d.kind == kind)
            })
            .map(|(_, row)| row);
        if last {
            found.next_back()
        } else {
            found.next()
        }
    };
    let reached = !matches!(stage.reach, Reach::NotReached) || stage.running.is_none();
    let gate = |column: Column, suffix: &str| {
        cell(column, of(format!("{}-{suffix}", stage.name)), true, stage)
    };
    let cells = [
        gate(Column::Eligible, "eligible"),
        gate(Column::Promoted, "promotion"),
        gate(Column::Verified, "verification"),
        next_step_cell(kind("Argo Rollout", false), reached, stage),
        cell(
            Column::Synced,
            kind("Argo CD Application", false),
            reached,
            stage,
        ),
        cell(Column::Running, kind("Pods", true), reached, stage),
        cell(
            Column::AfterDeploy,
            kind("Deployment", true),
            reached,
            stage,
        ),
    ];
    let tone = cells
        .iter()
        .filter(|cell| cell.reached)
        .map(|cell| status(cell.tone))
        .max_by_key(|tone| rank(*tone))
        .unwrap_or(Tone::Outline);
    StageRow {
        name: stage.name.clone().into(),
        cluster: stage
            .cluster
            .cluster()
            .unwrap_or("cluster unknown")
            .to_owned()
            .into(),
        line2: line2(stage).into(),
        cells,
        group,
        hops: change
            .hops
            .iter()
            .filter(|hop| hop.group == group)
            .map(|hop| hop.key.clone().into())
            .collect(),
        tone,
        waiting: matches!(stage.promotion, Promotion::Waiting | Promotion::NotYet),
        at: kit::RailAt::Only,
        rail: [kit::Track::None; 2],
    }
}

/// The Next step cell: where the Stage's Rollout is, from the hop the
/// Rollout stands for when the change has one. A Rollout that couldn't be
/// read says its cause, never blank; a Stage with none is a hairline.
fn next_step_cell(row: Option<&HopRow>, reached: bool, stage: &Stage) -> Cell {
    let next = &stage.next_step;
    let hop = row.map(|row| row.key.clone());
    let label = Column::NextStep.label();
    // A Stage with no Rollout isn't expected to have a next step.
    if matches!(next, NextStep::NoRollout) {
        return Cell {
            tone: Tone::Outline,
            word: next.word().into(),
            hop,
            reached: false,
            link: None,
            not_used: true,
            tooltip: format!("{label}: {}", next.detail()).into(),
        };
    }
    if !reached && !matches!(next, NextStep::Unknown { .. }) {
        return Cell {
            tone: Tone::Outline,
            word: "Not reached".into(),
            hop,
            reached: false,
            link: None,
            not_used: false,
            tooltip: format!("{label}: {} still runs other Freight", stage.name).into(),
        };
    }
    let tone = match next {
        NextStep::Done => Tone::Good,
        NextStep::Step { .. } | NextStep::Paused { .. } => Tone::Info,
        NextStep::Progressing => Tone::Waiting,
        NextStep::Aborted => Tone::Crit,
        NextStep::Unknown { .. } | NextStep::NoRollout => Tone::Unknown,
    };
    Cell {
        tone,
        word: next.word().into(),
        hop,
        reached: true,
        link: None,
        not_used: false,
        tooltip: format!("{label}: {}", next.detail()).into(),
    }
}

/// The Stage cell's second line: since when its Freight runs there, or
/// the Freight it still runs.
fn line2(stage: &Stage) -> String {
    if let Some(running) = &stage.running {
        return format!("still runs {running}");
    }
    let at = match &stage.promotion {
        Promotion::Automatic { at } | Promotion::ByHand { at } | Promotion::Other { at, .. } => *at,
        _ => None,
    };
    match at {
        Some(at) => format!("since {}", clock(at)),
        None => match stage.eligible {
            Eligible::Verified { .. } | Eligible::Approved { .. } => "not promoted yet".into(),
            _ => "not eligible yet".into(),
        },
    }
}

/// A cell from the hop it stands for, if the change has one.
fn cell(column: Column, row: Option<&HopRow>, reached: bool, stage: &Stage) -> Cell {
    let Some(row) = row else {
        return Cell {
            tone: Tone::Outline,
            word: "Not reached".into(),
            hop: None,
            reached: false,
            link: None,
            not_used: false,
            tooltip: format!("{}: the change hasn't got this far", column.label()).into(),
        };
    };
    // A step the application isn't expected to have is dim and counted
    // nowhere, whatever the Stage runs.
    if row
        .cause
        .as_ref()
        .is_some_and(|cause| cause.is_not_used_here())
    {
        return Cell {
            tone: Tone::Outline,
            word: Cause::not_used_here("").word().into(),
            hop: Some(row.key.clone()),
            reached: false,
            link: None,
            not_used: true,
            tooltip: format!("{}: {}", column.label(), row.detail).into(),
        };
    }
    // A Stage that still runs other Freight says nothing of this change
    // downstream, unless something there couldn't be read or went wrong.
    let own = row.cause.is_some() || matches!(row.tone, Tone::Crit | Tone::Warn);
    if !reached && !own {
        return Cell {
            tone: Tone::Outline,
            word: "Not reached".into(),
            hop: Some(row.key.clone()),
            reached: false,
            link: None,
            not_used: false,
            tooltip: format!(
                "{}: {} still runs other Freight",
                column.label(),
                stage.name
            )
            .into(),
        };
    }
    let (tone, word) = if column == Column::AfterDeploy {
        match (&row.coroot, row.coroot_why) {
            (Some(mark), _) => (mark.tone, mark_word(mark.tone)),
            // Coroot answered with a cause: it is not set up, or the piece
            // isn't there, or it couldn't be read. Never a failure.
            (None, Some(why)) => (Tone::Unknown, why),
            (None, None) => (Tone::Outline, "Not looked"),
        }
    } else {
        (row.tone, word(column, row))
    };
    let tooltip = match (&row.coroot, column) {
        (Some(mark), Column::AfterDeploy) => mark.tooltip.clone(),
        _ => format!("{}: {}", column.label(), row.detail).into(),
    };
    Cell {
        tone,
        word: word.into(),
        hop: Some(row.key.clone()),
        reached: column != Column::AfterDeploy || row.coroot.is_some() || row.coroot_why.is_some(),
        link: if column == Column::AfterDeploy {
            None
        } else {
            row.link
        },
        not_used: false,
        tooltip,
    }
}

/// Coroot's verdict as a word.
fn mark_word(tone: Tone) -> &'static str {
    match tone {
        Tone::Good => "Healthy",
        Tone::Crit | Tone::Died => "Worse",
        Tone::Warn => "Needs a look",
        Tone::Info => "To know",
        _ => "Unknown",
    }
}

/// A cell's word: its cause's when it has one, else from its tone.
fn word(column: Column, row: &HopRow) -> &'static str {
    match (&row.cause, row.tone) {
        (Some(cause), _) => cause.word(),
        (None, Tone::Good) => column.ok(),
        (None, Tone::Info) => match column {
            Column::Eligible => "Approved",
            _ => "Claimed",
        },
        (None, Tone::Waiting) => "Waiting",
        (None, Tone::Warn) => "Needs a look",
        (None, Tone::Crit | Tone::Died) => "Failed",
        (None, _) => "Unknown",
    }
}

/// Where a hop is in the matrix: its Stage's row and, when a cell stands
/// for it, the column.
pub fn locate(rows: &[StageRow], key: &str) -> Option<(usize, Option<Column>)> {
    rows.iter().enumerate().find_map(|(ix, row)| {
        let cell = Column::ALL
            .into_iter()
            .zip(&row.cells)
            .find(|(_, cell)| cell.hop.as_deref() == Some(key))
            .map(|(column, _)| column);
        (cell.is_some() || row.hops.iter().any(|hop| hop.as_ref() == key)).then_some((ix, cell))
    })
}
