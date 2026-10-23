//! Prepared rows, groups and sentences for the Routes page; render does no
//! collection work.
use super::*;
use crate::resources::route_trace::{TraceModel, verdict_tone};
use freshkube_core::resources::traffic::routes::{RouteEntry, RouteVerdict, RoutesRead};

pub(crate) struct Field {
    pub label: &'static str,
    pub value: SharedString,
}

pub(crate) struct Row {
    pub key: SharedString,
    pub id: SharedString,
    pub label: SharedString,
    /// Route, Name, Gateway and Status, after the glyph.
    pub cells: [SharedString; 4],
    pub tone: ui::Tone,
    pub verdict: RouteVerdict,
    pub namespace: String,
    pub name: String,
    pub uid: String,
    /// Everything the filter matches, lowercased.
    pub search: String,
    pub model: TraceModel,
    pub fields: Vec<Field>,
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
    /// What the table draws: rows and group headers, filtered. A cluster
    /// where every route serves gets a flat list, with no group rows.
    pub lines: Vec<Entry>,
    /// "14 routes".
    pub summary: SharedString,
    /// "5 not serving · 1 partly serving", when some are.
    pub attention: Option<SharedString>,
    /// Reads that were refused or cut, and a bundle installed in pieces.
    pub notices: Vec<SharedString>,
    /// Why there is no table: no API, no routes.
    pub empty: SharedString,
    /// The API itself can't be traced (not installed, partly installed).
    pub api_problem: bool,
    pub columns: (Vec<table::Column>, f32),
    filter: String,
    /// The group of each row's verdict, before filtering.
    flat: bool,
}

impl Default for Display {
    fn default() -> Self {
        Self::new(None)
    }
}

fn plural(count: usize, one: &str, many: &str) -> String {
    format!("{count} {}", if count == 1 { one } else { many })
}

fn search_text(entry: &RouteEntry, summary: &str) -> String {
    let facts = &entry.facts;
    [
        facts.label().as_str(),
        facts.namespace.as_str(),
        facts.name.as_str(),
        &facts.hostnames.join(" "),
        &entry
            .trace
            .parents
            .iter()
            .map(|p| p.label.as_str())
            .collect::<Vec<_>>()
            .join(" "),
        summary,
        entry.trace.verdict.label(),
    ]
    .join(" ")
    .to_lowercase()
}

fn row(ix: usize, entry: &RouteEntry) -> Row {
    let facts = &entry.facts;
    let trace = &entry.trace;
    let summary = trace.summary.line();
    let parents = if facts.parents.is_empty() {
        "no parent".to_owned()
    } else {
        facts
            .parents
            .iter()
            .map(|p| p.label(&facts.namespace))
            .collect::<Vec<_>>()
            .join(", ")
    };
    let id = format!("routes-row-{}-{}", facts.namespace, facts.name);
    let paths = facts
        .rules
        .iter()
        .flat_map(|rule| rule.paths.iter().cloned())
        .collect::<Vec<_>>()
        .join(", ");
    let model = TraceModel::new(entry, &format!("routes-trace-{ix}"));
    Row {
        key: format!("{}/{}", facts.namespace, facts.name).into(),
        label: facts.label().into(),
        cells: [
            facts.label().into(),
            format!("{}/{}", facts.namespace, facts.name).into(),
            parents.clone().into(),
            summary.clone().into(),
        ],
        tone: verdict_tone(trace.verdict),
        verdict: trace.verdict,
        namespace: facts.namespace.clone(),
        name: facts.name.clone(),
        uid: facts.uid.clone(),
        search: search_text(entry, &summary),
        fields: vec![
            Field {
                label: "Route",
                value: format!("{}/{}", facts.namespace, facts.name).into(),
            },
            Field {
                label: "Hosts",
                value: if facts.hostnames.is_empty() {
                    "any host".into()
                } else {
                    facts.hostnames.join(", ").into()
                },
            },
            Field {
                label: "Paths",
                value: paths.into(),
            },
            Field {
                label: "Parents",
                value: parents.into(),
            },
        ],
        model,
        id: id.into(),
    }
}

