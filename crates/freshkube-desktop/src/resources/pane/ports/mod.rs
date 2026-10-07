//! The Ports section of a pod's, Service's or workload's Details
//! (docs/PORT_FORWARD.md): the ports the object declares, each with a
//! local port field and Forward,
//! an "Other port" row for any number, and the forwards already running
//! from this object. It reads nothing itself; starting goes through the
//! app's forwards, so closing the pane leaves them running.

use freshkube_core::resources::{DeclaredPort, ForwardTarget, ResourceKind, preferred_port};
use gpui_kit::component::input::{InputEvent, InputState};
use gpui_kit::*;
use tokio::runtime::Handle;

use crate::forwards::{self, ForwardList, ForwardSpec, Row};
use crate::resources::KubeAccess;
use crate::resources::model::ResourceIdentity;

#[cfg(test)]
mod tests;
mod view;

/// Whether `kind` has a Ports section.
pub(crate) fn forwardable(kind: &ResourceKind) -> bool {
    ForwardTarget::of(kind, "", "").is_some()
}

/// The object the tab forwards.
#[derive(Clone)]
struct Object {
    identity: ResourceIdentity,
    target: ForwardTarget,
    access: KubeAccess,
    context: String,
}

/// A declared port, ready to draw.
struct PortRow {
    port: u16,
    /// "http · 8080/TCP".
    title: SharedString,
    /// "container web", or "to 8080" for a Service.
    detail: SharedString,
    /// Why it can't start: UDP and SCTP can't be forwarded.
    blocked: Option<SharedString>,
    /// Typed local port; empty means automatic. Made once the window is at
    /// hand, so it may lag the row by an effect cycle.
    local: Option<Entity<InputState>>,
    /// The forwards of this port from this object.
    forwards: Vec<Row>,
}

pub(crate) struct PortsView {
    runtime: Handle,
    window: AnyWindowHandle,
    list: Entity<ForwardList>,
    object: Option<Object>,
    /// The context of the connection the pane opens objects on.
    context: String,
    /// The document's ports, once read; `None` until then.
    declared: Option<Vec<DeclaredPort>>,
    rows: Vec<PortRow>,
    /// The forwards of ports the object doesn't declare.
    other_forwards: Vec<Row>,
    other_port: Entity<InputState>,
    other_local: Entity<InputState>,
    /// Why the last Forward didn't start: a port that isn't a number.
    feedback: Option<SharedString>,
    _subscriptions: Vec<Subscription>,
}

impl PortsView {
    pub(crate) fn new(runtime: Handle, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let list = forwards::list(cx);
        let other_port = cx.new(|cx| InputState::new(window, cx).placeholder("Port"));
        let other_local = cx.new(|cx| InputState::new(window, cx).placeholder("Automatic"));
        let subscriptions = vec![
            cx.observe(&list, |view, _, cx| {
                view.derive_forwards(cx);
                cx.notify();
            }),
            cx.subscribe(&other_port, |view, input, event, cx| {
                view.other_port_event(input, event, cx)
            }),
            cx.subscribe(&other_local, |view, input, event, cx| {
                view.other_port_event(input, event, cx)
            }),
        ];
        Self {
            runtime,
            window: window.window_handle(),
            list,
            object: None,
            context: String::new(),
            declared: None,
            rows: Vec::new(),
            other_forwards: Vec::new(),
            other_port,
            other_local,
            feedback: None,
            _subscriptions: subscriptions,
        }
    }

    pub(crate) fn set_context(&mut self, context: String) {
        self.context = context;
    }

    pub(crate) fn set_access(&mut self, access: KubeAccess) {
        if let Some(object) = &mut self.object {
            object.access = access;
        }
    }

    /// Shows `target`'s ports once its document arrives, or nothing.
    pub(crate) fn show(
        &mut self,
        target: Option<(ResourceIdentity, ResourceKind)>,
        access: Option<KubeAccess>,
        cx: &mut Context<Self>,
    ) {
        self.object = target.zip(access).and_then(|((identity, kind), access)| {
            let target = ForwardTarget::of(&kind, &identity.name, &identity.uid)?;
            Some(Object {
                identity,
                target,
                access,
                context: self.context.clone(),
            })
        });
        self.declared = None;
        self.rows.clear();
        self.feedback = None;
        self.derive_forwards(cx);
        cx.notify();
    }

    /// The ports the document declares.
    pub(crate) fn set_ports(&mut self, declared: Vec<DeclaredPort>, cx: &mut Context<Self>) {
        if self.declared.as_ref() == Some(&declared) {
            return;
        }
        let mut inputs: Vec<(u16, Entity<InputState>)> = self
            .rows
            .drain(..)
            .filter_map(|row| Some((row.port, row.local?)))
            .collect();
        self.rows = declared
            .iter()
            .map(|port| {
                let local = inputs
                    .iter()
                    .position(|(number, _)| *number == port.port)
                    .map(|ix| inputs.swap_remove(ix).1);
                port_row(port, local)
            })
            .collect();
        self.declared = Some(declared);
        self.derive_forwards(cx);
        self.make_inputs(cx);
        cx.notify();
    }

