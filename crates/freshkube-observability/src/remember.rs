//! What the Coroot connection remembers between launches. `coroot.json`
//! beside the preferences holds the server, how it signs in, the project,
//! whether to reconnect and whether to keep the key. The key itself goes only
//! to `keys.json` ([`crate::secrets`]), named by the server.
use super::*;
use crate::connection::ConnectOrigin;
use crate::secrets::{SecretStore, Secrets};
use freshkube_core::workspace::MAX_BYTES;
use freshkube_prefs::{Document, Layout};
use serde::{Deserialize, Deserializer, Serialize, Serializer, de::Error as _};
use serde_json::{Map, Value, json};
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, Default, PartialEq)]
pub(super) struct Saved {
    pub(super) url: String,
    /// Index into the sign-in choices: API key, session cookie, anonymous.
    pub(super) auth: usize,
    pub(super) project: Option<String>,
    /// Connect again when the page first shows. Disconnect clears it.
    pub(super) reconnect: bool,
    /// Keep the key in `keys.json`.
    pub(super) remember: bool,
    /// Keys this build does not know, written back as read.
    extra: Map<String, Value>,
}

const AUTH: [&str; 3] = ["api-key", "session", "anonymous"];

fn file(preferences: &Path) -> PathBuf {
    preferences.with_file_name("coroot.json")
}

/// `coroot.json`: at most 64 KiB, a new file open to the user's umask. The
/// key is never in it.
fn document(path: &Path) -> Document<Option<Saved>> {
    Document::at(path, Layout::Pretty, 0o666).limited_to(64 * 1024)
}

/// Reads what was saved; anything unreadable counts as nothing saved. A file
/// that exists but can't be used is logged.
pub(super) fn load(preferences: &Path) -> Option<Saved> {
    let loaded = document(&file(preferences)).load();
    if let Some(problem) = &loaded.problem {
        tracing::warn!("Coroot connection not restored: {problem}");
    }
    loaded.value
}

fn write(file: &Path, saved: &Saved) -> std::io::Result<()> {
    document(file).save(&Some(saved.clone()))
}

/// `{url, auth, project, reconnect, remember}`; `project` is `null` when
/// none, plus unknown keys written back as read. No credential is added.
impl Serialize for Saved {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut value = self.extra.clone();
        value.extend(
            json!({
                "url": self.url,
                "auth": AUTH[self.auth.min(2)],
                "project": self.project,
                "reconnect": self.reconnect,
                "remember": self.remember,
            })
            .as_object()
            .expect("saved connection is an object")
            .clone(),
        );
        value.serialize(serializer)
    }
}

/// The server and a known sign-in choice are required; the rest is lenient.
/// Keys this build does not know are kept when saved again.
impl<'de> Deserialize<'de> for Saved {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let value = Value::deserialize(deserializer)?;
        Self::from_json(&value)
            .ok_or_else(|| D::Error::custom("no server, or an unknown sign-in choice"))
    }
}

impl Saved {
    fn from_json(value: &Value) -> Option<Self> {
        let url = value.get("url")?.as_str()?.to_owned();
        let auth = value.get("auth")?.as_str()?;
        let auth = AUTH.iter().position(|known| *known == auth)?;
        let flag = |key: &str| value.get(key).and_then(Value::as_bool) == Some(true);
        Some(Saved {
            url,
            auth,
            project: value
                .get("project")
                .and_then(Value::as_str)
                .map(str::to_owned),
            reconnect: flag("reconnect"),
            remember: flag("remember"),
            extra: value
                .as_object()?
                .iter()
                .filter(|(key, _)| {
                    !["url", "auth", "project", "reconnect", "remember"].contains(&key.as_str())
                })
                .map(|(key, value)| (key.clone(), value.clone()))
                .collect(),
        })
    }
}

/// The credential store's name for a server's key.
pub(super) fn account(url: &str) -> String {
    format!("Coroot {url}")
}

