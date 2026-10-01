//! What the detail pane knows about one object: its document as lines ready
//! to draw and search, its events, and any Secret values revealed. Pure
//! state; the pane (`pane.rs`) reads the cluster and draws it.

use std::collections::BTreeMap;
use std::ops::Range;
use std::sync::Arc;
use std::time::{Duration, Instant};

use freshkube_core::resources::{
    EventUpdate, Failure, FailureKind, ObjectDocument, ObjectEvent, ResourceKind, SecretValue,
};
use gpui_kit::SharedString;

use super::model::ResourceIdentity;

/// The most events the pane lists; the count still says how many exist.
pub(crate) const MAX_EVENTS: usize = 200;
/// Longer lines are cut for drawing; copying still takes the whole line.
const MAX_DRAWN_CHARS: usize = 2_000;
/// A search stops counting here.
pub(crate) const MAX_MATCHES: usize = 10_000;
/// An open object is read again at most this often as the list reports
/// new versions of it.
pub(crate) const FOLLOW_INTERVAL: Duration = Duration::from_secs(1);

/// The object the pane shows: an identity from the list, with its kind.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct DetailTarget {
    pub(crate) identity: ResourceIdentity,
    pub(crate) kind: ResourceKind,
}

/// One line of YAML, ready to draw.
#[derive(Clone, Debug)]
pub(crate) struct YamlLine {
    /// Where the whole line lies in the document's YAML.
    range: Range<usize>,
    /// What is drawn: the line, cut short when very long.
    pub(crate) text: SharedString,
    /// How many bytes of the line `text` starts with; past them is a note
    /// on what was cut.
    pub(crate) drawn: usize,
    /// The mapping key the line starts with, tinted when drawn.
    pub(crate) key: Option<Range<usize>>,
}

/// A document with its YAML split into lines and prepared for search,
/// derived once when it arrives, never while drawing.
#[derive(Debug)]
pub(crate) struct DocumentView {
    pub(crate) document: ObjectDocument,
    pub(crate) lines: Vec<YamlLine>,
    /// The line with the most characters, which sets the scroll width.
    pub(crate) longest: usize,
    /// The YAML with ASCII letters lowercased, at the same byte offsets.
    folded: String,
}

impl DocumentView {
    pub(crate) fn new(document: ObjectDocument) -> Self {
        let yaml = &document.yaml;
        let mut lines = Vec::new();
        let mut block = None;
        let (mut longest, mut most) = (0, 0);
        let mut start = 0;
        for line in yaml.split_inclusive('\n') {
            let text = line.strip_suffix('\n').unwrap_or(line);
            let chars = text.chars().count();
            if chars > most {
                (longest, most) = (lines.len(), chars);
            }
            let (drawn, shown) = match text.char_indices().nth(MAX_DRAWN_CHARS) {
                Some((cut, _)) => (
                    cut,
                    format!(
                        "{} … {} more characters",
                        &text[..cut],
                        chars - MAX_DRAWN_CHARS
                    ),
                ),
                None => (text.len(), text.to_owned()),
            };
            lines.push(YamlLine {
                range: start..start + text.len(),
                key: line_key(text, &mut block).filter(|key| key.end <= drawn),
                text: shown.into(),
                drawn,
            });
            start += line.len();
        }
        Self {
            folded: yaml.to_ascii_lowercase(),
            document,
            lines,
            longest,
        }
    }

    /// The whole line, however long.
    pub(crate) fn line(&self, ix: usize) -> &str {
        &self.document.yaml[self.lines[ix].range.clone()]
    }

    /// Lines `range` as one text, the way they appear in the document.
    pub(crate) fn text(&self, range: Range<usize>) -> &str {
        let (first, last) = (&self.lines[range.start], &self.lines[range.end - 1]);
        &self.document.yaml[first.range.start..last.range.end]
    }

