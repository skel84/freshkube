use std::io::{Read, Write};
use std::net::TcpStream;
use std::time::Duration;

use std::sync::Arc;

use freshkube_core::resources::{
    ForwardFailure, ForwardFailureKind, ForwardTarget, builtin, listen_local,
};
use gpui_kit::component::Root;
use gpui_kit::test::{TestAppContextExt, TestWindowExt};
use gpui_kit::{AnyWindowHandle, AppContext, Entity, TestAppContext, px, size};
use tokio::runtime::Runtime;

// Not `super::*`: gpui_kit's glob would shadow the built-in `#[test]`.
use super::view::Phase;
use super::{ForwardList, ForwardSpec, ForwardsIndicator, Listen, list, running, stop_all};
use crate::resources::KubeAccess;
use crate::resources::model::ResourceIdentity;
use crate::resources::{example, live};

/// How long, in test time, a forward may take to answer.
const PATIENCE: Duration = Duration::from_secs(5);

struct Mounted {
    runtime: Runtime,
    list: Entity<ForwardList>,
    window: AnyWindowHandle,
}

impl Mounted {
    fn spec(&self, kind: &str, name: &str, port: u16, local_port: Option<u16>) -> ForwardSpec {
        let identity = object(kind, name);
        let resource = builtin(kind).unwrap();
        let target = ForwardTarget::of(&resource, &identity.name, &identity.uid).unwrap();
        ForwardSpec {
            runtime: self.runtime.handle().clone(),
            access: KubeAccess::Example,
            identity,
            target,
            context: "homelab".into(),
            port,
            local_port,
        }
    }

    /// Starts a forward and waits for its port, which example mode binds
    /// on Tokio's blocking pool.
    async fn start(&self, cx: &mut TestAppContext, spec: ForwardSpec) -> u64 {
        let view = cx.update(|cx| self.list.update(cx, |list, cx| list.start(spec, cx)));
        let id = cx.read(|cx| view.read(cx).id);
        self.settle(cx, id).await;
        id
    }

    /// Waits until a forward has its port, or has failed to get one.
    async fn settle(&self, cx: &mut TestAppContext, id: u64) {
        let list = self.list.clone();
        cx.wait_for(self.window, PATIENCE, move |_, cx| {
            list.read(cx).get(id, cx).unwrap().read(cx).phase != Phase::Starting
        })
        .await;
        cx.run_until_parked();
    }

    fn click(&self, cx: &mut TestAppContext, id: &str) {
        let id = gpui_kit::SharedString::from(id.to_owned());
        cx.update_window(self.window, |_, window, cx| {
            window.render_frame(cx);
            window.click(id, cx);
        })
        .unwrap();
        cx.run_until_parked();
    }

    fn label(&self, cx: &mut TestAppContext, id: &str) -> Option<String> {
        let id = gpui_kit::SharedString::from(id.to_owned());
        cx.update_window(self.window, |_, window, cx| {
            window.render_frame(cx);
            window
                .try_find(id)
                .and_then(|element| element.label().map(str::to_owned))
        })
        .unwrap()
    }

    /// The text of the status bar's entry, or `None` while it is hidden.
    fn entry(&self, cx: &mut TestAppContext) -> Option<String> {
        cx.update_window(self.window, |_, window, cx| {
            window.render_frame(cx);
            window
                .try_find("forwards")
                .and_then(|element| element.label().map(str::to_owned))
        })
        .unwrap()
    }

    fn local_port(&self, cx: &mut TestAppContext, id: u64) -> u16 {
        cx.read(|cx| {
            let list = self.list.read(cx);
            list.get(id, cx).unwrap().read(cx).local_port.unwrap()
        })
    }

    fn phase(&self, cx: &mut TestAppContext, id: u64) -> Phase {
        cx.read(|cx| self.list.read(cx).get(id, cx).unwrap().read(cx).phase)
    }

    async fn wait_until(&self, cx: &mut TestAppContext, what: impl Fn(&str) -> bool, id: u64) {
        let list = self.list.clone();
        cx.wait_for(self.window, PATIENCE, move |_, cx| {
            let label = list
                .read(cx)
                .get(id, cx)
                .unwrap()
                .read(cx)
                .display
                .label
                .clone();
            what(&label)
        })
        .await;
    }
}

fn mount(cx: &mut TestAppContext) -> Mounted {
    cx.update(|cx| {
        gpui_kit::init(cx);
        crate::theme::install(cx);
        cx.set_reduce_motion(true);
    });
    let runtime = Runtime::new().unwrap();
    let window = cx.open_window(size(px(900.), px(700.)), |window, cx| {
        let view = cx.new(ForwardsIndicator::new);
        Root::new(view, window, cx)
    });
    let list = cx.update(list);
    cx.run_until_parked();
    Mounted {
        runtime,
        list,
        window: window.into(),
    }
}

