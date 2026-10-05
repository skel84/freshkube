//! Cached display order, health tally and groups. Source joins stay role/name ordered.
use super::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Status {
    Failing,
    Warning,
    Unknown,
    Healthy,
}

impl Status {
    pub(super) const ALL: [Self; 4] = [Self::Failing, Self::Warning, Self::Unknown, Self::Healthy];

    pub(super) fn index(self) -> usize {
        self as usize
    }

    pub(super) fn of(row: &NodeRow) -> Self {
        match row.tone {
            ui::Tone::Crit => Self::Failing,
            ui::Tone::Warn => Self::Warning,
            ui::Tone::Good => Self::Healthy,
            _ => Self::Unknown,
        }
    }

    pub(super) fn tone(self) -> ui::Tone {
        match self {
            Self::Failing => ui::Tone::Crit,
            Self::Warning => ui::Tone::Warn,
            Self::Unknown => ui::Tone::Unknown,
            Self::Healthy => ui::Tone::Good,
        }
    }

    pub(super) fn label(self) -> &'static str {
        match self {
            Self::Failing => "Failing",
            Self::Warning => "Warning",
            Self::Unknown => "Unknown",
            Self::Healthy => "Healthy",
        }
    }

    pub(super) fn what(self) -> &'static str {
        match self {
            Self::Failing => "failing nodes",
            Self::Warning => "warning nodes",
            Self::Unknown => "unknown nodes",
            Self::Healthy => "healthy nodes",
        }
    }

    pub(super) fn id(self) -> &'static str {
        match self {
            Self::Failing => "nodes-tally-failing",
            Self::Warning => "nodes-tally-warning",
            Self::Unknown => "nodes-tally-unknown",
            Self::Healthy => "nodes-tally-healthy",
        }
    }

    pub(super) fn group_id(self) -> &'static str {
        match self {
            Self::Failing => "nodes-group-failing",
            Self::Warning => "nodes-group-warning",
            Self::Unknown => "nodes-group-unknown",
            Self::Healthy => "nodes-group-healthy",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Item {
    Group(Status),
    Row(usize),
}

impl Nodes {
    pub(super) fn set_rows(&mut self, mut rows: Vec<NodeRow>, talos_current: bool) {
        for row in &mut rows {
            row.tone = assessed_tone(row, talos_current);
        }
        self.search_keys = rows
            .iter()
            .map(|row| format!("{} {} {}", row.name, row.address, row.role.label()).to_lowercase())
            .collect();
        self.rows = Arc::new(rows);
    }

    pub(super) fn rebuild_lines(&mut self) {
        self.counts = [0; 4];
        self.group_counts = [0; 4];
        self.lines.clear();
        self.items.clear();
        let mut groups: [Vec<usize>; 4] = Default::default();
        for (ix, row) in self.rows.iter().enumerate() {
            if !self.query_text.is_empty()
                && !self
                    .search_keys
                    .get(ix)
                    .is_some_and(|key| key.contains(&self.query_text))
            {
                continue;
            }
            let status = Status::of(row);
            self.counts[status.index()] += 1;
            if self.filter.is_none() || self.filter == Some(status) {
                // Cards preserve the existing role/name order; only the table has groups.
                self.lines.push(ix);
                groups[status.index()].push(ix);
            }
        }
        let problems = self.counts[..Status::Healthy.index()].iter().sum::<usize>() > 0;
        self.healthy_folded = problems
            && !self.healthy_open
            && self.query_text.is_empty()
            && self.filter.is_none()
            && !groups[Status::Healthy.index()].is_empty();
        for status in Status::ALL {
            let rows = &groups[status.index()];
            self.group_counts[status.index()] = rows.len();
            if rows.is_empty() {
                continue;
            }
            self.items.push(Item::Group(status));
            if status != Status::Healthy || !self.healthy_folded {
                self.items.extend(rows.iter().copied().map(Item::Row));
            }
        }
    }

    pub(in crate::desktop) fn healthy_collapsed(&self) -> bool {
        self.healthy_folded
    }

    pub(super) fn show_selected_healthy(&mut self) {
        if self.healthy_collapsed()
            && self
                .row()
                .is_some_and(|row| Status::of(row) == Status::Healthy)
        {
            self.healthy_open = true;
            self.rebuild_lines();
        }
    }
}

pub(super) fn assessed_tone(row: &NodeRow, talos_current: bool) -> ui::Tone {
    // A current negative Ready condition is evidence even when Talos is missing.
    // Otherwise an incomplete assessment cannot establish a healthy node.
    if row.kubernetes_current && row.ready == "NotReady" {
        ui::Tone::Crit
    } else if !talos_current
        || !row.kubernetes_current
        || row.talos.is_none()
        || row.kubernetes.is_none()
    {
        ui::Tone::Unknown
    } else {
        row.tone
    }
}
