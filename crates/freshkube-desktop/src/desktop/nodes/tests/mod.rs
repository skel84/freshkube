mod cards;
mod keys;
mod layout;
mod menu;
mod metrics;
mod pane;
mod projection;
mod table;
use super::{NodeTab, join};
use crate::{
    desktop::{Page, tests::fixture},
    fixture as examples, presentation,
    resources::example,
    ui::Tone,
};
use gpui_kit::{AppContext, Focusable, TestAppContext, test::TestWindowExt};

fn projection_sources() -> (
    presentation::NodeSummary,
    freshkube_core::kubernetes_summary::NodeSummary,
) {
    let talos = presentation::NodeSummary {
        name: "joined".into(),
        address: "10.0.0.1".into(),
        role: presentation::Role::Worker,
        etcd_member: true,
        responding: true,
        version: Some("v1.12.0".into()),
        cores: Some(8),
        memory: Some(presentation::Memory {
            used: 9216,
            total: 10240,
        }),
        load: Some([1., 2., 3.]),
        services: vec![talos_rs::ServiceInfo {
            id: "kubelet".into(),
            state: "Running".into(),
            health: Some(talos_rs::ServiceHealth {
                unknown: false,
                healthy: false,
                last_message: String::new(),
            }),
        }],
    };
    let summary = example::summary("prod-fra", 0);
    let mut kube = summary.nodes.loaded().unwrap()[0].clone();
    kube.name = talos.name.clone();
    kube.roles = vec!["master".into()];
    kube.conditions.truncate(1);
    kube.conditions[0].kind = "Ready".into();
    kube.conditions[0].status = "False".into();
    kube.conditions[0].reason = "KubeletNotReady".into();
    kube.conditions[0].message = "runtime unavailable".into();
    kube.addresses = vec![
        ("Hostname".into(), "joined.example".into()),
        ("InternalIP".into(), "10.0.0.2".into()),
    ];
    kube.capacity =
        serde_json::from_value(serde_json::json!({"cpu": "16", "memory": "16Gi"})).unwrap();
    kube.kubelet_version = "v1.35.0".into();
    kube.unschedulable = true;
    kube.taints = vec!["dedicated=infra:NoSchedule".into()];
    kube.pods = 7;
    (talos, kube)
}

/// A wheel scroll of `y` at `position`, in ten steps with a frame after
/// each, as a trackpad sends it.
fn wheel(
    window: &mut gpui_kit::Window,
    position: gpui_kit::Point<gpui_kit::Pixels>,
    y: f32,
    cx: &mut gpui_kit::App,
) {
    use gpui_kit::{InputEvent, ScrollDelta, ScrollWheelEvent, TouchPhase, point, px};
    for step in 0..10 {
        window.dispatch_event(
            ScrollWheelEvent {
                position,
                delta: ScrollDelta::Pixels(point(px(0.), px(y / 10.))),
                touch_phase: if step == 0 {
                    TouchPhase::Started
                } else {
                    TouchPhase::Moved
                },
                ..Default::default()
            }
            .to_platform_input(),
            cx,
        );
        window.render_frame(cx);
    }
}

fn drawn(tooltip: &str) -> usize {
    freshkube_ui::tooltip::drawn(tooltip)
}

/// Opens the first control-plane node from the table.
fn open_first_node(window: &mut gpui_kit::Window, cx: &mut gpui_kit::App) {
    window.render_frame(cx);
    window.click("nav-nodes", cx);
    window.render_frame(cx);
    crate::desktop::tests::expand_healthy_nodes(window, cx);
    crate::desktop::tests::click_node(window, cx, "node-talos-cp-fra1-01");
    window.render_frame(cx);
}
