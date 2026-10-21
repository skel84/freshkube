//! Results borrows an app-owned forward. Only an explicit action starts it.
use freshkube_core::resources::{ForwardEnd, ForwardTarget};
use freshkube_core::setup::Forward;
use freshkube_core::workspace::{Part, Sources};
use freshkube_delivery::entry::pool::{ResultsRoute, ResultsWait};
use freshkube_resources::forwards::{
    self, Epoch, ForwardList, ForwardSpec, ForwardView, Snapshot, State,
};
use gpui_kit::*;
use std::collections::HashMap;
use tokio::runtime::Handle;

use crate::resources::{
    ClusterSource, ResourceLink, Tab,
    model::{ObjectRef, ResourceIdentity},
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Target {
    entry: String,
    connection: String,
    context: String,
    forward: Forward,
    here: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct Key {
    entry: String,
    connection: String,
    namespace: String,
    service: String,
    port: u16,
    here: bool,
}
#[derive(Default)]
struct Sessions(HashMap<Key, WeakEntity<Session>>);
impl Global for Sessions {}

impl Target {
    fn key(&self) -> Key {
        Key {
            entry: self.entry.clone(),
            connection: self.connection.clone(),
            namespace: self.forward.namespace.clone(),
            service: self.forward.service.clone(),
            port: self.forward.port,
            here: self.here,
        }
    }

    pub(crate) fn of(
        sources: &Sources,
        open: Option<&str>,
        source: &ClusterSource,
    ) -> Option<Self> {
        let entry = sources.get(Part::Builds).entry()?;
        let forward = entry.results.as_ref()?.forward.as_ref()?;
        Some(Self {
            entry: entry.id.clone(),
            connection: source.id.clone(),
            context: source.context.clone(),
            forward: Forward::new(&forward.namespace, &forward.service, forward.port),
            here: open == Some(entry.id.as_str()),
        })
    }

    fn link(&self) -> ResourceLink {
        ResourceLink::Object(
            freshkube_core::resources::builtin("services").unwrap(),
            ObjectRef {
                namespace: self.forward.namespace.clone(),
                name: self.forward.service.clone(),
                uid: String::new(),
                connection: None,
            },
            Tab::Ports,
        )
    }
}

/// A page's subscription, not a listener or a forward guard.
#[derive(Default)]
pub(crate) struct Binding {
    pub(crate) session: Option<Entity<Session>>,
    pub(crate) snapshot: Option<Snapshot>,
    pub(crate) words: Option<SharedString>,
    target: Option<Target>,
    _subscription: Option<Subscription>,
}

impl Binding {
    pub(crate) fn configure<T: 'static>(
        &mut self,
        target: Option<Target>,
        source: &ClusterSource,
        runtime: Handle,
        changed: fn(&mut T, &mut Context<T>),
        cx: &mut Context<T>,
    ) -> bool {
        let same = match (&self.target, &target) {
            (None, None) => true,
            (Some(old), Some(new)) => {
                old.entry == new.entry
                    && old.connection == new.connection
                    && old.forward == new.forward
                    && old.here == new.here
            }
            _ => false,
        };
        if same {
            return false;
        }
        *self = Self::default();
        self.target = target.clone();
        let Some(target) = target else {
            return true;
        };
        let key = target.key();
        if cx.try_global::<Sessions>().is_none() {
            cx.set_global(Sessions::default());
        }
        let existing = cx
            .global::<Sessions>()
            .0
            .get(&key)
            .and_then(WeakEntity::upgrade);
        let session = existing.unwrap_or_else(|| {
            let list = forwards::list(cx);
            let access = source.access.clone();
            let session = cx.new(|cx| {
                let observer = cx.observe(&list, |this: &mut Session, _, cx| this.sync(cx));
                Session {
                    target,
                    runtime,
                    access,
                    list,
                    owner: None,
                    snapshot: None,
                    label: "Forward".into(),
                    words: "".into(),
                    enabled: true,
                    _list: observer,
                    _owner: None,
                }
            });
            session.update(cx, |this, cx| this.sync(cx));
            cx.global_mut::<Sessions>()
                .0
                .retain(|_, session| session.upgrade().is_some());
            cx.global_mut::<Sessions>()
                .0
                .insert(key, session.downgrade());
            session
        });
        self.snapshot = session.read(cx).snapshot.clone();
        self.words = Some(session.read(cx).words.clone());
        self._subscription = Some(cx.observe(&session, move |this, _, cx| changed(this, cx)));
        self.session = Some(session);
        true
    }

    pub(crate) fn sync(&mut self, cx: &App) {
        self.snapshot = self
            .session
            .as_ref()
            .and_then(|session| session.read(cx).snapshot.clone());
        self.words = self
            .session
            .as_ref()
            .map(|session| session.read(cx).words.clone());
    }

    pub(crate) fn epoch(&self) -> Option<Epoch> {
        self.snapshot.as_ref().map(|snapshot| snapshot.epoch)
    }

    pub(crate) fn route(&self) -> ResultsRoute {
        match self.snapshot.as_ref().map(|snapshot| &snapshot.state) {
            Some(State::Listening(local_port)) => ResultsRoute::Forward {
                local_port: *local_port,
            },
            Some(State::Starting) => ResultsRoute::Waiting(ResultsWait::Starting),
            Some(State::NoReadyPod) => ResultsRoute::Waiting(ResultsWait::NoReadyPod),
            Some(State::Ended(end)) => ResultsRoute::Ended(match end {
                ForwardEnd::Stopped => "stopped",
                ForwardEnd::PodGone(_) => "the pod went away",
                ForwardEnd::TargetDeleted(_) => "the target was deleted",
                ForwardEnd::Failed(_) => "the forward failed",
            }),
            _ => ResultsRoute::Address,
        }
    }
}

pub(crate) struct Session {
    target: Target,
    runtime: Handle,
    access: crate::resources::ClusterAccess,
    list: Entity<ForwardList>,
    owner: Option<Entity<ForwardView>>,
    snapshot: Option<Snapshot>,
    pub(crate) label: SharedString,
    pub(crate) words: SharedString,
    pub(crate) enabled: bool,
    _list: Subscription,
    _owner: Option<Subscription>,
}

impl Session {
    fn sync(&mut self, cx: &mut Context<Self>) {
        if self.owner.is_none() && self.target.here {
            let target = &self.target;
            self.owner = self.list.read(cx).find(
                &target.connection,
                &target.forward.namespace,
                &target.forward.service,
                target.forward.port,
                cx,
            );
            if let Some(owner) = &self.owner {
                self._owner = Some(cx.observe(owner, |this, _, cx| this.sync(cx)));
            }
        }
        let snapshot = self.owner.as_ref().map(|owner| owner.read(cx).snapshot());
        let initial = self.words.is_empty();
        if snapshot == self.snapshot && !initial {
            return;
        }
        self.snapshot = snapshot;
        self.describe();
        cx.notify();
    }

    fn describe(&mut self) {
        if !self.target.here {
            self.label = format!("Switch to {}", self.target.entry).into();
            self.words = format!("Tekton Results isn't forwarded yet · {} (F)", self.label).into();
            self.enabled = true;
            return;
        }
        let (words, enabled) = match self.snapshot.as_ref().map(|snapshot| &snapshot.state) {
            None => ("Tekton Results isn't forwarded yet".to_owned(), true),
            Some(State::Starting) => (ResultsWait::Starting.words().into(), false),
            Some(State::Listening(_)) => ("Tekton Results forward is listening".into(), false),
            Some(State::NoReadyPod) => (ResultsWait::NoReadyPod.words().into(), false),
            Some(State::Ended(end)) => (
                match end {
                    ForwardEnd::Stopped => "Tekton Results forward ended: stopped".into(),
                    ForwardEnd::PodGone(why) | ForwardEnd::TargetDeleted(why) => {
                        format!("Tekton Results forward ended: {why}")
                    }
                    ForwardEnd::Failed(why) => format!("Tekton Results forward failed: {why}"),
                },
                true,
            ),
        };
        self.label = "Forward".into();
        self.words = if enabled {
            format!("{words} · Forward (F)").into()
        } else {
            words.into()
        };
        self.enabled = enabled;
    }

    /// Called only from the page's explicit Forward/key/menu action.
    pub(crate) fn act(&mut self, cx: &mut Context<Self>) -> Option<(SharedString, ResourceLink)> {
        if !self.enabled {
            return None;
        }
        if !self.target.here {
            return Some((self.target.entry.clone().into(), self.target.link()));
        }
        if let Some(owner) = &self.owner {
            let id = owner.read(cx).id;
            if self.list.read(cx).contains(id, cx) {
                self.list.update(cx, |list, cx| list.start_again(id, cx));
                self.sync(cx);
                return None;
            }
        }
        let target = &self.target;
        let spec = ForwardSpec {
            runtime: self.runtime.clone(),
            access: self.access.clone(),
            identity: ResourceIdentity {
                connection: target.connection.clone(),
                resource: "services".into(),
                namespace: target.forward.namespace.clone(),
                name: target.forward.service.clone(),
                uid: String::new(),
            },
            target: ForwardTarget::Service {
                name: target.forward.service.clone(),
            },
            context: target.context.clone(),
            port: target.forward.port,
            local_port: None,
        };
        let owner = self.list.update(cx, |list, cx| list.start(spec, cx));
        self._owner = Some(cx.observe(&owner, |this, _, cx| this.sync(cx)));
        self.owner = Some(owner);
        self.sync(cx);
        None
    }
}
