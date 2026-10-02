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