/// An example object of `kind` named `name`, or a Running pod of the app
/// `name` for pods.
fn object(kind: &str, name: &str) -> ResourceIdentity {
    let (_, rows) = example::read("homelab", kind, None, live::now()).unwrap();
    rows.into_iter()
        .find(|row| match kind {
            "pods" => row.cells[0].starts_with(&format!("{name}-")) && row.cells[2] == "Running",
            _ => row.identity.name == name,
        })
        .unwrap()
        .identity
}

/// Sends a request to the loopback port and reads what comes back.
fn fetch(port: u16) -> std::io::Result<String> {
    let mut stream = TcpStream::connect(("127.0.0.1", port))?;
    stream.set_read_timeout(Some(Duration::from_secs(5)))?;
    stream.write_all(b"GET / HTTP/1.0\r\n\r\n")?;
    let mut answer = String::new();
    stream.read_to_string(&mut answer)?;
    Ok(answer)
}

/// Whether nothing listens on `port` within a second of real time: a stop
/// returns before Tokio drops the listener, and the test clock doesn't
/// wait for it.
fn freed(port: u16) -> bool {
    (0..50).any(|_| {
        let refused = fetch(port).is_err();
        if !refused {
            std::thread::sleep(Duration::from_millis(20));
        }
        refused
    })
}

#[gpui_kit::test]
async fn a_pod_forward_answers_on_its_port_and_the_status_bar_counts_it(cx: &mut TestAppContext) {
    cx.executor().allow_parking();
    let forwards = mount(cx);
    assert_eq!(forwards.entry(cx), None, "hidden while nothing is listed");
    let pod = object("pods", "grafana");
    let id = forwards
        .start(cx, forwards.spec("pods", "grafana", 8080, None))
        .await;
    assert_eq!(forwards.phase(cx, id), Phase::Running);
    assert_eq!(forwards.entry(cx).as_deref(), Some("⇄ 1 forward"));
    let port = forwards.local_port(cx, id);
    let answer = fetch(port).unwrap();
    assert!(answer.starts_with("HTTP/1.1 200 OK"), "{answer}");
    assert!(
        answer.contains(&format!("Example forward to {} port 8080", pod.name)),
        "{answer}"
    );
    forwards.click(cx, "forwards");
    let row = forwards.label(cx, &format!("forward-{id}")).unwrap();
    assert!(
        row.starts_with(&format!(
            "Pod monitoring/{} in homelab: localhost:{port} → 8080, Listening",
            pod.name
        )),
        "{row}"
    );
}

#[gpui_kit::test]
async fn stop_frees_the_port_and_start_again_takes_it_back(cx: &mut TestAppContext) {
    cx.executor().allow_parking();
    let forwards = mount(cx);
    let id = forwards
        .start(cx, forwards.spec("pods", "grafana", 8080, None))
        .await;
    let port = forwards.local_port(cx, id);
    forwards.click(cx, "forwards");
    forwards.click(cx, &format!("forward-{id}-stop"));
    assert_eq!(forwards.phase(cx, id), Phase::Ended);
    assert_eq!(cx.read(running), 0);
    assert_eq!(
        forwards.entry(cx).as_deref(),
        Some("⇄ Forwards"),
        "listed until removed"
    );
    let row = forwards.label(cx, &format!("forward-{id}")).unwrap();
    assert!(row.contains("Stopped, You stopped it"), "{row}");
    assert!(freed(port), "nothing listens on {port} after Stop");
    forwards.click(cx, &format!("forward-{id}-again"));
    forwards.settle(cx, id).await;
    assert_eq!(forwards.phase(cx, id), Phase::Running);
    assert_eq!(
        forwards.local_port(cx, id),
        port,
        "the same port comes back"
    );
    assert!(fetch(port).unwrap().contains("Example forward"));
    assert_eq!(forwards.entry(cx).as_deref(), Some("⇄ 1 forward"));
}

#[gpui_kit::test]
async fn remove_takes_an_ended_forward_off_and_hides_the_entry(cx: &mut TestAppContext) {
    cx.executor().allow_parking();
    let forwards = mount(cx);
    let id = forwards
        .start(cx, forwards.spec("pods", "grafana", 8080, None))
        .await;
    forwards.click(cx, "forwards");
    assert!(
        forwards
            .label(cx, &format!("forward-{id}-remove"))
            .is_none(),
        "a running forward offers Stop, not Remove"
    );
    forwards.click(cx, &format!("forward-{id}-stop"));
    forwards.click(cx, &format!("forward-{id}-remove"));
    assert!(cx.read(|cx| forwards.list.read(cx).items.is_empty()));
    assert_eq!(forwards.entry(cx), None);
}