    /// Every place `query` occurs, ignoring ASCII case, as (line, byte range
    /// within the line), in document order and at most `MAX_MATCHES`.
    pub(crate) fn find(&self, query: &str) -> Vec<(usize, Range<usize>)> {
        let query = query.to_ascii_lowercase();
        let mut found = Vec::new();
        if query.is_empty() {
            return found;
        }
        for (ix, line) in self.lines.iter().enumerate() {
            let folded = &self.folded[line.range.clone()];
            for (at, _) in folded.match_indices(&query) {
                if found.len() == MAX_MATCHES {
                    return found;
                }
                found.push((ix, at..at + query.len()));
            }
        }
        found
    }
}

/// The mapping key a line of YAML starts with, if any. `block` tracks a
/// block scalar (`|`, `>`), whose lines are text, never keys: it holds the
/// indentation the scalar's lines must exceed.
fn line_key(line: &str, block: &mut Option<usize>) -> Option<Range<usize>> {
    let indent = line.len() - line.trim_start_matches(' ').len();
    if let Some(parent) = *block {
        if line.trim().is_empty() || indent > parent {
            return None;
        }
        *block = None;
    }
    // Sequence markers come before a key: `- name: x`, `- - a: b`.
    let mut start = indent;
    while line[start..].starts_with("- ") {
        start += 2;
    }
    let rest = &line[start..];
    let end = match rest.chars().next()? {
        quote @ ('"' | '\'') => {
            let close = rest[1..].find(quote)? + 2;
            rest[close..].starts_with(':').then_some(close)?
        }
        '#' | '[' | '{' | '|' | '>' | '&' | '*' | '!' | '?' | '%' | '@' | '`' => return None,
        _ => {
            let colon = match rest.find(": ") {
                Some(colon) => colon,
                None => rest.strip_suffix(':').map(str::len)?,
            };
            if rest[..colon].contains(" #") {
                return None;
            }
            colon
        }
    };
    let value = rest[end + 1..].trim();
    let mut indicator = value.chars();
    if matches!(indicator.next(), Some('|' | '>'))
        && indicator.all(|c| matches!(c, '-' | '+' | '0'..='9'))
    {
        *block = Some(start);
    }
    Some(start..start + end)
}

/// Lines picked in the YAML: from `anchor` to `cursor`, both included.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct LineSelection {
    pub(crate) anchor: usize,
    pub(crate) cursor: usize,
}

impl LineSelection {
    pub(crate) fn range(self) -> Range<usize> {
        self.anchor.min(self.cursor)..self.anchor.max(self.cursor) + 1
    }

