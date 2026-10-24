//! The transcript an agent's session leaves: what was asked, the agent's
//! thoughts and speech, its steps (their state and arguments, updated in
//! place) and notes such as a refused tool.
//!
//! Everything the agent sends is bounded here, whatever it sends:
//!
//! - a chunk of text is cut at [`MAX_CHUNK_BYTES`];
//! - the text of the transcript holds at most [`MAX_TRANSCRIPT_BYTES`] in at
//!   most [`MAX_TEXT_ENTRIES`] thought and speech entries; past either the
//!   oldest thought text is dropped first, then the oldest speech. An entry
//!   that loses all its text is removed, so what the transcript holds is
//!   bounded in memory as well as in bytes, and [`Investigation::trimmed`]
//!   says that earlier text is gone;
//! - a step's shown arguments are cut at [`MAX_ARGS_BYTES`];
//! - a session keeps at most [`MAX_STEPS`] steps and [`MAX_NOTES`] notes.
//!
//! The follower that moves the page is not here; it comes with the dock tab.

use serde_json::Value;

/// Text in a transcript: what was asked, thoughts, speech and notes.
pub const MAX_TRANSCRIPT_BYTES: usize = 1024 * 1024;
/// A step's arguments as shown.
pub const MAX_ARGS_BYTES: usize = 4 * 1024;
/// Steps in one session; later ones are refused with one note.
pub const MAX_STEPS: usize = 1_000;
/// One chunk of agent text.
pub const MAX_CHUNK_BYTES: usize = 64 * 1024;
/// Notes in one session.
pub const MAX_NOTES: usize = 1_000;
/// Thought and speech entries held at once.
pub const MAX_TEXT_ENTRIES: usize = 2_000;

const MAX_NOTE_BYTES: usize = 512;
const MAX_LABEL_BYTES: usize = 256;
const MAX_ID_BYTES: usize = 128;

/// What a session reports, in the dock's terms. The ACP client turns the
/// agent's session updates into these.
#[derive(Debug, Clone, PartialEq)]
pub enum Event {
    /// The person's prompt; it starts a turn.
    Prompt(String),
    /// A chunk of a thought; consecutive chunks join.
    Thought(String),
    /// A chunk of what the agent says; consecutive chunks join.
    Said(String),
    StepStarted {
        id: String,
        tool: String,
        args: Args,
    },
    /// A step's label or arguments became known after it started.
    StepDescribed {
        id: String,
        tool: Option<String>,
        args: Option<Args>,
    },
    StepEnded {
        id: String,
        failed: bool,
    },
    /// Something the person should see that isn't the agent's own words, such
    /// as a refused permission.
    Note(String),
    /// A tool call was refused, by the name the agent gave it (`None`: it
    /// gave none). The transcript says nothing of it: the note does.
    Refused {
        name: Option<String>,
    },
    /// How the turn ended; the first outcome of a turn wins.
    Ended(Outcome),
}

/// A step's arguments as shown: compact JSON cut at [`MAX_ARGS_BYTES`].
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Args {
    shown: String,
    cut: bool,
}

impl Args {
    pub fn from_value(value: &Value) -> Self {
        if value.is_null() {
            return Self::default();
        }
        let text = value.to_string();
        let shown = clip(&text, MAX_ARGS_BYTES);
        Self {
            cut: shown.len() < text.len(),
            shown: shown.to_string(),
        }
    }

    pub fn shown(&self) -> &str {
        &self.shown
    }

