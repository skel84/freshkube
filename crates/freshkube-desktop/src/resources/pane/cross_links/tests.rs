use freshkube_core::resources::{Failure, FailureKind, PodLinks};
use gpui_kit::TestAppContext;
use std::{sync::Arc, time::Duration};

#[gpui_kit::test]
fn refused_services_keep_other_links_and_late_answers_are_ignored(cx: &mut TestAppContext) {
    let (_runtime, pane, _, _) = super::super::tests::mount(cx);
    let (pod, _) = super::super::tests::crashing_pod();
    cx.update(|cx| {
        super::super::tests::open(&pane, &pod, Duration::ZERO, cx);
        pane.update(cx, |pane, cx| {
            let document = pane
                .detail
                .as_ref()
                .unwrap()
                .view
                .as_ref()
                .unwrap()
                .document
                .clone();
            let denied = PodLinks::from_sources(
                &document,
                Err(Failure::new(FailureKind::Forbidden, "forbidden")),
                None,
            );
            let sequence = pane.links_seq;
            pane.finish_links(&pod.identity, sequence.wrapping_sub(1), denied.clone(), cx);
            assert!(pane.cross_links.errors.is_empty());
            let mut other = pod.identity.clone();
            other.uid = "replacement".into();
            pane.finish_links(&other, sequence, denied.clone(), cx);
            assert!(pane.cross_links.errors.is_empty());
            pane.finish_links(&pod.identity, sequence, denied, cx);
            assert!(pane.cross_links.services.is_empty());
            assert!(pane.cross_links.errors[0].contains("forbidden"));
            assert!(!pane.cross_links.owners.is_empty());
            assert!(!pane.cross_links.containers.is_empty());
            let reads = pane.links_seq;
            pane.set_node_rows(Arc::default(), cx);
            assert_eq!(pane.links_seq, reads);
            assert!(pane.links_job.is_none());
        });
    });
}

fn node_row(responding: Option<bool>) -> crate::desktop::nodes::NodeRow {
    use crate::desktop::nodes::{NodeKey, NodeRow};
    let mut talos =
        crate::presentation::node_summaries(&crate::fixture::cluster("prod-fra", 0))[0].clone();
    let name = "node-a".to_string();
    let talos = responding.map(|responding| {
        talos.responding = responding;
        talos
    });
    NodeRow {
        kubernetes_current: true,
        key: NodeKey {
            kubernetes: Some(name.clone()),
            talos: talos.as_ref().map(|talos| talos.name.clone()),
        },
        id: "node-a".into(),
        open_id: "node-a-open".into(),
        pod_label: "Pods".into(),
        kubelet_pods: "Pods".into(),
        service_problem: false,
        name: name.into(),
        role: crate::presentation::node_summaries(&crate::fixture::cluster("prod-fra", 0))[0].role,
        tone: crate::ui::Tone::Good,
        kubernetes: None,
        talos,
        ready: "Ready",
        talos_state: "".into(),
        address: "".into(),
        load: "".into(),
        table_load: "".into(),
        memory: "61% of 16 GiB".into(),
        pods: "".into(),
        services: "".into(),
        service_status: Default::default(),
        note: "".into(),
        chips: Vec::new(),
        facts: Vec::new(),
        problems: Vec::new(),
    }
}

/// The pod's node shows its memory as last known unless Talos answers for
/// that node right now.
#[test]
fn a_node_link_marks_memory_stale_unless_talos_responds() {
    let stale = |row: Option<&crate::desktop::nodes::NodeRow>| {
        let link = super::node_link("node-a", row);
        (link.memory_stale, link.memory_label.to_string())
    };
    let responding = node_row(Some(true));
    let silent = node_row(Some(false));
    let kubernetes_only = node_row(None);
    assert_eq!(stale(Some(&responding)), (false, "61% of 16 GiB".into()));
    assert_eq!(stale(Some(&silent)), (true, "61% of 16 GiB".into()));
    assert_eq!(
        stale(Some(&kubernetes_only)),
        (true, "61% of 16 GiB".into())
    );
    assert_eq!(stale(None), (true, "Unavailable".into()));
}
