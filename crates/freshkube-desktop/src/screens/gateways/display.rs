//! Prepared rows, groups and sentences for the Gateways and GatewayClasses
//! pages; render does no collection work.
use super::*;
use crate::resources::route_trace::verdict_tone;
use freshkube_core::resources::traffic::routes::{
    ClassEntry, ClassState, GatewayEntry, GatewayState, GatewaysView, RoutesRead,
};

pub(crate) struct Field {
    pub label: &'static str,
    pub value: SharedString,
}

/// The most listeners, routes or Gateways the Inspector lists; the rest are
/// counted.
pub(crate) const DETAIL_LINES: usize = 50;

/// One listener's lines, prepared when the read lands.
pub(crate) struct ListenerBlock {
    pub lines: Vec<String>,
}

pub(crate) struct RouteBlock {
    pub tone: ui::Tone,
    pub state: &'static str,
    pub text: String,
}

pub(crate) struct GatewayDetail {
    pub state: &'static str,
    pub reason: String,
    /// Whose problem it is, in words.
    pub owner: Option<&'static str>,
    /// What the controller reports, kept apart.
    pub says: Option<String>,
    pub listeners: Vec<ListenerBlock>,
    pub listeners_more: usize,
    pub routes_title: String,
    pub routes: Vec<RouteBlock>,
    pub routes_more: usize,
    pub contested: Vec<String>,
    pub contested_more: usize,
}

pub(crate) struct ClassDetail {
    pub state: &'static str,
    pub reason: String,
    pub says: Option<String>,
    /// `(line, state, tone)`: the line is `key · state`.
    pub gateways: Vec<(String, &'static str, ui::Tone)>,
    pub gateways_more: usize,
}

/// What a row's Inspector draws, as strings made once per read.
pub(crate) enum Detail {
    Gateway(Box<GatewayDetail>),
    Class(Box<ClassDetail>),
}

fn owner_words(
    owner: Option<freshkube_core::resources::traffic::routes::Owner>,
) -> Option<&'static str> {
    use freshkube_core::resources::traffic::routes::Owner;
    owner.map(|owner| match owner {
        Owner::Platform => "the platform's",
        Owner::Yours => "yours",
        Owner::Controller => "the controller's",
        Owner::ThisMachine => "this machine's",
    })
}

fn more(total: usize) -> usize {
    total.saturating_sub(DETAIL_LINES)
}

fn gateway_detail(entry: &GatewayEntry) -> GatewayDetail {
    GatewayDetail {
        state: entry.state.label(),
        reason: entry.reason.clone(),
        owner: owner_words(entry.owner),
        says: entry.says.clone(),
        listeners: entry
            .listeners
            .iter()
            .take(DETAIL_LINES)
            .map(|l| {
                let host = l.hostname.as_deref().unwrap_or("any host");
                let attached = match l.attached {
                    Some(n) => format!("{n} attached"),
                    None => "no status from the controller".to_owned(),
                };
                let mut lines = vec![
                    format!("{} · {}/{} · {host}", l.name, l.port, l.protocol),
                    format!("allows {}", l.allows),
                    format!("{} · {attached}", l.state.label()),
                ];
                if let Some(says) = &l.says {
                    lines.push(format!("controller: {says}"));
                }
                if !l.certificates.is_empty() {
                    lines.push(format!(
                        "certificate {} (by name only)",
                        l.certificates.join(", ")
                    ));
                }
                ListenerBlock { lines }
            })
            .collect(),
        listeners_more: more(entry.listeners.len()),
        routes_title: format!("Routes · {}", entry.routes_line()),
        routes: entry
            .routes
            .iter()
            .take(DETAIL_LINES)
            .map(|r| RouteBlock {
                tone: verdict_tone(r.verdict),
                state: r.verdict.label(),
                text: format!("{}/{} — {}", r.namespace, r.name, r.line),
            })
            .collect(),
        routes_more: more(entry.routes.len()),
        contested: entry
            .contested
            .iter()
            .take(DETAIL_LINES)
            .map(|c| format!("{} is also named by {}", c.host, c.also.join(", ")))
            .collect(),
        contested_more: more(entry.contested.len()),
    }
}

fn class_detail(entry: &ClassEntry) -> ClassDetail {
    ClassDetail {
        state: entry.state.label(),
        reason: entry.reason.clone(),
        says: entry.says.clone(),
        gateways: entry
            .gateways
            .iter()
            .take(DETAIL_LINES)
            .map(|(key, state)| {
                (
                    format!("{key} · {}", state.label()),
                    state.label(),
                    gateway_tone(*state),
                )
            })
            .collect(),
        gateways_more: more(entry.gateways.len()),
    }
}

