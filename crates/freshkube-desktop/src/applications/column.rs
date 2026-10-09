//! What the shell's Applications column lists (docs/DESIGN.md, "The column
//! as a source list"): each application under the rule that found it, or
//! why there are none. The page derives it with each read and the column
//! reads it as it is; the column reads nothing of its own.
use super::*;
use display::{AppRow, RULES, rule_index, rule_label};
use freshkube_core::applications::Rule;

/// One application's row.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct ColumnApp {
    pub(crate) key: SharedString,
    pub(crate) name: SharedString,
    /// What it is and where, after its name in the row's tooltip.
    pub(crate) detail: SharedString,
    /// Its entry in the menu past the cap: its name, with what it is when
    /// another application shares the name.
    pub(crate) menu: SharedString,
    /// Why it may be incomplete, for its mark.
    pub(crate) incomplete: Option<SharedString>,
}

/// A rule's applications, under its section label, in the table's order.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct ColumnSection {
    pub(crate) rule: Rule,
    pub(crate) label: &'static str,
    pub(crate) apps: Vec<ColumnApp>,
}

/// Why the column lists no applications, in a few words, with the page's
/// whole sentence in its tooltip.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct ColumnNote {
    pub(crate) text: SharedString,
    pub(crate) tooltip: Option<SharedString>,
    /// A failed read offers Retry, as the page's state does.
    pub(crate) retry: bool,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Column {
    pub(crate) sections: Vec<ColumnSection>,
    pub(crate) note: Option<ColumnNote>,
    /// Every application, for All applications' count.
    pub(crate) total: usize,
}

impl Column {
    /// The column for what the page shows: its rows, or why it has none.
    /// While a read is in flight, a page with no rows says so, and a failed
    /// read's Retry waits for it.
    pub(super) fn new(display: &Display, connected: bool, read: bool, reading: bool) -> Self {
        let note = |text: &str, tooltip: Option<String>, retry| ColumnNote {
            text: text.to_owned().into(),
            tooltip: tooltip.map(Into::into),
            retry,
        };
        let sentence = |title: &SharedString, description: &SharedString, reason: Option<&_>| {
            let mut text = format!("{title}. {description}");
            if let Some(reason) = reason {
                text = format!("{text} {reason}");
            }
            Some(text)
        };
        let note = match &display.body {
            _ if !connected => Some(note("Not connected", None, false)),
            Body::Table if !read => Some(note("Reading…", None, false)),
            Body::Table => None,
            _ if reading => Some(note("Reading…", None, false)),
            Body::Empty { title, description } => Some(note(
                "No applications found",
                sentence(title, description, None),
                false,
            )),
            Body::Refused {
                title,
                description,
                reason,
            } => Some(note(
                "Not permitted",
                sentence(title, description, Some(reason)),
                false,
            )),
            Body::Failed {
                title,
                description,
                reason,
            } => Some(note(
                "Couldn't read",
                sentence(title, description, Some(reason)),
                true,
            )),
        };
        // Rows are sorted by rule, as the table groups them; a rule with
        // nothing found, such as overrides before any is made, shows no
        // section.
        let mut sections: Vec<ColumnSection> = RULES
            .iter()
            .map(|&rule| ColumnSection {
                rule,
                label: rule_label(rule),
                apps: Vec::new(),
            })
            .collect();
        if note.is_none() {
            for row in &display.rows {
                let shared = display
                    .rows
                    .iter()
                    .filter(|other| other.name == row.name)
                    .nth(1)
                    .is_some();
                sections[rule_index(row.rule)].apps.push(app(row, shared));
            }
        }
        sections.retain(|section| !section.apps.is_empty());
        Self {
            total: display.rows.len(),
            sections,
            note,
        }
    }
}

fn app(row: &AppRow, shared: bool) -> ColumnApp {
    ColumnApp {
        key: row.key.clone(),
        name: row.name.clone(),
        detail: format!("{} · {}", row.what, row.clusters).into(),
        menu: if shared {
            format!("{} · {}", row.name, row.what).into()
        } else {
            row.name.clone()
        },
        incomplete: (row.mark == Mark::Incomplete).then(|| row.mark_words.clone()),
    }
}