    /// True when the arguments were longer than what is shown.
    pub fn is_cut(&self) -> bool {
        self.cut
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum Entry {
    /// A prompt the person sent; `failed` once its turn ended as failed, so
    /// two failed turns read as two.
    Asked {
        text: String,
        failed: bool,
    },
    Thought(Text),
    Said(Text),
    Note(String),
    Step(Step),
}

/// Thought or speech text; `trimmed` is set once its oldest part was dropped
/// to keep the transcript within its bound.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Text {
    pub text: String,
    pub trimmed: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Step {
    pub id: String,
    pub tool: String,
    pub args: Args,
    pub state: StepState,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StepState {
    Running,
    Done,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    Running,
    Answered,
    Cancelled,
    Failed(String),
}

impl Outcome {
    /// A failure with a bounded reason.
    pub fn failed(reason: impl AsRef<str>) -> Self {
        Self::Failed(clip(reason.as_ref(), MAX_NOTE_BYTES).to_string())
    }
}

#[derive(Debug, Default)]
pub struct Investigation {
    entries: Vec<Entry>,
    outcome: Option<Outcome>,
    text_bytes: usize,
    text_entries: usize,
    steps: usize,
    notes: usize,
    trimmed: bool,
    steps_refused: bool,
}

impl Investigation {
    pub fn entries(&self) -> &[Entry] {
        &self.entries
    }

    /// How the current turn stands; `Running` until it ends.
    pub fn outcome(&self) -> Outcome {
        self.outcome.clone().unwrap_or(Outcome::Running)
    }

    /// Bytes of transcript text held now, at most [`MAX_TRANSCRIPT_BYTES`].
    pub fn text_bytes(&self) -> usize {
        self.text_bytes
    }

    /// True once any text was dropped to stay within the bound.
    pub fn trimmed(&self) -> bool {
        self.trimmed
    }

    pub fn step_count(&self) -> usize {
        self.steps
    }

    pub fn record(&mut self, event: Event) {
        match event {
            Event::Prompt(text) => {
                self.outcome = None;
                let text = clip(&text, MAX_CHUNK_BYTES).to_string();
                self.text_bytes += text.len();
                self.entries.push(Entry::Asked {
                    text,
                    failed: false,
                });
            }
            Event::Thought(chunk) => self.append(chunk, Kind::Thought),
            Event::Said(chunk) => self.append(chunk, Kind::Said),
            Event::StepStarted { id, tool, args } => self.start_step(id, tool, args),
            Event::StepDescribed { id, tool, args } => {
                if let Some(step) = self.step_mut(&id) {
                    if let Some(tool) = tool {
                        step.tool = clip(&tool, MAX_LABEL_BYTES).to_string();
                    }
                    if let Some(args) = args {
                        step.args = args;
                    }
                }
            }
            Event::StepEnded { id, failed } => {
                if let Some(step) = self.step_mut(&id) {
                    step.state = if failed {
                        StepState::Failed
                    } else {
                        StepState::Done
                    };
                }
            }
            Event::Note(text) => self.note(&text),
            Event::Refused { .. } => {}
            Event::Ended(outcome) => {
                if self.outcome.is_none() {
                    if matches!(outcome, Outcome::Failed(_))
                        && let Some(Entry::Asked { failed, .. }) = self
                            .entries
                            .iter_mut()
                            .rev()
                            .find(|entry| matches!(entry, Entry::Asked { .. }))
                    {
                        *failed = true;
                    }
                    self.outcome = Some(outcome);
                }
            }
        }
        self.enforce_bound();
    }

    fn note(&mut self, text: &str) {
        if self.notes >= MAX_NOTES {
            return;
        }
        self.notes += 1;
        let text = clip(text, MAX_NOTE_BYTES).to_string();
        self.text_bytes += text.len();
        self.entries.push(Entry::Note(text));
    }

    fn append(&mut self, chunk: String, kind: Kind) {
        let chunk = clip(&chunk, MAX_CHUNK_BYTES);
        if chunk.is_empty() {
            return;
        }
        self.text_bytes += chunk.len();
        match (self.entries.last_mut(), kind) {
            (Some(Entry::Thought(text)), Kind::Thought) | (Some(Entry::Said(text)), Kind::Said) => {
                text.text.push_str(chunk)
            }
            _ => {
                self.text_entries += 1;
                let text = Text {
                    text: chunk.to_string(),
                    trimmed: false,
                };
                self.entries.push(match kind {
                    Kind::Thought => Entry::Thought(text),
                    Kind::Said => Entry::Said(text),
                });
            }
        }
    }

    fn start_step(&mut self, id: String, tool: String, args: Args) {
        let id = clip(&id, MAX_ID_BYTES).to_string();
        if let Some(step) = self.step_mut(&id) {
            step.tool = clip(&tool, MAX_LABEL_BYTES).to_string();
            step.args = args;
            return;
        }
        if self.steps >= MAX_STEPS {
            if !self.steps_refused {
                self.steps_refused = true;
                self.note(&format!("Step limit reached: {MAX_STEPS} steps are kept"));
            }
            return;
        }
        self.steps += 1;
        self.entries.push(Entry::Step(Step {
            id,
            tool: clip(&tool, MAX_LABEL_BYTES).to_string(),
            args,
            state: StepState::Running,
        }));
    }

    fn step_mut(&mut self, id: &str) -> Option<&mut Step> {
        self.entries.iter_mut().rev().find_map(|entry| match entry {
            Entry::Step(step) if step.id == id => Some(step),
            _ => None,
        })
    }

    /// Drops the oldest thought text first, then the oldest speech, until the
    /// text fits in bytes and in entries. An entry left empty is removed.
    fn enforce_bound(&mut self) {
        while self.text_bytes > MAX_TRANSCRIPT_BYTES || self.text_entries > MAX_TEXT_ENTRIES {
            let Some(index) = self.oldest_trimmable() else {
                return;
            };
            let excess = self.text_bytes.saturating_sub(MAX_TRANSCRIPT_BYTES);
            let (Entry::Thought(text) | Entry::Said(text)) = &mut self.entries[index] else {
                return;
            };
            let before = text.text.len();
            self.trimmed = true;
            if before <= excess || self.text_entries > MAX_TEXT_ENTRIES {
                // The whole entry goes.
                self.text_bytes -= before;
                self.text_entries -= 1;
                self.entries.remove(index);
            } else {
                let mut cut = excess;
                while !text.text.is_char_boundary(cut) {
                    cut += 1;
                }
                text.text.drain(..cut);
                text.trimmed = true;
                self.text_bytes -= cut;
            }
        }
    }

    /// The oldest thought, else the oldest speech.
    fn oldest_trimmable(&self) -> Option<usize> {
        let position = |wanted: Kind| {
            self.entries.iter().position(|entry| {
                matches!(
                    (entry, wanted),
                    (Entry::Thought(_), Kind::Thought) | (Entry::Said(_), Kind::Said)
                )
            })
        };
        position(Kind::Thought).or_else(|| position(Kind::Said))
    }
}

#[derive(Clone, Copy)]
enum Kind {
    Thought,
    Said,
}

/// The longest prefix of `text` within `max` bytes that ends on a character.
pub(crate) fn clip(text: &str, max: usize) -> &str {
    if text.len() <= max {
        return text;
    }
    let mut end = max;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    &text[..end]
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn thought(len: usize) -> Event {
        Event::Thought("t".repeat(len))
    }

    #[test]
    fn the_transcript_stays_bounded() {
        let mut inv = Investigation::default();
        // A 10 MiB flood of 64 KiB thoughts, split by a step so entries differ.
        for i in 0..160 {
            inv.record(thought(MAX_CHUNK_BYTES));
            if i % 40 == 0 {
                inv.record(Event::StepStarted {
                    id: format!("s{i}"),
                    tool: "freshkube_get_object".into(),
                    args: Args::default(),
                });
            }
            assert!(inv.text_bytes() <= MAX_TRANSCRIPT_BYTES, "after chunk {i}");
        }
        assert!(inv.trimmed());
        let held: usize = inv
            .entries()
            .iter()
            .map(|e| match e {
                Entry::Thought(t) => t.text.len(),
                _ => 0,
            })
            .sum();
        assert_eq!(held, inv.text_bytes());
        assert!(held <= MAX_TRANSCRIPT_BYTES && held > MAX_TRANSCRIPT_BYTES / 2);
        // The oldest thoughts lost their text first, and an entry with none
        // left is gone; the newest still has some.
        let thoughts: Vec<_> = inv
            .entries()
            .iter()
            .filter_map(|e| match e {
                Entry::Thought(t) => Some(t),
                _ => None,
            })
            .collect();
        assert!(thoughts.len() < 4, "{} thoughts kept", thoughts.len());
        assert!(thoughts.iter().all(|t| !t.text.is_empty()));
        assert!(!thoughts.last().unwrap().text.is_empty());
        // Steps are not text and stay.
        assert_eq!(inv.step_count(), 4);
    }

    #[test]
    fn thoughts_go_before_speech() {
        let mut inv = Investigation::default();
        inv.record(Event::Said("answer ".repeat(1000)));
        inv.record(Event::Thought("t".repeat(MAX_CHUNK_BYTES)));
        for _ in 0..20 {
            inv.record(Event::Thought("t".repeat(MAX_CHUNK_BYTES)));
        }
        let said = inv.entries().iter().find_map(|e| match e {
            Entry::Said(t) => Some(t),
            _ => None,
        });
        assert!(said.is_some_and(|t| !t.trimmed && t.text.len() == 7000));
        assert!(inv.text_bytes() <= MAX_TRANSCRIPT_BYTES);
        // Speech alone is trimmed too, oldest first, and marked.
        let mut inv = Investigation::default();
        for _ in 0..20 {
            inv.record(Event::Said("s".repeat(MAX_CHUNK_BYTES)));
        }
        assert!(inv.trimmed());
        assert!(matches!(&inv.entries()[0], Entry::Said(t) if t.trimmed));
        assert!(inv.text_bytes() <= MAX_TRANSCRIPT_BYTES);
    }

    #[test]
    fn a_chunk_is_cut_on_a_character() {
        let mut inv = Investigation::default();
        inv.record(Event::Said("é".repeat(MAX_CHUNK_BYTES)));
        let Entry::Said(said) = &inv.entries()[0] else {
            panic!("speech")
        };
        assert_eq!(said.text.len(), MAX_CHUNK_BYTES);
        assert!(said.text.chars().all(|c| c == 'é'));
    }

    #[test]
    fn step_arguments_are_cut_at_4_kib() {
        let long = json!({"command": "x".repeat(10_000)});
        let args = Args::from_value(&long);
        assert_eq!(args.shown().len(), MAX_ARGS_BYTES);
        assert!(args.is_cut());
        let short = Args::from_value(&json!({"kind": "Pod"}));
        assert_eq!(short.shown(), r#"{"kind":"Pod"}"#);
        assert!(!short.is_cut());
        assert_eq!(Args::from_value(&Value::Null).shown(), "");
    }

    #[test]
    fn a_step_updates_in_place_and_the_count_is_bounded() {
        let mut inv = Investigation::default();
        inv.record(Event::StepStarted {
            id: "t1".into(),
            tool: "freshkube_get_object".into(),
            args: Args::default(),
        });
        inv.record(Event::StepDescribed {
            id: "t1".into(),
            tool: None,
            args: Some(Args::from_value(&json!({"kind": "Deployment"}))),
        });
        inv.record(Event::StepEnded {
            id: "t1".into(),
            failed: false,
        });
        let [Entry::Step(step)] = inv.entries() else {
            panic!("one step")
        };
        assert_eq!(step.state, StepState::Done);
        assert_eq!(step.args.shown(), r#"{"kind":"Deployment"}"#);

        for i in 0..MAX_STEPS + 50 {
            inv.record(Event::StepStarted {
                id: format!("n{i}"),
                tool: "x".into(),
                args: Args::default(),
            });
        }
        assert_eq!(inv.step_count(), MAX_STEPS);
        let notes = inv
            .entries()
            .iter()
            .filter(|e| matches!(e, Entry::Note(_)))
            .count();
        assert_eq!(notes, 1, "one note for the refused steps");
    }

    #[test]
    fn the_first_outcome_of_a_turn_wins_and_a_prompt_starts_a_turn() {
        let mut inv = Investigation::default();
        assert_eq!(inv.outcome(), Outcome::Running);
        inv.record(Event::Ended(Outcome::Cancelled));
        inv.record(Event::Ended(Outcome::Answered));
        assert_eq!(inv.outcome(), Outcome::Cancelled);
        inv.record(Event::Prompt("again".into()));
        assert_eq!(inv.outcome(), Outcome::Running);
        inv.record(Event::Ended(Outcome::Answered));
        assert_eq!(inv.outcome(), Outcome::Answered);
    }

    #[test]
    fn alternating_chunks_keep_the_entry_count_bounded() {
        let mut inv = Investigation::default();
        for i in 0..20_000 {
            inv.record(if i % 2 == 0 {
                Event::Thought("t".into())
            } else {
                Event::Said("s".into())
            });
            assert!(inv.entries().len() <= MAX_TEXT_ENTRIES, "after event {i}");
        }
        assert!(inv.trimmed());
        assert_eq!(inv.entries().len(), MAX_TEXT_ENTRIES);
        // Thoughts went first, so what is left is mostly speech.
        let said = inv
            .entries()
            .iter()
            .filter(|e| matches!(e, Entry::Said(_)))
            .count();
        assert!(said > MAX_TEXT_ENTRIES / 2);
        // Every held byte is counted.
        let held: usize = inv
            .entries()
            .iter()
            .map(|e| match e {
                Entry::Thought(t) | Entry::Said(t) => t.text.len(),
                _ => 0,
            })
            .sum();
        assert_eq!(inv.text_bytes(), held);
    }

    #[test]
    fn notes_are_bounded() {
        let mut inv = Investigation::default();
        for _ in 0..MAX_NOTES + 10 {
            inv.record(Event::Note("n".repeat(5_000)));
        }
        assert_eq!(inv.entries().len(), MAX_NOTES);
        assert!(matches!(&inv.entries()[0], Entry::Note(n) if n.len() == 512));
    }
}