pub(crate) struct Row {
    pub key: SharedString,
    pub id: SharedString,
    pub label: SharedString,
    /// The cells after the glyph, in the mode's column order.
    pub cells: Vec<SharedString>,
    pub tone: ui::Tone,
    /// Words for the glyph's tooltip and label.
    pub state: &'static str,
    /// Which group the row is in.
    pub group: usize,
    pub namespace: String,
    pub name: String,
    /// The object's UID, for a link on a live cluster.
    pub uid: String,
    /// There is an object to open.
    pub openable: bool,
    /// Everything the filter matches, lowercased.
    pub search: String,
    pub fields: Vec<Field>,
    pub detail: Detail,
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
    /// Rows and group headers, filtered. A cluster where everything is
    /// serving gets a flat list, with no group rows.
    pub lines: Vec<Entry>,
    pub summary: SharedString,
    /// `(count, words, tone)`, one part of the status bar each.
    pub attention: Vec<(usize, &'static str, ui::Tone)>,
    /// Reads that were cut, and a bundle installed in pieces.
    pub notices: Vec<SharedString>,
    /// Why there is no table: no API, nothing in the cluster.
    pub empty: SharedString,
    /// The API itself can't be read (not installed, partly installed).
    pub api_problem: bool,
    /// The list this page shows was refused: an error, never an empty list.
    pub error: Option<String>,
    pub columns: (Vec<table::Column>, f32),
    filter: String,
    flat: bool,
}

pub(super) fn gateway_tone(state: GatewayState) -> ui::Tone {
    match state {
        GatewayState::Failed | GatewayState::Missing => ui::Tone::Crit,
        GatewayState::Partly => ui::Tone::Warn,
        GatewayState::Pending => ui::Tone::Waiting,
        // Unknown is never green.
        GatewayState::Blind | GatewayState::Unknown => ui::Tone::Unknown,
        GatewayState::Serving => ui::Tone::Good,
    }
}

pub(super) fn class_tone(state: ClassState) -> ui::Tone {
    match state {
        ClassState::Refused | ClassState::Missing => ui::Tone::Crit,
        ClassState::Pending => ui::Tone::Waiting,
        ClassState::Blind => ui::Tone::Unknown,
        ClassState::Claimed => ui::Tone::Good,
    }
}

fn slug(label: &str) -> &'static str {
    match label {
        "Failed" => "failed",
        "Not found" => "not-found",
        "Partly serving" => "partly-serving",
        "Pending" => "pending",
        "Blind" => "blind",
        "Unknown" => "unknown",
        "Serving" => "serving",
        "Not accepted" => "not-accepted",
        "Claimed" => "claimed",
        _ => "other",
    }
}

fn plural(count: usize, one: &str, many: &str) -> String {
    format!("{count} {}", if count == 1 { one } else { many })
}

fn gateway_row(entry: &GatewayEntry) -> (Row, usize) {
    let facts = &entry.facts;
    let mut status = format!("{} — {}", entry.state.label(), entry.reason);
    if entry.stale {
        status.push_str(" · stale verdict");
    }
    let address = entry.address().unwrap_or_else(|| "none".into());
    let class = if facts.class.is_empty() {
        "—".to_owned()
    } else {
        facts.class.clone()
    };
    let key = entry.key();
    let search = [
        key.as_str(),
        &status,
        &address,
        &class,
        &facts
            .listeners
            .iter()
            .filter_map(|l| l.hostname.clone())
            .collect::<Vec<_>>()
            .join(" "),
        &entry
            .routes
            .iter()
            .map(|r| r.label.as_str())
            .collect::<Vec<_>>()
            .join(" "),
    ]
    .join(" ")
    .to_lowercase();
    let group = GatewayState::ALL
        .iter()
        .position(|state| *state == entry.state)
        .unwrap_or(0);
    let row = Row {
        key: key.clone().into(),
        id: format!("gateways-row-{}-{}", facts.namespace, facts.name).into(),
        label: facts.name.clone().into(),
        cells: vec![
            key.into(),
            status.into(),
            address.clone().into(),
            class.clone().into(),
            entry.routes_line().into(),
        ],
        tone: gateway_tone(entry.state),
        state: entry.state.label(),
        group,
        namespace: facts.namespace.clone(),
        name: facts.name.clone(),
        uid: facts.uid.clone(),
        openable: entry.state != GatewayState::Missing,
        search,
        fields: vec![
            Field {
                label: "Gateway",
                value: format!("{}/{}", facts.namespace, facts.name).into(),
            },
            Field {
                label: "Class",
                value: class.into(),
            },
            Field {
                label: "Address",
                value: address.into(),
            },
        ],
        detail: Detail::Gateway(Box::new(gateway_detail(entry))),
    };
    (row, group)
}

