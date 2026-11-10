//! Prepared rows, groups and sentences for the Certificates page; render
//! does no collection work.
use super::*;
use freshkube_core::resources::traffic::certs::{
    CertEntry, CertState, CertsView, SecretOutcome, SecretState, Verdict,
};

pub(crate) struct Field {
    pub label: &'static str,
    pub value: SharedString,
}

/// The most names, hosts or servers the Inspector lists; the rest are
/// counted.
pub(crate) const DETAIL_LINES: usize = 50;

/// One step of the issuance chain, drawn like a trace step.
pub(crate) struct ChainLine {
    pub tone: ui::Tone,
    pub state: &'static str,
    pub text: String,
}

/// The cert-manager Certificate that keeps a Secret, for a link.
#[derive(Clone)]
pub(crate) struct CertificateRef {
    pub namespace: String,
    pub name: String,
    pub uid: String,
}

/// What a row's Inspector draws, as strings made once per read.
pub(crate) struct Detail {
    pub reason: String,
    /// What the certificate names, or why that isn't known.
    pub names: Result<Vec<String>, String>,
    pub names_more: usize,
    /// One line per listener or Ingress that serves it, with its hosts.
    pub served: Vec<String>,
    pub served_more: usize,
    pub wrong: Vec<String>,
    pub wrong_more: usize,
    /// The issuance chain; empty for a Secret no Certificate keeps.
    pub chain: Vec<ChainLine>,
    /// A line on where the facts came from and what was (not) read.
    pub source: String,
}

pub(crate) struct Row {
    pub key: SharedString,
    pub id: SharedString,
    pub label: SharedString,
    /// The cells after the glyph.
    pub cells: Vec<SharedString>,
    pub tone: ui::Tone,
    /// Words for the glyph's tooltip and label.
    pub state: &'static str,
    /// Which group the row is in.
    pub group: usize,
    /// The cert-manager Certificate to open, when one keeps the Secret.
    pub certificate: Option<CertificateRef>,
    /// The Secret may be read on request.
    pub can_read: bool,
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
    /// Rows and group headers, filtered. A cluster where everything is valid
    /// gets a flat list, with no group rows.
    pub lines: Vec<Entry>,
    pub summary: SharedString,
    /// `(count, words, tone)`, one part of the status bar each.
    pub attention: Vec<(usize, &'static str, ui::Tone)>,
    /// `(title, words)`: cert-manager missing, reads that were refused or cut.
    pub notices: Vec<(&'static str, SharedString)>,
    /// Why there is no table: nothing in the cluster, or what could not be read.
    pub empty: SharedString,
    /// Nothing could be read at all: an error with Retry, never an empty list.
    pub error: Option<String>,
    pub columns: (Vec<table::Column>, f32),
    filter: String,
    flat: bool,
}

pub(super) fn tone(state: CertState) -> ui::Tone {
    match state {
        CertState::Broken => ui::Tone::Crit,
        CertState::WrongName | CertState::Failing | CertState::Expiring => ui::Tone::Warn,
        CertState::Issuing => ui::Tone::Waiting,
        // Unknown is never green.
        CertState::Unknown => ui::Tone::Unknown,
        CertState::Healthy => ui::Tone::Good,
    }
}

fn slug(state: CertState) -> &'static str {
    match state {
        CertState::Broken => "not-valid",
        CertState::WrongName => "wrong-name",
        CertState::Failing => "renewal-failing",
        CertState::Issuing => "issuing",
        CertState::Expiring => "expiring",
        CertState::Unknown => "unknown",
        CertState::Healthy => "valid",
    }
}

/// The status bar's word for a group that needs attention.
fn attention_words(state: CertState) -> &'static str {
    match state {
        CertState::Broken => "not valid",
        CertState::WrongName => "wrong name",
        CertState::Failing => "renewal failing",
        CertState::Issuing => "issuing",
        CertState::Expiring => "expiring",
        CertState::Unknown => "unknown",
        CertState::Healthy => "valid",
    }
}

fn chain_tone(verdict: Verdict) -> ui::Tone {
    match verdict {
        Verdict::Done => ui::Tone::Good,
        Verdict::Waiting => ui::Tone::Waiting,
        Verdict::Warn => ui::Tone::Warn,
        Verdict::Broken => ui::Tone::Crit,
        Verdict::Unknown => ui::Tone::Unknown,
        Verdict::NotReached | Verdict::NotUsed => ui::Tone::Outline,
    }
}

fn plural(count: usize, one: &str, many: &str) -> String {
    format!("{count} {}", if count == 1 { one } else { many })
}

