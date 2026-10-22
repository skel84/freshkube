//! The Alertmanager and vmalert Services the alert read found are
//! remembered per context, saved in `monitoring.json`, and given to the
//! alert pages to try first.
use super::*;
use freshkube_core::monitoring::alerts::Remembered;
use freshkube_ui::real_io::until_io;

fn service(name: &str, port: u16) -> PrometheusService {
    PrometheusService::new("monitoring", name, port)
}

#[gpui_kit::test]
fn the_services_that_answered_are_remembered_per_context_and_kept_when_less_is_found(
    cx: &mut TestAppContext,
) {
    let directory = std::env::temp_dir().join(format!(
        "freshkube-monitoring-remembered-{}-{}",
        std::process::id(),
        line!()
    ));
    let _ = std::fs::remove_dir_all(&directory);
    std::fs::create_dir_all(&directory).unwrap();
    let preferences = directory.join("preferences.json");
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let (_handle, page) = mount_with(
        cx,
        &runtime,
        Some(example_source()),
        Some(&preferences),
        None,
    );
    let manager = Remembered {
        alertmanager: Some(service("alertmanager-operated", 9093)),
        vmalert: None,
    };
    assert_eq!(
        cx.read(|cx| page.read(cx).remembered_alerts()),
        Remembered::default()
    );
    cx.update(|cx| page.update(cx, |page, cx| page.remember_alerts(manager.clone(), cx)));
    assert_eq!(cx.read(|cx| page.read(cx).remembered_alerts()), manager);
    until_io(cx, "monitoring.json saved", |_| {
        crate::store::load(&preferences)
            .alerts
            .contains_key("prod-fra")
    });
    assert_eq!(crate::store::load(&preferences).alerts["prod-fra"], manager);

    // A later read that finds only vmalert adds it and drops nothing.
    let both = Remembered {
        vmalert: Some(service("vmalert-shop", 8880)),
        ..manager.clone()
    };
    cx.update(|cx| {
        page.update(cx, |page, cx| {
            page.remember_alerts(
                Remembered {
                    alertmanager: None,
                    vmalert: both.vmalert.clone(),
                },
                cx,
            )
        })
    });
    assert_eq!(cx.read(|cx| page.read(cx).remembered_alerts()), both);

    // Another context has its own.
    cx.update(|cx| {
        page.update(cx, |page, cx| {
            page.set_source(
                Some(ClusterSource {
                    id: "other".into(),
                    context: "prod-ams".into(),
                    access: ClusterAccess::EXAMPLE,
                }),
                cx,
            )
        })
    });
    assert_eq!(
        cx.read(|cx| page.read(cx).remembered_alerts()),
        Remembered::default()
    );
    std::fs::remove_dir_all(directory).unwrap();
}
