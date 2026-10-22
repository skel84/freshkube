//! The page's side of frozen rows (#877, `crate::frozen`): freezing the list
//! a tab leaves, showing it again, muted, while the list is read again, and
//! telling which rows changed meanwhile once the new list lands.

use std::collections::HashMap;
use std::time::{Duration, SystemTime};

use super::*;
use crate::frozen::FrozenRows;

/// How long a freeze waits for the restart that shows it. A list restarted
/// later is not what the tab returned to.
const HOLD_LIFE: Duration = Duration::from_secs(2);

/// The rows on screen are frozen ones: no list has landed since.
pub(super) struct Thawed {
    kept: usize,
    total: usize,
    captured: SystemTime,
    /// The banner's words, derived when the rows freeze and again as the
    /// age clock ticks, so render only draws them.
    pub(super) text: SharedString,
    /// Each row's version when it froze.
    versions: HashMap<ResourceIdentity, String>,
}

impl Thawed {
    /// "200 of 1,204 kept · last known 4 min ago. …", at `now`.
    fn derive_text(&mut self, now: i64) {
        let age = now.saturating_sub(
            self.captured
                .duration_since(SystemTime::UNIX_EPOCH)
                .map_or(0, |since| since.as_secs() as i64),
        );
        let kept = if self.kept == self.total {
            format!("{} kept", count(self.kept))
        } else {
            format!("{} of {} kept", count(self.kept), count(self.total))
        };
        self.text = format!(
            "{kept} · last known {} ago. Listing again; changed rows flash when it answers.",
            ago(age)
        )
        .into();
    }
}

impl ResourcesScreen {
    /// The age clock ticked: the banner's words again.
    pub(super) fn tick_frozen(&mut self, cx: &mut Context<Self>) {
        let now = self.now;
        if let Some(thawed) = &mut self.frozen {
            thawed.derive_text(now);
            cx.notify();
        }
    }

    /// The rows on screen, frozen, if they are the kind and namespace asked
    /// for. Nothing for a page with no rows to keep: still loading, refused,
    /// over [`crate::frozen::MAX_ROWS`] rows, or the node pane's list.
    pub fn freeze(&self, kind: &str, namespace: Option<&str>) -> Option<FrozenRows> {
        let source = self.source.as_ref()?;
        let listed = self.namespace.as_deref().filter(|_| self.kind.namespaced);
        if self.embedded || self.kind.key() != kind || listed != namespace {
            return None;
        }
        let (captured, total) = match &self.frozen {
            // Frozen rows freeze again as they were, still last known then.
            Some(thawed) => (thawed.captured, thawed.total),
            None if self.store.read_state().shows_rows() => (
                self.updated.unwrap_or_else(SystemTime::now),
                self.store.len(),
            ),
            None => return None,
        };
        FrozenRows::capture(
            (
                self.kind.key(),
                listed.map(str::to_owned),
                source.id.clone(),
            ),
            self.store.columns().to_vec(),
            self.store.entries().iter().map(|entry| entry.row().clone()),
            total,
            captured,
        )
    }

    /// Whether the rows shown are from before the page last stopped
    /// showing. An action on a row (Restart, Delete) must stay disabled
    /// while they are: it would act on what the list showed minutes ago.
    pub fn rows_are_frozen(&self) -> bool {
        self.frozen.is_some()
    }

    /// Shows `frozen` in place of the loading rows at the next read of its
    /// kind and namespace, if that comes soon.
    pub fn hold_frozen(&mut self, frozen: FrozenRows, cx: &App) {
        self.pending_frozen = Some((frozen, cx.background_executor().now()));
    }

    fn accepts(&self, frozen: &FrozenRows) -> bool {
        self.source
            .as_ref()
            .is_some_and(|source| source.id == frozen.connection)
            && self.kind.key() == frozen.kind
            && self.namespace.as_deref().filter(|_| self.kind.namespaced)
                == frozen.namespace.as_deref()
    }

