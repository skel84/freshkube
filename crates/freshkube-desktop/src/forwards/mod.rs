//! The app's port forwards (docs/PORT_FORWARD.md). They belong to the app,
//! not to a pane: a forward lives until Stop, through page, pane, context
//! and kubeconfig changes, on the connection it started with. The status
//! bar lists them all (`indicator.rs`), the Ports tab starts them, and
//! quitting or closing the window asks first, together with a running
//! shell (`shell::may_close`). An ended forward stays listed until removed.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use freshkube_core::resources::{ForwardFailure, Listeners, PodWatches, listen_local};
use gpui_kit::*;
use tokio::task::JoinHandle;

mod example;
mod indicator;
#[cfg(test)]
mod tests;
mod view;

pub(crate) use indicator::{ForwardsIndicator, render_forward};
#[cfg(test)]
pub(crate) use view::Phase;
pub(crate) use view::{ForwardSpec, ForwardView, Row};

/// How long quitting or closing the window waits for forwards to let their
/// ports go.
const CLOSE_GRACE: Duration = Duration::from_secs(1);

/// The pod watches of each connection, by its id, shared by its forwards.
#[derive(Clone, Default)]
pub(crate) struct Watches(Arc<Mutex<HashMap<String, PodWatches>>>);

impl Watches {
    /// The watches of `connection`, made with `client` the first time.
    fn of(&self, connection: &str, client: kube::Client) -> PodWatches {
        let mut watches = self
            .0
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        watches
            .entry(connection.to_owned())
            .or_insert_with(|| PodWatches::new(client))
            .clone()
    }

    /// Lets a connection's watches go after a failure, so the next start
    /// makes them with a fresh client. Running forwards keep theirs.
    fn forget(&self, connection: &str) {
        let mut watches = self
            .0
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        watches.remove(connection);
    }
}

/// Listens on the loopback for an example forward: on a typed port, or on
/// the automatic one for the remote port. It blocks, so it runs on Tokio's
/// blocking pool; tests put their own in its place.
pub(crate) type Listen =
    Arc<dyn Fn(Option<u16>, u16) -> Result<Listeners, ForwardFailure> + Send + Sync>;

/// Every forward, in the order started.
pub(crate) struct ForwardList {
    pub(crate) items: Vec<Entity<ForwardView>>,
    /// How many run, for the status bar.
    pub(crate) running: usize,
    next_id: u64,
    watches: Watches,
    /// How example forwards listen: [`listen_local`], but in tests.
    pub(super) listen: Listen,
    /// Stops still letting their ports go, which quitting waits for.
    closing: Vec<JoinHandle<()>>,
    observers: HashMap<u64, Subscription>,
}

struct Forwards(Entity<ForwardList>);

impl Global for Forwards {}

/// The app's forwards, made on first use.
pub(crate) fn list(cx: &mut App) -> Entity<ForwardList> {
    if let Some(forwards) = cx.try_global::<Forwards>() {
        return forwards.0.clone();
    }
    let list = cx.new(|_| ForwardList {
        items: Vec::new(),
        running: 0,
        next_id: 1,
        watches: Watches::default(),
        listen: Arc::new(listen_local),
        closing: Vec::new(),
        observers: HashMap::new(),
    });
    cx.set_global(Forwards(list.clone()));
    list
}

/// How many forwards run anywhere in the app.
pub(crate) fn running(cx: &App) -> usize {
    cx.try_global::<Forwards>()
        .map_or(0, |forwards| forwards.0.read(cx).running)
}

/// Stops every forward, and resolves once their ports are free or
/// [`CLOSE_GRACE`] has passed.
pub(crate) fn stop_all(cx: &mut App) -> impl Future<Output = ()> + use<> {
    let closing = match cx.try_global::<Forwards>() {
        Some(forwards) => {
            let list = forwards.0.clone();
            list.update(cx, |list, cx| {
                let ids: Vec<u64> = list.items.iter().map(|item| item.read(cx).id).collect();
                for id in ids {
                    list.stop(id, cx);
                }
                std::mem::take(&mut list.closing)
            })
        }
        None => Vec::new(),
    };
    let grace = cx.background_executor().timer(CLOSE_GRACE);
    async move {
        futures::future::select(futures::future::join_all(closing), grace).await;
    }
}

impl ForwardList {
    /// Starts a forward and lists it.
    pub(crate) fn start(
        &mut self,
        spec: ForwardSpec,
        cx: &mut Context<Self>,
    ) -> Entity<ForwardView> {
        let id = self.next_id;
        self.next_id += 1;
        let watches = self.watches.clone();
        let listen = self.listen.clone();
        let view = cx.new(|_| ForwardView::new(id, spec, watches, listen));
        let observer = cx.observe(&view, |list, _, cx| list.changed(cx));
        self.observers.insert(id, observer);
        self.items.push(view.clone());
        view.update(cx, |view, cx| view.start(cx));
        self.changed(cx);
        view
    }

    pub(crate) fn get(&self, id: u64, cx: &App) -> Option<Entity<ForwardView>> {
        self.items
            .iter()
            .find(|item| item.read(cx).id == id)
            .cloned()
    }

    pub(crate) fn stop(&mut self, id: u64, cx: &mut Context<Self>) {
        let Some(view) = self.get(id, cx) else {
            return;
        };
        if let Some(closing) = view.update(cx, |view, cx| view.stop(cx)) {
            self.closing.retain(|task| !task.is_finished());
            self.closing.push(closing);
        }
    }

    pub(crate) fn start_again(&mut self, id: u64, cx: &mut Context<Self>) {
        if let Some(view) = self.get(id, cx) {
            view.update(cx, |view, cx| view.start_again(cx));
        }
    }

    pub(crate) fn use_automatic(&mut self, id: u64, cx: &mut Context<Self>) {
        if let Some(view) = self.get(id, cx) {
            view.update(cx, |view, cx| view.use_automatic(cx));
        }
    }

    /// Takes an ended forward off the list.
    pub(crate) fn remove(&mut self, id: u64, cx: &mut Context<Self>) {
        self.stop(id, cx);
        self.items.retain(|item| item.read(cx).id != id);
        self.observers.remove(&id);
        self.changed(cx);
    }

    fn changed(&mut self, cx: &mut Context<Self>) {
        self.running = self
            .items
            .iter()
            .filter(|item| item.read(cx).running())
            .count();
        cx.notify();
    }
}
