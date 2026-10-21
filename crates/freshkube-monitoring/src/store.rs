//! What Monitoring remembers between launches: the user's dashboards
//! folder and, per context, the metrics source the user chose and the
//! Service that last answered discovery. Its own file beside the
//! preferences, so its writes never overwrite theirs. Nothing in it is a
//! credential: a URL's token lives in the system's credential store, and
//! this file only says whether one was saved.
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use freshkube_core::monitoring::PrometheusService;
use freshkube_core::monitoring::alerts::Remembered;
use freshkube_prefs::{Document, Layout};
use serde::{Deserialize, Deserializer, Serialize, Serializer, ser::Error as _};
use serde_json::{Map, Value, json};

#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Saved {
    pub(crate) folder: Option<PathBuf>,
    /// The Service discovery confirmed, by context name.
    pub(crate) services: BTreeMap<String, PrometheusService>,
    /// The source the user chose, by context name; none is automatic.
    pub(crate) choices: BTreeMap<String, Choice>,
    /// The Alertmanager and vmalert Services that last answered, by context
    /// name, which alert discovery tries first.
    pub(crate) alerts: BTreeMap<String, Remembered>,
    /// Unknown keys at the top level and within each context's entries.
    extra: Map<String, Value>,
    service_extra: BTreeMap<String, Map<String, Value>>,
    choice_extra: BTreeMap<String, Map<String, Value>>,
    alert_extra: BTreeMap<String, Map<String, Value>>,
}

const SERVICE_KEYS: &[&str] = &["namespace", "name", "port", "https", "path"];

fn extra(value: &Value, known: &[&str]) -> Map<String, Value> {
    value
        .as_object()
        .into_iter()
        .flatten()
        .filter(|(key, _)| !known.contains(&key.as_str()))
        .map(|(key, value)| (key.clone(), value.clone()))
        .collect()
}

fn with_extra(mut value: Value, extra: Option<&Map<String, Value>>) -> Value {
    let mut object = extra.cloned().unwrap_or_default();
    object.extend(std::mem::take(
        value.as_object_mut().expect("saved entry is an object"),
    ));
    Value::Object(object)
}

/// A metrics source the user chose for a context instead of discovery.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Choice {
    /// This Service only, through the Kubernetes API's service proxy.
    Service(PrometheusService),
    /// A URL from `normalise_url`, and whether a token for it is saved.
    Url { url: String, token: bool },
}

/// The credential store account that holds a URL's token.
pub fn token_account(url: &str) -> String {
    format!("Prometheus {url}")
}

fn read_service(value: &Value) -> Option<PrometheusService> {
    let text = |key: &str| value.get(key)?.as_str().map(str::to_owned);
    let port = u16::try_from(value.get("port")?.as_u64()?).ok()?;
    Some(PrometheusService {
        namespace: text("namespace")?,
        name: text("name")?,
        port,
        https: value.get("https").and_then(Value::as_bool) == Some(true),
        path: freshkube_core::monitoring::normalise_path(&text("path").unwrap_or_default()),
    })
}

fn write_service(service: &PrometheusService) -> Value {
    json!({
        "namespace": service.namespace,
        "name": service.name,
        "port": service.port,
        "https": service.https,
        "path": service.path,
    })
}

const ALERT_KEYS: &[&str] = &["alertmanager", "vmalert"];

/// A context's remembered alert Services; none when it names neither.
fn read_alerts(value: &Value) -> Option<Remembered> {
    let remembered = Remembered {
        alertmanager: value.get("alertmanager").and_then(read_service),
        vmalert: value.get("vmalert").and_then(read_service),
    };
    (remembered != Remembered::default()).then_some(remembered)
}

fn write_alerts(remembered: &Remembered) -> Value {
    let mut value = Map::new();
    if let Some(service) = &remembered.alertmanager {
        value.insert("alertmanager".into(), write_service(service));
    }
    if let Some(service) = &remembered.vmalert {
        value.insert("vmalert".into(), write_service(service));
    }
    Value::Object(value)
}

fn read_choice(value: &Value) -> Option<Choice> {
    match value.get("kind")?.as_str()? {
        "service" => read_service(value).map(Choice::Service),
        "url" => {
            let url =
                freshkube_core::monitoring::normalise_url(value.get("url")?.as_str()?).ok()?;
            let token = value.get("token").and_then(Value::as_bool) == Some(true);
            Some(Choice::Url { url, token })
        }
        _ => None,
    }
}

fn write_choice(choice: &Choice) -> Value {
    match choice {
        Choice::Service(service) => {
            let mut value = write_service(service);
            value["kind"] = "service".into();
            value
        }
        Choice::Url { url, token } => json!({"kind": "url", "url": url, "token": token}),
    }
}

