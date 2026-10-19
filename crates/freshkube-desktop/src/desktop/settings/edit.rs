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

    pub(super) fn open_form(
        &mut self,
        editing: Option<Entry>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.editable() || window.has_active_dialog(cx) {
            return;
        }
        form::open(cx.entity(), editing, window, cx);
    }

    pub(super) fn edit_selected(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(entry) = self.selected_entry().cloned() {
            self.open_form(Some(entry), window, cx);
        }
    }

    /// Applies the form: adds a cluster, or changes the one being edited.
    /// Returns why not, for the form to show; the file is untouched then.
    pub(super) fn upsert(
        &mut self,
        editing: Option<&str>,
        role: workspace::Role,
        context: &str,
        talosconfig: &str,
        talos_context: &str,
        cx: &mut Context<Self>,
    ) -> Result<(), SharedString> {
        if !self.editable() {
            return Err(self.why_not_editable().into());
        }
        let context = context.trim();
        let talosconfig = talosconfig.trim();
        let talos_context = Some(talos_context.trim().to_owned()).filter(|name| !name.is_empty());
        if talos_context.is_some() && talosconfig.is_empty() {
            return Err("A Talos context needs a talosconfig".into());
        }
        if context.is_empty() {
            return Err("Enter the kubeconfig context this cluster opens".into());
        }
        let mut next = self.workspace.clone();
        let talosconfig = (!talosconfig.is_empty()).then(|| PathBuf::from(talosconfig));
        if talosconfig.as_ref().is_some_and(|path| !path.is_absolute()) {
            return Err("Talosconfig must be an absolute path".into());
        }
        let what = match editing {
            Some(id) => {
                let entry = next
                    .clusters
                    .iter_mut()
                    .find(|entry| entry.id == id)
                    .ok_or_else(|| SharedString::from("That cluster is no longer listed"))?;
                entry.role = role;
                entry.context = context.to_owned();
                entry.talosconfig = talosconfig;
                entry.talos_context = talos_context;
                format!("Changed {id}.")
            }
            None => {
                let id = next.fresh_id(context);
                let mut entry = Entry::new(id.clone(), role, context);
                entry.talosconfig = talosconfig;
                entry.talos_context = talos_context;
                next.clusters.push(entry);
                // The new row is the selected one once it is saved.
                self.select_on_save = Some(id.clone().into());
                format!("Added {id}.")
            }
        };
        next.check_saveable()
            .map_err(|why| SharedString::from(capitalized(&why.to_string())))?;
        self.commit(next, what, cx);
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
            dialog
                .w(ui::dp_px(420., window))
                .title(format!("Remove {id}?"))
                .child(
                    v_flex()
                        .id("settings-remove-dialog")
                        .test_support()
                        .role(Role::Dialog)
                        .aria_label(format!("Remove {id}"))
                        .gap_3()
                        .text_size(dp(13.))
                        .child(body.clone())
                        .child(
                            h_flex()
                                .gap_2()
                                .justify_end()
                                .child(
                                    Button::new("settings-remove-cancel")
                                        .outline()
                                        .label("Cancel")
                                        .on_click(|_, window, cx| window.close_dialog(cx)),
                                )
                                .child(
                                    Button::new("settings-remove-confirm")
                                        .danger()
                                        .label("Remove")
                                        .on_click(move |_, window, cx| {
                                            window.close_dialog(cx);
                                            _ = confirm_page.update(cx, |page, cx| {
                                                page.remove(&confirm_id, cx)
                                            });
                                        }),
                                ),
                        ),
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
        if let Some(fault) = next.destinations[at].fault() {
            return Err(match fault {
                Fault::EmptyServer => "Enter the server URL the Application names".into(),
                Fault::EmptyName => "Enter Argo CD's name for the cluster".into(),
                Fault::NotHttp => "The server must be an http or https address".into(),
                Fault::ArgoCdOwn => {
                    "Argo CD knows its own cluster already; it needs no mapping".into()
                }
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
        let save = file.update(cx, |file, cx| {
            file.save_workspace(next, self.seen.clone(), cx)
        });
        self.notice = None;
        self.saving = Some(cx.spawn(async move |this, cx| {
            let outcome = save.await;
            _ = this.update(cx, |this, cx| this.saved(what, outcome, cx));
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
                }
                if let Some(receipt) = &file.read(cx).receipt {
                    self.notice = Some(Notice::Saved(format!("{what} {receipt}").into()));
                }
            }
            Err(error) => {
                self.select_on_save = None;
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
        self.saving = Some(cx.spawn(async move |this, cx| {
            read.await;
            _ = this.update(cx, |this, cx| {
                this.saving = None;
                this.notice = None;
                this.pending_setup = false;
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