impl ApplicationsPage {
    /// What the column lists.
    pub(crate) fn column(&self) -> &Column {
        &self.column
    }

    /// Changes whenever what the column shows does: its lines, or the
    /// application whose page shows.
    pub(crate) fn column_revision(&self) -> usize {
        self.column_revision
    }

    /// The application whose page shows in the list's place.
    pub(crate) fn shown_application(&self, cx: &App) -> Option<SharedString> {
        let (page, _) = self.open.as_ref()?;
        Some(page.read(cx).id().as_str().to_owned().into())
    }

    /// Derives the column again from the display, after a read or a read
    /// starting.
    pub(super) fn derive_column(&mut self) {
        let column = Column::new(
            &self.display,
            self.source.is_some(),
            self.snapshot.data().is_some() || self.snapshot.error().is_some(),
            self.pending,
        );
        if column != self.column {
            self.column = column;
            self.column_revision += 1;
        }
    }

    /// The column's row: that application's page, selected on the list
    /// for when it comes back. A filter or chip that hides it is cleared,
    /// so the list never comes back filtered against its selection.
    pub(crate) fn open_key(&mut self, key: &str, window: &mut Window, cx: &mut Context<Self>) {
        let Some(key) = self
            .display
            .rows
            .iter()
            .find(|row| row.key.as_ref() == key)
            .map(|row| row.key.clone())
        else {
            return;
        };
        // Its page shows already; a change followed from it gives way.
        if self.shown_application(cx).as_ref() == Some(&key) {
            self.close_change(window, cx);
            return;
        }
        if !self.lists(&key) {
            self.mark = None;
            self.filter
                .update(cx, |input, cx| input.set_value("", window, cx));
            self.refilter(cx);
        }
        self.select(key.clone(), cx);
        self.open_application(&key, window, cx);
    }

    /// Whether the list's lines show the application, under its filters.
    fn lists(&self, key: &SharedString) -> bool {
        self.lines
            .iter()
            .any(|entry| matches!(entry, Entry::Row(ix) if &self.display.rows[*ix].key == key))
    }

    /// All applications: the list, with the application whose page showed
    /// selected.
    pub(crate) fn show_list(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.close_application(window, cx);
    }
}

#[cfg(test)]
impl ApplicationsPage {
    /// Puts `column` in the column's place, as a read with that many
    /// applications would.
    pub(crate) fn set_column(&mut self, column: Column, cx: &mut Context<Self>) {
        self.column = column;
        self.column_revision += 1;
        cx.notify();
    }
}

#[cfg(test)]
mod tests {
    use super::super::{Read, Variant, example};
    use super::{Column, Display};

    fn acme() -> Display {
        let inputs = example::inputs(Variant::Acme);
        let read = Read::of(
            &inputs,
            example::labels(&inputs),
            example::connections("example:prod-fra"),
        );
        Display::new(&read.derived, &read.labels)
    }

    #[test]
    fn a_name_two_applications_share_says_what_each_is_in_the_menu() {
        let mut display = acme();
        // status-page's Argo CD Application takes cart's name.
        let ix = display
            .rows
            .iter()
            .position(|row| row.name.as_ref() == "status-page")
            .unwrap();
        display.rows[ix].name = "cart".into();
        let column = Column::new(&display, true, true, false);
        let menus: Vec<&str> = column
            .sections
            .iter()
            .flat_map(|section| &section.apps)
            .map(|app| app.menu.as_ref())
            .collect();
        assert_eq!(
            menus,
            [
                "cart · Kargo Project",
                "checkout",
                "catalog",
                "cart · Argo CD Application",
                "loyalty"
            ]
        );
    }

    #[test]
    fn a_read_in_flight_keeps_the_rows_and_hides_retry() {
        let display = acme();
        let reading = Column::new(&display, true, true, true);
        assert_eq!(reading.note, None, "a refresh keeps the rows");
        assert_eq!(reading.total, 5);
        let first = Column::new(&Display::default(), true, false, true);
        assert_eq!(first.note.map(|note| note.text), Some("Reading…".into()));
    }
}
