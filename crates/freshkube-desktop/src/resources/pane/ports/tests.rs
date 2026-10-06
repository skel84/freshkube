use std::io::{Read, Write};
use std::net::TcpStream;
use std::time::Duration;

use freshkube_core::resources::builtin;
use gpui_kit::component::Root;
use gpui_kit::test::{TestAppContextExt, TestWindowExt};
use gpui_kit::{AnyWindowHandle, AppContext, Entity, SharedString, TestAppContext, px, size};
use tokio::runtime::Runtime;

// Not `super::*`: gpui_kit's glob would shadow the built-in `#[test]`.
use crate::forwards::{self, ForwardList};
use crate::resources::detail::DetailTarget;
use crate::resources::pane::DetailPane;
use crate::resources::screen::KubeAccess;
use crate::resources::{example, live};

struct Mounted {
    _runtime: Runtime,
    pane: Entity<DetailPane>,
    list: Entity<ForwardList>,
    window: AnyWindowHandle,
}

impl Mounted {
    fn step(
        &self,
        cx: &mut TestAppContext,
        act: impl FnOnce(&mut gpui_kit::Window, &mut gpui_kit::App),
    ) {
        cx.update_window(self.window, |_, window, cx| {
            window.render_frame(cx);
            act(window, cx);
        })
        .unwrap();
        cx.run_until_parked();
    }

    fn open(&self, cx: &mut TestAppContext, target: &DetailTarget) {
        let target = target.clone();
        self.step(cx, |_, cx| {
            self.pane.update(cx, |pane, cx| {
                pane.set_context("homelab".into(), cx);
                pane.open(target, KubeAccess::Example, "1", Duration::ZERO, cx)
            });
        });
    }

    fn click(&self, cx: &mut TestAppContext, id: &str) {
        let id = SharedString::from(id.to_owned());
        self.step(cx, |window, cx| window.click(id, cx));
    }

    fn type_into(&self, cx: &mut TestAppContext, id: &str, text: &str) {
        let id = SharedString::from(id.to_owned());
        let text = text.to_owned();
        self.step(cx, |window, cx| {
            window.click(id, cx);
            window.input(&text, cx);
        });
    }

    fn label(&self, cx: &mut TestAppContext, id: &str) -> Option<String> {
        let id = SharedString::from(id.to_owned());
        cx.update_window(self.window, |_, window, cx| {
            window.render_frame(cx);
            window
                .try_find(id)
                .and_then(|element| element.label().map(str::to_owned))
        })
        .unwrap()
    }

    fn present(&self, cx: &mut TestAppContext, id: &str) -> bool {
        let id = SharedString::from(id.to_owned());
        cx.update_window(self.window, |_, window, cx| {
            window.render_frame(cx);
            window.try_find(id).is_some()
        })
        .unwrap()
    }

    /// The local ports of the listed forwards, with whether each runs.
    fn forwards(&self, cx: &mut TestAppContext) -> Vec<(u64, Option<u16>, bool)> {
        cx.read(|cx| {
            self.list
                .read(cx)
                .items
                .iter()
                .map(|item| {
                    let view = item.read(cx);
                    (view.id, view.local_port, view.running())
                })
                .collect()
        })
    }
}

fn mount(cx: &mut TestAppContext) -> Mounted {
    // Forwards listen on Tokio, which wakes GPUI from its own threads.
    cx.executor().allow_parking();
    cx.update(|cx| {
        gpui_kit::init(cx);
        crate::theme::install(cx);
        cx.set_reduce_motion(true);
    });
    let runtime = Runtime::new().unwrap();
    let mut pane = None;
    let window = cx.open_window(size(px(820.), px(820.)), |window, cx| {
        let view = cx.new(|cx| {
            let mut pane = DetailPane::new(runtime.handle().clone(), window, cx);
            pane.set_active(true, cx);
            pane
        });
        pane = Some(view.clone());
        Root::new(view, window, cx)
    });
    let list = cx.update(forwards::list);
    cx.run_until_parked();
    Mounted {
        _runtime: runtime,
        pane: pane.unwrap(),
        list,
        window: window.into(),
    }
}

fn target(kind: &str, pick: impl Fn(&[String]) -> bool) -> DetailTarget {
    let (_, rows) = example::read("homelab", kind, None, live::now()).unwrap();
    let row = rows.iter().find(|row| pick(&row.cells)).unwrap();
    DetailTarget {
        identity: row.identity.clone(),
        kind: builtin(kind).unwrap(),
    }
}

fn running_pod(app: &str) -> DetailTarget {
    target("pods", |cells| {
        cells[0].starts_with(&format!("{app}-")) && cells[2] == "Running"
    })
}

fn fetch(port: u16) -> std::io::Result<String> {
    let mut stream = TcpStream::connect(("127.0.0.1", port))?;
    stream.set_read_timeout(Some(Duration::from_secs(5)))?;
    stream.write_all(b"GET / HTTP/1.0\r\n\r\n")?;
    let mut answer = String::new();
    stream.read_to_string(&mut answer)?;
    Ok(answer)
}

#[gpui_kit::test]
fn a_pod_lists_its_declared_port_and_forward_starts_it(cx: &mut TestAppContext) {
    let pane = mount(cx);
    let pod = running_pod("grafana");
    pane.open(cx, &pod);
    pane.click(cx, "detail-tab-ports");
    assert_eq!(
        pane.label(cx, "ports-row-8080").as_deref(),
        Some("8080/TCP")
    );
    assert!(
        pane.present(cx, "ports-local-8080"),
        "the field arrives with the row"
    );
    pane.click(cx, "ports-forward-8080");
    let listed = pane.forwards(cx);
    assert_eq!(listed.len(), 1);
    let (id, port, running) = listed[0];
    assert!(running);
    let port = port.unwrap();
    assert!(fetch(port).unwrap().contains(&format!(
        "Example forward to {} port 8080",
        pod.identity.name
    )));
    let row = pane.label(cx, &format!("ports-forward-item-{id}")).unwrap();
    assert!(
        row.contains(&format!("localhost:{port} → 8080, Listening")),
        "{row}"
    );
    pane.click(cx, &format!("ports-forward-item-{id}-stop"));
    assert!(!pane.forwards(cx)[0].2, "Stop in the tab stops it");
}

