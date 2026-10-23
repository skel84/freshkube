//! One application's incident timeline (#844): what happened to it in a
//! window, from every source that can say, in one list.
//!
//! The sources are Kargo promotions, Argo CD syncs, Coroot incidents,
//! Deployment rollouts, Kubernetes Warning events, pod restarts and node
//! changes. Each is read on its own and handed to [`derive`] as a
//! [`SourceRead`]: its entries, or why it wasn't read. Nothing is guessed
//! and nothing missing is quiet:
//!
//! - a source that couldn't be read is a chip saying why, and contributes no
//!   quiet hours;
//! - Kubernetes events only go back as far as the cluster keeps them, so a
//!   source with a `kept_from` later than the window's start gets a row
//!   where its entries stop, "not kept by the cluster", and the time before
//!   it never counts as read by that source;
//! - a source read in part, because a cap cut its listing off, says what was
//!   cut and speaks for no quiet hour;
//! - a stretch of hours with no entry is a quiet row that names every
//!   source that was read for it, and the ones that weren't.
//!
//! [`derive`] reads nothing; [`example`] invents the lists for `--fixture`.
//! Times are Unix seconds, as Monitoring's markers are.

pub mod example;
pub mod live;
#[cfg(test)]
mod tests;

/// The windows the tab offers, in hours; the first is the default.
pub const WINDOWS: [u32; 4] = [3, 6, 12, 24];

/// How long a cluster keeps Events unless it is set otherwise (the API
/// server's `--event-ttl`), in seconds.
pub const EVENT_RETENTION: i64 = 3600;

const HOUR: i64 = 3600;

/// Where an entry was read from.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Source {
    Promotions,
    Syncs,
    Incidents,
    Deploys,
    Events,
    Restarts,
    Nodes,
}

impl Source {
    pub const ALL: [Source; 7] = [
        Self::Promotions,
        Self::Syncs,
        Self::Incidents,
        Self::Deploys,
        Self::Events,
        Self::Restarts,
        Self::Nodes,
    ];

    /// The chip's word.
    pub fn label(self) -> &'static str {
        match self {
            Self::Promotions => "Promotions",
            Self::Syncs => "Syncs",
            Self::Incidents => "Incidents",
            Self::Deploys => "Deploys",
            Self::Events => "Events",
            Self::Restarts => "Restarts",
            Self::Nodes => "Nodes",
        }
    }

    /// The tool or object the source is read from.
    pub fn origin(self) -> &'static str {
        match self {
            Self::Promotions => "Kargo Promotions",
            Self::Syncs => "Argo CD Applications",
            Self::Incidents => "Coroot",
            Self::Deploys => "ReplicaSets",
            Self::Events => "Kubernetes Warning events",
            Self::Restarts => "Pod container statuses",
            Self::Nodes => "Node conditions and events",
        }
    }

    /// A stable word for ids.
    pub fn slug(self) -> &'static str {
        match self {
            Self::Promotions => "promotions",
            Self::Syncs => "syncs",
            Self::Incidents => "incidents",
            Self::Deploys => "deploys",
            Self::Events => "events",
            Self::Restarts => "restarts",
            Self::Nodes => "nodes",
        }
    }
}

/// How much an entry matters.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Severity {
    Info,
    Warn,
    Bad,
}

/// One thing a source reported, before it is placed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Item {
    pub at: i64,
    pub severity: Severity,
    pub title: String,
    pub detail: String,
}

/// What one source gave.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SourceRead {
    pub source: Source,
    /// Where it was read: a cluster, a tool.
    pub from: String,
    /// When it was read.
    pub read_at: i64,
    /// Its items, or why it wasn't read.
    pub outcome: Result<Vec<Item>, String>,
    /// The earliest time the source can say anything about, when that is
    /// later than a window may ask for: Events' retention.
    pub kept_from: Option<i64>,
    /// What a cap cut off, when the source was read in part: its entries
    /// are real, but nothing is known of what is missing, so it speaks for
    /// no quiet hour.
    pub cut: Option<String>,
    /// The source only shows what is true now: a container's last
    /// termination, a node's current Ready transition. What it reads can't
    /// say that nothing else happened earlier, so it speaks for no quiet
    /// hour; a quiet row lists it apart.
    pub state_only: bool,
}

