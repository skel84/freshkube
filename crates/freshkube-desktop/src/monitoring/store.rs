//! What Monitoring remembers between launches: the user's dashboards
//! folder and, per context, the Prometheus Service that answered. Its own
//! file beside the preferences, so its writes never overwrite theirs, and
//! nothing in it is a credential.
use std::collections::BTreeMap;
use std::fs::OpenOptions;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicU64, Ordering},
};

use freshkube_core::monitoring::PrometheusService;
use serde_json::{Value, json};

#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Saved {
    pub(crate) folder: Option<PathBuf>,
    /// The confirmed Service by context name.
    pub(crate) services: BTreeMap<String, PrometheusService>,
}

fn file(preferences: &Path) -> PathBuf {
    preferences.with_file_name("monitoring.json")
}

/// Reads what was saved; anything unreadable is left out.
pub(crate) fn load(preferences: &Path) -> Saved {
    let Some(value) = std::fs::read(file(preferences))
        .ok()
        .filter(|bytes| bytes.len() <= 256 * 1024)
        .and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok())
    else {
        return Saved::default();
    };
    let folder = value
        .get("dashboards")
        .and_then(Value::as_str)
        .map(PathBuf::from)
        .filter(|path| path.is_absolute());
    let services = value
        .get("prometheus")
        .and_then(Value::as_object)
        .map(|services| {
            services
                .iter()
                .filter_map(|(context, service)| {
                    let text = |key: &str| service.get(key)?.as_str().map(str::to_owned);
                    let port = u16::try_from(service.get("port")?.as_u64()?).ok()?;
                    Some((
                        context.clone(),
                        PrometheusService {
                            namespace: text("namespace")?,
                            name: text("name")?,
                            port,
                            https: service.get("https").and_then(Value::as_bool) == Some(true),
                        },
                    ))
                })
                .collect()
        })
        .unwrap_or_default();
    Saved { folder, services }
}

/// Saves the latest state on a background thread. Writes are serialized,
/// and each writes whatever is latest, so a late write never restores an
/// older state.
#[derive(Clone)]
pub(crate) struct MonitoringStore {
    file: PathBuf,
    latest: Arc<Mutex<Saved>>,
    writer: Arc<Mutex<()>>,
}

impl MonitoringStore {
    pub(crate) fn new(preferences: &Path, saved: Saved) -> Self {
        Self {
            file: file(preferences),
            latest: Arc::new(Mutex::new(saved)),
            writer: Default::default(),
        }
    }

    pub(crate) fn update(&self, change: impl FnOnce(&mut Saved)) {
        change(&mut self.latest.lock().expect("monitoring preference mutex"));
    }

    pub(crate) fn save_latest(&self) -> std::io::Result<()> {
        let _writer = self
            .writer
            .lock()
            .map_err(|_| std::io::Error::other("monitoring writer mutex poisoned"))?;
        let saved = self
            .latest
            .lock()
            .map_err(|_| std::io::Error::other("monitoring preference mutex poisoned"))?
            .clone();
        let services: serde_json::Map<String, Value> = saved
            .services
            .iter()
            .map(|(context, service)| {
                (
                    context.clone(),
                    json!({
                        "namespace": service.namespace,
                        "name": service.name,
                        "port": service.port,
                        "https": service.https,
                    }),
                )
            })
            .collect();
        let value = json!({ "dashboards": saved.folder, "prometheus": services });
        if let Some(parent) = self.file.parent() {
            std::fs::create_dir_all(parent)?;
        }
        static NEXT_WRITE: AtomicU64 = AtomicU64::new(0);
        let temporary = self.file.with_extension(format!(
            "json.{}.{}.tmp",
            std::process::id(),
            NEXT_WRITE.fetch_add(1, Ordering::Relaxed)
        ));
        let result = (|| {
            let mut output = OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&temporary)?;
            output.write_all(format!("{value:#}\n").as_bytes())?;
            output.sync_all()?;
            std::fs::rename(&temporary, &self.file)
        })();
        if result.is_err() {
            let _ = std::fs::remove_file(temporary);
        }
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn saves_the_folder_and_services_and_reads_them_back() {
        let directory = std::env::temp_dir().join(format!(
            "freshkube-monitoring-store-{}-{}",
            std::process::id(),
            line!()
        ));
        let _ = std::fs::remove_dir_all(&directory);
        std::fs::create_dir_all(&directory).unwrap();
        let preferences = directory.join("preferences.json");
        std::fs::write(&preferences, "{\"text_size\":18}").unwrap();
        assert_eq!(load(&preferences), Saved::default());
        let store = MonitoringStore::new(&preferences, Saved::default());
        let mut service = PrometheusService::new("monitoring", "prometheus-operated", 9090);
        service.https = true;
        store.update(|saved| {
            saved.folder = Some(directory.join("dashboards"));
            saved.services.insert("prod".into(), service.clone());
        });
        store.save_latest().unwrap();
        let saved = load(&preferences);
        assert_eq!(saved.folder, Some(directory.join("dashboards")));
        assert_eq!(saved.services.get("prod"), Some(&service));
        // The text size is another file's business.
        assert_eq!(
            std::fs::read_to_string(&preferences).unwrap(),
            "{\"text_size\":18}"
        );
        std::fs::write(file(&preferences), "{\"dashboards\":\"relative\"}").unwrap();
        assert_eq!(load(&preferences), Saved::default());
        std::fs::remove_dir_all(&directory).unwrap();
    }
}