fn file(preferences: &Path) -> PathBuf {
    preferences.with_file_name("monitoring.json")
}

/// `monitoring.json`: at most 256 KiB, a new file open to the user's umask.
fn document(path: &Path) -> Document<Saved> {
    Document::at(path, Layout::Pretty, 0o666).limited_to(256 * 1024)
}

/// Reads what was saved; anything unreadable is left out. A file that exists
/// but can't be used is logged.
pub(crate) fn load(preferences: &Path) -> Saved {
    match load_checked(preferences) {
        Ok(saved) => saved,
        Err(problem) => {
            tracing::warn!("monitoring preferences not used: {problem}");
            Saved::default()
        }
    }
}

fn load_checked(preferences: &Path) -> Result<Saved, String> {
    let loaded = document(&file(preferences)).load();
    match loaded.problem {
        Some(problem) => Err(problem),
        None => Ok(loaded.value),
    }
}

/// Token URLs named by persisted per-context sources. Read off the UI thread;
/// an unreadable file cannot establish that an account is unused.
pub fn saved_token_urls(preferences: &Path) -> Result<BTreeSet<String>, String> {
    load_checked(preferences).map(|saved| saved.token_urls())
}

/// `{dashboards, prometheus, sources, alerts}`, the last three by context name.
impl Serialize for Saved {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let services: serde_json::Map<String, Value> = self
            .services
            .iter()
            .map(|(context, service)| {
                (
                    context.clone(),
                    with_extra(write_service(service), self.service_extra.get(context)),
                )
            })
            .collect();
        let choices: serde_json::Map<String, Value> = self
            .choices
            .iter()
            .map(|(context, choice)| {
                (
                    context.clone(),
                    with_extra(write_choice(choice), self.choice_extra.get(context)),
                )
            })
            .collect();
        let mut alerts = serde_json::Map::new();
        for (context, remembered) in &self.alerts {
            alerts.insert(
                context.clone(),
                with_extra(write_alerts(remembered), self.alert_extra.get(context)),
            );
        }
        let mut written = json!({
            "dashboards": serde_json::to_value(&self.folder).map_err(S::Error::custom)?,
            "prometheus": services,
            "sources": choices,
        });
        // Only once there is something to say, so a file without alerts is
        // written as it always was.
        if !alerts.is_empty() {
            written["alerts"] = Value::Object(alerts);
        }
        with_extra(written, Some(&self.extra)).serialize(serializer)
    }
}

/// Lenient, as the file always was: a key that is missing or malformed, and
/// an entry that is, is left out. Unknown keys at the top level and within
/// usable entries are written back as read.
impl<'de> Deserialize<'de> for Saved {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        Ok(Self::from_json(&Value::deserialize(deserializer)?))
    }
}

impl Saved {
    pub(crate) fn token_urls(&self) -> BTreeSet<String> {
        self.choices
            .values()
            .filter_map(|choice| match choice {
                Choice::Url { url, token: true } => Some(url.clone()),
                _ => None,
            })
            .collect()
    }

