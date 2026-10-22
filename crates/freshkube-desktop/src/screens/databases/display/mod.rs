//! Prepared rows, groups, inspector sections and state sentences; render
//! does no collection work.
mod cluster;
mod format;
mod other;

use super::*;
use format::{plural, time};
use freshkube_core::databases::coroot::Evidence;
use freshkube_core::databases::{Coverage, Fact, Identity};

const MAX_DETAIL_FIELDS: usize = 128;

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Mode {
    Clusters,
    Backups,
    Schedules,
}
impl Mode {
    pub(super) fn label(self) -> &'static str {
        match self {
            Self::Clusters => "Clusters",
            Self::Backups => "Backups",
            Self::Schedules => "Schedules",
        }
    }
    fn singular(self) -> &'static str {
        match self {
            Self::Clusters => "cluster",
            Self::Backups => "backup",
            Self::Schedules => "schedule",
        }
    }
}

pub(crate) struct Field {
    pub label: String,
    pub value: String,
    pub monospace: bool,
    pub tone: Option<ui::Tone>,
    /// Drawn dimmed: an answer that is shown but not used.
    pub dim: bool,
}
impl Field {
    pub(super) fn prose(label: &str, value: impl Into<String>) -> Self {
        Self {
            label: label.into(),
            value: value.into(),
            monospace: false,
            tone: None,
            dim: false,
        }
    }
    pub(super) fn mono(label: &str, value: impl Into<String>) -> Self {
        Self {
            monospace: true,
            ..Self::prose(label, value)
        }
    }
    pub(super) fn fact<T: std::fmt::Display>(label: &str, value: &Fact<T>) -> Self {
        Self {
            monospace: value.known().is_some(),
            ..Self::prose(label, format::fact(value))
        }
    }
    pub(super) fn time(label: &str, value: Option<chrono::DateTime<Utc>>) -> Self {
        Self {
            monospace: value.is_some(),
            ..Self::prose(label, time(value))
        }
    }
    pub(super) fn tone(mut self, tone: ui::Tone) -> Self {
        self.tone = Some(tone);
        self
    }
    pub(super) fn dimmed(mut self) -> Self {
        self.dim = true;
        self
    }
}

/// A captioned group of fields in the Inspector.
pub(crate) struct Section {
    pub id: &'static str,
    pub title: &'static str,
    pub fields: Vec<Field>,
}
impl Section {
    pub(super) fn new(id: &'static str, title: &'static str, fields: Vec<Field>) -> Self {
        Self { id, title, fields }
    }
}

/// What the Inspector opens with: the state in a sentence, why, and next.
pub(crate) struct Verdict {
    pub tone: ui::Tone,
    pub headline: SharedString,
    /// Operator, Coroot or Both; none while the state is unknown.
    pub source: Option<&'static str>,
    pub why: Vec<SharedString>,
    pub next: Vec<SharedString>,
}

/// Where Open in Observability goes.
#[derive(Clone)]
pub(crate) struct Observe {
    pub app: freshkube_core::coroot::AppId,
    pub tab: freshkube_nav::ObservabilityTab,
}

pub(crate) struct Row {
    pub key: SharedString,
    pub id: SharedString,
    pub label: SharedString,
    pub identity: Identity,
    /// One text per logical column, after the glyph.
    pub cells: Vec<SharedString>,
    /// A cell's tooltip when it differs from its text.
    pub tips: Vec<Option<SharedString>>,
    pub tone: ui::Tone,
    /// The Operator / Coroot / Both chip of the State column.
    pub chip: Option<&'static str>,
    /// Index into [`Display::groups`], for the Clusters view.
    pub group: Option<usize>,
    /// Everything the filter matches, lowercased.
    pub search: String,
    /// The engine, for the Inspector's header.
    pub engine: SharedString,
    pub verdict: Verdict,
    pub sections: Vec<Section>,
    pub observe: Option<Observe>,
}

pub(crate) struct GroupLine {
    pub slug: &'static str,
    pub label: &'static str,
    pub tone: ui::Tone,
    pub count: usize,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Entry {
    Row(usize),
    Group(usize),
}

pub(super) struct Display {
    pub rows: Vec<Row>,
    pub groups: Vec<GroupLine>,
    /// What the table draws: rows and group headers, filtered.
    pub lines: Vec<Entry>,
    pub summary: SharedString,
    /// One line per kind or operator that was not read completely.
    pub notice: Option<SharedString>,
    pub empty: SharedString,
    pub columns: (Vec<table::Column>, f32),
    mode: Mode,
    filter: String,
    engine_shown: bool,
}
impl Default for Display {
    fn default() -> Self {
        Self::new(None, &Evidence::default(), Mode::Clusters, Utc::now())
    }
}

/// The coverage banner: one line per state, each naming what it is about.
fn notices(inv: &Inventory) -> Vec<String> {
    let mut lines = Vec::new();
    if inv.discovery == Coverage::NotServed {
        lines.push("CloudNativePG and the MariaDB operator: API not served".to_owned());
    }
    for (engine, coverage) in &inv.engines {
        if *coverage != Coverage::NotServed && *coverage != Coverage::Complete {
            lines.push(format!(
                "{}: {}{}",
                engine.label(),
                coverage.label(),
                if inv.retained.contains(engine) {
                    " · last read retained"
                } else {
                    ""
                }
            ));
        }
    }
    for (name, coverage, stale, at) in [
        (
            "Clusters",
            &inv.databases.coverage,
            inv.databases.stale,
            inv.databases.observed_at,
        ),
        (
            "Backups",
            &inv.backups.coverage,
            inv.backups.stale,
            inv.backups.observed_at,
        ),
        (
            "Schedules",
            &inv.schedules.coverage,
            inv.schedules.stale,
            inv.schedules.observed_at,
        ),
    ] {
        if *coverage != Coverage::Complete && *coverage != Coverage::NotServed {
            lines.push(format!(
                "{name}: {}{}",
                coverage.label(),
                if stale {
                    format!(" · last read {}", time(Some(at)))
                } else {
                    String::new()
                }
            ));
        }
    }
    lines
}

fn empty_words(inv: &Inventory, mode: Mode) -> String {
    let coverage = match mode {
        Mode::Clusters => &inv.databases.coverage,
        Mode::Backups => &inv.backups.coverage,
        Mode::Schedules => &inv.schedules.coverage,
    };
    if inv.discovery == Coverage::NotServed {
        "No database operator is installed (CloudNativePG and MariaDB APIs not served)".into()
    } else if *coverage == Coverage::Complete || *coverage == Coverage::NotServed {
        format!("No {} found in this scope", plural_word(mode))
    } else {
        format!("{}: {} · contents unknown", mode.label(), coverage.label())
    }
}
fn plural_word(mode: Mode) -> &'static str {
    match mode {
        Mode::Clusters => "clusters",
        Mode::Backups => "backups",
        Mode::Schedules => "schedules",
    }
}

