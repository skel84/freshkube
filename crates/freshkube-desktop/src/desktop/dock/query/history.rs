//! What the Query tab ran, newest first, kept in memory for the tab's life.
//!
//! It is the tab's own list and nothing else: never saved, never in the audit
//! log, a deep link or Search, and the login Secret is not in it because no
//! text the person typed holds it. Closing the tab drops it.

/// The most statements kept; the oldest go first.
pub(super) const MAX: usize = 100;

/// How a statement ended, in one word for the list.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Ended {
    Rows(usize),
    Plan,
    Refused,
    TimedOut,
    Cancelled,
    NotSent,
    Failed,
}

impl Ended {
    pub(super) fn label(self) -> String {
        match self {
            Self::Rows(1) => "1 row".into(),
            Self::Rows(n) => format!("{n} rows"),
            Self::Plan => "plan".into(),
            Self::Refused => "refused".into(),
            Self::TimedOut => "timed out".into(),
            Self::Cancelled => "cancelled".into(),
            Self::NotSent => "not sent".into(),
            Self::Failed => "failed".into(),
        }
    }

    pub(super) fn is_error(self) -> bool {
        !matches!(self, Self::Rows(_) | Self::Plan)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Entry {
    pub(super) id: u64,
    pub(super) sql: String,
    pub(super) ended: Ended,
    /// "14:05:09", the local time it finished.
    pub(super) at: String,
}

#[derive(Default)]
pub(super) struct History {
    /// Newest first.
    entries: Vec<Entry>,
    next: u64,
}

impl History {
    /// Records a statement. Running the same text again moves it to the
    /// front, with its new outcome, rather than listing it twice in a row.
    pub(super) fn push(&mut self, sql: &str, ended: Ended, at: String) {
        let sql = sql.trim().to_owned();
        if sql.is_empty() {
            return;
        }
        if self.entries.first().is_some_and(|e| e.sql == sql) {
            self.entries.remove(0);
        }
        self.next += 1;
        self.entries.insert(
            0,
            Entry {
                id: self.next,
                sql,
                ended,
                at,
            },
        );
        self.entries.truncate(MAX);
    }

    pub(super) fn entries(&self) -> &[Entry] {
        &self.entries
    }

    pub(super) fn get(&self, id: u64) -> Option<&Entry> {
        self.entries.iter().find(|e| e.id == id)
    }

    pub(super) fn clear(&mut self) {
        self.entries.clear();
    }

    pub(super) fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn push(history: &mut History, sql: &str) {
        history.push(sql, Ended::Rows(1), "10:00:00".into());
    }

    #[test]
    fn newest_first_and_a_repeat_moves_to_the_front() {
        let mut history = History::default();
        push(&mut history, "select 1");
        push(&mut history, "select 2");
        push(&mut history, "select 2");
        let sql: Vec<_> = history.entries().iter().map(|e| e.sql.as_str()).collect();
        assert_eq!(sql, ["select 2", "select 1"]);
        // Not only the last: an older repeat is a new entry above the rest.
        push(&mut history, "select 1");
        assert_eq!(history.entries().len(), 3);
    }

    #[test]
    fn at_most_the_newest_hundred_are_kept() {
        let mut history = History::default();
        for i in 0..MAX + 20 {
            push(&mut history, &format!("select {i}"));
        }
        assert_eq!(history.entries().len(), MAX);
        assert_eq!(history.entries()[0].sql, format!("select {}", MAX + 19));
        assert_eq!(history.entries()[MAX - 1].sql, "select 20");
    }

    #[test]
    fn nothing_blank_is_kept_and_clear_empties_it() {
        let mut history = History::default();
        push(&mut history, "  \n ");
        assert!(history.is_empty());
        push(&mut history, "select 1");
        history.clear();
        assert!(history.is_empty());
    }

    #[test]
    fn an_entry_is_found_by_its_id() {
        let mut history = History::default();
        push(&mut history, "select 1");
        let id = history.entries()[0].id;
        assert_eq!(history.get(id).unwrap().sql, "select 1");
        assert!(history.get(id + 1).is_none());
    }
}
