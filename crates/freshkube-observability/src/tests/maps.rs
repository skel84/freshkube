use super::*;

#[gpui_kit::test]
fn map_selection_survives_reordering_and_clears_on_removal(cx: &mut TestAppContext) {
    let (_runtime, handle, page) = mount(cx, true);
    cx.update(|cx| page.update(cx, |page, cx| page.open(Destination::ServiceMap, cx)));
    cx.update_window(handle,|_,window,cx| {
        window.render_frame(cx);
        window.click("obs-map-link-fixture:payments:Deployment:worker--fixture:payments:Deployment:ledger-db",cx);
        let selected=page.read(cx).map_display.selected().cloned();assert!(selected.is_some());
        page.update(cx,|page,_| {
            let mut raw=example::map();raw.nodes.reverse();raw.edges.reverse();
            (page.nodes,page.connections)=super::projection::map(&raw);page.prepare_map();
            assert_eq!(page.map_display.selected().cloned(),selected);
            raw.edges.clear();(page.nodes,page.connections)=super::projection::map(&raw);page.prepare_map();
            assert!(page.map_display.selected().is_none());
        });
    }).unwrap();
}

/// A connection's evidence reads in each value's own unit, in its inspector
/// and its tooltip alike (#241).
#[test]
fn map_connection_evidence_reads_in_its_units() {
    let (_, connections) = super::projection::map(&example::map());
    let connection = &connections[0];
    for text in [&connection.detail, &connection.tooltip] {
        assert!(
            text.contains(
                "Requests: 12 rps\nLatency: 3 ms\nSent: 2.4 KB/s\nReceived: Not reported"
            ),
            "{text}"
        );
    }
}

#[gpui_kit::test]
fn bounded_projection_measurement(cx: &mut TestAppContext) {
    let (_runtime, handle, page) = mount(cx, true);
    let seed = example::applications()[0].clone();
    let apps: Vec<_> = (0..2000)
        .map(|ix| {
            let mut app = seed.clone();
            app.id =
                freshkube_core::coroot::AppId::new(format!("c:ns-{ix}:Deployment:app-{ix:04}"));
            app.category = format!("Category-{ix:04}");
            app
        })
        .collect();
    cx.update(|cx| {
        page.update(cx, |page, _| {
            let start = std::time::Instant::now();
            page.active_categories =
                std::rc::Rc::new(apps.iter().map(|app| app.category.clone()).collect());
            page.apply_applications(&apps);
            let app_time = start.elapsed();
            let raw = large_map();
            let start = std::time::Instant::now();
            (page.nodes, page.connections) = super::projection::map(&raw);
            page.prepare_map();
            eprintln!(
                "Coroot projection: 2000 applications {:?}; 120 nodes/300 links {:?}",
                app_time,
                start.elapsed()
            );
            assert_eq!(page.applications.len(), 2000);
            assert_eq!(page.connections.len(), 300);
        })
    });
    for destination in [Destination::Applications, Destination::ServiceMap] {
        cx.update(|cx| page.update(cx, |page, _| page.destination = destination));
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            let start = std::time::Instant::now();
            for _ in 0..10 {
                window.render_frame(cx);
            }
            eprintln!(
                "Coroot {:?}: ten headless frames {:?}",
                destination,
                start.elapsed()
            );
        })
        .unwrap();
    }
}
