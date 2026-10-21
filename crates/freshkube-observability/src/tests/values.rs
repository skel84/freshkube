use super::*;

#[test]
fn application_report_values_preserve_missing_and_healthy_states_without_glyphs() {
    use super::Status;
    let applications = super::projection::applications(&example::applications());
    let worker = applications
        .iter()
        .find(|app| app.key == example::WORKER)
        .unwrap();
    assert_eq!(worker.label.as_ref(), "payments / worker · Deployment");
    let missing = worker.check(Report::DiskIo);
    assert_eq!(missing.value.as_ref(), "—");
    assert_eq!(missing.status, Status::Absent);
    assert!(missing.status.report_tone().is_none());
    let unknown = worker.check(Report::Errors);
    assert!(unknown.value.is_empty());
    assert_eq!(unknown.status, Status::Unknown);
    assert_eq!(unknown.status.report_tone(), Some(super::Tone::Unknown));
    let healthy = worker.check(Report::Cpu);
    assert_eq!(healthy.value.as_ref(), "ok");
    assert_eq!(healthy.status, Status::Ok);
    assert!(healthy.status.report_tone().is_none());
    assert!(
        missing
            .tooltip
            .starts_with("payments / worker · Deployment")
    );
    assert!(!missing.tooltip.contains("fixture:"));
    assert_eq!(
        worker.check(Report::Net).status.report_tone(),
        Some(super::Tone::Crit)
    );
    assert_eq!(
        worker.check(Report::Logs).status.report_tone(),
        Some(super::Tone::Warn)
    );
}

#[gpui_kit::test]
fn application_cells_expose_missing_unknown_and_healthy_values(cx: &mut TestAppContext) {
    let (_runtime, handle, page) = mount(cx, true);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("obs-filter", cx);
        window.input("worker", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let worker = page
            .read(cx)
            .applications
            .iter()
            .find(|app| app.key == example::WORKER)
            .unwrap();
        for (report, label) in [
            (Report::DiskIo, "Disk I/O: not reported"),
            (Report::Errors, "Errors: unknown"),
            (Report::Cpu, "CPU: healthy"),
            (Report::Net, "Net: critical · refused"),
        ] {
            assert_eq!(
                window.find(worker.check(report).element_id.clone()).label(),
                Some(label)
            );
        }
        let p = crate::palette::palette(cx);
        let healthy: gpui_kit::Background = p.good.into();
        let side = crate::ui::dp_px(8., window).scale(window.scale_factor()).0;
        for report in [Report::Cpu, Report::DiskIo] {
            let bounds = window
                .find(worker.check(report).element_id.clone())
                .bounds()
                .scale(window.scale_factor());
            assert!(
                !window.painted_quads().iter().any(|quad| {
                    bounds.contains(&quad.bounds.center())
                        && (quad.bounds.size.width.0 - side).abs() < 0.5
                        && (quad.bounds.size.height.0 - side).abs() < 0.5
                        && (quad.background == healthy || quad.border_color == p.unk_ink)
                }),
                "healthy and unreported cells must not paint status dots: {report:?}"
            );
        }
    })
    .unwrap();
}

#[test]
fn upstream_count_preserves_failing_names_and_state_in_its_details() {
    use freshkube_core::coroot as api;
    let mut raw = example::applications();
    raw[0].signals.insert(
        "upstreams".into(),
        api::Signal {
            status: api::Status::Critical,
            value: "worker, ledger-db".into(),
        },
    );
    let applications = super::projection::applications(&raw);
    let app = applications.iter().find(|app| app.id == raw[0].id).unwrap();
    let check = app.check(Report::Upstreams);
    assert_eq!(check.value.as_ref(), "2");
    assert!(check.tooltip.contains("worker, ledger-db"));
    assert!(check.tooltip.contains("Critical"));
    assert!(check.label.contains("worker, ledger-db"));
    assert!(check.label.contains("critical"));
}
