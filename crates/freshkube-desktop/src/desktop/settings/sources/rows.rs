//! The rows, derived from where each part is read and what the last check
//! found, so `render` only reads them.
use super::*;
use freshkube_core::workspace::Why;

/// One row, as the table shows it.
pub(crate) struct SourceRow {
    pub(super) read: Read,
    pub(super) label: SharedString,
    pub(super) entry: SharedString,
    pub(super) why: SharedString,
    pub(super) explanation: SharedString,
    pub(super) detail: SharedString,
    pub(super) state: SharedString,
    pub(super) reads: SharedString,
    /// The state's glyph, once there is one to show.
    pub(super) tone: Option<ui::Tone>,
    /// Whether there is an entry to read: not missing or gone.
    pub(super) listed: bool,
    pub(super) tooltip: SharedString,
    /// `settings-source-kargo`, and its mark's and cells'.
    pub(super) id: SharedString,
    pub(super) mark_id: SharedString,
}

/// What the Workloads row reads on each entry.
const WORKLOADS_READ: &str = "Deployments, Rollouts, ReplicaSets, pods";

/// A workloads entry's id in a check: the entry's, or the open cluster's.
pub(super) fn workload_id(chosen: &Chosen) -> String {
    match chosen {
        Chosen::Entry { entry, .. } => entry.id.clone(),
        Chosen::Gone(id) => id.clone(),
        Chosen::Skipped(row) => row.label(),
        Chosen::Open | Chosen::NoEntry(_) => entry::OPEN.to_owned(),
    }
}

/// A state's glyph: readable is fine, refused needs a look, and the rest
/// say nothing is known.
fn tone(reach: &Reach) -> ui::Tone {
    match reach {
        Reach::Readable => ui::Tone::Good,
        Reach::Refused(_) => ui::Tone::Warn,
        Reach::Missing(_) | Reach::Unreachable(_) | Reach::NotServed(_) => ui::Tone::Unknown,
    }
}

/// The least sure of several states' glyphs.
fn worst(reaches: &[&Reach]) -> ui::Tone {
    let tones: Vec<ui::Tone> = reaches.iter().map(|reach| tone(reach)).collect();
    [ui::Tone::Unknown, ui::Tone::Warn]
        .into_iter()
        .find(|worse| tones.contains(worse))
        .unwrap_or(ui::Tone::Good)
}

impl SourcesList {
    pub(super) fn derive_rows(&mut self) {
        let mut rows: Vec<SourceRow> = self
            .sources
            .iter()
            .map(|(part, chosen)| self.part_row(part, chosen))
            .collect();
        rows.push(self.workloads_row());
        if let Some(problem) = &self.setup_problem {
            rows.push(
                self.row(
                    Read::SetupFile,
                    "freshkube.yaml".into(),
                    "Setup file".into(),
                    false,
                    (
                        (if self.setup_skips {
                            "Used with warnings"
                        } else {
                            "Not used"
                        })
                        .into(),
                        Some(ui::Tone::Warn),
                    ),
                    problem.clone(),
                    problem.clone(),
                ),
            );
        }
        self.rows = rows;
        (self.columns, self.width) = source::columns(&self.rows);
    }

    fn checked(&self) -> Option<&Checked> {
        match &self.check {
            Check::Done(checked) => Some(checked),
            _ => None,
        }
    }

    /// The state when nothing has answered: checking, failed, or not yet.
    fn unanswered(&self, what: &str, entry: &str) -> (String, Option<ui::Tone>, String) {
        match &self.check {
            Check::Running => (
                "Reading".to_owned(),
                None,
                format!("Reading one item of {what} from {entry}"),
            ),
            Check::Failed(why) => (
                "Unreachable".to_owned(),
                Some(ui::Tone::Unknown),
                why.clone(),
            ),
            _ => (
                "Not checked".to_owned(),
                None,
                "Check sources reads one item of each part".to_owned(),
            ),
        }
    }

    fn part_row(&self, part: Part, chosen: &Chosen) -> SourceRow {
        let (entry, why, listed) = match chosen {
            Chosen::Entry { entry, why } => (
                entry.id.clone(),
                match why {
                    Why::Pinned => "Pinned".to_owned(),
                    _ => why.words(),
                },
                true,
            ),
            Chosen::Open => (
                "The open cluster".to_owned(),
                "Workspace of one".to_owned(),
                true,
            ),
            Chosen::NoEntry(roles) => (
                "None".to_owned(),
                format!(
                    "No {} entry",
                    roles
                        .iter()
                        .map(|role| role.label())
                        .collect::<Vec<_>>()
                        .join(" or ")
                ),
                false,
            ),
            Chosen::Gone(id) => (id.clone(), "Pinned, not listed".to_owned(), false),
            Chosen::Skipped(row) => (row.label(), "Selected row skipped".to_owned(), false),
        };
        // In a sentence, the open cluster isn't a name.
        let named = match chosen {
            Chosen::Open => entry::OPEN.to_owned(),
            _ => entry.clone(),
        };
        let found = self.checked().and_then(|checked| {
            checked
                .parts
                .iter()
                .find(|(checked, _)| *checked == part)
                .map(|(_, reach)| reach.clone())
        });
        let reach = chosen.missing().map(Reach::missing).or(found);
        let (state, tone, sentence) = match &reach {
            Some(reach) => (
                reach.label().to_owned(),
                Some(tone(reach)),
                reach.words(part.label(), &named),
            ),
            None => self.unanswered(part.label(), &named),
        };
        let chosen_by = match chosen {
            Chosen::Entry {
                why: Why::Pinned, ..
            } => format!("{entry}, pinned in freshkube.yaml"),
            Chosen::Entry { why, .. } => {
                format!("{entry}, the first entry with the {}", why.words())
            }
            Chosen::Open => "the open cluster: the workspace has no entries".to_owned(),
            Chosen::Gone(id) => {
                format!("pinned to {id} in freshkube.yaml, but the entry is no longer listed")
            }
            _ => chosen.missing().unwrap_or_default(),
        };
        // Argo CD is read where it runs, which only a check finds.
        let argocd_in = self
            .checked()
            .filter(|checked| part == Part::ArgoCd && !checked.argocd_in.is_empty())
            .map(|checked| checked.argocd_in.join(", "));
        let reads = match &argocd_in {
            Some(namespaces) => format!("{} in {namespaces}", part.reads()),
            None => part.reads().to_owned(),
        };
        let mut tooltip = format!("{}: {chosen_by}\n{sentence}\nReads {reads}", part.label());
        if part == Part::ArgoCd && argocd_in.is_none() {
            tooltip.push_str(", where its workloads labelled part-of=argocd run, else in argocd");
        }
        self.row(
            Read::Part(part),
            entry,
            why,
            listed,
            (state, tone),
            reads,
            tooltip,
        )
    }

