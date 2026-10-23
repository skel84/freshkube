use super::MonitoringPage;
use super::{
    Check, Field, Pilot, Resolved, SettingsPage, SettingsSection, Verdict, form, setup, ui,
};
use crate::{
    GpuiOptions,
    desktop::{
        layout_check,
        testing::{fixture, mount},
    },
};
use freshkube_ui::real_io::RealIo;
use gpui_kit::{AnyWindowHandle, TestAppContext, test::TestWindowExt};
use gpui_kit::{App, AppContext, Entity, Focusable, Window, px};
use std::sync::Arc;

mod checks;
mod context;
mod drafts;
mod folders;
mod layout;
mod modes;
mod per_entry;
mod saves;
mod tokens;

struct NeverClient;

impl freshkube_core::cluster_source::KubeClientSource for NeverClient {
    fn client(
        &self,
    ) -> futures::future::BoxFuture<'_, Result<kube::Client, freshkube_core::failure::Failure>>
    {
        Box::pin(async {
            Err(freshkube_core::failure::Failure::new(
                freshkube_core::failure::FailureKind::Unreachable,
                "The invented source is unreachable",
            ))
        })
    }
}

const SETUP: &str = "\
version: 1
clusters:
- {id: cluster-a, role: core, context: cluster-a, future_entry: {keep: true}}
sources: {argocd: cluster-a}
destinations: [{name: example, entry: cluster-a}]
metrics:
  mode: central
  store:
    entry: cluster-a
    service: {namespace: metrics, name: prometheus, port: 9090, future_service: [7, null]}
    future_store: {keep: true}
  future_metrics: [example, false]
coroot: {server: 'https://coroot.example.test', sign_in: anonymous, future_coroot: true}
supply_chain: {transparency_log: false, future_chain: true}
future: {keep: [7, null, true]}
";

