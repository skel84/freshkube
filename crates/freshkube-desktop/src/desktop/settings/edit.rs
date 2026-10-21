//! Changing the workspace: the actions on the selection, and the save they
//! end in. A change is a changed copy of the workspace, checked with
//! `Workspace::validate`, written whole off the UI thread, and shown once the
//! write has happened; a failed write leaves the page and the file as they
//! were. SetupFile protects hand edits and never writes over a broken file.
use super::*;
use freshkube_core::workspace::{Destination, Entry, Fault, Key};

impl SettingsPage {
    /// Whether a change can be saved now: not example data, not without a
    /// preferences folder, and not while a save runs.
    pub(super) fn editable(&self) -> bool {
        self.file.is_some()
            && self.origin != Origin::Example
            && self.saving.is_none()
            && !self.setup_refusal
            && !self.pending_setup
    }

    /// Why the changing actions are off, for their tooltips.
    pub(super) fn why_not_editable(&self) -> &'static str {
        if self.pending_setup {
            "freshkube.yaml changed on disk since it was read. Nothing was written; reload to see the file as it is."
        } else if self.setup_skips {
            crate::setup_file::SKIPPED_SAVE
        } else if self.setup_refusal {
            "freshkube.yaml isn't usable. Nothing was written; fix the file and reload."
        } else if self.origin == Origin::Example {
            "Example data isn’t saved"
        } else if self.file.is_none() {
            "This window keeps no preferences, so there is nowhere to save"
        } else {
            "Saving"
        }
    }

    fn selected_entry(&self) -> Option<&Entry> {
        let key = self.selected.as_ref()?;
        self.workspace
            .clusters
            .iter()
            .find(|entry| entry.id.as_str() == key.as_ref())
    }

    pub(super) fn open_form(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.editable() || window.has_active_dialog(cx) {
            return;
        }
        form::open(cx.entity(), window, cx);
    }

    /// Adds a cluster to the workspace, its id derived from the context.
    /// Everything else about it is edited in the Inspector. Returns why not,
    /// for the dialog to show; the file is untouched then.
    pub(super) fn add_cluster(
        &mut self,
        role: workspace::Role,
        context: &str,
        cx: &mut Context<Self>,
    ) -> Result<(), SharedString> {
        if !self.editable() {
            return Err(self.why_not_editable().into());
        }
        let context = context.trim();
        if context.is_empty() {
            return Err("Enter the kubeconfig context this cluster opens".into());
        }
        let mut next = self.workspace.clone();
        let id = next.fresh_id(context);
        next.clusters.push(Entry::new(id.clone(), role, context));
        // The new row is the selected one once it is saved.
        self.select_on_save = Some(id.clone().into());
        if let Err(why) = next.check_saveable() {
            self.select_on_save = None;
            return Err(capitalized(&why.to_string()).into());
        }
        self.commit(next, format!("Added {id}."), cx);
        Ok(())
    }

    pub(super) fn ask_remove_selected(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(id) = self.selected_entry().map(|entry| entry.id.clone()) else {
            return;
        };
        if !self.editable() || window.has_active_dialog(cx) {
            return;
        }
        let page = cx.entity().downgrade();
        let mapped = self
            .workspace
            .destinations
            .iter()
            .filter(|row| row.entry == id)
            .count();
        let body = match mapped {
            0 => String::new(),
            1 => format!(
                " The Argo CD destination mapped to {id} stays mapped to it, marked, until \
                 it is edited."
            ),
            n => format!(
                " The {n} Argo CD destinations mapped to {id} stay mapped to it, marked, \
                 until they are edited."
            ),
        };
        let body = format!(
            "This takes the cluster out of freshkube.yaml. Nothing on the cluster, and \
             no kubeconfig or talosconfig, is touched.{body}"
        );
        window.open_dialog(cx, move |dialog, window, _| {
            let confirm_page = page.clone();
            let confirm_id = id.clone();
            freshkube_ui::dialog::frame(
                dialog,
                420.,
                format!("Remove {id}?"),
                v_flex()
                    .id("settings-remove-dialog")
                    .test_support()
                    .role(Role::Dialog)
                    .aria_label(format!("Remove {id}"))
                    .gap_3()
                    .text_size(dp(13.))
                    .child(body.clone()),
                freshkube_ui::form::actions(
                    freshkube_ui::form::Action::new(
                        "settings-remove-confirm",
                        "Remove",
                        move |_, window, cx| {
                            window.close_dialog(cx);
                            _ = confirm_page.update(cx, |page, cx| page.remove(&confirm_id, cx));
                        },
                    )
                    .danger(),
                    freshkube_ui::form::Action::cancel("settings-remove-cancel"),
                ),
                window,
            )
        });
    }

    fn remove(&mut self, id: &str, cx: &mut Context<Self>) {
        if !self.editable() {
            return;
        }
        let mut next = self.workspace.clone();
        next.clusters.retain(|entry| entry.id != id);
        self.commit(next, format!("Removed {id}."), cx);
    }

    /// Applies the destination form: maps a destination to a workspace
    /// cluster, or changes the mapping at `editing`, which must still match
    /// `was`. Returns what the row now matches once the save has started, or
    /// why not, for the form to show; the file is untouched then.
    pub(super) fn map_destination(
        &mut self,
        editing: Option<(usize, &Key)>,
        key: Key,
        entry: &str,
        cx: &mut Context<Self>,
    ) -> Result<Key, SharedString> {
        if !self.editable() {
            return Err(self.why_not_editable().into());
        }
        let key = match key {
            Key::Server(server) => Key::Server(server.trim().to_owned()),
            Key::Name(name) => Key::Name(name.trim().to_owned()),
        };
        if !key.value().is_empty() && !self.workspace.clusters.iter().any(|e| e.id == entry) {
            return Err("Pick a workspace cluster".into());
        }
        let mut next = self.workspace.clone();
        let at = match editing {
            Some((at, was)) => {
                let row = next
                    .destinations
                    .get_mut(at)
                    .filter(|row| row.key() == *was)
                    .ok_or_else(|| SharedString::from("That mapping is no longer listed"))?;
                row.set_key(key.clone());
                row.entry = entry.to_owned();
                at
            }
            None => {
                next.destinations
                    .push(Destination::new(key.clone(), entry.to_owned()));
                next.destinations.len() - 1
            }
        };
        if let Some(fault) = next.destinations[at].fault_in(&next) {
            return Err(match fault {
                Fault::EmptyServer => "Enter the server URL the Application names".into(),
                Fault::EmptyName => "Enter Argo CD's name for the cluster".into(),
                Fault::NotHttp => "The server must be an http or https address".into(),
                fault => capitalized(&format!("the mapping {}", fault.words())).into(),
            });
        }
        if let Some(other) = next
            .destinations
            .iter()
            .enumerate()
            .find(|(ix, row)| *ix != at && row.key().same(&key))
        {
            let entry = &other.1.entry;
            let what = match &key {
                Key::Server(server) => format!("Server {server}"),
                Key::Name(name) => format!("Argo CD's cluster {name}"),
            };
            return Err(
                match self.workspace.clusters.iter().any(|e| e.id == *entry) {
                    true => format!("{what} is already mapped to {entry}"),
                    false => format!(
                        "{what} is already mapped to {entry}, which the workspace no longer \
                         lists: edit that mapping"
                    ),
                }
                .into(),
            );
        }
        next.check_saveable()
            .map_err(|why| SharedString::from(capitalized(&why.to_string())))?;
        let what = match editing {
            Some(_) => format!("Changed the mapping for {}.", key.describe()),
            None => format!("Mapped {} to {entry}.", key.describe()),
        };
        self.commit(next, what, cx);
        Ok(key)
    }

    /// Accepts a destination suggestion: maps each of `rows` that isn't
    /// mapped yet to the cluster `entry`, in one save. Checked against the
    /// workspace as it is now, not as the suggestion was made; the file is
    /// untouched, and the banner says why, when it can't be.
    pub(super) fn map_suggested(&mut self, entry: &str, rows: &[Key], cx: &mut Context<Self>) {
        let refuse = |this: &mut Self, why: String, cx: &mut Context<Self>| {
            this.notice = Some(Notice::Failed(capitalized(&why).into()));
            cx.notify();
        };
        if !self.editable() {
            let why = self.why_not_editable().to_owned();
            return refuse(self, why, cx);
        }
        if !self.workspace.clusters.iter().any(|e| e.id == entry) {
            return refuse(self, format!("{entry} is no longer in the workspace"), cx);
        }
        let mut next = self.workspace.clone();
        let mut described = Vec::new();
        for key in rows {
            if next.destinations.iter().any(|row| row.key().same(key)) {
                continue;
            }
            next.destinations
                .push(Destination::new(key.clone(), entry.to_owned()));
            described.push(key.describe());
        }
        if described.is_empty() {
            return refuse(self, "That destination is already mapped".to_owned(), cx);
        }
        let first_new = next.destinations.len() - described.len();
        let faulty = (first_new..next.destinations.len())
            .find_map(|at| next.destinations[at].fault_in(&next));
        if let Some(fault) = faulty {
            return refuse(self, format!("the mapping {}", fault.words()), cx);
        }
        if let Err(why) = next.check_saveable() {
            return refuse(self, why.to_string(), cx);
        }
        self.commit(
            next,
            format!("Mapped {} to {entry}.", described.join(" and ")),
            cx,
        );
    }

    /// Ignores a suggestion for the session. Nothing is written.
    pub(super) fn ignore_suggestion(&mut self, ignored: find::Ignored, cx: &mut Context<Self>) {
        if self.ignored.insert(ignored) {
            cx.notify();
        }
    }

    /// Removes the mapping at `at`, if it still matches `was`.
    pub(super) fn remove_destination(&mut self, at: usize, was: &Key, cx: &mut Context<Self>) {
        if !self.editable() {
            return;
        }
        let mut next = self.workspace.clone();
        if next
            .destinations
            .get(at)
            .is_none_or(|row| row.key() != *was)
        {
            return;
        }
        next.destinations.remove(at);
        self.commit(
            next,
            format!("Removed the mapping for {}.", was.describe()),
            cx,
        );
    }

    /// Accepts the build namespace suggestion: sets only
    /// `delivery.build_namespace`, keeping the rest of the setup, through the
    /// setup file's save. Refused as any save is; the banner says why.
    pub(super) fn set_build_namespace(&mut self, namespace: String, cx: &mut Context<Self>) {
        let refuse = |this: &mut Self, why: String, cx: &mut Context<Self>| {
            this.notice = Some(Notice::Failed(capitalized(&why).into()));
            cx.notify();
        };
        if !self.editable() {
            let why = self.why_not_editable().to_owned();
            return refuse(self, why, cx);
        }
        let valid = (1..=63).contains(&namespace.len())
            && namespace
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
            && !namespace.starts_with('-')
            && !namespace.ends_with('-');
        let Some(file) = self.setup_file.clone().filter(|_| valid) else {
            return refuse(self, "that isn't a namespace name".to_owned(), cx);
        };
        let what = format!("Set delivery.build_namespace to {namespace}.");
        let seen = self.seen.clone();
        let save = file.update(cx, |file, cx| {
            file.save_with(
                seen,
                move |setup| {
                    setup
                        .delivery
                        .get_or_insert_with(Default::default)
                        .build_namespace = Some(namespace);
                },
                cx,
            )
        });
        self.notice = None;
        self.saving = Some(cx.spawn(async move |this, cx| {
            let outcome = save.await;
            _ = this.update(cx, |this, cx| this.saved(what, outcome, cx));
        }));
        cx.notify();
    }

    /// Chooses the workspace's own kubeconfig, or `None` for automatic. It
    /// stands alone, so it saves at once.
    pub(super) fn set_kubeconfig(&mut self, path: Option<PathBuf>, cx: &mut Context<Self>) {
        if !self.editable() {
            return;
        }
        let mut next = self.workspace.clone();
        let what = match &path {
            Some(_) => "Chose the kubeconfig.",
            None => "The kubeconfig is automatic again.",
        };
        next.kubeconfig = path;
        if let Err(why) = next.check_saveable() {
            self.notice = Some(Notice::Failed(capitalized(&why.to_string()).into()));
            cx.notify();
            return;
        }
        self.commit(next, what.to_owned(), cx);
    }

    /// Moves the selected cluster one place; the first and last stay put.
    pub(super) fn move_selected(&mut self, delta: isize, cx: &mut Context<Self>) {
        if !self.editable() {
            return;
        }
        let Some(from) = self.selected.as_ref().and_then(|key| {
            self.workspace
                .clusters
                .iter()
                .position(|e| e.id == key.as_ref())
        }) else {
            return;
        };
        let Some(to) = from
            .checked_add_signed(delta)
            .filter(|to| *to < self.workspace.clusters.len())
        else {
            return;
        };
        let mut next = self.workspace.clone();
        next.clusters.swap(from, to);
        let id = next.clusters[to].id.clone();
        self.commit(next, format!("Moved {id}."), cx);
    }

    /// Writes `next` off the UI thread. The page shows it once it is on
    /// disk; if the write fails, the page and the file stay as they were.
    /// The file is read again just before the write and compared with what
    /// was last read or written: a hand edit, or a file another window made,
    /// is never replaced.
    pub(super) fn commit(&mut self, next: Workspace, what: String, cx: &mut Context<Self>) {
        if !self.editable() {
            return;
        }
        let Some(file) = self.setup_file.clone() else {
            return;
        };
        let before = file.read(cx).setup.clone();
        let secrets = self.metrics_secrets.clone();
        let preferences = self
            .file
            .as_ref()
            .map(|file| file.with_file_name("preferences.json"));
        let save = file.update(cx, |file, cx| {
            file.save_workspace(next, self.seen.clone(), cx)
        });
        self.notice = None;
        self.saving = Some(cx.spawn(async move |this, cx| {
            let outcome = save.await;
            let cleanup = if outcome.is_ok() {
                let after = file.read_with(cx, |file, _| file.setup.clone());
                cx.background_spawn(async move {
                    super::metrics_tokens::forget_replaced(secrets, before, after, preferences)
                })
                .await
            } else {
                Ok(())
            };
            _ = this.update(cx, |this, cx| {
                this.saved(what, outcome, cx);
                if let Err(why) = cleanup {
                    this.notice = Some(Notice::Failed(why.into()));
                    cx.notify();
                }
            });
        }));
        cx.notify();
    }

    fn saved(
        &mut self,
        what: String,
        outcome: Result<(), crate::setup_file::SaveError>,
        cx: &mut Context<Self>,
    ) {
        self.saving = None;
        let Some(file) = self.setup_file.clone() else {
            return;
        };
        match outcome {
            Ok(()) => {
                self.sync_setup(file.clone(), false, cx);
                if let Some(id) = self.select_on_save.take() {
                    self.select(id, cx);
                    if let Some(window) = self.window {
                        let view = cx.entity().downgrade();
                        _ = window.update(cx, |_, window, _| {
                            table::reveal_when_settled(
                                view,
                                |page| page.selected.is_some(),
                                window,
                            );
                        });
                    }
                }
                if std::mem::take(&mut self.deselect_on_save) {
                    self.selected = None;
                }
                if std::mem::take(&mut self.open_form_on_save) {
                    self.open_form_after_save(cx);
                }
                if let Some(receipt) = &file.read(cx).receipt {
                    self.notice = Some(Notice::Saved(format!("{what} {receipt}").into()));
                }
            }
            Err(error) => {
                self.select_on_save = None;
                self.deselect_on_save = false;
                self.open_form_on_save = false;
                self.pending_setup |= matches!(error, crate::setup_file::SaveError::Changed);
                self.sync_setup(file, self.pending_setup, cx);
                self.notice = Some(Notice::Failed(error.to_string().into()));
            }
        }
        cx.notify();
    }

    /// Reads immediately through the owner, dropping a draft only on Reload.
    pub(super) fn reload(&mut self, cx: &mut Context<Self>) {
        if self.origin == Origin::Example || self.saving.is_some() {
            return;
        }
        let Some(file) = self.setup_file.clone() else {
            return;
        };
        let read = file.update(cx, |file, cx| file.reload(cx));
        // Reload drops unsaved changes only to take a change made on disk,
        // as the banner says; otherwise they are rebased on the new file.
        let drops = self.pending_setup;
        self.saving = Some(cx.spawn(async move |this, cx| {
            read.await;
            _ = this.update(cx, |this, cx| {
                this.saving = None;
                this.notice = None;
                this.pending_setup = false;
                if drops {
                    this.drop_entry();
                }
                this.sync_setup(file, false, cx);
            });
        }));
        cx.notify();
    }
}

pub(super) fn capitalized(text: &str) -> String {
    let mut chars = text.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect(),
        None => String::new(),
    }
}