/// The page's memory: what was saved, and the key read back for its server.
pub(super) struct Memory {
    /// `Some(None)` is a setup that deliberately names no Coroot connection.
    setup: Option<Option<freshkube_core::setup::Coroot>>,
    setup_generation: u64,
    file: PathBuf,
    secrets: Option<Secrets>,
    pub(super) saved: Saved,
    /// Reconnect on the first show, once.
    pub(super) restore: bool,
    /// The key the store returned, with the server it belongs to. It is only
    /// ever sent to that server.
    pub(super) key: Option<(String, String)>,
    pub(super) reading: bool,
    pub(super) error: Option<SharedString>,
    /// Store and file work runs one step at a time, in the order asked: each
    /// step waits for the one before it.
    queue: Option<futures::channel::oneshot::Receiver<()>>,
}

impl Memory {
    pub(super) fn new(preferences: &Path, secrets: Option<Secrets>) -> Self {
        let saved = load(preferences).unwrap_or_default();
        Self {
            setup: None,
            setup_generation: 0,
            file: file(preferences),
            secrets,
            restore: saved.reconnect && !saved.url.is_empty(),
            saved,
            key: None,
            reading: false,
            error: None,
            queue: None,
        }
    }

    pub(super) fn can_remember(&self) -> bool {
        self.secrets.is_some()
    }

    /// The remembered key, if it belongs to this server.
    pub(super) fn key_for(&self, url: &str) -> Option<&str> {
        self.key
            .as_ref()
            .filter(|(server, _)| server == url)
            .map(|(_, key)| key.as_str())
    }
}

impl ObservabilityPage {
    /// The shell hands over only a parsed setup. Until then the remembered
    /// connection follows its existing path. This never writes the setup.
    pub fn set_setup_connection(
        &mut self,
        connection: &Option<freshkube_core::setup::Coroot>,
        reconnect: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(memory) = &mut self.memory else {
            return;
        };
        if memory.setup.as_ref() == Some(connection) {
            return;
        }
        // The launch flag seeds this choice once. Later setup changes keep
        // the choice that Connect or Disconnect made in this session.
        let reconnect = if memory.setup.is_some() {
            memory.saved.reconnect
        } else {
            reconnect
        };
        memory.setup = Some(connection.clone());
        memory.setup_generation = memory.setup_generation.wrapping_add(1);
        memory.key = None;
        memory.reading = false;
        memory.error = None;
        memory.saved = connection.as_ref().map_or_else(
            || Saved {
                reconnect,
                ..Saved::default()
            },
            |connection| Saved {
                url: connection.server.clone(),
                auth: match connection.sign_in {
                    freshkube_core::setup::SignIn::ApiKey => 0,
                    freshkube_core::setup::SignIn::Session => 1,
                    freshkube_core::setup::SignIn::Anonymous => 2,
                },
                project: connection.project.clone(),
                reconnect,
                remember: connection.key.is_some(),
                extra: Map::new(),
            },
        );
        memory.restore = reconnect && connection.is_some();
        self.invalidate_connection();
        self.fill_from_memory(window, cx);
        self.secret
            .update(cx, |input, cx| input.set_value("", window, cx));
        self.settings_open = true;
        if self.live.visible {
            self.restore(ConnectOrigin::Page, cx);
        } else if !self.wanting.is_empty() {
            self.restore(ConnectOrigin::Change, cx);
        }
        // Whatever the change page read over the old connection is foreign.
        cx.emit(ObservabilityEvent::Connection);
        cx.notify();
    }

