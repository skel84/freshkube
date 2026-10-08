//! Changing the workspace: the actions on the selection, and the save they
//! end in. A change is a changed copy of the workspace, checked with
//! `Workspace::validate`, written whole off the UI thread, and shown once the
//! write has happened; a failed write leaves the page and the file as they
//! were. The first save after a refused file sets that file aside.
use super::*;
use chrono::Utc;
use freshkube_core::workspace::Entry;

impl SettingsPage {
    /// Whether a change can be saved now: not example data, not without a
    /// preferences folder, and not while a save runs.
    pub(super) fn editable(&self) -> bool {
        self.file.is_some() && self.origin != Origin::Example && self.saving.is_none()
    }

    /// Why the changing actions are off, for their tooltips.
    pub(super) fn why_not_editable(&self) -> &'static str {
        if self.origin == Origin::Example {
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
        cx: &mut Context<Self>,
    ) -> Result<(), SharedString> {
        if !self.editable() {
            return Err(self.why_not_editable().into());
        }
        let context = context.trim();
        let talosconfig = talosconfig.trim();
        if context.is_empty() {
            return Err("Enter the kubeconfig context this cluster opens".into());
        }
        let mut next = self.workspace.clone();
        let talosconfig = (!talosconfig.is_empty()).then(|| PathBuf::from(talosconfig));
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
                format!("Changed {id}.")
            }
            None => {
                let id = next.fresh_id(context);
                let mut entry = Entry::new(id.clone(), role, context);
                entry.talosconfig = talosconfig;
                next.clusters.push(entry);
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
                        .child(
                            "This takes the cluster out of the workspace file. Nothing on the \
                             cluster, and no kubeconfig or talosconfig, is touched.",
                        )
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
        let Some(file) = self.file.clone() else {
            return;
        };
        let refused = matches!(self.origin, Origin::Refused(_));
        let expected = self.seen.clone();
        self.notice = None;
        let shown = next.clone();
        self.saving = Some(cx.spawn(async move |this, cx| {
            let outcome = cx
                .background_spawn(async move {
                    workspace::commit(&file, &expected, refused, &next, Utc::now())
                })
                .await;
            _ = this.update(cx, |this, cx| this.saved(shown, what, outcome, cx));
        }));
        cx.notify();
    }

    fn saved(
        &mut self,
        saved: Workspace,
        what: String,
        outcome: Result<(Option<PathBuf>, workspace::Seen), workspace::SaveError>,
        cx: &mut Context<Self>,
    ) {
        use workspace::SaveError;
        self.saving = None;
        match outcome {
            Ok((aside, seen)) => {
                self.set_workspace(&Loaded::Workspace(saved), false, cx);
                self.seen = seen;
                self.notice = Some(Notice::Saved(
                    match aside {
                        Some(path) => format!(
                            "{what} The earlier file was set aside as {}.",
                            path.display()
                        ),
                        None => what,
                    }
                    .into(),
                ));
            }
            Err(SaveError::Changed) => {
                self.notice = Some(Notice::Failed(
                    "workspace.json changed on disk since it was read. Nothing was written; \
                     reload to see the file as it is."
                        .into(),
                ));
            }
            Err(SaveError::Invalid(why)) => {
                self.notice = Some(Notice::Failed(capitalized(&why.to_string()).into()));
            }
            Err(SaveError::Aside(error)) => {
                self.notice = Some(Notice::Failed(
                    format!(
                        "Couldn’t set the unused workspace.json aside: {error}. Nothing was \
                         written."
                    )
                    .into(),
                ));
            }
            Err(SaveError::Write { aside, error }) => {
                // An unused file moved before the write failed: the page now
                // has no file to protect.
                let moved = aside
                    .map(|path| format!(" The earlier file was set aside as {}.", path.display()));
                if moved.is_some() {
                    self.set_workspace(&Loaded::Missing, false, cx);
                    self.seen = workspace::Seen::Missing;
                }
                let rest = moved.unwrap_or_else(|| " The file is as it was.".into());
                self.notice = Some(Notice::Failed(format!("{error}.{rest}").into()));
            }
        }
        cx.notify();
    }

    /// Reads the file again, as the person asked: after a change made by
    /// hand, or by another window. The read is the bounded one the launch
    /// makes, on the UI thread, and happens only on this action.
    pub(super) fn reload(&mut self, cx: &mut Context<Self>) {
        let Some(file) = self.file.clone() else {
            return;
        };
        if self.origin == Origin::Example || self.saving.is_some() {
            return;
        }
        let (loaded, seen) = workspace::load_seen(&file);
        self.set_workspace(&loaded, false, cx);
        self.seen = seen;
        self.notice = None;
        cx.notify();
    }
}

fn capitalized(text: &str) -> String {
    let mut chars = text.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect(),
        None => String::new(),
    }
}