fn more(total: usize) -> usize {
    total.saturating_sub(DETAIL_LINES)
}

/// `2026-12-31` from Unix seconds.
fn date(at: i64) -> String {
    chrono::DateTime::from_timestamp(at, 0)
        .map_or_else(|| "?".into(), |t| t.format("%Y-%m-%d").to_string())
}

fn entry_row(entry: &CertEntry) -> Row {
    let key = entry.key();
    let state_words = entry.state.label();
    let status = if entry.state == CertState::Healthy {
        state_words.to_owned()
    } else {
        format!("{state_words} — {}", entry.reason)
    };
    let expiry = match entry.left_secs {
        Some(left) if entry.not_after.is_some() => certs::expiry_words(left),
        _ => "—".to_owned(),
    };
    let serves = if entry.servings.is_empty() {
        "nothing yet".to_owned()
    } else {
        let mut vias: Vec<String> = Vec::new();
        for serving in &entry.servings {
            let word = match &serving.via {
                certs::Via::Listener { gateway, .. } => format!("Gateway {gateway}"),
                certs::Via::Ingress { ingress } => format!("Ingress {ingress}"),
            };
            if !vias.contains(&word) {
                vias.push(word);
            }
        }
        vias.join(", ")
    };
    let source = match (&entry.managed, &entry.secret_state) {
        (Some(_), _) => "cert-manager".to_owned(),
        (None, SecretState::Read(SecretOutcome::Read(_))) => "Secret (read)".to_owned(),
        (None, _) => "Secret, not read".to_owned(),
    };
    let names = entry.names.clone().unwrap_or_default();
    let search = [
        key.as_str(),
        &status,
        &serves,
        &names.join(" "),
        &entry.hosts().join(" "),
        entry.issuer.as_deref().unwrap_or(""),
    ]
    .join(" ")
    .to_lowercase();
    let group = CertState::ALL
        .iter()
        .position(|state| *state == entry.state)
        .unwrap_or(0);
    let can_read = entry.can_read_secret();
    let mut fields = vec![Field {
        label: "Certificate",
        value: key.clone().into(),
    }];
    if let Some(end) = entry.not_after {
        fields.push(Field {
            label: "Expires",
            value: format!(
                "{} ({})",
                date(end),
                certs::expiry_words(entry.left_secs.unwrap_or(0))
            )
            .into(),
        });
    }
    if let Some(start) = entry.not_before {
        fields.push(Field {
            label: "Valid from",
            value: date(start).into(),
        });
    }
    if let Some(issuer) = &entry.issuer {
        fields.push(Field {
            label: "Issuer",
            value: issuer.clone().into(),
        });
    }
    if let Some(subject) = &entry.subject {
        fields.push(Field {
            label: "Subject",
            value: subject.clone().into(),
        });
    }
    let names_note = match (&entry.names, &entry.secret_state) {
        (Some(names), _) => Ok(names.clone()),
        (None, SecretState::NotRead) if entry.withheld.is_some() => {
            Err(entry.withheld.clone().unwrap_or_default())
        }
        (None, SecretState::NotRead) if entry.managed.is_none() => Err(
            "Not known: the Secret has not been read. Read Secret fetches it once and keeps only the certificate's names, issuer and dates; its key is never kept."
                .to_owned(),
        ),
        (None, _) => Err("Not known.".to_owned()),
    };
    let names_total = names_note.as_ref().map_or(0, Vec::len);
    let served: Vec<String> = entry
        .servings
        .iter()
        .map(|serving| {
            let hosts = if serving.hosts.is_empty() {
                "no host named".to_owned()
            } else {
                serving.hosts.join(", ")
            };
            format!("{} — {hosts}", serving.via.words())
        })
        .collect();
    let chain = entry
        .chain
        .as_ref()
        .map(|chain| {
            chain
                .steps
                .iter()
                .map(|step| {
                    let mut text = step.kind.label().to_owned();
                    if let Some(object) = &step.object {
                        text.push(' ');
                        text.push_str(object);
                    }
                    if !step.says.is_empty() {
                        text.push_str(" — ");
                        text.push_str(&step.says);
                    }
                    ChainLine {
                        tone: chain_tone(step.verdict),
                        state: step.verdict.label(),
                        text,
                    }
                })
                .collect()
        })
        .unwrap_or_default();
    // The Inspector's headline says where, and the chain below says why; the
    // row's cell carries the whole line. A valid certificate has nothing to add.
    let headline = match (&entry.chain, entry.state) {
        (_, CertState::Healthy) => String::new(),
        (Some(chain), CertState::Broken | CertState::Issuing) if chain.stopped_at().is_some() => {
            let at = chain
                .stopped_at()
                .map(|s| s.kind.label())
                .unwrap_or_default();
            match entry.state {
                CertState::Issuing => format!("In progress at {at}"),
                _ => format!("Stopped at {at}"),
            }
        }
        _ => entry.reason.clone(),
    };
    let detail = Detail {
        reason: headline,
        names: names_note.map(|names| names.into_iter().take(DETAIL_LINES).collect()),
        names_more: more(names_total),
        served_more: more(served.len()),
        served: served.into_iter().take(DETAIL_LINES).collect(),
        wrong_more: more(entry.wrong.len()),
        wrong: entry.wrong.iter().take(DETAIL_LINES).cloned().collect(),
        chain,
        source: match (&entry.managed, &entry.secret_state) {
            (Some(cert), _) => format!(
                "From the cert-manager Certificate {} (its spec and status). The Secret was not read.",
                cert.key()
            ),
            (None, SecretState::Read(SecretOutcome::Read(_))) => {
                "From the Secret's tls.crt, read on request. Only its names, issuer and dates were kept; the key never left the cluster read.".to_owned()
            }
            (None, SecretState::Read(SecretOutcome::Refused(_))) => {
                "Reading the Secret failed or was refused. Read Secret tries again.".to_owned()
            }
            (None, SecretState::Read(_)) => "The Secret was read on request.".to_owned(),
            (None, SecretState::NotRead) => "Not read.".to_owned(),
        },
    };
    Row {
        key: key.clone().into(),
        id: format!("certificates-row-{}-{}", entry.namespace, entry.secret).into(),
        label: entry.secret.clone().into(),
        cells: vec![
            key.into(),
            status.into(),
            expiry.into(),
            serves.into(),
            source.into(),
        ],
        tone: tone(entry.state),
        state: state_words,
        group,
        certificate: entry.managed.as_ref().map(|cert| CertificateRef {
            namespace: cert.namespace.clone(),
            name: cert.name.clone(),
            uid: cert.uid.clone(),
        }),
        can_read,
        search,
        fields,
        detail,
    }
}