    /// Workloads: every entry an Argo CD destination maps to, each one's
    /// state counted, and each in the tooltip.
    fn workloads_row(&self) -> SourceRow {
        let chosen = self.sources.workloads();
        let open = matches!(chosen, [Chosen::Open]);
        let ids: Vec<String> = chosen.iter().map(workload_id).collect();
        let (entry, why) = match (chosen.is_empty(), open) {
            (true, _) => ("None".to_owned(), "No destinations".to_owned()),
            (_, true) => ("The open cluster".to_owned(), "Workspace of one".to_owned()),
            _ => (ids.join(", "), "Argo CD destinations".to_owned()),
        };
        let answers: Vec<Option<Reach>> = chosen
            .iter()
            .zip(&ids)
            .map(|(chosen, id)| {
                chosen.missing().map(Reach::missing).or_else(|| {
                    self.checked()?
                        .workloads
                        .iter()
                        .find(|(checked, _)| checked == id)
                        .map(|(_, reach)| reach.clone())
                })
            })
            .collect();
        let named = |id: &str| match open {
            true => entry::OPEN.to_owned(),
            false => id.to_owned(),
        };
        let reaches: Vec<&Reach> = answers.iter().flatten().collect();
        let (state, tone, lines) = if chosen.is_empty() {
            let why = "No Argo CD destination is mapped to an entry; map one under Destinations";
            (
                "Not in the workspace".to_owned(),
                Some(ui::Tone::Unknown),
                vec![why.to_owned()],
            )
        } else if reaches.len() == chosen.len() {
            let lines = reaches
                .iter()
                .zip(&ids)
                .map(|(reach, id)| reach.words("Workloads", &named(id)))
                .collect();
            (counted(&reaches), Some(worst(&reaches)), lines)
        } else {
            let (state, tone, sentence) = self.unanswered("Workloads", &entry);
            let lines = answers
                .iter()
                .zip(&ids)
                .map(|(reach, id)| match reach {
                    Some(reach) => reach.words("Workloads", &named(id)),
                    None => format!("{}: {state}", named(id)),
                })
                .chain([sentence])
                .collect();
            (state, tone, lines)
        };
        let tooltip = format!(
            "Workloads: on each entry an Argo CD destination maps to\n{}\nReads {WORKLOADS_READ}",
            lines.join("\n")
        );
        self.row(
            Read::Workloads,
            entry,
            why,
            !chosen.is_empty(),
            (state, tone),
            WORKLOADS_READ.to_owned(),
            tooltip,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn row(
        &self,
        read: Read,
        entry: String,
        why: String,
        listed: bool,
        (state, tone): (String, Option<ui::Tone>),
        reads: String,
        tooltip: String,
    ) -> SourceRow {
        let key = read.key();
        SourceRow {
            read,
            label: read.label().into(),
            entry: entry.into(),
            why: why.into(),
            state: state.into(),
            reads: reads.into(),
            tone,
            listed,
            explanation: tooltip.lines().next().unwrap_or_default().to_owned().into(),
            detail: tooltip
                .lines()
                .skip(1)
                .take_while(|line| !line.starts_with("Reads "))
                .collect::<Vec<_>>()
                .join("\n")
                .into(),
            tooltip: tooltip.into(),
            id: format!("settings-source-{key}").into(),
            mark_id: format!("settings-source-{key}-mark").into(),
        }
    }
}

/// Several entries' states in a few words: `3 readable`, `2 readable, 1
/// unreachable`; one entry's is its label.
fn counted(reaches: &[&Reach]) -> String {
    if let [reach] = reaches {
        return reach.label().to_owned();
    }
    let mut counts: Vec<(&'static str, usize)> = Vec::new();
    for reach in reaches {
        match counts.iter_mut().find(|(label, _)| *label == reach.label()) {
            Some((_, count)) => *count += 1,
            None => counts.push((reach.label(), 1)),
        }
    }
    let order = |label: &str| {
        [
            "Readable",
            "Refused",
            "Not served",
            "Unreachable",
            "Not in the workspace",
        ]
        .iter()
        .position(|known| *known == label)
    };
    counts.sort_by_key(|(label, _)| order(label));
    counts
        .iter()
        .map(|(label, count)| format!("{count} {}", label.to_lowercase()))
        .collect::<Vec<_>>()
        .join(", ")
}