#[gpui_kit::test]
async fn a_typed_port_in_use_fails_and_offers_the_automatic_one(cx: &mut TestAppContext) {
    cx.executor().allow_parking();
    let forwards = mount(cx);
    let taken = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let port = taken.local_addr().unwrap().port();
    let id = forwards
        .start(cx, forwards.spec("pods", "grafana", 8080, Some(port)))
        .await;
    assert_eq!(forwards.phase(cx, id), Phase::Ended);
    forwards.click(cx, "forwards");
    let row = forwards.label(cx, &format!("forward-{id}")).unwrap();
    assert!(
        row.contains(&format!("Failed, In use · Port {port} is in use")),
        "{row}"
    );
    forwards.click(cx, &format!("forward-{id}-automatic"));
    forwards.settle(cx, id).await;
    assert_eq!(forwards.phase(cx, id), Phase::Running);
    let automatic = forwards.local_port(cx, id);
    assert_ne!(automatic, port);
    assert!(fetch(automatic).unwrap().contains("Example forward"));
    drop(taken);
}

/// Asking whether a port is free can take seconds on Windows (#392): the
/// start returns at once, Starting, and the forward listens once the probe
/// answers, off the UI thread.
#[gpui_kit::test]
async fn starting_returns_before_the_port_probe_answers(cx: &mut TestAppContext) {
    cx.executor().allow_parking();
    let forwards = mount(cx);
    let (answer, answered) = std::sync::mpsc::channel::<()>();
    let answered = std::sync::Mutex::new(answered);
    let slow: Listen = Arc::new(move |local_port, remote| {
        // Never past the test's patience, even if the test fails first.
        let _ = answered
            .lock()
            .unwrap()
            .recv_timeout(Duration::from_secs(5));
        listen_local(local_port, remote)
    });
    cx.update(|cx| forwards.list.update(cx, |list, _| list.listen = slow));
    let view = cx.update(|cx| {
        forwards.list.update(cx, |list, cx| {
            list.start(forwards.spec("pods", "grafana", 8080, None), cx)
        })
    });
    cx.run_until_parked();
    let id = cx.read(|cx| view.read(cx).id);
    assert_eq!(forwards.phase(cx, id), Phase::Starting);
    assert_eq!(cx.read(running), 1, "a starting forward counts as running");
    assert!(
        cx.read(|cx| view.read(cx).display.label.contains("Starting")),
        "{}",
        cx.read(|cx| view.read(cx).display.label.clone())
    );
    answer.send(()).unwrap();
    forwards.settle(cx, id).await;
    assert_eq!(forwards.phase(cx, id), Phase::Running);
    let port = forwards.local_port(cx, id);
    assert!(fetch(port).unwrap().contains("Example forward"));
}

/// A typed port in a range Windows reserves says so and offers the
/// automatic port, as a taken one does (#392). The probe stands in for
/// Windows, so this runs on every platform.
#[gpui_kit::test]
async fn a_reserved_port_says_so_and_offers_the_automatic_one(cx: &mut TestAppContext) {
    cx.executor().allow_parking();
    let forwards = mount(cx);
    let reserving: Listen = Arc::new(|local_port, remote| match local_port {
        Some(port) => Err(ForwardFailure::new(
            ForwardFailureKind::PortReserved,
            format!("Windows reserves port {port}"),
        )),
        None => listen_local(None, remote),
    });
    cx.update(|cx| forwards.list.update(cx, |list, _| list.listen = reserving));
    let id = forwards
        .start(cx, forwards.spec("pods", "grafana", 8080, Some(50_000)))
        .await;
    assert_eq!(forwards.phase(cx, id), Phase::Ended);
    forwards.click(cx, "forwards");
    let row = forwards.label(cx, &format!("forward-{id}")).unwrap();
    assert!(
        row.contains("Failed, Reserved · Windows reserves port 50000"),
        "{row}"
    );
    forwards.click(cx, &format!("forward-{id}-automatic"));
    forwards.settle(cx, id).await;
    assert_eq!(forwards.phase(cx, id), Phase::Running);
    let automatic = forwards.local_port(cx, id);
    assert_ne!(automatic, 50_000);
    assert!(fetch(automatic).unwrap().contains("Example forward"));
}

