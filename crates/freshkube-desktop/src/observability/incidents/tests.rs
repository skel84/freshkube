use super::{Incidents, detail};
use freshkube_core::coroot::{Incident, IncidentView};
fn incident(key: &str) -> Incident {
    serde_json::from_value(serde_json::json!({"key":key,"app":"c:ns:Deployment:api","cluster":"Production","severity":"warning","state":"open","opened_at":null,"duration_seconds":120,"impact_percent":0,"description":"SLO violation"})).unwrap()
}
#[test]
fn selection_survives_reorder_and_clears_removed_identity() {
    let mut state = Incidents::default();
    assert!(state.prepare_list(&[incident("a"), incident("b")]));
    assert!(!state.prepare_list(&[incident("b"), incident("a")]));
    assert_eq!(state.selected.as_ref().unwrap().0, "a");
    assert!(state.prepare_list(&[incident("b")]));
    assert_eq!(state.selected.as_ref().unwrap().0, "b");
    assert!(state.prepare_list(&[]));
    assert!(state.selected.is_none());
}
#[test]
fn missing_evidence_stays_unknown_and_reported_zero_is_preserved() {
    let view: IncidentView =
        serde_json::from_value(serde_json::json!({"incident":incident("a")})).unwrap();
    let projected = detail(&view);
    assert_eq!(projected.objectives[0].compliance, "Not reported");
    assert_eq!(projected.objectives[0].severity, super::Status::Unknown);
    assert_eq!(projected.objectives[0].state, "Not reported");
    assert_eq!(
        projected.objectives[0].impact,
        "Affected requests: Not reported"
    );
    assert!(projected.objectives[0].rates.is_empty());
    assert!(projected.evidence.is_empty());
    assert!(projected.related.is_empty());
    assert!(projected.summary.contains("0.0% affected"));
}

mod ui_tests {
    use super::super::{PAGE_SIZE, detail};
    use crate::observability::{Destination, Status, connection::Subject, tests::mount_size};
    use freshkube_core::coroot as api;
    use gpui_kit::test::TestWindowExt;
    use gpui_kit::{AppContext, TestAppContext};
    #[gpui_kit::test]
    fn bounded_incidents_prepare_and_render_at_large_text(cx: &mut TestAppContext) {
        let (_runtime, handle, page) = mount_size(cx, false, 760., 560.);
        let rows=(0..100).map(|ix|serde_json::from_value::<api::Incident>(serde_json::json!({"key":format!("k{ix}"),"app":"c:ns:Deployment:api","cluster":"Production","severity":"warning","state":"open","opened_at":null,"duration_seconds":120,"impact_percent":0,"description":"Source incident"})).unwrap()).collect::<Vec<_>>();
        let related=(0..100).map(|ix|serde_json::json!({"app_id":format!("c:ns:Deployment:related-{ix}"),"status":"warning","issues":["Source observation"]})).collect::<Vec<_>>();
        let mut i = serde_json::to_value(&rows[0]).unwrap();
        i["rca"] = serde_json::json!({"status":"OK","root_cause":"Source root cause ".repeat(512),"propagation":related});
        let view:api::IncidentView=serde_json::from_value(serde_json::json!({"incident":i,"availability":{"objective":"99% of requests should not fail","compliance":"97.2%","violated":true}})).unwrap();
        cx.update(|cx| {
            crate::text_size::set(20., cx);
            page.update(cx, |page, _| {
                let provider =
                    api::Provider::new("http://127.0.0.1:1", api::Credentials::None).unwrap();
                page.live.source = Some(provider.source(&api::ProjectInfo {
                    id: "p1".into(),
                    name: "Production".into(),
                }));
                page.live.provider = Some(provider);
                page.destination = Destination::Incidents;
                let start = std::time::Instant::now();
                page.incident_observations.prepare_list(&rows);
                page.incident_observations.detail = Some(detail(&view));
                page.incident_observations.prepare_related();
                eprintln!(
                    "Incidents: 100 rows/100 propagation preparation {:?}",
                    start.elapsed()
                );
                let request = page
                    .live
                    .incidents
                    .begin(page.live.identity(Subject::Incidents).unwrap());
                page.live.incidents.apply(&request, Ok(rows));
                let request = page.live.incident.begin(
                    page.live
                        .identity(Subject::Incident(
                            "k0".into(),
                            api::AppId::new("c:ns:Deployment:api"),
                        ))
                        .unwrap(),
                );
                page.live.incident.apply(&request, Ok(view));
                assert_eq!(
                    page.incident_observations
                        .detail
                        .as_ref()
                        .unwrap()
                        .objectives[0]
                        .severity,
                    Status::Warning
                );
            });
        });
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            assert!(window.try_find("obs-live-incident-k0").is_some());
            assert!(
                window
                    .try_find(gpui_kit::SharedString::from(format!(
                        "obs-live-incident-k{PAGE_SIZE}"
                    )))
                    .is_none()
            );
            assert!(
                window
                    .try_find("obs-incident-app-c:ns:Deployment:related-10")
                    .is_none()
            );
            let bounds = window.find("obs-incident-detail").bounds();
            assert!(bounds.right() <= window.viewport_size().width);
            let start = std::time::Instant::now();
            for _ in 0..10 {
                window.render_frame(cx);
            }
            eprintln!(
                "Incidents: ten bounded headless frames {:?}",
                start.elapsed()
            );
        })
        .unwrap();
    }
}