    fn from_json(value: &Value) -> Self {
        let folder = value
            .get("dashboards")
            .and_then(Value::as_str)
            .map(PathBuf::from)
            .filter(|path| path.is_absolute());
        let by_context = |key: &str| {
            value
                .get(key)
                .and_then(Value::as_object)
                .into_iter()
                .flatten()
                .map(|(context, value)| (context.clone(), value))
                .collect::<Vec<_>>()
        };
        let services = by_context("prometheus")
            .into_iter()
            .filter_map(|(context, value)| Some((context, read_service(value)?)))
            .collect();
        let choices = by_context("sources")
            .into_iter()
            .filter_map(|(context, value)| Some((context, read_choice(value)?)))
            .collect();
        let service_extra = by_context("prometheus")
            .into_iter()
            .filter_map(|(context, value)| {
                read_service(value)?;
                Some((context, extra(value, SERVICE_KEYS)))
            })
            .collect();
        let choice_extra = by_context("sources")
            .into_iter()
            .filter_map(|(context, value)| {
                let known = match read_choice(value)? {
                    Choice::Service(_) => [SERVICE_KEYS, &["kind"]].concat(),
                    Choice::Url { .. } => vec!["kind", "url", "token"],
                };
                Some((context, extra(value, &known)))
            })
            .collect();
        let alerts = by_context("alerts")
            .into_iter()
            .filter_map(|(context, value)| Some((context, read_alerts(value)?)))
            .collect();
        let alert_extra = by_context("alerts")
            .into_iter()
            .filter_map(|(context, value)| {
                read_alerts(value)?;
                Some((context, extra(value, ALERT_KEYS)))
            })
            .collect();
        Saved {
            folder,
            services,
            choices,
            alerts,
            extra: extra(value, &["dashboards", "prometheus", "sources", "alerts"]),
            service_extra,
            choice_extra,
            alert_extra,
        }
    }
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
        document(&self.file).save(&saved)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn saved_token_urls_read_normalized_token_references_through_the_typed_store() {
        let directory = std::env::temp_dir().join(format!(
            "freshkube-monitoring-token-urls-{}-{}",
            std::process::id(),
            line!()
        ));
        let _ = std::fs::remove_dir_all(&directory);
        std::fs::create_dir_all(&directory).unwrap();
        let preferences = directory.join("preferences.json");
        assert!(saved_token_urls(&preferences).unwrap().is_empty());
        let value = json!({"sources": {
            "outside-a": {"kind": "url", "url": "https://metrics.example.test", "token": true},
            "outside-b": {"kind": "url", "url": "https://metrics.example.test/", "token": true},
            "anonymous": {"kind": "url", "url": "https://anonymous.example.test", "token": false},
            "service": {"kind": "service", "namespace": "metrics", "name": "prometheus", "port": 9090}
        }});
        std::fs::write(file(&preferences), serde_json::to_vec(&value).unwrap()).unwrap();
        assert_eq!(
            saved_token_urls(&preferences).unwrap(),
            ["https://metrics.example.test/".to_owned()].into()
        );
        std::fs::write(file(&preferences), "{").unwrap();
        assert!(saved_token_urls(&preferences).is_err());
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn remembered_alert_services_round_trip_with_their_unknown_keys() {
        let service = json!({"namespace": "monitoring", "name": "alertmanager-operated",
            "port": 9093, "https": false, "path": ""});
        let value = json!({"alerts": {
            "cluster-a": {"alertmanager": service, "future": [1, 2]},
            "cluster-b": {"nothing": true}
        }});
        let saved = Saved::from_json(&value);
        assert_eq!(
            saved.alerts.len(),
            1,
            "an entry naming no Service is left out"
        );
        let remembered = &saved.alerts["cluster-a"];
        assert_eq!(remembered.alertmanager.as_ref().unwrap().port, 9093);
        assert!(remembered.vmalert.is_none());
        assert_eq!(
            serde_json::to_value(&saved).unwrap()["alerts"],
            json!({"cluster-a": {"alertmanager": service, "future": [1, 2]}})
        );
        // A file that says nothing of alerts is written as it always was.
        assert!(
            serde_json::to_value(Saved::default())
                .unwrap()
                .get("alerts")
                .is_none()
        );
    }

    #[test]
    fn a_folder_edit_keeps_unknown_keys_in_monitoring_json() {
        let directory = std::env::temp_dir().join(format!(
            "freshkube-monitoring-folder-{}-{}",
            std::process::id(),
            line!()
        ));
        std::fs::create_dir_all(&directory).unwrap();
        let preferences = directory.join("preferences.json");
        let mut value = json!({
            "dashboards": null,
            "prometheus": {"cluster-a": {"namespace": "metrics", "name": "prometheus", "port": 9090, "https": false, "path": "", "future": [false, null, 7]}},
            "sources": {"cluster-b": {"kind": "url", "url": "https://metrics.example.test/", "token": false, "future": {"label": "example"}}},
            "future": {"options": [true, null, "example"]}
        });
        std::fs::write(file(&preferences), serde_json::to_vec(&value).unwrap()).unwrap();
        let store = MonitoringStore::new(&preferences, load(&preferences));
        let folder = directory.join("dashboards");
        store.update(|saved| saved.folder = Some(folder.clone()));
        store.save_latest().unwrap();
        value["dashboards"] = serde_json::to_value(folder).unwrap();
        let written: Value =
            serde_json::from_slice(&std::fs::read(file(&preferences)).unwrap()).unwrap();
        assert_eq!(written, value);
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn unknown_keys_round_trip_unchanged() {
        let directory = std::env::temp_dir().join(format!(
            "freshkube-monitoring-unknown-{}-{}",
            std::process::id(),
            line!()
        ));
        let _ = std::fs::remove_dir_all(&directory);
        std::fs::create_dir_all(&directory).unwrap();
        let preferences = directory.join("preferences.json");
        let service = json!({
            "namespace": "metrics", "name": "prometheus", "port": 9090,
            "https": false, "path": "", "future": {"labels": ["example", null]},
        });
        let mut selected_service = service.clone();
        selected_service["kind"] = "service".into();
        let mut value = json!({
            "dashboards": directory.join("dashboards"),
            "prometheus": {"example": service},
            "sources": {
                "example": selected_service,
                "lab": {
                    "kind": "url", "url": "https://metrics.example/", "token": false,
                    "future": {"interval": 7, "enabled": true},
                },
            },
            "future": {"options": [null, false, 7, "example"]},
        });
        std::fs::write(file(&preferences), serde_json::to_vec(&value).unwrap()).unwrap();
        let store = MonitoringStore::new(&preferences, load(&preferences));
        store.save_latest().unwrap();
        let read = || {
            serde_json::from_slice::<Value>(&std::fs::read(file(&preferences)).unwrap()).unwrap()
        };
        assert_eq!(read(), value);

        store.update(|saved| {
            saved.services.get_mut("example").unwrap().port = 9091;
            saved.choices.insert(
                "lab".into(),
                Choice::Url {
                    url: "https://other.example/".into(),
                    token: true,
                },
            );
        });
        value["prometheus"]["example"]["port"] = 9091.into();
        value["sources"]["lab"]["url"] = "https://other.example/".into();
        value["sources"]["lab"]["token"] = true.into();
        store.save_latest().unwrap();
        assert_eq!(read(), value);
        std::fs::remove_dir_all(directory).unwrap();
    }

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
        let select =
            PrometheusService::new("vm", "vmselect", 8481).with_path("/select/0/prometheus/");
        let url = Choice::Url {
            url: "https://metrics.example.com/".into(),
            token: true,
        };
        store.update(|saved| {
            saved.folder = Some(directory.join("dashboards"));
            saved.services.insert("prod".into(), service.clone());
            saved
                .choices
                .insert("prod".into(), Choice::Service(select.clone()));
            saved.choices.insert("lab".into(), url.clone());
        });
        store.save_latest().unwrap();
        let saved = load(&preferences);
        assert_eq!(saved.folder, Some(directory.join("dashboards")));
        assert_eq!(saved.services.get("prod"), Some(&service));
        assert_eq!(saved.choices.get("prod"), Some(&Choice::Service(select)));
        assert_eq!(saved.choices.get("lab"), Some(&url));
        // The text size is another file's business.
        assert_eq!(
            std::fs::read_to_string(&preferences).unwrap(),
            "{\"text_size\":18}"
        );
        std::fs::write(file(&preferences), "{\"dashboards\":\"relative\"}").unwrap();
        assert_eq!(load(&preferences), Saved::default());
        std::fs::remove_dir_all(&directory).unwrap();
    }

    /// The example file in `freshkube-prefs`, as the app writes it: what
    /// loads from it saves back byte for byte.
    // The committed example uses POSIX paths, which Windows does not treat as absolute.
    #[cfg(unix)]
    #[test]
    fn the_example_file_round_trips_byte_for_byte() {
        round_trip(
            include_str!("../../freshkube-prefs/src/examples/monitoring.json"),
            "/home/acme/dashboards",
        );
    }

    /// The same round trip with a Windows path, written inline.
    #[cfg(not(unix))]
    #[test]
    fn the_example_file_round_trips_byte_for_byte_off_unix() {
        let example = include_str!("../../freshkube-prefs/src/examples/monitoring.json")
            .replace("/home/acme/dashboards", "C:\\\\acme\\\\dashboards");
        round_trip(&example, "C:\\acme\\dashboards");
    }

    fn round_trip(example: &str, folder: &str) {
        let directory = std::env::temp_dir().join(format!(
            "freshkube-monitoring-example-{}-{}",
            std::process::id(),
            line!()
        ));
        let _ = std::fs::remove_dir_all(&directory);
        std::fs::create_dir_all(&directory).unwrap();
        let preferences = directory.join("preferences.json");
        std::fs::write(file(&preferences), example).unwrap();
        let saved = load(&preferences);
        assert_eq!(saved.folder, Some(PathBuf::from(folder)));
        assert_eq!((saved.services.len(), saved.choices.len()), (1, 2));
        MonitoringStore::new(&preferences, saved)
            .save_latest()
            .unwrap();
        assert_eq!(
            std::fs::read_to_string(file(&preferences)).unwrap(),
            example
        );
        std::fs::remove_dir_all(&directory).unwrap();
    }

    #[test]
    fn an_unusable_file_is_nothing_saved() {
        let directory = std::env::temp_dir().join(format!(
            "freshkube-monitoring-unusable-{}-{}",
            std::process::id(),
            line!()
        ));
        let _ = std::fs::remove_dir_all(&directory);
        std::fs::create_dir_all(&directory).unwrap();
        let preferences = directory.join("preferences.json");
        assert_eq!(load(&preferences), Saved::default());
        let padding = " ".repeat(256 * 1024);
        for text in ["not json", "[]", "null", &format!("{{}}{padding}")] {
            std::fs::write(file(&preferences), text).unwrap();
            assert_eq!(load(&preferences), Saved::default(), "{text:.20}");
        }
        std::fs::remove_file(file(&preferences)).unwrap();
        std::fs::create_dir(file(&preferences)).unwrap();
        assert_eq!(load(&preferences), Saved::default());
        std::fs::remove_dir_all(&directory).unwrap();
    }
}