impl SourceRead {
    pub fn read(source: Source, from: &str, read_at: i64, items: Vec<Item>) -> Self {
        Self {
            source,
            from: from.to_owned(),
            read_at,
            outcome: Ok(items),
            kept_from: None,
            cut: None,
            state_only: false,
        }
    }

    pub fn not_read(source: Source, from: &str, read_at: i64, why: &str) -> Self {
        Self {
            source,
            from: from.to_owned(),
            read_at,
            outcome: Err(why.to_owned()),
            kept_from: None,
            cut: None,
            state_only: false,
        }
    }

    pub fn kept_from(mut self, at: i64) -> Self {
        self.kept_from = Some(at);
        self
    }

    /// Marks the source as showing the present state only.
    pub fn state_only(mut self) -> Self {
        self.state_only = true;
        self
    }

    /// Marks the source read in part, as a cap left it.
    pub fn cut(mut self, what: &str) -> Self {
        self.cut = Some(what.to_owned());
        self
    }
}

/// An item in the list, with where and when it was read.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    /// Unique within a timeline.
    pub id: String,
    pub at: i64,
    pub source: Source,
    pub severity: Severity,
    pub title: String,
    pub detail: String,
    pub read_from: String,
    pub read_at: i64,
}

/// One source's chip.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Chip {
    pub source: Source,
    pub state: ChipState,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ChipState {
    /// Read: this many entries in the window, and from when the source can
    /// say, when that is after the window's start, and what a cap cut off.
    Read {
        count: usize,
        kept_from: Option<i64>,
        /// What a cap cut off: the count is of what was read.
        cut: Option<String>,
        /// It shows the present state only, not what happened before.
        state_only: bool,
    },
    NotRead(String),
}

/// One line of the list, newest first.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Row {
    Entry(Entry),
    /// The source's entries stop here: before `until` the cluster no longer
    /// has them, back to the window's start `from`.
    NotKept {
        source: Source,
        from: i64,
        until: i64,
    },
    /// Hours with no entry. `read` names each source that covered every
    /// hour of it, `state_only` the ones that read the present state alone
    /// and can't vouch for the hours, `not_read` the others.
    Quiet {
        from: i64,
        until: i64,
        read: Vec<Source>,
        state_only: Vec<Source>,
        not_read: Vec<Source>,
    },
}