fn open(
    cx: &mut TestAppContext,
    handle: AnyWindowHandle,
    pilot: &Entity<Pilot>,
) -> Entity<SettingsPage> {
    cx.update_window(handle, |_, window, cx| {
        pilot.update(cx, |pilot, cx| {
            pilot.show_settings(SettingsSection::Monitoring, window, cx)
        });
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| window.render_frame(cx))
        .unwrap();
    cx.read(|cx| pilot.read(cx).settings_page.clone())
}

pub(in crate::desktop::settings) async fn live(
    cx: &mut TestAppContext,
    directory: &std::path::Path,
    context: &str,
) -> (
    tokio::runtime::Runtime,
    AnyWindowHandle,
    Entity<Pilot>,
    Entity<SettingsPage>,
) {
    cx.executor().allow_parking();
    let options = GpuiOptions::kubernetes_only(
        Some(directory.join("missing-kubeconfig")),
        Some(context.into()),
        100,
    )
    .with_preferences(Some(directory.join("preferences.json")));
    let (runtime, handle, pilot) = mount(cx, options, 1280., 1600.);
    cx.wait_for_io(handle, |_, cx| !pilot.read(cx).config_loading)
        .await;
    cx.update_window(handle, |_, window, cx| {
        pilot.update(cx, |pilot, cx| {
            pilot.adopt_launched_entry(
                &crate::desktop::switch::Launched {
                    kubernetes_only: true,
                    kubeconfig: None,
                    kube_context: Some(context.into()),
                    talosconfig: None,
                    talos_context: None,
                },
                cx,
            );
            pilot.pages.monitoring.update(cx, |page, cx| {
                page.set_source(
                    Some(freshkube_core::cluster_source::ClusterSource {
                        id: format!("fake-{context}"),
                        context: context.into(),
                        access: freshkube_core::cluster_source::ClusterAccess::Live(Arc::new(
                            NeverClient,
                        )),
                    }),
                    cx,
                )
            });
        });
        window.activate_window();
    })
    .unwrap();
    cx.run_until_parked();
    let page = open(cx, handle, &pilot);
    (runtime, handle, pilot, page)
}

pub(in crate::desktop::settings) fn replace(
    window: &mut Window,
    cx: &mut App,
    id: &'static str,
    value: &str,
) {
    window.click(id, cx);
    window.press("secondary-a", cx);
    window.input(value, cx);
}

async fn saved(cx: &mut TestAppContext, handle: AnyWindowHandle, page: &Entity<SettingsPage>) {
    cx.wait_for_io(handle, |_, cx| {
        page.read(cx).monitoring.as_ref().unwrap().saving.is_none()
    })
    .await;
    cx.run_until_parked();
}

async fn reload(cx: &mut TestAppContext, handle: AnyWindowHandle, page: &Entity<SettingsPage>) {
    cx.update(|cx| {
        let file = page.read(cx).monitoring.as_ref().unwrap().file.clone();
        file.update(cx, |file, cx| file.reload(cx)).detach();
    });
    cx.wait_for_io(handle, |_, cx| !page.read(cx).setup_refusal)
        .await;
    cx.run_until_parked();
}

fn url_setup() -> String {
    let mut setup = setup::parse(SETUP.as_bytes()).unwrap();
    let store = setup.metrics.store.as_mut().unwrap();
    store.entry = None;
    store.service = None;
    store.url = Some("https://metrics.example.test".into());
    store.token = Some(setup::KeysJson);
    setup::to_text(&setup)
}

fn set_secrets(
    cx: &mut TestAppContext,
    runtime: &tokio::runtime::Runtime,
    page: &Entity<SettingsPage>,
    directory: &std::path::Path,
    secrets: freshkube_core::secrets::Secrets,
) {
    cx.update(|cx| {
        let monitoring = cx.new(|cx| {
            super::MonitoringPage::new(
                runtime.handle().clone(),
                Some(&directory.join("preferences.json")),
                Some(secrets),
                cx,
            )
        });
        page.update(cx, |page, _| {
            page.monitoring.as_mut().unwrap().page = monitoring
        });
    });
}

fn write_context_source(directory: &std::path::Path, token: bool) {
    let value = serde_json::json!({"sources": {
        "outside-example": {"kind": "url", "url": "https://metrics.example.test", "token": token}
    }});
    std::fs::write(
        directory.join("monitoring.json"),
        serde_json::to_vec(&value).unwrap(),
    )
    .unwrap();
}

/// Deletion must see a committed file with no remaining reference to the
/// account. The fake never reads a key file or the system credential store.
struct TokenStore {
    memory: freshkube_core::secrets::MemoryStore,
    file: std::path::PathBuf,
    forgotten: std::sync::atomic::AtomicUsize,
}

impl TokenStore {
    fn new(file: std::path::PathBuf) -> Arc<Self> {
        let store = Arc::new(Self {
            memory: freshkube_core::secrets::MemoryStore::default(),
            file,
            forgotten: std::sync::atomic::AtomicUsize::new(0),
        });
        store.memory.keys.lock().unwrap().insert(
            "Prometheus https://metrics.example.test/".into(),
            "invented-token".into(),
        );
        store
    }

    fn assert_kept(&self, kept: bool) {
        assert_eq!(
            self.memory
                .keys
                .lock()
                .unwrap()
                .contains_key("Prometheus https://metrics.example.test/"),
            kept,
        );
        assert_eq!(
            self.forgotten.load(std::sync::atomic::Ordering::SeqCst),
            usize::from(!kept)
        );
    }
}

impl freshkube_core::secrets::SecretStore for TokenStore {
    fn read(&self, account: &str) -> Result<Option<String>, String> {
        self.memory.read(account)
    }

    fn write(&self, account: &str, value: &str) -> Result<(), String> {
        self.memory.write(account, value)
    }

    fn forget(&self, account: &str) -> Result<(), String> {
        let setup = setup::parse(&std::fs::read(&self.file).unwrap()).unwrap();
        assert!(
            !freshkube_core::monitoring::scope::token_urls(&setup)
                .iter()
                .any(|url| crate::monitoring::token_account(url) == account),
            "the YAML reference must be saved away before deleting its token"
        );
        assert!(
            !crate::monitoring::saved_token_urls(&self.file.with_file_name("preferences.json"))
                .unwrap()
                .iter()
                .any(|url| crate::monitoring::token_account(url) == account),
            "a context still naming this token must keep it"
        );
        self.forgotten
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        self.memory.forget(account)
    }
}