#[gpui_kit::test]
async fn a_service_forward_shows_its_pod_and_a_refused_port_as_the_last_error(
    cx: &mut TestAppContext,
) {
    cx.executor().allow_parking();
    let forwards = mount(cx);
    let pod = example::ready_pod("homelab", "web", "gateway").unwrap();
    let id = forwards
        .start(cx, forwards.spec("services", "gateway", 443, None))
        .await;
    let port = forwards.local_port(cx, id);
    forwards
        .wait_until(cx, |label| label.contains("Listening"), id)
        .await;
    let row = cx.read(|cx| {
        let list = forwards.list.read(cx);
        list.get(id, cx).unwrap().read(cx).display.route.clone()
    });
    assert_eq!(row, format!("localhost:{port} → 443 · {pod}:8443"));
    // The example pods serve 8080 only: 8443 answers like a refusal.
    assert_eq!(fetch(port).unwrap(), "");
    forwards
        .wait_until(
            cx,
            |label| label.ends_with("last error: Connection refused in the pod on port 8443"),
            id,
        )
        .await;
    assert_eq!(forwards.phase(cx, id), Phase::Running, "it keeps listening");
}

#[gpui_kit::test]
async fn a_service_without_a_selector_and_a_pod_not_running_fail(cx: &mut TestAppContext) {
    cx.executor().allow_parking();
    let forwards = mount(cx);
    let label = |cx: &mut TestAppContext, id: u64| {
        cx.read(|cx| {
            let list = forwards.list.read(cx);
            list.get(id, cx).unwrap().read(cx).display.label.clone()
        })
    };
    let id = forwards
        .start(cx, forwards.spec("services", "kubernetes", 443, None))
        .await;
    assert_eq!(forwards.phase(cx, id), Phase::Ended);
    let ended = label(cx, id);
    assert!(
        ended.contains("Failed, No pods · The Service selects no pods"),
        "{ended}"
    );
    let (_, rows) = example::read("homelab", "pods", None, live::now()).unwrap();
    let pending = rows
        .iter()
        .find(|row| row.cells[2] == "ContainerCreating")
        .unwrap();
    let mut spec = forwards.spec("pods", "grafana", 8080, None);
    spec.target = ForwardTarget::Pod {
        name: pending.identity.name.clone(),
        uid: pending.identity.uid.clone(),
    };
    spec.identity = pending.identity.clone();
    let id = forwards.start(cx, spec).await;
    assert_eq!(forwards.phase(cx, id), Phase::Ended);
    let ended = label(cx, id);
    assert!(
        ended.contains("Failed, Not running · The pod is Pending"),
        "{ended}"
    );
}

#[gpui_kit::test]
async fn stop_all_stops_every_forward_and_frees_their_ports(cx: &mut TestAppContext) {
    cx.executor().allow_parking();
    let forwards = mount(cx);
    let first = forwards
        .start(cx, forwards.spec("pods", "grafana", 8080, None))
        .await;
    let second = forwards
        .start(cx, forwards.spec("pods", "grafana", 8080, None))
        .await;
    let ports = [
        forwards.local_port(cx, first),
        forwards.local_port(cx, second),
    ];
    assert_ne!(
        ports[0], ports[1],
        "the same port twice takes two local ports"
    );
    assert_eq!(cx.read(running), 2);
    assert_eq!(forwards.entry(cx).as_deref(), Some("⇄ 2 forwards"));
    let stopping = cx.update(stop_all);
    stopping.await;
    assert_eq!(cx.read(running), 0);
    for port in ports {
        assert!(freed(port), "nothing listens on {port}");
    }
}

#[gpui_kit::test]
async fn closing_the_window_asks_then_stops_every_forward(cx: &mut TestAppContext) {
    cx.executor().allow_parking();
    let forwards = mount(cx);
    let id = forwards
        .start(cx, forwards.spec("pods", "grafana", 8080, None))
        .await;
    let port = forwards.local_port(cx, id);
    let closed = std::rc::Rc::new(std::cell::Cell::new(false));
    let ask = |cx: &mut TestAppContext| {
        let closed = closed.clone();
        cx.update_window(forwards.window, move |_, window, cx| {
            crate::resources::shell::may_close(window, cx, move |_, _| closed.set(true))
        })
        .unwrap()
    };
    assert!(!ask(cx), "a running forward holds the window");
    cx.run_until_parked();
    let (message, _) = cx.pending_prompt().unwrap();
    assert_eq!(message, "Stop 1 forward?");
    cx.simulate_prompt_answer("Cancel");
    cx.run_until_parked();
    assert!(!closed.get());
    assert_eq!(cx.read(running), 1, "Cancel leaves it running");
    assert!(!ask(cx));
    cx.run_until_parked();
    cx.simulate_prompt_answer("Stop");
    let flag = closed.clone();
    cx.wait_for(forwards.window, PATIENCE, move |_, _| flag.get())
        .await;
    assert_eq!(cx.read(running), 0);
    assert!(freed(port), "the port is free once it closes");
}
