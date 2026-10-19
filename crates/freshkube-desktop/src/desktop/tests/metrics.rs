//! The setup file's `metrics` block reaches Monitoring as its scope: from the
//! setup the shell hands `Pages` and the entry the page is opened on.
use super::*;
use crate::monitoring::page::MonitoringPage;
use crate::resources::ClusterSource;
use crate::screens::{PageSource, ScreenHandle};
use freshkube_core::cluster_source::ClusterAccess;
use freshkube_core::monitoring::scope::Resolved;
use freshkube_core::setup;
use gpui_kit::component::Root;

use super::super::entity_pages::{hosted_by, set_metrics_setup};

const CENTRAL_BODY: &str = "clusters:
  - id: cluster-a
    role: core
    context: hub-context
  - id: cluster-b
    role: environment
    context: cluster-b
metrics:
  mode: central
  store:
    entry: cluster-a
    service:
      namespace: monitoring
      name: central
      port: 9090
";

/// The workspace's kubeconfig: an absolute path on every platform.
fn hub() -> std::path::PathBuf {
    std::env::temp_dir().join("acme-hub-kubeconfig")
}

/// A file naming that kubeconfig, then `body`.
fn file(body: &str) -> String {
    format!("version: 1\nkubeconfig: '{}'\n{body}", hub().display())
}

fn central() -> String {
    file(CENTRAL_BODY)
}

fn parse(text: &str) -> setup::Setup {
    setup::parse(text.as_bytes()).unwrap_or_else(|invalid| panic!("{invalid}"))
}

fn source(context: &str) -> ClusterSource {
    ClusterSource {
        id: format!("access-{context}"),
        context: context.into(),
        access: ClusterAccess::EXAMPLE,
    }
}

/// The page alone in a window, reached through its handle as the shell does.
fn mount_monitoring(
    cx: &mut TestAppContext,
) -> (
    tokio::runtime::Runtime,
    AnyWindowHandle,
    Entity<MonitoringPage>,
    ScreenHandle,
) {
    cx.update(|cx| {
        gpui_kit::init(cx);
        cx.set_reduce_motion(true);
    });
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let mut page = None;
    let window = cx.open_window(size(px(800.), px(600.)), |window, cx| {
        let view = cx.new(|cx| MonitoringPage::new(runtime.handle().clone(), None, None, cx));
        page = Some(view.clone());
        Root::new(view, window, cx)
    });
    let page = page.unwrap();
    let handle = ScreenHandle::new(page.clone());
    (runtime, window.into(), page, handle)
}

fn open_entry(
    cx: &mut TestAppContext,
    window: AnyWindowHandle,
    handle: &ScreenHandle,
    entry: Option<&str>,
    context: &str,
) {
    let source = PageSource {
        kube: Some(source(context)),
        entry: entry.map(str::to_owned),
        ..Default::default()
    };
    cx.update_window(window, |_, window, cx| {
        handle.set_page_source(source, window, cx)
    })
    .unwrap();
}

fn resolved(cx: &mut TestAppContext, page: &Entity<MonitoringPage>) -> Resolved {
    cx.read(|cx| page.read(cx).scope().resolved.clone())
}

#[test]
fn a_store_hosted_by_another_entry_is_reached_with_the_workspaces_kubeconfig_and_its_context() {
    let setup = parse(&central());
    let mut kept = Default::default();
    let access = hosted_by(&setup.workspace, "cluster-a", &mut kept).unwrap();
    assert_eq!(access.context(), "hub-context");
    assert_eq!(access.sources(), [hub()]);
    // The same host again keeps its client; another context is another one.
    let again = hosted_by(&setup.workspace, "cluster-a", &mut kept).unwrap();
    assert_eq!(again.id(), access.id());
    let mut moved = setup.workspace.clone();
    moved.clusters[0].context = "other".into();
    let other = hosted_by(&moved, "cluster-a", &mut kept).unwrap();
    assert_eq!(other.context(), "other");
    assert_ne!(other.id(), access.id());
    assert!(hosted_by(&setup.workspace, "nowhere", &mut kept).is_none());
}

#[gpui_kit::test]
fn the_open_entrys_scope_follows_the_setup_and_the_entry(cx: &mut TestAppContext) {
    let (_runtime, window, page, handle) = mount_monitoring(cx);
    // Before any setup, and for a context outside the workspace, discovery.
    open_entry(cx, window, &handle, Some("cluster-b"), "cluster-b");
    assert_eq!(resolved(cx, &page), Resolved::Discovery);

    cx.update(|cx| set_metrics_setup(&page, parse(&central()), cx));
    let Resolved::Central { host, matcher, .. } = resolved(cx, &page) else {
        panic!("not central");
    };
    assert_eq!(host.as_deref(), Some("cluster-a"));
    assert_eq!(matcher.value(), "cluster-b");
    // A store hosted by another entry gets that entry's access.
    assert!(matches!(
        cx.read(|cx| page.read(cx).scope().host.clone()),
        Some(ClusterAccess::Direct(_))
    ));

    // The entry that hosts it reads through the open connection.
    open_entry(cx, window, &handle, Some("cluster-a"), "hub-context");
    assert!(matches!(
        cx.read(|cx| page.read(cx).scope().host.clone()),
        Some(ClusterAccess::Example(_))
    ));
    assert_eq!(
        resolved(cx, &page).matcher().map(|m| m.value().to_owned()),
        Some("cluster-a".to_owned())
    );

    // A context that is no entry keeps discovery.
    open_entry(cx, window, &handle, None, "stranger");
    assert_eq!(resolved(cx, &page), Resolved::Discovery);
}

#[gpui_kit::test]
fn a_setup_change_to_per_entry_leaves_an_entry_with_no_source_unknown(cx: &mut TestAppContext) {
    let (_runtime, window, page, handle) = mount_monitoring(cx);
    open_entry(cx, window, &handle, Some("cluster-b"), "cluster-b");
    cx.update(|cx| set_metrics_setup(&page, parse(&central()), cx));
    let per_entry =
        file(CENTRAL_BODY.split("metrics:").next().unwrap()) + "metrics:\n  mode: per-entry\n";
    cx.update(|cx| set_metrics_setup(&page, parse(&per_entry), cx));
    assert_eq!(
        resolved(cx, &page),
        Resolved::Unknown("cluster-b names no metrics source".into())
    );
    // Back to automatic, as before the file.
    let automatic = per_entry.replace("per-entry", "automatic");
    cx.update(|cx| set_metrics_setup(&page, parse(&automatic), cx));
    assert_eq!(resolved(cx, &page), Resolved::Discovery);
}

#[gpui_kit::test]
fn two_windows_on_different_entries_keep_their_own_scope(cx: &mut TestAppContext) {
    let (_r1, window1, page1, handle1) = mount_monitoring(cx);
    let (_r2, window2, page2, handle2) = mount_monitoring(cx);
    open_entry(cx, window1, &handle1, Some("cluster-a"), "hub-context");
    open_entry(cx, window2, &handle2, Some("cluster-b"), "cluster-b");
    // A setup change in one window leaves the other on its own entry.
    cx.update(|cx| set_metrics_setup(&page1, parse(&central()), cx));
    cx.update(|cx| set_metrics_setup(&page2, parse(&central()), cx));
    cx.update(|cx| set_metrics_setup(&page1, parse(&central()), cx));
    let value =
        |cx: &mut TestAppContext, page| resolved(cx, page).matcher().map(|m| m.value().to_owned());
    assert_eq!(value(cx, &page1).as_deref(), Some("cluster-a"));
    assert_eq!(value(cx, &page2).as_deref(), Some("cluster-b"));
}
