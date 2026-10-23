//! What the shell's Applications column lists (docs/DESIGN.md, "The column
//! as a source list"): each application under the rule that found it, or
//! why there are none. The page derives it with each read and the column
//! reads it as it is; the column reads nothing of its own.
use super::*;
use display::{AppRow, RULES, rule_index, rule_label};
use freshkube_delivery::applications::Rule;

/// One application's row.
#[derive(Clone, Debug, PartialEq)]
pub struct ColumnApp {
    pub key: SharedString,
    pub name: SharedString,
    /// What it is and where, after its name in the row's tooltip.
    pub detail: SharedString,
    /// Its entry in the menu past the cap: its name, with what it is when
    /// another application shares the name.
    pub menu: SharedString,
    /// Why it may be incomplete, for its mark.
    pub incomplete: Option<SharedString>,
}

/// A rule's applications, under its section label, in the table's order.
#[derive(Clone, Debug, PartialEq)]
pub struct ColumnSection {
    pub rule: Rule,
    pub label: &'static str,
    pub apps: Vec<ColumnApp>,
}

/// Why the column lists no applications, in a few words, with the page's
/// whole sentence in its tooltip.
#[derive(Clone, Debug, PartialEq)]
pub struct ColumnNote {
    pub text: SharedString,
    pub tooltip: Option<SharedString>,
    /// A failed read offers Retry, as the page's state does.
    pub retry: bool,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Column {
    pub sections: Vec<ColumnSection>,
    pub note: Option<ColumnNote>,
    /// Every application, for All applications' count.
    pub total: usize,
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
    pub fn column(&self) -> &Column {
        &self.column
    }

    /// Changes whenever what the column shows does: its lines, or the
    /// application whose page shows.
    pub fn column_revision(&self) -> usize {
        self.column_revision
    }

    /// The application whose page shows in the list's place.
    pub fn shown_application(&self, cx: &App) -> Option<SharedString> {
        if self.changes_shown || self.waiting_shown {
            return None;
        }
        let (page, _) = self.open.as_ref()?;
        Some(page.read(cx).id().as_str().to_owned().into())
    }

    /// Derives the column again from the display, after a read or a read
    /// starting.
    pub(super) fn derive_column(&mut self) {
        let column = Column::new(
            &self.display,
            self.source.is_some(),
            self.loader.data().is_some() || self.loader.error().is_some(),
            self.loader.is_loading(),
        );
        if column != self.column {
            self.column = column;
            self.column_revision += 1;
        }
    }

    /// The column's row: that application's page, selected on the list
    /// for when it comes back. A filter or chip that hides it is cleared,
    /// so the list never comes back filtered against its selection.
    pub fn open_key(&mut self, key: &str, window: &mut Window, cx: &mut Context<Self>) {
        self.hide_changes(cx);
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
    pub fn show_list(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.hide_changes(cx);
        self.close_application(window, cx);
    }

    /// Changes: the Changes list in the list's place, which reads while it
    /// shows. A change it opened shows again; another press comes back to
    /// the list.
    pub fn show_changes(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.changes_shown {
            self.changes
                .update(cx, |changes, cx| changes.close_change(window, cx));
            return;
        }
        self.hide_waiting(cx);
        self.changes_shown = true;
        self.column_revision += 1;
        let visible = self.visible;
        self.changes.update(cx, |changes, cx| {
            changes.set_visible(visible, cx);
            changes.focus_shown(window, cx);
        });
        self.sync_timeline(cx);
        cx.notify();
    }

    /// Waiting to ship: the page in the list's place, which reads while it
    /// shows. Another press keeps it.
    pub fn show_waiting(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.waiting_shown {
            self.waiting
                .update(cx, |waiting, cx| waiting.focus(window, cx));
            return;
        }
        self.hide_changes_only(cx);
        self.waiting_shown = true;
        self.column_revision += 1;
        let visible = self.visible;
        self.waiting.update(cx, |waiting, cx| {
            waiting.set_visible(visible, cx);
            waiting.focus(window, cx);
        });
        self.sync_timeline(cx);
        cx.notify();
    }

    /// Whether Waiting to ship shows in the list's place.
    pub fn waiting_shown(&self) -> bool {
        self.waiting_shown
    }

    /// Waiting to ship, shown or not.
    #[cfg(any(test, feature = "testing"))]
    pub fn waiting_page(&self) -> &Entity<WaitingPage> {
        &self.waiting
    }

    /// Whether the Changes list shows in the list's place.
    pub fn changes_shown(&self) -> bool {
        self.changes_shown
    }

    /// The Changes list, shown or not.
    #[cfg(any(test, feature = "testing"))]
    pub fn changes_page(&self) -> &Entity<ChangesPage> {
        &self.changes
    }

    /// The list or an application's page again: the Changes list stops
    /// reading.
    fn hide_changes(&mut self, cx: &mut Context<Self>) {
        self.hide_waiting(cx);
        self.hide_changes_only(cx);
    }

    /// Waiting to ship stops reading.
    fn hide_waiting(&mut self, cx: &mut Context<Self>) {
        if !self.waiting_shown {
            return;
        }
        self.waiting_shown = false;
        self.column_revision += 1;
        self.waiting
            .update(cx, |waiting, cx| waiting.set_visible(false, cx));
        self.sync_timeline(cx);
        cx.notify();
    }

    fn hide_changes_only(&mut self, cx: &mut Context<Self>) {
        if !self.changes_shown {
            return;
        }
        self.changes_shown = false;
        self.column_revision += 1;
        self.changes
            .update(cx, |changes, cx| changes.set_visible(false, cx));
        self.sync_timeline(cx);
        cx.notify();
    }
}

#[cfg(any(test, feature = "testing"))]
impl ApplicationsPage {
    /// Puts `column` in the column's place, as a read with that many
    /// applications would.
    pub fn set_column(&mut self, column: Column, cx: &mut Context<Self>) {
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
            example::connections("example:prod-fra", None),
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
                "argocd",
                "loyalty"
            ]
        );
    }

    #[test]
    fn a_read_in_flight_keeps_the_rows_and_hides_retry() {
        let display = acme();
        let reading = Column::new(&display, true, true, true);
        assert_eq!(reading.note, None, "a refresh keeps the rows");
        assert_eq!(reading.total, 6);
        let first = Column::new(&Display::default(), true, false, true);
        assert_eq!(first.note.map(|note| note.text), Some("Reading…".into()));
    }
}
