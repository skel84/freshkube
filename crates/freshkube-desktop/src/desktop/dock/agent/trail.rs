//! The trail: each read the agent made, with its place as text and a mark.
//! A row of a read that has a view is a link the person may press: a click
//! is their own navigation to that view (`AgentView::go_to_read`).

use freshkube_agent::{Outcome, ReadEvent};
use freshkube_core::agent_reads::Code;
use freshkube_nav::{ViewState, ViewTab};

/// At most this many reads show; a session has 500 at most, so this is
/// a bound, not a cut in practice.
const MAX_ROWS: usize = 500;
/// A citation is `[r` and up to this many digits and `]`.
const MAX_CITE_DIGITS: usize = 6;
/// At most this many citations are read from one text.
const MAX_CITATIONS: usize = 1_000;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Mark {
    /// Read; the agent hasn't cited it.
    Evidence,
    /// The agent cited it as `[r7]` in its words.
    Cited,
    /// Not read because the app or the cluster said no: access off, another
    /// context, a tool that isn't one of the four reads, a kind that isn't
    /// served, a 401 or 403, or a bound. Neutral: nothing broke.
    Refused,
    /// The read worked and the object isn't there. Neutral too: nothing
    /// was refused and nothing broke.
    NotFound,
    /// The read broke: the cluster was unreachable, it took too long, or the
    /// server's answer was an error or couldn't be read.
    CouldntRead,
}

impl Mark {
    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::Evidence => "evidence",
            Self::Cited => "cited",
            Self::Refused => "refused",
            Self::NotFound => "Not found",
            Self::CouldntRead => "couldn't read",
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Row {
    pub(crate) number: u32,
    /// `r7`.
    pub(crate) id: String,
    pub(crate) tool: String,
    /// The kind, namespace, object and tab as text; for a refused read the
    /// refusal's code.
    pub(crate) place: String,
    pub(crate) mark: Mark,
    /// The entry the read was made on when it isn't the session's own.
    pub(crate) elsewhere: Option<String>,
    /// Where the window would go to show the read, when it ran on the
    /// session's own cluster and found something to show.
    pub(crate) view: Option<ViewState>,
}

#[derive(Debug, Default)]
pub(crate) struct Trail {
    rows: Vec<Row>,
    cited: Vec<u32>,
}

impl Trail {
    pub(crate) fn rows(&self) -> &[Row] {
        &self.rows
    }

    pub(crate) fn clear(&mut self) {
        self.rows.clear();
        self.cited.clear();
    }

    /// Adds a read of the session whose own entry is `own`.
    pub(crate) fn read(&mut self, event: &ReadEvent, own: &str) {
        if self.rows.len() >= MAX_ROWS {
            return;
        }
        let number = event.read.number();
        let (place, mark) = match &event.outcome {
            Outcome::Refused(code) => (
                refusal(*code).to_owned(),
                match code {
                    Code::NotFound => Mark::NotFound,
                    code if broke(*code) => Mark::CouldntRead,
                    _ => Mark::Refused,
                },
            ),
            Outcome::Answered { .. } => (
                event.view.as_ref().map(place).unwrap_or_default(),
                if self.cited.contains(&number) {
                    Mark::Cited
                } else {
                    Mark::Evidence
                },
            ),
        };
        self.rows.push(Row {
            number,
            id: event.read.to_string(),
            tool: event.tool.to_owned(),
            place,
            mark,
            elsewhere: (event.cluster != own).then(|| event.cluster.clone()),
            view: event.view.clone().filter(|_| {
                event.cluster == own && matches!(event.outcome, Outcome::Answered { .. })
            }),
        });
    }

    /// The row of read `number`.
    pub(crate) fn row(&self, number: u32) -> Option<&Row> {
        self.rows.iter().find(|row| row.number == number)
    }

    /// A shell or any other tool the agent asked for that Freshkube refused
    /// before any read: it never reaches the endpoint, so the trail adds it
    /// from the transcript's refusal note.
    pub(crate) fn refused_tool(&mut self, what: &str) {
        if self.rows.len() >= MAX_ROWS {
            return;
        }
        let n = self.rows.iter().filter(|row| row.number == 0).count() + 1;
        self.rows.push(Row {
            number: 0,
            id: format!("x{n}"),
            tool: what.to_owned(),
            place: "Not one of Freshkube's four reads".to_owned(),
            mark: Mark::Refused,
            elsewhere: None,
            view: None,
        });
    }