pub(super) const LABELS: [&str; 5] = ["Certificate", "Status", "Expires", "Serves", "Source"];

impl Display {
    pub(super) fn empty() -> Self {
        Self {
            rows: Vec::new(),
            groups: Vec::new(),
            lines: Vec::new(),
            summary: "".into(),
            attention: Vec::new(),
            notices: Vec::new(),
            empty: "".into(),
            error: None,
            // The header and the loading rows are there before any answer.
            columns: table::columns(&[]),
            filter: String::new(),
            flat: true,
        }
    }

    pub(super) fn new(view: Option<&CertsView>) -> Self {
        let mut display = Self::empty();
        let Some(view) = view else {
            return display;
        };
        // cert-manager missing is said on the page, never as "no certificates".
        if let Some(problem) = view.manager.problem() {
            display.notices.push(("cert-manager", problem.into()));
        }
        for note in &view.notes {
            display.notices.push(("Read coverage", note.clone().into()));
        }
        let mut entries: Vec<&CertEntry> = view.entries.iter().collect();
        entries.sort_by_key(|a| (a.state, a.key()));
        let rows: Vec<Row> = entries.into_iter().map(entry_row).collect();
        if rows.is_empty() && !view.notes.is_empty() {
            // Everything that could name a certificate was refused.
            display.error = Some(view.notes.join("; "));
            return display;
        }
        let count = |group: usize| rows.iter().filter(|r| r.group == group).count();
        display.groups = CertState::ALL
            .iter()
            .enumerate()
            .filter(|(group, _)| count(*group) > 0)
            .map(|(group, state)| GroupLine {
                slug: slug(*state),
                label: state.label(),
                tone: tone(*state),
                count: count(group),
            })
            .collect();
        let good = CertState::ALL.len() - 1;
        display.flat = rows.iter().all(|r| r.group == good);
        display.attention = CertState::ALL[..good]
            .iter()
            .enumerate()
            .filter(|(group, _)| count(*group) > 0)
            .map(|(group, state)| (count(group), attention_words(*state), tone(*state)))
            .collect();
        display.summary = plural(rows.len(), "certificate", "certificates").into();
        display.empty = match view.manager.problem() {
            Some(problem) => {
                format!("{problem} No Gateway listener or Ingress names a TLS Secret either.")
            }
            None => {
                "No Gateway listener or Ingress serves a certificate, and cert-manager has none."
                    .to_owned()
            }
        }
        .into();
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
