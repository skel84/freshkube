use super::*;

fn revision(started: i64) -> DeploymentRevision {
    DeploymentRevision {
        id: format!("7d9f1c:{started}"),
        hash: "7d9f1c".into(),
        started_at: chrono::DateTime::from_timestamp(started, 0).unwrap(),
        version: "7d9f1c: auth:2.4.0".into(),
        status: Status::Ok,
        findings: vec![],
        note: Some("No notable changes".into()),
    }
}

#[tokio::test]
async fn a_revision_view_reads_the_window_around_its_start() {
    let server = server(vec![(200, app_view_wire())]).await;
    let provider = Provider::new(&server.url, Credentials::None).unwrap();
    let app = AppId::new("c:shop:Deployment:auth");
    // The charts' 61 places run a minute apart from 1789996380 s, Coroot's
    // `from` truncated to the step; the start at 1789998000 s is the 28th.
    let view = provider
        .revision_view(
            &source(&provider),
            &app,
            &revision(1_789_998_000),
            RevisionWindow::both(Duration::from_secs(1800)),
        )
        .await
        .unwrap();
    let line = server.requests.lock().unwrap()[0].clone();
    assert!(line.contains("from=1789996200000"), "{line}");
    assert!(line.contains("to=1789999800000"), "{line}");
    assert_eq!(view.from.timestamp(), 1_789_996_200);
    assert_eq!(view.to.timestamp(), 1_789_999_800);
    let Around::View(page) = &view.answer else {
        panic!("expected the page");
    };
    let chart = page.reports[1]
        .widgets
        .iter()
        .find_map(|w| match &w.kind {
            WidgetKind::Chart(chart) => Some(chart),
            _ => None,
        })
        .unwrap();
    // The layout and the history come from the same answer.
    assert_eq!(chart.title, "Node CPU usage, %");
    let history = chart.history.as_ref().unwrap();
    assert_eq!(
        view.split(history),
        ChartSplit {
            before: 27,
            after: 34
        }
    );
    // Coroot sent more than was asked, not less.
    assert!(!view.ends_early(history));
}

#[tokio::test]
async fn only_coroots_own_404_for_a_revision_window_is_no_data() {
    let server = server(vec![
        (404, serde_json::json!("Application not found\n")),
        (
            404,
            serde_json::json!("<html><body>Not Found</body></html>"),
        ),
        (404, serde_json::json!("404 page not found")),
        (403, serde_json::json!({})),
    ])
    .await;
    let provider = Provider::new(&server.url, Credentials::None).unwrap();
    let app = AppId::new("c:shop:Deployment:auth");
    let source = source(&provider);
    let revision = revision(1_789_998_000);
    let read = || provider.revision_view(&source, &app, &revision, RevisionWindow::default());
    assert!(matches!(read().await.unwrap().answer, Around::NoData));
    assert_eq!(read().await.unwrap_err(), ReadError::Missing);
    assert_eq!(read().await.unwrap_err(), ReadError::Unsupported);
    assert_eq!(read().await.unwrap_err(), ReadError::Refused);
}

#[tokio::test]
async fn a_revision_window_is_checked_before_anything_is_sent() {
    let server = server(vec![]).await;
    let provider = Provider::new(&server.url, Credentials::None).unwrap();
    let source = source(&provider);
    let revision = revision(1_789_998_000);
    let app = AppId::new("c:shop:Deployment:auth");
    for (app, window) in [
        (&app, RevisionWindow::both(Duration::from_secs(30))),
        (&app, RevisionWindow::both(Duration::from_secs(13 * 3600))),
        (&AppId::new(""), RevisionWindow::default()),
    ] {
        assert_eq!(
            provider
                .revision_view(&source, app, &revision, window)
                .await
                .unwrap_err(),
            ReadError::InvalidSelection
        );
    }
    assert!(server.requests.lock().unwrap().is_empty());
}