#[gpui_kit::test]
fn a_typed_local_port_is_used_exactly(cx: &mut TestAppContext) {
    let pane = mount(cx);
    let free = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let port = free.local_addr().unwrap().port();
    drop(free);
    pane.open(cx, &running_pod("grafana"));
    pane.click(cx, "detail-tab-ports");
    pane.type_into(cx, "ports-local-8080", &port.to_string());
    pane.click(cx, "ports-forward-8080");
    assert_eq!(pane.forwards(cx)[0].1, Some(port));
    assert!(fetch(port).unwrap().contains("Example forward"));
    pane.type_into(cx, "ports-local-8080", "x");
    pane.click(cx, "ports-forward-8080");
    assert_eq!(
        pane.forwards(cx).len(),
        1,
        "a local port that isn't a number starts nothing"
    );
    assert!(pane.present(cx, "ports-feedback"));
}

#[gpui_kit::test]
async fn other_port_forwards_any_number_and_refuses_text(cx: &mut TestAppContext) {
    let pane = mount(cx);
    pane.open(cx, &running_pod("grafana"));
    pane.click(cx, "detail-tab-ports");
    pane.type_into(cx, "ports-other-port", "http");
    pane.click(cx, "ports-forward-other");
    assert!(pane.forwards(cx).is_empty());
    assert!(pane.present(cx, "ports-feedback"));
    pane.step(cx, |window, cx| {
        window.click("ports-other-port", cx);
        window.press("secondary-a", cx);
        window.input("9090", cx);
        window.press("enter", cx);
    });
    let listed = pane.forwards(cx);
    assert_eq!(listed.len(), 1, "Enter in the field starts it");
    assert!(!pane.present(cx, "ports-feedback"));
    let (id, port, _) = listed[0];
    let row = pane.label(cx, &format!("ports-forward-item-{id}")).unwrap();
    assert!(row.contains("→ 9090, Listening"), "{row}");
    // The example pods serve 8080 only.
    assert_eq!(fetch(port.unwrap()).unwrap(), "");
    let list = pane.list.clone();
    cx.wait_for(pane.window, Duration::from_secs(5), move |_, cx| {
        let view = list.read(cx).get(id, cx).unwrap();
        view.read(cx)
            .display
            .label
            .ends_with("last error: Connection refused in the pod on port 9090")
    })
    .await;
}

#[gpui_kit::test]
fn a_forward_outlives_the_pane_and_shows_again_with_its_object(cx: &mut TestAppContext) {
    let pane = mount(cx);
    let pod = running_pod("grafana");
    pane.open(cx, &pod);
    pane.click(cx, "detail-tab-ports");
    pane.click(cx, "ports-forward-8080");
    let (id, port, _) = pane.forwards(cx)[0];
    pane.step(cx, |_, cx| pane.pane.update(cx, |pane, cx| pane.close(cx)));
    pane.open(cx, &running_pod("prometheus"));
    assert!(
        pane.forwards(cx)[0].2,
        "closing the pane and opening another object leave it"
    );
    assert!(
        !pane.present(cx, &format!("ports-forward-item-{id}")),
        "another object's tab doesn't list it"
    );
    assert!(fetch(port.unwrap()).unwrap().contains(&pod.identity.name));
    pane.open(cx, &pod);
    assert!(pane.present(cx, &format!("ports-forward-item-{id}")));
}

#[gpui_kit::test]
fn the_tabs_follow_the_kind(cx: &mut TestAppContext) {
    let pane = mount(cx);
    let tabs = |pane: &Mounted, cx: &mut TestAppContext| {
        ["detail-open-logs", "detail-tab-shell", "detail-tab-ports"].map(|id| pane.present(cx, id))
    };
    pane.open(cx, &running_pod("grafana"));
    assert_eq!(tabs(&pane, cx), [true, true, true]);
    let service = target("services", |cells| cells[0] == "gateway");
    pane.click(cx, "detail-tab-ports");
    pane.open(cx, &service);
    assert_eq!(tabs(&pane, cx), [false, false, true]);
    assert_eq!(
        pane.label(cx, "ports-row-443").as_deref(),
        Some("https · 443/TCP"),
        "a Service keeps the Ports tab and lists its own ports"
    );
    assert!(pane.present(cx, "ports-row-80"));
    // Ports is last: the next tab wraps to Overview.
    pane.step(cx, |_, cx| {
        pane.pane.update(cx, |pane, cx| pane.turn_tab(1, cx))
    });
    assert!(!pane.present(cx, "ports"));
    pane.step(cx, |_, cx| {
        pane.pane.update(cx, |pane, cx| pane.turn_tab(-1, cx))
    });
    assert!(pane.present(cx, "ports"));
    let secret = target("secrets", |_| true);
    pane.open(cx, &secret);
    assert_eq!(tabs(&pane, cx), [false, false, false]);
    assert!(
        !pane.present(cx, "ports"),
        "a kind without ports goes back to Overview"
    );
    let deployment = target("deployments.apps", |cells| cells[0] == "grafana");
    pane.open(cx, &deployment);
    assert_eq!(
        tabs(&pane, cx),
        [true, false, true],
        "a workload has Logs and Ports"
    );
}
