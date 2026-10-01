use talos_pilot_core::logs::{LogTarget, SingleServiceLogs};
use talos_pilot_core::types::LogLevel;

pub(crate) const RENDER_LIMIT: usize = 300;
pub(crate) const BATCH_LIMIT: usize = 256;
pub(crate) const MAX_LINE_BYTES: usize = 16 * 1024;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Target {
    pub context: String,
    pub node: Option<String>,
    pub address: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Ticket {
    pub config: u64,
    pub generation: u64,
    pub target: Option<Target>,
    pub service: Option<String>,
}

/// Independent request generations: refreshing never invalidates a live log stream.
#[derive(Default)]
pub(crate) struct Identity {
    pub config: u64,
    pub target: Option<Target>,
    snapshot: u64,
    stream: u64,
    service: Option<String>,
}

impl Identity {
    /// Loading even the same path invalidates results from its previous contents.
    pub fn reload(&mut self) -> u64 {
        self.config += 1;
        self.target = None;
        self.snapshot += 1;
        self.stop_stream();
        self.config
    }

    /// True means the caller must cancel both old snapshot and stream workers.
    pub fn select(&mut self, target: Option<Target>) -> bool {
        if self.target == target {
            return false;
        }
        self.target = target;
        self.snapshot += 1;
        self.stop_stream();
        true
    }

    pub fn snapshot(&mut self) -> Ticket {
        self.snapshot += 1;
        self.ticket(self.snapshot, None)
    }

    pub fn start_stream(&mut self, service: String) -> Ticket {
        self.stream += 1;
        self.service = Some(service.clone());
        self.ticket(self.stream, Some(service))
    }

    pub fn stop_stream(&mut self) {
        self.stream += 1;
        self.service = None;
    }

    pub fn accepts_snapshot(&self, ticket: &Ticket) -> bool {
        ticket == &self.ticket(self.snapshot, None)
    }

    pub fn accepts_stream(&self, ticket: &Ticket) -> bool {
        self.service.is_some() && ticket == &self.ticket(self.stream, self.service.clone())
    }

    fn ticket(&self, generation: u64, service: Option<String>) -> Ticket {
        Ticket {
            config: self.config,
            generation,
            target: self.target.clone(),
            service,
        }
    }
}

pub(crate) struct LogView {
    pub logs: SingleServiceLogs,
    pub received: usize,
    pub dropped: usize,
    pub truncated: usize,
}

impl LogView {
    pub fn new(address: String, service: String) -> Self {
        Self {
            logs: SingleServiceLogs::new(LogTarget::new(address, service)),
            received: 0,
            dropped: 0,
            truncated: 0,
        }
    }

    pub fn append(&mut self, lines: Vec<String>, dropped: usize) {
        self.received += lines.len() + dropped;
        self.dropped += dropped;
        let lines = lines
            .into_iter()
            .map(|mut line| {
                if line.len() > MAX_LINE_BYTES {
                    let mut boundary = MAX_LINE_BYTES;
                    while !line.is_char_boundary(boundary) {
                        boundary -= 1;
                    }
                    line.truncate(boundary);
                    line.push_str(" … [line truncated]");
                    self.truncated += 1;
                }
                line
            })
            .collect::<Vec<_>>();
        self.logs.append_batch(lines);
    }

    /// Newest filtered rows first: bounded DOM, live rows visible without JavaScript scrolling.
    pub fn rendered_indices(&self) -> (Vec<usize>, usize) {
        let visible = self.logs.buffer().visible_indices();
        let count = visible.len();
        (
            visible.into_iter().rev().take(RENDER_LIMIT).collect(),
            count,
        )
    }

    pub fn query(&mut self, query: String) {
        self.logs.buffer_mut().set_query(query);
    }

    pub fn toggle(&mut self, level: LogLevel) {
        let mut filters = self.logs.buffer().filters().clone();
        let active = !filters.levels.accepts(&level);
        filters.levels.set(&level, active);
        self.logs.buffer_mut().set_filters(filters);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use talos_pilot_core::constants::MAX_LOG_ENTRIES;

    fn node(context: &str, address: &str) -> Target {
        Target {
            context: context.into(),
            node: Some("node".into()),
            address: Some(address.into()),
        }
    }

    #[test]
    fn rejects_prior_config_even_for_same_target() {
        let mut identity = Identity::default();
        identity.reload();
        identity.select(Some(node("prod", "10.0.0.1")));
        let snapshot = identity.snapshot();
        let stream = identity.start_stream("kubelet".into());
        identity.reload();
        identity.select(Some(node("prod", "10.0.0.1")));
        assert!(!identity.accepts_snapshot(&snapshot));
        assert!(!identity.accepts_stream(&stream));
    }

    #[test]
    fn selection_change_requires_cancellation_and_rejects_old_events() {
        let mut identity = Identity::default();
        assert!(identity.select(Some(node("prod", "10.0.0.1"))));
        let snapshot = identity.snapshot();
        let stream = identity.start_stream("kubelet".into());
        assert!(!identity.select(Some(node("prod", "10.0.0.1"))));
        assert!(identity.accepts_snapshot(&snapshot));
        assert!(identity.accepts_stream(&stream));
        assert!(identity.select(Some(node("prod", "10.0.0.2"))));
        assert!(!identity.accepts_snapshot(&snapshot));
        assert!(!identity.accepts_stream(&stream));
    }

    #[test]
    fn stop_service_change_and_reconnect_invalidate_generation() {
        let mut identity = Identity::default();
        identity.select(Some(node("prod", "10.0.0.1")));
        let old = identity.start_stream("kubelet".into());
        let new = identity.start_stream("apid".into());
        assert!(!identity.accepts_stream(&old));
        assert!(identity.accepts_stream(&new));
        identity.stop_stream();
        assert!(!identity.accepts_stream(&new));
        let reconnect = identity.start_stream("apid".into());
        assert!(!identity.accepts_stream(&new));
        assert!(identity.accepts_stream(&reconnect));
        let snapshot = identity.snapshot();
        assert!(identity.accepts_stream(&reconnect));
        assert!(identity.accepts_snapshot(&snapshot));
        let newer = identity.snapshot();
        assert!(!identity.accepts_snapshot(&snapshot));
        assert!(identity.accepts_snapshot(&newer));
    }

    #[test]
    fn limits_retention_dom_and_unicode_line_bytes() {
        let mut view = LogView::new("10.0.0.1".into(), "kubelet".into());
        for start in (0..MAX_LOG_ENTRIES + 100).step_by(BATCH_LIMIT) {
            let end = (start + BATCH_LIMIT).min(MAX_LOG_ENTRIES + 100);
            view.append((start..end).map(|i| format!("info line {i}")).collect(), 0);
        }
        assert_eq!(view.logs.buffer().entries().len(), MAX_LOG_ENTRIES);
        let (rows, total) = view.rendered_indices();
        assert_eq!(total, MAX_LOG_ENTRIES);
        assert_eq!(rows.len(), RENDER_LIMIT);
        assert_eq!(rows[0], MAX_LOG_ENTRIES - 1);
        view.append(vec!["é".repeat(MAX_LINE_BYTES)], 7);
        assert_eq!(view.dropped, 7);
        assert_eq!(view.truncated, 1);
        assert!(view.logs.buffer().entries().last().unwrap().raw.len() < MAX_LINE_BYTES + 64);
    }

    #[test]
    fn uses_shared_case_insensitive_search_and_level_filters() {
        let mut view = LogView::new("node".into(), "kubelet".into());
        view.append(
            vec![
                "INFO Ready".into(),
                "error READY".into(),
                "warn waiting".into(),
            ],
            0,
        );
        view.query("ready".into());
        assert_eq!(view.rendered_indices().1, 2);
        view.toggle(LogLevel::Error);
        assert_eq!(view.rendered_indices().1, 1);
        view.query("missing".into());
        assert_eq!(view.rendered_indices().1, 0);
    }
}