impl Display {
    pub(super) fn new(
        inventory: Option<&Inventory>,
        evidence: &Evidence,
        mode: Mode,
        now: chrono::DateTime<Utc>,
    ) -> Self {
        let mut rows = Vec::new();
        let mut notice = Vec::new();
        let mut empty = "Not connected".to_owned();
        if let Some(inv) = inventory {
            notice = notices(inv);
            empty = empty_words(inv, mode);
            rows = match mode {
                Mode::Clusters => cluster::rows(inv, evidence, now),
                Mode::Backups => other::backups(inv),
                Mode::Schedules => other::schedules(inv),
            };
        }
        rows.sort_by(|a, b| {
            (a.group, &a.identity.namespace, &a.identity.name).cmp(&(
                b.group,
                &b.identity.namespace,
                &b.identity.name,
            ))
        });
        let mut display = Self {
            rows,
            groups: Vec::new(),
            lines: Vec::new(),
            summary: SharedString::default(),
            notice: (!notice.is_empty()).then(|| notice.join(" · ").into()),
            empty: empty.into(),
            columns: (Vec::new(), 0.),
            mode,
            filter: String::new(),
            engine_shown: true,
        };
        display.arrange();
        display
    }

    /// Narrows the rows to those whose words hold `filter`.
    pub(super) fn set_filter(&mut self, filter: &str) {
        let filter = filter.trim().to_lowercase();
        if filter != self.filter {
            self.filter = filter;
            self.arrange();
        }
    }

    /// Whether the Engine column fits; the Inspector's header says it when
    /// it does not.
    pub(super) fn set_engine_shown(&mut self, shown: bool) {
        if shown != self.engine_shown {
            self.engine_shown = shown;
            self.columns = table::columns(self.mode, self.engine_shown, &self.rows);
        }
    }
    pub(super) fn engine_shown(&self) -> bool {
        self.engine_shown
    }

    /// Lines, groups, the summary and the columns, from the rows and the
    /// filter.
    fn arrange(&mut self) {
        let matching: Vec<usize> = (0..self.rows.len())
            .filter(|&ix| self.filter.is_empty() || self.rows[ix].search.contains(&self.filter))
            .collect();
        self.groups.clear();
        self.lines.clear();
        if self.mode == Mode::Clusters {
            let mut open: Option<usize> = None;
            for &ix in &matching {
                let group = self.rows[ix].group.unwrap_or(0);
                if open != Some(group) {
                    open = Some(group);
                    self.groups.push(cluster::group_line(group));
                    self.lines.push(Entry::Group(self.groups.len() - 1));
                }
                if let Some(line) = self.groups.last_mut() {
                    line.count += 1;
                }
                self.lines.push(Entry::Row(ix));
            }
        } else {
            self.lines.extend(matching.iter().map(|&ix| Entry::Row(ix)));
        }
        let needs = matching
            .iter()
            .filter(|&&ix| self.rows[ix].group == Some(cluster::NEEDS_A_LOOK))
            .count();
        let mut summary = plural(matching.len(), self.mode.singular());
        if self.mode == Mode::Clusters && needs > 0 {
            summary.push_str(&format!(" · {needs} need a look"));
        }
        if self.notice.is_some() {
            summary.push_str(" · coverage incomplete");
        }
        self.summary = summary.into();
        self.columns = table::columns(self.mode, self.engine_shown, &self.rows);
    }
}

impl Row {
    pub(super) fn finish(mut self) -> Self {
        if self.sections.iter().map(|s| s.fields.len()).sum::<usize>() > MAX_DETAIL_FIELDS {
            let mut left = MAX_DETAIL_FIELDS;
            let mut omitted = 0;
            for section in &mut self.sections {
                let keep = section.fields.len().min(left);
                omitted += section.fields.len() - keep;
                section.fields.truncate(keep);
                left -= keep;
            }
            self.sections.push(Section::new(
                "more",
                "More evidence",
                vec![Field::prose(
                    "Not shown",
                    format!(
                        "{omitted} additional fields are not shown; open the object in Resources for its full status"
                    ),
                )],
            ));
        }
        self
    }
}

pub(super) fn identity_key(identity: &Identity) -> (SharedString, SharedString) {
    (
        format!(
            "{}/{}/{}/{}",
            identity.group, identity.namespace, identity.kind, identity.uid
        )
        .into(),
        format!(
            "databases-row-{}-{}-{}",
            identity.kind, identity.namespace, identity.uid
        )
        .into(),
    )
}