    pub(crate) fn contains(self, ix: usize) -> bool {
        self.range().contains(&ix)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum DocumentRead {
    /// The first read of this object hasn't finished.
    Loading,
    Loaded,
    /// The identity may not read this object.
    Refused(String),
    /// Nothing was read, and the read failed.
    Failed(String),
    /// Reading it again failed; the document shown is the last one read.
    Stale(String),
    /// The object no longer exists; the document shown, if any, is the
    /// last one read.
    Deleted,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum EventsRead {
    Loading,
    Loaded,
    Refused(String),
    /// Nothing was listed, and listing failed.
    Failed(String),
    /// The watch broke and is retrying; the events shown may be out of date.
    Stale(String),
}

/// The events recorded about the object, newest first.
#[derive(Debug)]
pub(crate) struct EventList {
    read: EventsRead,
    events: Vec<ObjectEvent>,
    warnings: usize,
}

impl EventList {
    pub(crate) fn new() -> Self {
        Self {
            read: EventsRead::Loading,
            events: Vec::new(),
            warnings: 0,
        }
    }

    pub(crate) fn read(&self) -> &EventsRead {
        &self.read
    }

    /// The events to list: the newest `MAX_EVENTS`.
    pub(crate) fn shown(&self) -> &[ObjectEvent] {
        &self.events[..self.events.len().min(MAX_EVENTS)]
    }

    pub(crate) fn len(&self) -> usize {
        self.events.len()
    }

    pub(crate) fn warnings(&self) -> usize {
        self.warnings
    }

    pub(crate) fn apply(&mut self, update: EventUpdate) {
        match update {
            EventUpdate::Reset(events) => {
                self.events = events;
                self.read = EventsRead::Loaded;
            }
            EventUpdate::Upsert(event) => {
                match self.events.iter_mut().find(|known| known.uid == event.uid) {
                    Some(known) => *known = event,
                    None => self.events.push(event),
                }
            }
            EventUpdate::Delete(uid) => self.events.retain(|event| event.uid != uid),
            EventUpdate::Failed { failure, .. } => {
                self.read = match (&self.read, failure.kind) {
                    (_, FailureKind::Forbidden) => {
                        self.events.clear();
                        EventsRead::Refused(failure.message)
                    }
                    (EventsRead::Loaded | EventsRead::Stale(_), _) => {
                        EventsRead::Stale(failure.to_string())
                    }
                    _ => EventsRead::Failed(failure.to_string()),
                };
            }
        }
        // Newest first: by when each was last seen, then first seen.
        self.events.sort_by(|a, b| {
            (b.last_seen, b.first_seen, &b.uid).cmp(&(a.last_seen, a.first_seen, &a.uid))
        });
        self.warnings = self
            .events
            .iter()
            .filter(|event| event.is_warning())
            .count();
    }
}

/// A Secret value the user asked to see.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Reveal {
    Reading,
    Shown(SecretValue),
    Failed(String),
}

/// What is known about the open object.
#[derive(Debug)]
pub(crate) struct Detail {
    pub(crate) target: DetailTarget,
    pub(crate) read: DocumentRead,
    /// The last document read for this object.
    pub(crate) view: Option<Arc<DocumentView>>,
    /// An object with the same address but a new UID: this one was deleted
    /// and created again. Shown as a way to open it, never switched to.
    pub(crate) recreated: Option<ResourceIdentity>,
    pub(crate) events: EventList,
    /// Secret values revealed, by key. A new version of the Secret hides
    /// them all again.
    pub(crate) reveals: BTreeMap<String, Reveal>,
    /// Advances whenever `reveals` is cleared, so a reveal that lands after
    /// that is dropped.
    pub(crate) reveal_epoch: u64,
}

impl Detail {
    pub(crate) fn new(target: DetailTarget) -> Self {
        Self {
            target,
            read: DocumentRead::Loading,
            view: None,
            recreated: None,
            events: EventList::new(),
            reveals: BTreeMap::new(),
            reveal_epoch: 0,
        }
    }

    /// The version of the document shown, if one is.
    pub(crate) fn version(&self) -> Option<&str> {
        self.view
            .as_ref()
            .map(|view| view.document.resource_version.as_str())
    }

    pub(crate) fn clear_reveals(&mut self) {
        self.reveals.clear();
        self.reveal_epoch += 1;
    }

    /// A read of the object finished.
    pub(crate) fn finish_read(&mut self, result: Result<DocumentView, Failure>) {
        // The list is the authority on deletion; a read that started before
        // it saw the delete doesn't bring the object back.
        if self.read == DocumentRead::Deleted {
            return;
        }
        match result {
            Ok(view) if view.document.uid != self.target.identity.uid => {
                self.read = DocumentRead::Deleted;
                self.recreated = Some(ResourceIdentity {
                    uid: view.document.uid.clone(),
                    ..self.target.identity.clone()
                });
            }
            Ok(view) => {
                if self.version() != Some(view.document.resource_version.as_str()) {
                    self.clear_reveals();
                }
                self.view = Some(Arc::new(view));
                self.read = DocumentRead::Loaded;
            }
            Err(failure) if failure.kind == FailureKind::NotFound => {
                self.read = DocumentRead::Deleted;
            }
            Err(failure) => {
                let refused = failure.kind == FailureKind::Forbidden;
                let reason = if refused {
                    failure.message
                } else {
                    failure.to_string()
                };
                self.read = match (&self.view, refused) {
                    (Some(_), _) => DocumentRead::Stale(reason),
                    (None, true) => DocumentRead::Refused(reason),
                    (None, false) => DocumentRead::Failed(reason),
                };
            }
        }
    }

    /// The list no longer has the object; `successor` is one it has at the
    /// same address. Returns whether anything changed.
    pub(crate) fn gone(&mut self, successor: Option<ResourceIdentity>) -> bool {
        let changed = self.read != DocumentRead::Deleted || self.recreated != successor;
        self.read = DocumentRead::Deleted;
        self.recreated = successor;
        changed
    }
}

/// When to read an open object again as the list reports new versions of
/// it: once for each version seen after the last read began, and at most
/// once per `FOLLOW_INTERVAL`.
#[derive(Debug, Default)]
pub(crate) struct Follow {
    /// The latest version the list showed.
    seen: Option<String>,
    /// What `seen` was when the last read began.
    requested: Option<String>,
    last_read: Option<Instant>,
}

impl Follow {
    pub(crate) fn new(version: &str) -> Self {
        Self {
            seen: Some(version.to_owned()).filter(|version| !version.is_empty()),
            ..Self::default()
        }
    }

    pub(crate) fn observe(&mut self, version: &str) {
        if !version.is_empty() {
            self.seen = Some(version.to_owned());
        }
    }

    /// Whether the list has shown a version newer than both the document
    /// shown and whatever the last read could have returned.
    pub(crate) fn wanted(&self, shown: Option<&str>) -> bool {
        self.seen
            .as_deref()
            .is_some_and(|seen| Some(seen) != shown && self.requested.as_deref() != Some(seen))
    }

    /// How long until another read is allowed.
    pub(crate) fn delay(&self, now: Instant) -> Duration {
        self.last_read.map_or(Duration::ZERO, |last| {
            (last + FOLLOW_INTERVAL).saturating_duration_since(now)
        })
    }

    pub(crate) fn begin(&mut self, now: Instant) {
        self.requested.clone_from(&self.seen);
        self.last_read = Some(now);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use freshkube_core::resources::{builtin, object_from_yaml};

    const DEPLOYMENT: &str = "apiVersion: apps/v1
kind: Deployment
metadata:
  name: web
  namespace: shop
  uid: u-1
  resourceVersion: '7'
  annotations:
    note: |-
      first: not a key
      second
  labels:
    app: web
spec:
  replicas: 2
  template:
    spec:
      containers:
      - name: web
        image: web:1.0
        args:
        - --listen=:8080
        - 'quoted: value'
status:
  conditions:
  - type: Available
    message: 'Deployment has minimum availability: yes'
";

    fn view(yaml: &str) -> DocumentView {
        DocumentView::new(object_from_yaml(&builtin("deployments.apps").unwrap(), yaml).unwrap())
    }

    fn target(uid: &str) -> DetailTarget {
        DetailTarget {
            identity: ResourceIdentity {
                connection: "c".into(),
                resource: "deployments.apps".into(),
                namespace: "shop".into(),
                name: "web".into(),
                uid: uid.into(),
            },
            kind: builtin("deployments.apps").unwrap(),
        }
    }

    fn keys(view: &DocumentView) -> Vec<&str> {
        view.lines
            .iter()
            .filter_map(|line| line.key.clone().map(|key| &line.text[key]))
            .collect()
    }

    #[test]
    fn lines_tint_keys_but_not_block_text_or_list_values() {
        let view = view(DEPLOYMENT);
        assert_eq!(
            keys(&view),
            [
                "apiVersion",
                "kind",
                "metadata",
                "name",
                "namespace",
                "uid",
                "resourceVersion",
                "annotations",
                "note",
                "labels",
                "app",
                "spec",
                "replicas",
                "template",
                "spec",
                "containers",
                "name",
                "image",
                "args",
                "status",
                "conditions",
                "type",
                "message"
            ]
        );
        let longest = view.line(view.longest);
        assert!(longest.contains("minimum availability"), "{longest}");
        assert_eq!(view.text(0..2), "apiVersion: apps/v1\nkind: Deployment");
    }

    #[test]
    fn very_long_lines_are_cut_for_drawing_only() {
        let long = "x".repeat(MAX_DRAWN_CHARS + 50);
        let view = view(&format!(
            "apiVersion: v1\nkind: Deployment\nmetadata:\n  name: web\n  uid: u\n  annotations:\n    big: {long}\n"
        ));
        let ix = view.longest;
        assert_eq!(view.line(ix).len(), "    big: ".len() + long.len());
        assert!(view.lines[ix].text.ends_with("… 59 more characters"));
        assert_eq!(
            view.lines[ix].drawn,
            "    big: ".len() + MAX_DRAWN_CHARS - 9
        );
        assert_eq!(view.lines[ix].key, Some(4..7));
    }

    #[test]
    fn search_ignores_ascii_case_and_reports_lines_and_ranges() {
        let view = view(DEPLOYMENT);
        let found = view.find("WEB");
        let lines: Vec<&str> = found
            .iter()
            .map(|(line, range)| &view.line(*line)[range.clone()])
            .collect();
        assert_eq!(lines, ["web", "web", "web", "web"]);
        assert!(found.windows(2).all(|pair| pair[0].0 <= pair[1].0));
        assert!(view.find("").is_empty());
        assert!(view.find("absent").is_empty());
    }

    #[test]
    fn selections_cover_both_ends_in_either_direction() {
        let selection = LineSelection {
            anchor: 5,
            cursor: 2,
        };
        assert_eq!(selection.range(), 2..6);
        assert!(selection.contains(2) && selection.contains(5) && !selection.contains(6));
    }

    #[test]
    fn reads_load_go_stale_and_never_undo_a_deletion() {
        let mut detail = Detail::new(target("u-1"));
        assert_eq!(detail.read, DocumentRead::Loading);
        detail.finish_read(Err(Failure::new(FailureKind::Forbidden, "no")));
        assert_eq!(detail.read, DocumentRead::Refused("no".into()));
        detail.finish_read(Err(Failure::new(FailureKind::Unreachable, "down")));
        assert_eq!(
            detail.read,
            DocumentRead::Failed("Unreachable · down".into())
        );

        detail.finish_read(Ok(view(DEPLOYMENT)));
        assert_eq!(detail.read, DocumentRead::Loaded);
        assert_eq!(detail.version(), Some("7"));
        // With a document shown, a failed read again keeps it, marked stale.
        detail.finish_read(Err(Failure::new(FailureKind::Timeout, "slow")));
        assert_eq!(detail.read, DocumentRead::Stale("Timed out · slow".into()));
        assert!(detail.view.is_some());

        detail.finish_read(Err(Failure::new(FailureKind::NotFound, "gone")));
        assert_eq!(detail.read, DocumentRead::Deleted);
        assert!(detail.view.is_some());
        detail.finish_read(Ok(view(DEPLOYMENT)));
        assert_eq!(detail.read, DocumentRead::Deleted);
    }

    #[test]
    fn a_new_uid_at_the_address_is_offered_never_shown() {
        let mut detail = Detail::new(target("u-0"));
        detail.finish_read(Ok(view(DEPLOYMENT)));
        assert_eq!(detail.read, DocumentRead::Deleted);
        assert!(detail.view.is_none());
        assert_eq!(detail.recreated.as_ref().unwrap().uid, "u-1");

        let mut detail = Detail::new(target("u-1"));
        detail.finish_read(Ok(view(DEPLOYMENT)));
        assert!(detail.gone(None));
        assert!(!detail.gone(None));
        let successor = ResourceIdentity {
            uid: "u-2".into(),
            ..detail.target.identity.clone()
        };
        assert!(detail.gone(Some(successor.clone())));
        assert_eq!(detail.recreated, Some(successor));
        assert!(detail.view.is_some());
    }

    #[test]
    fn a_new_version_hides_revealed_values_again() {
        let mut detail = Detail::new(target("u-1"));
        detail.finish_read(Ok(view(DEPLOYMENT)));
        let epoch = detail.reveal_epoch;
        detail
            .reveals
            .insert("password".into(), Reveal::Shown(SecretValue::Binary(3)));
        // The same version read again keeps them.
        detail.finish_read(Ok(view(DEPLOYMENT)));
        assert_eq!(detail.reveals.len(), 1);
        detail.finish_read(Ok(view(&DEPLOYMENT.replace("'7'", "'8'"))));
        assert!(detail.reveals.is_empty());
        assert!(detail.reveal_epoch > epoch);
    }

    fn event(uid: &str, warning: bool, last: i64) -> ObjectEvent {
        ObjectEvent {
            uid: uid.into(),
            event_type: if warning { "Warning" } else { "Normal" }.into(),
            reason: "Reason".into(),
            message: "message".into(),
            count: 1,
            first_seen: chrono::DateTime::from_timestamp(last - 10, 0),
            last_seen: chrono::DateTime::from_timestamp(last, 0),
            source: "kubelet".into(),
            field_path: String::new(),
        }
    }

    #[test]
    fn events_sort_newest_first_count_warnings_and_cap_the_list() {
        let mut events = EventList::new();
        assert_eq!(events.read(), &EventsRead::Loading);
        events.apply(EventUpdate::Reset(vec![
            event("a", false, 10),
            event("b", true, 30),
            event("c", false, 20),
        ]));
        let order = |events: &EventList| -> Vec<String> {
            events
                .shown()
                .iter()
                .map(|event| event.uid.clone())
                .collect()
        };
        assert_eq!(order(&events), ["b", "c", "a"]);
        assert_eq!(events.warnings(), 1);
        events.apply(EventUpdate::Upsert(event("a", true, 40)));
        assert_eq!(order(&events), ["a", "b", "c"]);
        assert_eq!(events.warnings(), 2);
        events.apply(EventUpdate::Delete("b".into()));
        assert_eq!(order(&events), ["a", "c"]);

        let failure = |kind| EventUpdate::Failed {
            failure: Failure::new(kind, "x"),
            retrying: true,
        };
        events.apply(failure(FailureKind::Unreachable));
        assert_eq!(events.read(), &EventsRead::Stale("Unreachable · x".into()));
        assert_eq!(events.len(), 2);
        events.apply(failure(FailureKind::Forbidden));
        assert_eq!(events.read(), &EventsRead::Refused("x".into()));
        assert_eq!(events.len(), 0);
        let mut fresh = EventList::new();
        fresh.apply(failure(FailureKind::Timeout));
        assert_eq!(fresh.read(), &EventsRead::Failed("Timed out · x".into()));

        events.apply(EventUpdate::Reset(
            (0..MAX_EVENTS as i64 + 5)
                .map(|n| event(&n.to_string(), false, n))
                .collect(),
        ));
        assert_eq!(events.len(), MAX_EVENTS + 5);
        assert_eq!(events.shown().len(), MAX_EVENTS);
        assert_eq!(events.shown()[0].uid, (MAX_EVENTS + 4).to_string());
    }

    #[test]
    fn following_reads_each_new_version_once_and_at_most_once_a_second() {
        let start = Instant::now();
        let mut follow = Follow::new("1");
        // The first read covers the version the list showed.
        assert!(follow.wanted(None));
        follow.begin(start);
        assert!(!follow.wanted(None));
        // It returned a newer version than the list had shown: nothing to do,
        // even once the list catches up.
        assert!(!follow.wanted(Some("2")));
        follow.observe("2");
        assert!(!follow.wanted(Some("2")));
        // A version neither read nor requested is wanted, after the interval.
        follow.observe("3");
        assert!(follow.wanted(Some("2")));
        assert_eq!(
            follow.delay(start + Duration::from_millis(400)),
            Duration::from_millis(600)
        );
        assert_eq!(follow.delay(start + FOLLOW_INTERVAL), Duration::ZERO);
        follow.begin(start + FOLLOW_INTERVAL);
        // The read came back older than the list (it was already moving):
        // one more read is not wanted for the same seen version.
        assert!(!follow.wanted(Some("2")));
        follow.observe("");
        assert!(!follow.wanted(Some("2")));
    }
}