impl Row {
    /// The time the row sits at, for the order.
    pub fn at(&self) -> i64 {
        match self {
            Self::Entry(entry) => entry.at,
            Self::NotKept { until, .. } => *until,
            Self::Quiet { until, .. } => *until,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Timeline {
    pub now: i64,
    pub since: i64,
    pub hours: u32,
    pub chips: Vec<Chip>,
    pub rows: Vec<Row>,
}

impl Timeline {
    /// How many entries the window holds, whatever the source.
    pub fn entries(&self) -> usize {
        self.rows
            .iter()
            .filter(|row| matches!(row, Row::Entry(_)))
            .count()
    }
}

/// Merges the sources into one list for the window ending at `now`. A
/// source not among `reads` is not read, said so.
pub fn derive(reads: &[SourceRead], now: i64, hours: u32) -> Timeline {
    let since = now - i64::from(hours) * HOUR;
    let mut chips = Vec::new();
    let mut entries = Vec::new();
    for source in Source::ALL {
        let Some(read) = reads.iter().find(|read| read.source == source) else {
            chips.push(Chip {
                source,
                state: ChipState::NotRead("not asked".into()),
            });
            continue;
        };
        match &read.outcome {
            Err(why) => chips.push(Chip {
                source,
                state: ChipState::NotRead(why.clone()),
            }),
            Ok(items) => {
                let before = entries.len();
                for (ix, item) in items.iter().enumerate() {
                    if item.at < since || item.at > now + 60 {
                        continue;
                    }
                    entries.push(Entry {
                        id: format!("{}-{}-{ix}", source.slug(), item.at),
                        at: item.at,
                        source,
                        severity: item.severity,
                        title: item.title.clone(),
                        detail: item.detail.clone(),
                        read_from: read.from.clone(),
                        read_at: read.read_at,
                    });
                }
                chips.push(Chip {
                    source,
                    state: ChipState::Read {
                        count: entries.len() - before,
                        kept_from: read.kept_from.filter(|at| *at > since),
                        cut: read.cut.clone(),
                        state_only: read.state_only,
                    },
                });
            }
        }
    }
    let mut rows: Vec<Row> = entries.into_iter().map(Row::Entry).collect();
    rows.extend(not_kept(&chips, reads, since));
    rows.extend(quiet(reads, &rows, now, since));
    // Newest first; at one time an entry sits above the marker that
    // explains the time below it.
    rows.sort_by_key(|row| {
        let rank = match row {
            Row::Entry(_) => 0,
            Row::NotKept { .. } => 1,
            Row::Quiet { .. } => 2,
        };
        (std::cmp::Reverse(row.at()), rank)
    });
    Timeline {
        now,
        since,
        hours,
        chips,
        rows,
    }
}

fn not_kept(chips: &[Chip], reads: &[SourceRead], since: i64) -> Vec<Row> {
    chips
        .iter()
        .filter_map(|chip| match chip.state {
            ChipState::Read {
                kept_from: Some(until),
                ..
            } => reads
                .iter()
                .any(|read| read.source == chip.source)
                .then_some(Row::NotKept {
                    source: chip.source,
                    from: since,
                    until,
                }),
            _ => None,
        })
        .collect()
}

/// The hours with no entry, each naming what was read for it; consecutive
/// hours read by the same sources are one row.
fn quiet(reads: &[SourceRead], rows: &[Row], now: i64, since: i64) -> Vec<Row> {
    let mut quiet: Vec<Row> = Vec::new();
    let mut hi = now;
    while hi > since {
        let lo = (hi - HOUR).max(since);
        let busy = rows
            .iter()
            .any(|row| matches!(row, Row::Entry(entry) if entry.at >= lo && (entry.at < hi || hi == now)));
        let (read, state_only, not_read) = coverage(reads, lo);
        if !busy && !read.is_empty() {
            match quiet.last_mut() {
                Some(Row::Quiet {
                    from,
                    read: before,
                    state_only: latest,
                    not_read: missing,
                    ..
                }) if *from == hi
                    && *before == read
                    && *latest == state_only
                    && *missing == not_read =>
                {
                    *from = lo
                }
                _ => quiet.push(Row::Quiet {
                    from: lo,
                    until: hi,
                    read,
                    state_only,
                    not_read,
                }),
            }
        }
        hi = lo;
    }
    quiet
}

/// The sources that can speak for the hour starting at `lo`, the ones that
/// only show the present state, and the rest.
fn coverage(reads: &[SourceRead], lo: i64) -> (Vec<Source>, Vec<Source>, Vec<Source>) {
    let (mut read, mut state_only, mut not_read) = (Vec::new(), Vec::new(), Vec::new());
    for source in Source::ALL {
        let found = reads.iter().find(|read| {
            read.source == source
                && read.outcome.is_ok()
                && read.cut.is_none()
                && read.kept_from.is_none_or(|at| at <= lo)
        });
        match found {
            Some(read) if read.state_only => &mut state_only,
            Some(_) => &mut read,
            None => &mut not_read,
        }
        .push(source);
    }
    (read, state_only, not_read)
}

/// The time of day in UTC, `14:02`, as the list prints it.
pub fn clock(at: i64) -> String {
    chrono::DateTime::from_timestamp(at, 0)
        .map(|time| time.format("%H:%M").to_string())
        .unwrap_or_default()
}

/// A length of time in words: `15 min`, `1 h`, `1 h 30 min`.
pub fn span_words(seconds: i64) -> String {
    let minutes = seconds.max(0) / 60;
    match (minutes / 60, minutes % 60) {
        (0, minutes) => format!("{minutes} min"),
        (hours, 0) => format!("{hours} h"),
        (hours, minutes) => format!("{hours} h {minutes} min"),
    }
}