fn class_row(entry: &ClassEntry) -> (Row, usize) {
    let controller = entry
        .controller
        .clone()
        .unwrap_or_else(|| "no such class".to_owned());
    let mut status = format!("{} — {}", entry.state.label(), entry.reason);
    if entry.stale {
        status.push_str(" · stale verdict");
    }
    let users = match entry.gateways.len() {
        0 => "no Gateways".to_owned(),
        n => plural(n, "Gateway", "Gateways"),
    };
    let search = [
        entry.name.as_str(),
        &status,
        &controller,
        &entry
            .gateways
            .iter()
            .map(|(key, _)| key.as_str())
            .collect::<Vec<_>>()
            .join(" "),
    ]
    .join(" ")
    .to_lowercase();
    let group = ClassState::ALL
        .iter()
        .position(|state| *state == entry.state)
        .unwrap_or(0);
    let row = Row {
        key: entry.name.clone().into(),
        id: format!("gateway-classes-row-{}", entry.name).into(),
        label: entry.name.clone().into(),
        cells: vec![
            entry.name.clone().into(),
            status.into(),
            controller.clone().into(),
            users.into(),
        ],
        tone: class_tone(entry.state),
        state: entry.state.label(),
        group,
        namespace: String::new(),
        name: entry.name.clone(),
        uid: entry.uid.clone(),
        openable: entry.state != ClassState::Missing,
        search,
        fields: vec![
            Field {
                label: "Class",
                value: entry.name.clone().into(),
            },
            Field {
                label: "Controller",
                value: controller.into(),
            },
        ],
        detail: Detail::Class(Box::new(class_detail(entry))),
    };
    (row, group)
}

impl Display {
    pub(super) fn empty(mode: Mode) -> Self {
        Self {
            rows: Vec::new(),
            groups: Vec::new(),
            lines: Vec::new(),
            summary: "".into(),
            attention: Vec::new(),
            notices: Vec::new(),
            empty: "".into(),
            api_problem: false,
            error: None,
            // The header and the loading rows are there before any answer.
            columns: table::columns(mode, &[]),
            filter: String::new(),
            flat: true,
        }
    }

    pub(super) fn new(mode: Mode, read: Option<&RoutesRead>) -> Self {
        let mut display = Self::empty(mode);
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
        let view = GatewaysView::new(read);
        let built: Result<Vec<(Row, usize)>, String> = match mode {
            Mode::Gateways => view
                .gateways
                .as_ref()
                .map(|list| list.iter().map(gateway_row).collect())
                .map_err(|failure| format!("{}: {}", failure.kind.label(), failure.message)),
            Mode::Classes => view
                .classes
                .as_ref()
                .map(|list| list.iter().map(class_row).collect())
                .map_err(|failure| format!("{}: {}", failure.kind.label(), failure.message)),
        };
        let mut rows = match built {
            Ok(rows) => rows,
            Err(error) => {
                display.error = Some(error);
                return display;
            }
        };
        // The entries come sorted by state, then name.
        rows.sort_by(|a, b| (a.1, &a.0.key).cmp(&(b.1, &b.0.key)));
        let rows = rows.into_iter().map(|(row, _)| row).collect::<Vec<_>>();
        let (labels, tones): (Vec<&'static str>, Vec<ui::Tone>) = match mode {
            Mode::Gateways => GatewayState::ALL
                .iter()
                .map(|s| (s.label(), gateway_tone(*s)))
                .unzip(),
            Mode::Classes => ClassState::ALL
                .iter()
                .map(|s| (s.label(), class_tone(*s)))
                .unzip(),
        };
        let count = |group: usize| rows.iter().filter(|r| r.group == group).count();
        display.groups = (0..labels.len())
            .filter(|group| count(*group) > 0)
            .map(|group| GroupLine {
                slug: slug(labels[group]),
                label: labels[group],
                tone: tones[group],
                count: count(group),
            })
            .collect();
        let good = labels.len() - 1;
        display.flat = rows.iter().all(|r| r.group == good);
        display.attention = (0..good)
            .filter(|group| count(*group) > 0)
            .map(|group| (count(group), attention_words(labels[group]), tones[group]))
            .collect();
        let noun = match mode {
            Mode::Gateways => ("Gateway", "Gateways"),
            Mode::Classes => ("class", "classes"),
        };
        display.summary = plural(rows.len(), noun.0, noun.1).into();
        display.empty = match mode {
            Mode::Gateways => "There are no Gateways in this cluster.".into(),
            Mode::Classes => "There are no GatewayClasses in this cluster.".into(),
        };
        display.rows = rows;
        display.columns = table::columns(mode, &display.rows);
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
        let shown = |row: &Row| self.filter.is_empty() || row.search.contains(&self.filter);
        let mut lines = Vec::new();
        for (group_ix, group) in self.groups.iter().enumerate() {
            let rows = self
                .rows
                .iter()
                .enumerate()
                .filter(|(_, r)| r.state == group.label && shown(r))
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

/// The status bar's word for a group that needs attention.
fn attention_words(label: &str) -> &'static str {
    match label {
        "Failed" => "failed",
        "Not found" => "not found",
        "Partly serving" => "partly",
        "Pending" => "pending",
        "Blind" => "blind",
        "Unknown" => "unknown",
        "Not accepted" => "not accepted",
        _ => "other",
    }
}
