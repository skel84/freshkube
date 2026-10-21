use super::*;

#[test]
fn revisions_come_newest_first_with_coroot_s_own_words() {
    let mut wire = app_view_wire();
    let rows = serde_json::json!([
        deployment_row(
            "9c41e7:1789999000",
            "9c41e7: example.test/shop/auth:1.4.0",
            serde_json::json!([{"report":"SLO","ok":false,"message":"Availability: 97% (objective: 99%)","time":null}])
        ),
        deployment_row(
            "2b70aa:1789990000",
            "2b70aa: example.test/shop/auth:1.3.9",
            serde_json::json!([{"report":"CPU","ok":true,"message":"CPU usage unchanged","time":null}])
        ),
    ]);
    wire["data"]["reports"]
        .as_array_mut()
        .unwrap()
        .push(deployments_report(rows));
    let view = decode_wire(wire).unwrap();
    let revisions = view.revisions.unwrap().unwrap();
    let ids: Vec<_> = revisions.iter().map(|r| r.id.as_str()).collect();
    assert_eq!(ids, ["9c41e7:1789999000", "2b70aa:1789990000"]);
    assert_eq!(revisions[0].started_at.timestamp(), 1_789_999_000);
    assert_eq!(
        revisions[0].findings[0].message,
        "Availability: 97% (objective: 99%)"
    );
    assert_eq!(view.history_error, None);
}

#[test]
fn bad_revisions_fail_alone() {
    let row = |id: &str| {
        deployment_row(
            id,
            "9c41e7",
            serde_json::json!([{"report":"SLO","ok":true,"message":"ok","time":null}]),
        )
    };
    let mut wire = app_view_wire();
    wire["data"]["reports"]
        .as_array_mut()
        .unwrap()
        .push(deployments_report(serde_json::json!([
            row("9c41e7:1789999000"),
            row("9c41e7:1789999000")
        ])));
    let view = decode_wire(wire).unwrap();
    assert_eq!(view.revisions, Some(Err(ReadError::InvalidResponse)));
    assert_eq!(view.history_error, None);
    assert_eq!(view.reports.len(), 8);
}

#[test]
fn revision_text_is_bounded() {
    let revision = |message: String| coroot_rs::DeploymentRevision {
        id: "9c41e7:1789999000".into(),
        hash: "9c41e7".into(),
        started_at: chrono::DateTime::from_timestamp(1_789_999_000, 0).unwrap(),
        version: "9c41e7".into(),
        status: Status::Ok,
        findings: vec![coroot_rs::RevisionFinding {
            report: "SLO".into(),
            ok: true,
            message,
            time: None,
        }],
        note: None,
    };
    assert_eq!(limits::revisions(&[revision("ok".into())]), Ok(()));
    assert_eq!(
        limits::revisions(&[revision("x".repeat(4_097))]),
        Err(ReadError::Limit)
    );
}
