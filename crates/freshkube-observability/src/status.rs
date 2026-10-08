//! The page's segment of the status bar: the Coroot it reads, and on
//! Applications, Incidents and Traces how much they list, whether it is
//! stale and when it last changed.
use super::*;
use freshkube_ui::status::{Part, Segment};
use std::time::SystemTime;

/// What a list puts in the segment.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct Read<'a> {
    pub count: &'a str,
    pub stale: bool,
    pub time: Option<SystemTime>,
    /// What the tooltip adds, such as how the list was sampled.
    pub note: Option<&'static str>,
}

/// What the segment was last derived from.
#[derive(Default)]
pub(super) struct StatusKey {
    url: Option<String>,
    destination: Option<Destination>,
    read: Option<(String, bool, Option<SystemTime>, Option<&'static str>)>,
}

impl ObservabilityPage {
    /// The page's segment, derived again only when what it shows changes,
    /// so the shell can read it on every frame.
    pub fn status(&mut self) -> &Segment {
        let url = self.live.provider.as_ref().map(|provider| provider.url());
        let read = self.status_read();
        let key = &self.status.0;
        let known = key.read.as_ref().map(|(count, stale, time, note)| Read {
            count,
            stale: *stale,
            time: *time,
            note: *note,
        });
        if key.url.as_deref() != url || key.destination != Some(self.destination) || known != read {
            let key = StatusKey {
                url: url.map(str::to_owned),
                destination: Some(self.destination),
                read: read.map(|read| (read.count.to_owned(), read.stale, read.time, read.note)),
            };
            self.status = (key, self.derive_status());
        }
        &self.status.1
    }

    fn status_read(&self) -> Option<Read<'_>> {
        match self.destination {
            Destination::Applications => Some(self.applications_read()),
            Destination::Incidents => Some(self.incidents_read()),
            Destination::Traces => Some(self.traces_read()),
            _ => None,
        }
    }

    fn derive_status(&self) -> Segment {
        let source = self.live.provider.as_ref().map_or_else(
            || {
                if self.fixture {
                    "Example data"
                } else {
                    "Not connected"
                }
                .to_owned()
            },
            |provider| {
                let url = provider.url();
                url.split_once("://")
                    .map_or(url, |(_, rest)| rest)
                    .trim_end_matches('/')
                    .to_owned()
            },
        );
        let mut parts = vec![Part::new(source)];
        let read = self.status_read();
        if let Some(read) = &read {
            parts.push(Part::new(read.count.to_owned()));
            if read.stale {
                parts.push(Part::new("stale").tone(freshkube_ui::ui::Tone::Warn));
            }
            parts.extend(read.time.map(|time| Part::new(ui::clock(time))));
        }
        let segment = Segment::new(None::<SharedString>, parts);
        match read.and_then(|read| read.note) {
            Some(note) => segment.note(note),
            None => segment,
        }
    }

    /// When the list last changed: Coroot's last answer, or the example's
    /// end.
    pub(super) fn read_time(&self, last: Option<SystemTime>) -> Option<SystemTime> {
        last.or_else(|| self.live.range.to.filter(|_| self.fixture).map(Into::into))
    }
}
