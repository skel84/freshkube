//! What the dock would hold: the transcript the operator reads, and the
//! follower that decides when the main view moves.
//!
//! Following works like log follow. While on, the view goes where the agent
//! reads, but stays at least `DWELL` on each place so the operator can see it;
//! the latest place waits and the ones in between are skipped. Any operator
//! navigation turns following off until they turn it on again.

use std::time::{Duration, Instant};

use rpi_agent::{AgentEvent, AgentMessage};
use rpi_ai::types::{AssistantMessageEvent, StopReason};

use crate::session::Update;
use crate::tools::Place;

pub const DWELL: Duration = Duration::from_millis(1500);

/// What a backend reports, in the dock's terms. The embedded loop's updates
/// and an ACP agent's session updates both become these.
#[derive(Debug, Clone, PartialEq)]
pub enum Event {
    /// A chunk of a thought; consecutive chunks join.
    Thought(String),
    /// A chunk of what the agent says; consecutive chunks join.
    Said(String),
    StepStarted {
        id: String,
        tool: String,
        args: serde_json::Value,
    },
    /// A step's label or arguments became known after it started.
    StepDescribed {
        id: String,
        tool: Option<String>,
        args: Option<serde_json::Value>,
    },
    StepEnded {
        id: String,
        failed: bool,
    },
    /// Something the operator should see that isn't the agent's own words,
    /// such as a refused permission.
    Note(String),
    /// One of our tools read something. The embedded loop knows the step's
    /// id; an MCP call doesn't, so it goes to the latest step for that tool
    /// that has no place yet.
    Read {
        step: Option<String>,
        tool: String,
        place: Place,
    },
    /// The first outcome reported wins.
    Ended(Outcome),
}

