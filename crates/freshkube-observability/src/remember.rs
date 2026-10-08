//! What the Coroot connection remembers between launches. `coroot.json`
//! beside the preferences holds the server, how it signs in, the project,
//! whether to reconnect and whether to keep the key. The key itself goes only
//! to the system's credential store ([`crate::secrets`]), named by the server.
use super::*;
use crate::secrets::{STORE_NAME, SecretStore, Secrets};
use serde_json::{Value, json};
use std::fs::OpenOptions;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

#[derive(Clone, Debug, Default, PartialEq)]
pub(super) struct Saved {
    pub(super) url: String,
    /// Index into the sign-in choices: API key, session cookie, anonymous.
    pub(super) auth: usize,
    pub(super) project: Option<String>,
    /// Connect again when the page first shows. Disconnect clears it.
    pub(super) reconnect: bool,
    /// Keep the key in the credential store.
    pub(super) remember: bool,
}

const AUTH: [&str; 3] = ["api-key", "session", "anonymous"];

fn file(preferences: &Path) -> PathBuf {
    preferences.with_file_name("coroot.json")
}

/// Reads what was saved; anything unreadable counts as nothing saved.
pub(super) fn load(preferences: &Path) -> Option<Saved> {
    let value = std::fs::read(file(preferences))
        .ok()
        .filter(|bytes| bytes.len() <= 64 * 1024)
        .and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok())?;
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
    })
}

fn write(file: &Path, saved: &Saved) -> std::io::Result<()> {
    let value = json!({
        "url": saved.url,
        "auth": AUTH[saved.auth.min(2)],
        "project": saved.project,
        "reconnect": saved.reconnect,
        "remember": saved.remember,
    });
    if let Some(parent) = file.parent() {
        std::fs::create_dir_all(parent)?;
    }
    static NEXT_WRITE: AtomicU64 = AtomicU64::new(0);
    let temporary = file.with_extension(format!(
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
        std::fs::rename(&temporary, file)
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(temporary);
    }
    result
}

/// The credential store's name for a server's key.
pub(super) fn account(url: &str) -> String {
    format!("Coroot {url}")
}

/// The page's memory: what was saved, and the key read back for its server.
pub(super) struct Memory {
    file: PathBuf,
    secrets: Option<Secrets>,
    pub(super) saved: Saved,
    /// Reconnect on the first show, once.
    restore: bool,
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
                        memory.error = Some(error.into());
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
    pub(super) fn restore(&mut self, cx: &mut Context<Self>) {
        let Some(memory) = &mut self.memory else {
            return;
        };
        if !std::mem::take(&mut memory.restore) || self.live.provider.is_some() {
            return;
        }
        let saved = memory.saved.clone();
        if saved.auth == 2 {
            self.connect(cx);
            return;
        }
        if !saved.remember || !memory.can_remember() {
            return;
        }
        memory.reading = true;
        let account = account(&saved.url);
        let Some(read) = self.queue(
            move |_, store| store.map_or(Ok(None), |store| store.read(&account)),
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
                            this.connect(cx);
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
        let previous = std::mem::take(&mut memory.saved.url);
        let same_server = previous == url;
        memory.saved = Saved {
            project: memory.saved.project.take().filter(|_| same_server),
            url: url.clone(),
            auth,
            reconnect: true,
            remember: memory.saved.remember,
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
        if let Some(project) = project
            && let Some(project) = self.live.projects.iter().find(|p| p.id == project)
        {
            let project = project.clone();
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
        let Some(task) = self.queue(step, cx) else {
            return;
        };
        cx.spawn(async move |this, cx| {
            let Err(error) = task.await else {
                return;
            };
            _ = this.update(cx, |this, cx| {
                let Some(memory) = &mut this.memory else {
                    return;
                };
                if saving {
                    memory.saved.remember = false;
                    memory.key = None;
                    memory.error = Some(
                        format!(
                            "Couldn't save the key: {error}. It stays in memory until you quit."
                        )
                        .into(),
                    );
                    this.save_memory(cx);
                } else {
                    memory.error = Some(error.into());
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

    /// Remember on saves the key in use at once; off forgets the saved key.
    pub(super) fn set_remember(&mut self, remember: bool, cx: &mut Context<Self>) {
        let typed = self.secret.read(cx).value().to_string();
        let connected = self.live.provider.is_some();
        let Some(memory) = &mut self.memory else {
            return;
        };
        memory.saved.remember = remember;
        memory.error = None;
        let url = memory.saved.url.clone();
        let key = if !typed.is_empty() {
            Some(typed)
        } else {
            memory.key_for(&url).map(str::to_owned)
        };
        if !remember {
            memory.key = None;
        }
        self.save_memory(cx);
        if url.is_empty() {
            cx.notify();
            return;
        }
        if remember {
            if let Some(key) = key.filter(|_| connected && self.auth != 2) {
                if let Some(memory) = &mut self.memory {
                    memory.key = Some((url.clone(), key.clone()));
                }
                self.run_key_step(
                    move |_, store| store.map_or(Ok(()), |store| store.write(&account(&url), &key)),
                    true,
                    cx,
                );
            }
        } else {
            self.run_key_step(
                move |_, store| store.map_or(Ok(()), |store| store.forget(&account(&url))),
                false,
                cx,
            );
        }
        cx.notify();
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

    /// The Remember checkbox and what the store holds, under the key field.
    pub(super) fn render_memory(&self, cx: &Context<Self>) -> Option<AnyElement> {
        let memory = self.memory.as_ref()?;
        let url = self.url.read(cx).value().to_string();
        let typed = !self.secret.read(cx).value().is_empty();
        let note = if memory.reading {
            Some(format!("Reading the key from {STORE_NAME}…"))
        } else if !typed && self.auth != 2 && memory.key_for(&url).is_some() {
            Some(format!("Using the key saved in {STORE_NAME}."))
        } else {
            None
        };
        Some(
            v_flex()
                .gap(dp(6.))
                .when(memory.can_remember() && self.auth != 2, |this| {
                    this.child(
                        gpui_kit::component::checkbox::Checkbox::new("obs-remember")
                            .small()
                            .label(format!("Remember key in {STORE_NAME}"))
                            .checked(memory.saved.remember)
                            .on_click(cx.listener(|this, checked: &bool, _, cx| {
                                this.set_remember(*checked, cx)
                            })),
                    )
                })
                .when_some(note, |this, note| this.child(muted(note, cx)))
                .when_some(memory.error.clone(), |this, error| {
                    this.child(text(error).text_color(palette(cx).crit_ink))
                })
                .into_any_element(),
        )
    }

    /// Where the credential lives, for the line beside Connect.
    pub(super) fn credential_note(&self) -> String {
        match &self.memory {
            Some(memory) if memory.saved.remember && memory.can_remember() && self.auth != 2 => {
                format!("The key is kept in {STORE_NAME} until Disconnect.")
            }
            _ => "Credentials stay in memory until Disconnect or the window closes.".into(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{Memory, Saved, file, load, write};

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
}
