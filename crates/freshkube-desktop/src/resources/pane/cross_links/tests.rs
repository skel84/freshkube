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
            pane.set_node_facts(Arc::default(), cx);
            assert_eq!(pane.links_seq, reads);
            assert!(pane.links_job.is_none());
        });
    });
}

fn node_facts(memory_stale: bool) -> crate::resources::NodeFacts {
    crate::resources::NodeFacts {
        name: "node-a".into(),
        not_ready: false,
        not_ready_since: None,
        ready_label: "Ready".into(),
        tone: crate::ui::Tone::Good,
        memory: Some(crate::resources::NodeMemory {
            used: 10,
            total: 16,
        }),
        memory_label: "61% of 16 GiB".into(),
        memory_stale,
        problems: Vec::new(),
        services: true,
        kubelet: Some((crate::ui::Tone::Good, "kubelet running · healthy".into())),
    }
}

/// The pod's node shows its memory as last known when the shell says so,
/// and as unavailable when the shell knows nothing of the node.
#[test]
fn a_node_link_keeps_the_stale_flag_and_falls_back_without_facts() {
    let stale = |node: Option<&crate::resources::NodeFacts>| {
        let link = super::node_link("node-a", node);
        (link.memory_stale, link.memory_label.to_string())
    };
    let responding = node_facts(false);
    let silent = node_facts(true);
    assert_eq!(stale(Some(&responding)), (false, "61% of 16 GiB".into()));
    assert_eq!(stale(Some(&silent)), (true, "61% of 16 GiB".into()));
    assert_eq!(stale(None), (true, "Unavailable".into()));
    let unknown = super::node_link("node-a", None);
    assert_eq!(unknown.ready, "Node unavailable");
    assert!(unknown.memory.is_none());
    assert!(!unknown.services);
    assert!(unknown.kubelet.is_none());
}