#[derive(Debug, Clone, PartialEq)]
pub enum Entry {
    Thought(String),
    Said(String),
    Note(String),
    Step {
        id: String,
        tool: String,
        args: serde_json::Value,
        place: Option<Place>,
        state: StepState,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StepState {
    Running,
    Done,
    Failed,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Outcome {
    Running,
    Answered,
    Cancelled,
    Failed(String),
}

pub struct Investigation {
    pub entries: Vec<Entry>,
    pub outcome: Outcome,
    pub following: bool,
    /// Where the main view is.
    pub view: Option<Place>,
    /// Reads no transcript step claimed; the ACP backend's measure of how
    /// well MCP calls line up with the agent's tool calls.
    pub unmatched_reads: usize,
    pending: Option<Place>,
    moved_at: Option<Instant>,
}

impl Default for Investigation {
    fn default() -> Self {
        Self {
            entries: Vec::new(),
            outcome: Outcome::Running,
            following: true,
            view: None,
            unmatched_reads: 0,
            pending: None,
            moved_at: None,
        }
    }
}

impl Investigation {
    /// Records one event; returns a navigation for the main view, if any.
    pub fn record(&mut self, event: Event, now: Instant) -> Option<Place> {
        match event {
            Event::Thought(chunk) => match self.entries.last_mut() {
                Some(Entry::Thought(text)) => text.push_str(&chunk),
                _ => self.entries.push(Entry::Thought(chunk)),
            },
            Event::Said(chunk) => match self.entries.last_mut() {
                Some(Entry::Said(text)) => text.push_str(&chunk),
                _ => self.entries.push(Entry::Said(chunk)),
            },
            Event::StepStarted { id, tool, args } => self.entries.push(Entry::Step {
                id,
                tool,
                args,
                place: None,
                state: StepState::Running,
            }),
            Event::StepDescribed { id, tool, args } => {
                if let Some(Entry::Step {
                    tool: label,
                    args: slot,
                    ..
                }) = self.step_mut(&id)
                {
                    if let Some(tool) = tool {
                        *label = tool;
                    }
                    if let Some(args) = args {
                        *slot = args;
                    }
                }
            }
            Event::Note(text) => self.entries.push(Entry::Note(text)),
            Event::StepEnded { id, failed } => {
                if let Some(Entry::Step { state, .. }) = self.step_mut(&id) {
                    *state = if failed {
                        StepState::Failed
                    } else {
                        StepState::Done
                    };
                }
            }
            Event::Read { step, tool, place } => {
                let slot = match step {
                    Some(id) => self.step_mut(&id),
                    None => self.entries.iter_mut().rev().find(|e| {
                        matches!(e, Entry::Step { tool: t, place: None, .. } if t.contains(tool.as_str()))
                    }),
                };
                match slot {
                    Some(Entry::Step { place: slot, .. }) => *slot = Some(place.clone()),
                    _ => self.unmatched_reads += 1,
                }
                return self.offer(place, now);
            }
            Event::Ended(outcome) => {
                if self.outcome == Outcome::Running {
                    self.outcome = outcome;
                }
            }
        }
        None
    }

    /// Applies one update from the embedded loop.
    pub fn apply(&mut self, update: Update, now: Instant) -> Option<Place> {
        let event = match update {
            Update::Agent(event) => embedded_event(*event)?,
            Update::Follow {
                tool_call_id,
                place,
            } => Event::Read {
                step: Some(tool_call_id),
                tool: String::new(),
                place,
            },
            Update::Finished(Ok(())) => Event::Ended(Outcome::Answered),
            Update::Finished(Err(e)) => Event::Ended(Outcome::Failed(e)),
        };
        self.record(event, now)
    }

    /// Called from a timer while a place is pending.
    pub fn tick(&mut self, now: Instant) -> Option<Place> {
        let place = self.pending.take()?;
        self.offer(place, now)
    }

    pub fn has_pending(&self) -> bool {
        self.pending.is_some()
    }

    /// The operator navigated: stop following until asked again.
    pub fn operator_moved(&mut self, to: Place) {
        self.following = false;
        self.pending = None;
        self.view = Some(to);
    }

    /// Resume following and jump to the agent's latest place.
    pub fn follow_again(&mut self, now: Instant) -> Option<Place> {
        self.following = true;
        let latest = self.entries.iter().rev().find_map(|e| match e {
            Entry::Step { place, .. } => place.clone(),
            _ => None,
        })?;
        self.moved_at = None;
        self.offer(latest, now)
    }

    fn offer(&mut self, place: Place, now: Instant) -> Option<Place> {
        if !self.following || self.view.as_ref() == Some(&place) {
            return None;
        }
        let rested = self
            .moved_at
            .is_none_or(|at| now.duration_since(at) >= DWELL);
        if rested {
            self.pending = None;
            self.moved_at = Some(now);
            self.view = Some(place.clone());
            Some(place)
        } else {
            self.pending = Some(place);
            None
        }
    }

    fn step_mut(&mut self, id: &str) -> Option<&mut Entry> {
        self.entries
            .iter_mut()
            .rev()
            .find(|e| matches!(e, Entry::Step { id: step, .. } if step == id))
    }
}

fn embedded_event(event: AgentEvent) -> Option<Event> {
    Some(match event {
        AgentEvent::MessageUpdate {
            assistant_message_event,
            ..
        } => match assistant_message_event {
            AssistantMessageEvent::ThinkingStart { .. } => Event::Thought(String::new()),
            AssistantMessageEvent::ThinkingDelta { delta, .. } => Event::Thought(delta),
            AssistantMessageEvent::TextStart { .. } => Event::Said(String::new()),
            AssistantMessageEvent::TextDelta { delta, .. } => Event::Said(delta),
            _ => return None,
        },
        // A cancelled or failed model call ends its message with that stop
        // reason, and the loop itself still returns Ok.
        AgentEvent::MessageEnd {
            message: AgentMessage::Assistant(message),
        } => match message.stop_reason {
            StopReason::Aborted => Event::Ended(Outcome::Cancelled),
            StopReason::Error => {
                Event::Ended(Outcome::Failed(message.error_message.unwrap_or_default()))
            }
            _ => return None,
        },
        AgentEvent::ToolExecutionStart {
            tool_call_id,
            tool_name,
            args,
        } => Event::StepStarted {
            id: tool_call_id,
            tool: tool_name,
            args,
        },
        AgentEvent::ToolExecutionEnd {
            tool_call_id,
            is_error,
            ..
        } => Event::StepEnded {
            id: tool_call_id,
            failed: is_error,
        },
        _ => return None,
    })
}