    /// Marks the reads that `text` cites. Reads cited before they exist are
    /// remembered and marked when they arrive.
    pub(crate) fn cite(&mut self, text: &str) {
        for number in citations(text) {
            if !self.cited.contains(&number) && self.cited.len() < MAX_CITATIONS {
                self.cited.push(number);
            }
            for row in &mut self.rows {
                if row.number == number && row.mark == Mark::Evidence {
                    row.mark = Mark::Cited;
                }
            }
        }
    }
}

/// The read ids `[r7]` in `text`, in order, at most [`MAX_CITATIONS`].
pub(crate) fn citations(text: &str) -> Vec<u32> {
    let mut found = Vec::new();
    let mut rest = text;
    while let Some(at) = rest.find("[r") {
        rest = &rest[at + 2..];
        let digits: String = rest
            .chars()
            .take(MAX_CITE_DIGITS + 1)
            .take_while(char::is_ascii_digit)
            .collect();
        if !digits.is_empty()
            && digits.len() <= MAX_CITE_DIGITS
            && rest[digits.len()..].starts_with(']')
            && let Ok(number) = digits.parse()
        {
            found.push(number);
            if found.len() >= MAX_CITATIONS {
                break;
            }
        }
    }
    found
}

/// Whether a refused read's code means the read broke, rather than that
/// something said no.
pub(crate) fn broke(code: Code) -> bool {
    match code {
        Code::Unreachable | Code::TimedOut | Code::Failed => true,
        Code::Invalid
        | Code::UnknownCluster
        | Code::NotEnabled
        | Code::Refused
        | Code::NotFound
        | Code::NotServed
        | Code::TooLarge
        | Code::LimitReached => false,
    }
}

/// A refusal in words, for the trail.
fn refusal(code: Code) -> &'static str {
    match code {
        Code::Invalid => "The request wasn't valid",
        Code::UnknownCluster => "That cluster isn't one this session may read",
        Code::NotEnabled => "Agent access is off for this cluster",
        Code::Refused => "The cluster refused your access",
        Code::NotFound => "It wasn't found",
        Code::NotServed => "This kind isn't served on this cluster",
        Code::TooLarge => "The object is too large to read",
        Code::Unreachable => "The cluster couldn't be reached",
        Code::TimedOut => "The cluster took too long",
        Code::Failed => "The read failed",
        Code::LimitReached => "This session used all its reads",
    }
}

/// A view as words: `pods · shop/api-0 · Logs`.
pub(crate) fn place(view: &ViewState) -> String {
    let mut parts = Vec::new();
    if let Some(kind) = &view.kind {
        parts.push(kind.as_str().to_owned());
    }
    match (&view.ns, &view.object) {
        (Some(ns), Some(object)) => parts.push(format!("{}/{}", ns.as_str(), object.name.as_str())),
        (Some(ns), None) => parts.push(ns.as_str().to_owned()),
        (None, Some(object)) => parts.push(object.name.as_str().to_owned()),
        (None, None) => {}
    }
    if view.object.is_some() {
        parts.push(
            match view.tab {
                ViewTab::Details => "Details",
                ViewTab::Yaml => "YAML",
                ViewTab::Events => "Events",
                ViewTab::Ports => "Ports",
                ViewTab::Logs => "Logs",
            }
            .to_owned(),
        );
    }
    parts.join(" · ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_a_broken_read_is_not_a_refusal() {
        for code in [Code::Unreachable, Code::TimedOut, Code::Failed] {
            assert!(broke(code), "{code}");
        }
        for code in [
            Code::NotEnabled,
            Code::UnknownCluster,
            Code::Refused,
            Code::NotServed,
            Code::NotFound,
            Code::TooLarge,
            Code::LimitReached,
            Code::Invalid,
        ] {
            assert!(!broke(code), "{code}");
        }
    }

    #[test]
    fn a_missing_object_is_not_found_not_refused() {
        let mut next = 0;
        let mut trail = Trail::default();
        let mut event = |code| ReadEvent {
            session: freshkube_agent::SessionId::new(),
            read: {
                next += 1;
                freshkube_agent::ReadId::numbered(next)
            },
            tool: "get_object",
            cluster: "a".to_owned(),
            view: None,
            outcome: Outcome::Refused(code),
        };
        let missing = event(Code::NotFound);
        let refused = event(Code::Refused);
        trail.read(&missing, "a");
        trail.read(&refused, "a");
        let marks: Vec<_> = trail.rows().iter().map(|row| row.mark).collect();
        assert_eq!(marks, [Mark::NotFound, Mark::Refused]);
        assert_eq!(Mark::NotFound.label(), "Not found");
    }

    #[test]
    fn citations_are_bounded_and_exact() {
        assert_eq!(
            citations("seen in [r1] and [r12], not [r], [r1x] or [rr3]"),
            [1, 12]
        );
        assert_eq!(citations("[r1234567]"), Vec::<u32>::new());
        assert_eq!(citations(&"[r1]".repeat(5_000)).len(), MAX_CITATIONS);
        assert_eq!(citations("[r"), Vec::<u32>::new());
    }
}
