//! A consumer can borrow a route without owning the forward's lifetime.
use std::num::NonZeroU16;

use freshkube_core::resources::ForwardEnd;
use gpui_kit::{App, Entity};

use super::{ForwardList, ForwardView};

/// One start of one app-owned forward; the local port is not its identity.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Epoch {
    pub id: u64,
    pub start: u64,
}

#[derive(Clone, Debug, PartialEq)]
pub enum State {
    Starting,
    Listening(NonZeroU16),
    NoReadyPod,
    Ended(ForwardEnd),
}

/// Reported by the owner, including when a removed forward has stopped.
#[derive(Clone, Debug, PartialEq)]
pub struct Snapshot {
    pub epoch: Epoch,
    pub state: State,
}

impl ForwardList {
    pub fn contains(&self, id: u64, cx: &App) -> bool {
        self.items.iter().any(|item| item.read(cx).id == id)
    }

    /// The newest matching Service forward entry, including ended
    /// ones. A caller retains its selected entity rather than falling back
    /// to another forward after Stop or removal.
    pub fn find(
        &self,
        connection: &str,
        namespace: &str,
        service: &str,
        port: u16,
        cx: &App,
    ) -> Option<Entity<ForwardView>> {
        self.items
            .iter()
            .rev()
            .find(|item| {
                item.read(cx)
                    .matches_service(connection, namespace, service, port)
            })
            .cloned()
    }
}