    /// Makes the missing local port fields, which need the window, after
    /// this update.
    fn make_inputs(&mut self, cx: &mut Context<Self>) {
        if self.rows.iter().all(|row| row.local.is_some()) {
            return;
        }
        let window = self.window;
        let this = cx.entity().downgrade();
        cx.defer(move |cx| {
            _ = window.update(cx, |_, window, cx| {
                _ = this.update(cx, |view, cx| view.fill_inputs(window, cx));
            });
        });
    }

    fn fill_inputs(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let mut subscriptions = Vec::new();
        for row in self.rows.iter_mut().filter(|row| row.local.is_none()) {
            let placeholder = format!("Automatic · {}", preferred_port(row.port));
            let input = cx.new(|cx| InputState::new(window, cx).placeholder(placeholder));
            let port = row.port;
            subscriptions.push(cx.subscribe(&input, move |view, _, event, cx| {
                if let InputEvent::PressEnter { .. } = event {
                    view.forward_declared(port, cx);
                }
            }));
            row.local = Some(input);
        }
        self._subscriptions.extend(subscriptions);
        cx.notify();
    }

    /// Sorts the app's forwards of this object under the rows they belong
    /// to: a declared port's, or Other port's.
    fn derive_forwards(&mut self, cx: &App) {
        for row in &mut self.rows {
            row.forwards.clear();
        }
        self.other_forwards.clear();
        let Some(object) = &self.object else {
            return;
        };
        for item in &self.list.read(cx).items {
            let view = item.read(cx);
            let Some(port) = view.port_of(&object.identity) else {
                continue;
            };
            let row = view.row();
            match self.rows.iter_mut().find(|row| row.port == port) {
                Some(declared) => declared.forwards.push(row),
                None => self.other_forwards.push(row),
            }
        }
    }

    fn forward_declared(&mut self, port: u16, cx: &mut Context<Self>) {
        let Some(row) = self.rows.iter().find(|row| row.port == port) else {
            return;
        };
        if row.blocked.is_some() {
            return;
        }
        let typed = row
            .local
            .as_ref()
            .map(|input| input.read(cx).value().to_string())
            .unwrap_or_default();
        match local_port(&typed) {
            Ok(local) => self.start(port, local, cx),
            Err(message) => self.feedback = Some(message.into()),
        }
        cx.notify();
    }

    fn forward_other(&mut self, cx: &mut Context<Self>) {
        let port = self.other_port.read(cx).value().trim().to_owned();
        let local = self.other_local.read(cx).value().to_string();
        let port = match port.parse::<u16>() {
            Ok(port) if port > 0 => port,
            _ => {
                self.feedback = Some("Type a port from 1 to 65535 to forward".into());
                return cx.notify();
            }
        };
        match local_port(&local) {
            Ok(local) => self.start(port, local, cx),
            Err(message) => self.feedback = Some(message.into()),
        }
        cx.notify();
    }

    fn other_port_event(
        &mut self,
        _: Entity<InputState>,
        event: &InputEvent,
        cx: &mut Context<Self>,
    ) {
        if let InputEvent::PressEnter { .. } = event {
            self.forward_other(cx);
        }
    }

    fn start(&mut self, port: u16, local_port: Option<u16>, cx: &mut Context<Self>) {
        let Some(object) = self.object.clone() else {
            return;
        };
        self.feedback = None;
        let spec = ForwardSpec {
            runtime: self.runtime.clone(),
            access: object.access,
            identity: object.identity,
            target: object.target,
            context: object.context,
            port,
            local_port,
        };
        self.list.update(cx, |list, cx| list.start(spec, cx));
    }
}

fn port_row(port: &DeclaredPort, local: Option<Entity<InputState>>) -> PortRow {
    let number = format!("{}/{}", port.port, port.protocol);
    let title = if port.name.is_empty() {
        number
    } else {
        format!("{} · {number}", port.name)
    };
    let detail = if !port.container.is_empty() {
        format!("container {}", port.container)
    } else if !port.target.is_empty() {
        format!("to {}", port.target)
    } else {
        String::new()
    };
    PortRow {
        port: port.port,
        title: title.into(),
        detail: detail.into(),
        blocked: (!port.forwardable())
            .then(|| format!("{} can't be forwarded; only TCP can", port.protocol).into()),
        local,
        forwards: Vec::new(),
    }
}

/// A typed local port: empty is automatic.
fn local_port(typed: &str) -> Result<Option<u16>, &'static str> {
    let typed = typed.trim();
    if typed.is_empty() {
        return Ok(None);
    }
    match typed.parse::<u16>() {
        Ok(port) if port > 0 => Ok(Some(port)),
        _ => Err("Type a local port from 1 to 65535, or leave it empty for an automatic one"),
    }
}