    /// Runs one step of store or file work after the steps before it.
    fn queue<T: Send + 'static>(
        &mut self,
        step: impl FnOnce(&Path, Option<&dyn SecretStore>) -> T + Send + 'static,
        cx: &mut Context<Self>,
    ) -> Option<Task<T>> {
        let memory = self.memory.as_mut()?;
        let (done, finished) = futures::channel::oneshot::channel();
        let previous = memory.queue.replace(finished);
        let (file, secrets) = (memory.file.clone(), memory.secrets.clone());
        Some(cx.background_spawn(async move {
            if let Some(previous) = previous {
                _ = previous.await;
            }
            let result = step(&file, secrets.as_deref());
            _ = done.send(());
            result
        }))
    }

    /// Saves the memory's file; a failure shows beside the form.
    fn save_memory(&mut self, cx: &mut Context<Self>) {
        if self
            .memory
            .as_ref()
            .is_some_and(|memory| memory.setup.is_some())
        {
            return;
        }
        let Some(saved) = self.memory.as_ref().map(|memory| memory.saved.clone()) else {
            return;
        };
        self.run_memory(
            move |file, _| write(file, &saved).map_err(|e| e.to_string()),
            cx,
        );
    }

    fn run_memory(
        &mut self,
        step: impl FnOnce(&Path, Option<&dyn SecretStore>) -> Result<(), String> + Send + 'static,
        cx: &mut Context<Self>,
    ) {
        let Some(task) = self.queue(step, cx) else {
            return;
        };
        cx.spawn(async move |this, cx| {
            if let Err(error) = task.await {
                _ = this.update(cx, |this, cx| {
                    if let Some(memory) = &mut this.memory {
                        memory.error = Some(error.clone().into());
                    }
                    cx.notify();
                });
            }
        })
        .detach();
    }

    /// Fills the form from the saved connection.
    pub(super) fn fill_from_memory(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(saved) = self.memory.as_ref().map(|memory| memory.saved.clone()) else {
            return;
        };
        self.auth = saved.auth.min(2);
        self.url
            .update(cx, |input, cx| input.set_value(saved.url, window, cx));
    }

    /// On the first show, connects again as the last session did: at once
    /// without a key, or once the store returns the key.
    pub(super) fn restore(&mut self, origin: ConnectOrigin, cx: &mut Context<Self>) {
        let Some(memory) = &mut self.memory else {
            return;
        };
        if !std::mem::take(&mut memory.restore) || self.live.provider.is_some() {
            return;
        }
        let saved = memory.saved.clone();
        if saved.auth == 2 {
            self.connect(origin, cx);
            return;
        }
        let key_file = memory
            .setup
            .as_ref()
            .and_then(|setup| setup.as_ref())
            .and_then(|setup| setup.api_key_file.clone());
        if key_file.is_none() && (!saved.remember || !memory.can_remember()) {
            return;
        }
        let generation = memory.setup_generation;
        memory.reading = true;
        let account = account(&saved.url);
        let Some(read) = self.queue(
            move |_, store| match key_file {
                Some(file) => {
                    let bytes = freshkube_core::read_bounded_regular_file(&file, 64 * 1024)
                        .map_err(|_| {
                            "The configured Coroot key file could not be read".to_owned()
                        })?;
                    let key = String::from_utf8(bytes)
                        .map_err(|_| "The configured Coroot key file is not text".to_owned())?;
                    let key = key.trim();
                    if key.is_empty() {
                        return Err("The configured Coroot key file is empty".into());
                    }
                    Ok(Some(key.to_owned()))
                }
                None => store.map_or(Ok(None), |store| store.read(&account)),
            },
            cx,
        ) else {
            return;
        };
        cx.spawn(async move |this, cx| {
            let result = read.await;
            _ = this.update(cx, |this, cx| {
                let current = this.url.read(cx).value().to_string();
                let typed = !this.secret.read(cx).value().is_empty();
                let Some(memory) = &mut this.memory else {
                    return;
                };
                if memory.setup_generation != generation {
                    return;
                }
                memory.reading = false;
                match result {
                    Ok(Some(key)) => {
                        memory.key = Some((saved.url.clone(), key));
                        // The user may have moved on while the store answered.
                        if current == saved.url
                            && this.auth == saved.auth
                            && !typed
                            && this.live.provider.is_none()
                            && !this.live.connecting
                        {
                            if origin == ConnectOrigin::Settings {
                                this.connect(origin, cx);
                            } else if this.live.visible {
                                // The page was shown while the key was read,
                                // whoever asked: it connects as a show does.
                                this.connect(ConnectOrigin::Page, cx);
                            } else if origin == ConnectOrigin::Page || this.wanting.is_empty() {
                                // The key arrived after hiding, or after the
                                // change pages stopped asking. Retry on the
                                // next show, retaining the reconnect choice.
                                memory.restore = true;
                            } else {
                                this.connect(ConnectOrigin::Change, cx);
                            }
                        }
                    }
                    Ok(None) => {}
                    Err(error) => memory.error = Some(error.into()),
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// The key to send: what was typed, else the remembered key for this
    /// server.
    pub(super) fn credential_value(&self, url: &str, typed: String) -> String {
        if !typed.is_empty() || self.auth == 2 {
            return typed;
        }
        self.memory
            .as_ref()
            .and_then(|memory| memory.key_for(url))
            .map(str::to_owned)
            .unwrap_or_default()
    }

    /// After a successful connection: saves it, keeps or forgets the key as
    /// the user chose, and opens the remembered project on the same server.
    pub(super) fn remember_connection(
        &mut self,
        url: String,
        auth: usize,
        key: String,
        cx: &mut Context<Self>,
    ) {
        let Some(memory) = &mut self.memory else {
            return;
        };
        if memory.setup.is_some() {
            memory.saved.reconnect = true;
            // The configured block names `keys.json` when the key is kept;
            // a key typed in Settings goes there once it has worked.
            let keep = auth != 2
                && !key.is_empty()
                && memory.can_remember()
                && memory.saved.remember
                && memory.key_for(&url) != Some(key.as_str());
            memory.key = (auth != 2).then_some((url.clone(), key.clone()));
            let project = memory.saved.project.clone();
            if keep {
                self.run_key_step(
                    move |_, store| store.map_or(Ok(()), |store| store.write(&account(&url), &key)),
                    true,
                    cx,
                );
            }
            self.open_first_project(project.as_deref(), cx);
            self.save_reconnect(true, cx);
            return;
        }
        let previous = std::mem::take(&mut memory.saved.url);
        let same_server = previous == url;
        memory.saved = Saved {
            project: memory.saved.project.take().filter(|_| same_server),
            url: url.clone(),
            auth,
            reconnect: true,
            remember: memory.saved.remember,
            extra: std::mem::take(&mut memory.saved.extra),
        };
        memory.error = None;
        let keep = memory.saved.remember && auth != 2 && memory.can_remember();
        memory.key = keep.then(|| (url.clone(), key.clone()));
        let project = memory.saved.project.clone();
        self.save_memory(cx);
        if self.memory.as_ref().is_some_and(Memory::can_remember) {
            let step = move |_: &Path, store: Option<&dyn SecretStore>| {
                let Some(store) = store else {
                    return Ok(());
                };
                if !same_server && !previous.is_empty() {
                    store.forget(&account(&previous))?;
                }
                if keep {
                    store.write(&account(&url), &key)
                } else {
                    store.forget(&account(&url))
                }
            };
            self.run_key_step(step, keep, cx);
        }
        self.open_first_project(project.as_deref(), cx);
    }

    /// The project a new connection opens on: the open entry's linked one,
    /// else the one remembered for this server.
    fn open_first_project(&mut self, remembered: Option<&str>, cx: &mut Context<Self>) {
        if self.select_linked_project(cx) {
            return;
        }
        if let Some(project) = remembered
            && let Some(project) = self.live.projects.iter().find(|p| p.id == project).cloned()
        {
            self.select_project(&project, cx);
        }
    }

    /// A key step that, if saving fails, turns Remember off and says why.
    fn run_key_step(
        &mut self,
        step: impl FnOnce(&Path, Option<&dyn SecretStore>) -> Result<(), String> + Send + 'static,
        saving: bool,
        cx: &mut Context<Self>,
    ) {
        self.run_key_step_then(step, saving, None, cx);
    }

    /// [`Self::run_key_step`], and `kept` is announced once the store has it.
    fn run_key_step_then(
        &mut self,
        step: impl FnOnce(&Path, Option<&dyn SecretStore>) -> Result<(), String> + Send + 'static,
        saving: bool,
        kept: Option<String>,
        cx: &mut Context<Self>,
    ) {
        let Some(task) = self.queue(step, cx) else {
            return;
        };
        cx.spawn(async move |this, cx| {
            let Err(error) = task.await else {
                if let Some(server) = kept {
                    _ = this.update(cx, |_, cx| cx.emit(ObservabilityEvent::KeyKept(server)));
                }
                return;
            };
            _ = this.update(cx, |this, cx| {
                let Some(memory) = &mut this.memory else {
                    return;
                };
                if saving {
                    memory.saved.remember = false;
                    memory.key = None;
                    let words = format!(
                        "Couldn't save the key: {error}. It stays in memory until you quit."
                    );
                    memory.error = Some(words.clone().into());
                    if this.live.provider.is_none() {
                        this.live.error = Some(words);
                    }
                    this.save_memory(cx);
                } else {
                    memory.error = Some(error.clone().into());
                }
                if let Some(server) = kept {
                    cx.emit(ObservabilityEvent::KeyNotKept {
                        server,
                        why: format!(
                            "Couldn't save the key: {error}. It stays in memory until you quit."
                        ),
                    });
                }
                cx.notify();
            });
        })
        .detach();
    }

    pub(super) fn remember_project(&mut self, project: &str, cx: &mut Context<Self>) {
        let Some(memory) = &mut self.memory else {
            return;
        };
        if memory.saved.project.as_deref() == Some(project) {
            return;
        }
        memory.saved.project = Some(project.to_owned());
        self.save_memory(cx);
    }

    /// Setup is shared configuration; the launch choice stays machine-local.
    fn save_reconnect(&mut self, reconnect: bool, cx: &mut Context<Self>) {
        self.run_memory(
            move |file, _| {
                Document::<Map<String, Value>>::at(
                    &file.with_file_name("preferences.json"),
                    Layout::Pretty,
                    0o666,
                )
                .limited_to(MAX_BYTES)
                .update(|values| {
                    values.insert("coroot_reconnect".into(), Value::Bool(reconnect));
                })
                .map_err(|_| "Couldn't save the Coroot reconnect preference".to_owned())
            },
            cx,
        );
    }

    /// Disconnect forgets the saved key and stops reconnecting on launch; the
    /// server and sign-in choice stay filled in.
    pub(super) fn forget_connection(&mut self, cx: &mut Context<Self>) {
        let Some(memory) = &mut self.memory else {
            return;
        };
        memory.restore = false;
        memory.key = None;
        memory.saved.reconnect = false;
        if memory.setup.is_some() {
            self.save_reconnect(false, cx);
            return;
        }
        let url = memory.saved.url.clone();
        let can_remember = memory.can_remember();
        self.save_memory(cx);
        if can_remember && !url.is_empty() {
            self.run_key_step(
                move |_, store| store.map_or(Ok(()), |store| store.forget(&account(&url))),
                false,
                cx,
            );
        }
    }

    /// The store keeps `key` for `server`; a store that can't says why in the
    /// connection's state.
    pub(super) fn keep_key(&mut self, server: &str, key: &str, cx: &mut Context<Self>) {
        let Some(memory) = &mut self.memory else {
            return;
        };
        if !memory.can_remember() {
            return;
        }
        memory.key = Some((server.to_owned(), key.to_owned()));
        let (account, key) = (account(server), key.to_owned());
        let server = server.to_owned();
        self.run_key_step_then(
            move |_, store| store.map_or(Ok(()), |store| store.write(&account, &key)),
            true,
            Some(server),
            cx,
        );
    }

    /// The store no longer keeps the key for `server`.
    pub(super) fn forget_stored_key(&mut self, server: &str, cx: &mut Context<Self>) {
        let Some(memory) = &mut self.memory else {
            return;
        };
        if memory.key_for(server).is_some() {
            memory.key = None;
        }
        let account = account(server);
        self.run_key_step(
            move |_, store| store.map_or(Ok(()), |store| store.forget(&account)),
            false,
            cx,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::{Memory, Saved, file, load, write};

    const EXAMPLE: &str = include_str!("../../freshkube-prefs/src/examples/coroot.json");

    #[test]
    fn unknown_keys_round_trip_unchanged() {
        let directory = tempfile::tempdir().unwrap();
        let preferences = directory.path().join("preferences.json");
        let mut value: serde_json::Value = serde_json::from_str(EXAMPLE).unwrap();
        value["future"] = serde_json::json!({"enabled": true, "options": [null, 7, "example"]});
        std::fs::write(file(&preferences), serde_json::to_vec(&value).unwrap()).unwrap();

        let mut saved = load(&preferences).expect("the connection loads");
        write(&file(&preferences), &saved).unwrap();
        let read = || {
            serde_json::from_slice::<serde_json::Value>(&std::fs::read(file(&preferences)).unwrap())
                .unwrap()
        };
        assert_eq!(read(), value);

        saved.reconnect = !saved.reconnect;
        value["reconnect"] = saved.reconnect.into();
        write(&file(&preferences), &saved).unwrap();
        assert_eq!(read(), value);
    }

    #[test]
    fn saves_the_connection_without_its_key_and_reads_it_back() {
        let directory = tempfile::tempdir().unwrap();
        let preferences = directory.path().join("preferences.json");
        assert_eq!(load(&preferences), None);
        let saved = Saved {
            url: "https://coroot.example.com".into(),
            auth: 1,
            project: Some("p2".into()),
            reconnect: true,
            remember: true,
            ..Saved::default()
        };
        write(&file(&preferences), &saved).unwrap();
        assert_eq!(load(&preferences), Some(saved));
        let text = std::fs::read_to_string(file(&preferences)).unwrap();
        assert!(text.contains("\"auth\": \"session\""), "{text}");
        std::fs::write(file(&preferences), "{\"url\":\"x\",\"auth\":\"other\"}").unwrap();
        assert_eq!(load(&preferences), None);
    }

    #[test]
    fn a_stored_key_belongs_only_to_its_server() {
        let directory = tempfile::tempdir().unwrap();
        let mut memory = Memory::new(&directory.path().join("preferences.json"), None);
        memory.key = Some(("https://a.example".into(), "key".into()));
        assert_eq!(memory.key_for("https://a.example"), Some("key"));
        assert_eq!(memory.key_for("https://b.example"), None);
        assert_eq!(memory.key_for("https://a.example/"), None);
    }

    /// The example file in `freshkube-prefs`, as the app writes it: what
    /// loads from it saves back byte for byte.
    #[test]
    fn the_example_file_round_trips_byte_for_byte() {
        let directory = tempfile::tempdir().unwrap();
        let preferences = directory.path().join("preferences.json");
        std::fs::write(file(&preferences), EXAMPLE).unwrap();
        let saved = load(&preferences).expect("the example loads");
        assert_eq!(saved.url, "https://coroot.example");
        write(&file(&preferences), &saved).unwrap();
        assert_eq!(
            std::fs::read_to_string(file(&preferences)).unwrap(),
            EXAMPLE
        );
    }

    #[test]
    fn an_unusable_file_is_nothing_saved() {
        let directory = tempfile::tempdir().unwrap();
        let preferences = directory.path().join("preferences.json");
        let padding = " ".repeat(64 * 1024);
        for text in [
            "not json",
            "[]",
            "null",
            "{}",
            r#"{"auth":"session"}"#,
            &format!("{EXAMPLE}{padding}"),
        ] {
            std::fs::write(file(&preferences), text).unwrap();
            assert_eq!(load(&preferences), None, "{text:.20}");
        }
        std::fs::remove_file(file(&preferences)).unwrap();
        std::fs::create_dir(file(&preferences)).unwrap();
        assert_eq!(load(&preferences), None);
    }
}