impl Display {
    pub(super) fn new(read: Option<&RoutesRead>) -> Self {
        let mut display = Self {
            rows: Vec::new(),
            groups: Vec::new(),
            lines: Vec::new(),
            summary: "".into(),
            attention: None,
            notices: Vec::new(),
            empty: "".into(),
            api_problem: false,
            columns: (Vec::new(), 0.),
            filter: String::new(),
            flat: true,
        };
        let Some(read) = read else {
            return display;
        };
        if let Some(problem) = read.api.problem() {
            display.empty = problem.into();
            display.api_problem = true;
            return display;
        }
        display.notices = read.notes.iter().cloned().map(SharedString::from).collect();
        if let Some(mixed) = read.api.mixed() {
            display.notices.push(
                format!("Mixed Gateway API bundle: {mixed}. Each kind is read at its own version.")
                    .into(),
            );
        }
        let mut rows = read
            .routes
            .iter()
            .enumerate()
            .map(|(ix, entry)| row(ix, entry))
            .collect::<Vec<_>>();
        // Not serving first, then by name.
        rows.sort_by(|a, b| (a.verdict, &a.key).cmp(&(b.verdict, &b.key)));
        let count = |verdict| rows.iter().filter(|r| r.verdict == verdict).count();
        display.groups = RouteVerdict::ALL
            .iter()
            .filter(|verdict| count(**verdict) > 0)
            .map(|verdict| GroupLine {
                slug: match verdict {
                    RouteVerdict::NotServing => "not-serving",
                    RouteVerdict::Partly => "partly-serving",
                    RouteVerdict::Unknown => "unknown",
                    RouteVerdict::Serving => "serving",
                    RouteVerdict::Mesh => "mesh",
                },
                label: verdict.label(),
                tone: verdict_tone(*verdict),
                count: count(*verdict),
            })
            .collect();
        display.flat = rows.iter().all(|r| r.verdict == RouteVerdict::Serving);
        let attention = [
            (RouteVerdict::NotServing, "not serving"),
            (RouteVerdict::Partly, "partly serving"),
            (RouteVerdict::Unknown, "unknown"),
        ]
        .into_iter()
        .filter(|(verdict, _)| count(*verdict) > 0)
        .map(|(verdict, words)| format!("{} {words}", count(verdict)))
        .collect::<Vec<_>>();
        display.attention = (!attention.is_empty()).then(|| attention.join(" · ").into());
        display.summary = plural(rows.len(), "route", "routes").into()
            ;
        display.empty = "There are no HTTPRoutes in this cluster.".into();
        display.rows = rows;
        display.columns = table::columns(&display.rows);
        display.build_lines();
        display
    }

    pub(super) fn set_filter(&mut self, text: &str) {
        let text = text.trim().to_lowercase();
        if text == self.filter {
            return;
        }
        self.filter = text;
        self.build_lines();
    }

    fn build_lines(&mut self) {
        self.lines.clear();
        let shown = |row: &Row| self.filter.is_empty() || row.search.contains(&self.filter);
        let mut lines = Vec::new();
        for (group_ix, group) in self.groups.iter().enumerate() {
            let rows = self
                .rows
                .iter()
                .enumerate()
                .filter(|(_, r)| r.verdict.label() == group.label && shown(r))
                .map(|(ix, _)| Entry::Row(ix))
                .collect::<Vec<_>>();
            if rows.is_empty() {
                continue;
            }
            if !self.flat {
                lines.push(Entry::Group(group_ix));
            }
            lines.extend(rows);
        }
        self.lines = lines;
    }

    /// Whether a filter hides every row.
    pub(super) fn filtered_out(&self) -> bool {
        !self.rows.is_empty() && self.lines.is_empty()
    }
}