    /// Before a read starts over: keeps rows that are still frozen for the
    /// new one, so Retry does not empty the list, and forgets a freeze that
    /// is not for this read.
    pub(super) fn carry_frozen(&mut self, cx: &App) {
        let now = cx.background_executor().now();
        if self.pending_frozen.is_none()
            && self.frozen.is_some()
            && let Some(frozen) = self.freeze(&self.kind.key(), thawed_namespace(self))
        {
            self.pending_frozen = Some((frozen, now));
        }
        if self
            .pending_frozen
            .as_ref()
            .is_some_and(|(frozen, at)| !self.accepts(frozen) || now - *at > HOLD_LIFE)
        {
            self.pending_frozen = None;
        }
        // Whatever shows until the read answers is the new read's to say.
        self.frozen = None;
    }

    /// After the store started its session: fills it with the held rows
    /// while the page shows, leaving the read `Loading`.
    pub(super) fn seed_frozen(&mut self, epoch: u64, cx: &mut Context<Self>) {
        if !self.visible || self.source.is_none() {
            return;
        }
        let Some((frozen, _)) = self.pending_frozen.take() else {
            return;
        };
        let mut thawed = Thawed {
            kept: frozen.kept(),
            total: frozen.total,
            captured: frozen.captured,
            text: SharedString::default(),
            versions: frozen.versions(),
        };
        thawed.derive_text(self.now);
        let reset = ResourceEvent::reset(frozen.columns, frozen.rows);
        if self.store.apply(epoch, vec![vec![reset]], 0).is_none() {
            return;
        }
        self.projection.rebuild(&self.store);
        self.layout = TableLayout::new(
            &self.store,
            self.lists_all_namespaces(),
            self.lists_pods(),
            !self.embedded,
        );
        self.layout.hide(&self.hidden_columns);
        self.layout_for = Some((self.kind.clone(), self.lists_all_namespaces()));
        self.updated = Some(thawed.captured);
        self.frozen = Some(thawed);
        cx.notify();
    }

    /// The first list after frozen rows: the rows of it that changed since
    /// they froze, for the flash.
    pub(super) fn changed_since_frozen(&self, thawed: &Thawed, keep: usize) -> Changed {
        let changed = self
            .store
            .entries()
            .iter()
            .map(|entry| entry.row())
            .filter(|row| {
                thawed
                    .versions
                    .get(&row.identity)
                    .is_some_and(|version| *version != row.resource_version)
            });
        let count = changed.clone().count();
        if count > keep {
            return Changed::Many(count);
        }
        Changed::Rows(changed.map(|row| row.identity.clone()).collect())
    }

    /// "200 of 1,204 kept · last known 4 min ago", over the muted rows
    /// until the list answers; a failed read shows the stale banner
    /// instead, which names the same time.
    pub(super) fn frozen_banner(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let thawed = self.frozen.as_ref()?;
        if matches!(self.store.read_state(), ReadState::Stale(_)) {
            return None;
        }
        Some(
            div()
                .id("resource-frozen")
                .test_support()
                .role(Role::Status)
                .child(ui::banner(
                    ui::Tone::Info,
                    None,
                    thawed.text.clone(),
                    None,
                    cx,
                ))
                .into_any_element(),
        )
    }
}

fn thawed_namespace(screen: &ResourcesScreen) -> Option<&str> {
    screen
        .namespace
        .as_deref()
        .filter(|_| screen.kind.namespaced)
}

/// `1,204`.
fn count(n: usize) -> String {
    let digits = n.to_string();
    let mut out = String::new();
    for (at, digit) in digits.chars().enumerate() {
        if at > 0 && (digits.len() - at).is_multiple_of(3) {
            out.push(',');
        }
        out.push(digit);
    }
    out
}

/// `moments`, `4 min`, `2 h`.
fn ago(seconds: i64) -> String {
    match seconds {
        ..60 => "moments".to_owned(),
        60..3600 => format!("{} min", seconds / 60),
        _ => format!("{} h", seconds / 3600),
    }
}

#[cfg(test)]
mod tests {
    use super::{ago, count};

    #[test]
    fn counts_have_thousands_separators() {
        assert_eq!(count(7), "7");
        assert_eq!(count(200), "200");
        assert_eq!(count(1_204), "1,204");
        assert_eq!(count(12_345), "12,345");
    }

    #[test]
    fn ages_are_rough() {
        assert_eq!(ago(5), "moments");
        assert_eq!(ago(240), "4 min");
        assert_eq!(ago(7_300), "2 h");
    }
}
