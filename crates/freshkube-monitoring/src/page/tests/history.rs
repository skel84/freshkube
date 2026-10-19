use super::*;

#[gpui_kit::test]
fn history_reads_where_the_page_found_or_remembers_prometheus(cx: &mut TestAppContext) {
    use crate::history::HistoryKind;
    let (_runtime, _handle, page) = mount(cx, Some(example_source()));
    let kind = |cx: &mut TestAppContext| {
        cx.read(|cx| {
            page.read(cx).history().map(|history| match history.kind {
                HistoryKind::Example => "example".to_owned(),
                HistoryKind::Ready(prometheus) => {
                    format!("ready {}", prometheus.endpoint().label())
                }
                HistoryKind::Remembered { service, .. } => {
                    format!("remembered {}", service.label())
                }
            })
        })
    };
    // Example data answers without the page ever showing.
    assert_eq!(kind(cx).as_deref(), Some("example"));

    // A live context with nothing remembered has none until the page finds one.
    let live = live_source();
    cx.update(|cx| page.update(cx, |page, cx| page.set_source(Some(live.clone()), cx)));
    assert_eq!(kind(cx), None);

    let service = PrometheusService::new("monitoring", "prometheus-operated", 9090);
    cx.update(|cx| {
        page.update(cx, |page, cx| {
            page.saved
                .services
                .insert("prod-ams".into(), service.clone());
            page.set_source(None, cx);
            page.set_source(Some(live), cx);
        })
    });
    assert_eq!(
        kind(cx).as_deref(),
        Some("remembered monitoring/prometheus-operated:9090")
    );

    // Looked for and not found: none, so the panes show nothing.
    let missing = Discovery::Missing {
        candidates: Vec::new(),
        tried: Vec::new(),
    };
    cx.update(|cx| page.update(cx, |page, cx| page.discovered(Ok(missing), cx)));
    assert_eq!(kind(cx), None);
}
