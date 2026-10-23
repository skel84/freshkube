//! What an agent may read of a cluster (`docs/features/agent.md`): four
//! read-only tools, the closed types that parse what an agent sends, the
//! per-cluster gate that is off until a person turns it on, and the bounded
//! answers.
//!
//! No UI and no HTTP. The loopback endpoint in `freshkube-agent` calls
//! [`Call::parse`], then [`read`], and gives the answer a read id.
//!
//! - [`call`]: [`Call::parse`], the only parser of agent input.
//! - [`gate`]: [`AgentAccess`], granted per `(entry, context)`.
//! - [`read()`]: the gate, the deadline and the dispatch.
//! - `live` and `example`: the two sources, feeding one `shape`.
//! - [`refusal`]: [`Refusal`] and its codes.
//! - [`answer`]: [`Answer`], [`Body`] and [`Place`], with the bounds.
//!
//! Nothing here writes: every request is a `GET`, and the only subresource
//! is `log`. A Secret is read as metadata, so a value or a key name is never
//! asked for.

mod answer;
mod call;
mod example;
mod gate;
mod live;
mod read;
mod refusal;
mod shape;
mod target;

pub use answer::{Answer, Body, Cut, CutBy, EventRow, Item, Place, Tab};
pub use call::{
    Call, Container, EntryId, Events, Get, KindName, List, Logs, Name, Namespace, Request,
    SERVER_NAME, TOOLS,
};
pub use example::shared as example_objects;
pub use gate::AgentAccess;
pub use read::read;
pub use refusal::{Code, Refusal};
pub use target::{AgentTarget, ExampleObjects, Source};

#[cfg(test)]
mod tests;
