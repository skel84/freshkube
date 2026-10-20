//! The selected cluster, in the Inspector beside the table: one form group
//! per concern, Identity and Metrics, each with its own Save and Revert
//! (DESIGN.md, The Settings page). A group's fields are drafts the page
//! keeps until Save writes them through `commit`, which changes only that
//! entry's keys and keeps the rest of the setup.
mod identity;
mod metrics;
#[cfg(test)]
mod tests;

use super::*;
use freshkube_core::workspace::Entry;
use freshkube_ui::form;
use freshkube_ui::inspector::Inspector;
use freshkube_ui::tooltip::FollowTooltip as _;
use gpui_kit::component::input::{InputEvent, InputState};

/// Where every group of an entry is saved.
fn saved_in() -> form::SavedIn {
    form::SavedIn::file("freshkube.yaml")
}

/// A form group of the Inspector.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Group {
    Identity,
    Metrics,
}

/// A field of the Inspector's groups.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Field {
    Context,
    Talosconfig,
    TalosContext,
    Label,
    Namespace,
    Service,
    Port,
    Address,
}

impl Field {
    const ALL: [Field; 8] = [
        Field::Context,
        Field::Talosconfig,
        Field::TalosContext,
        Field::Label,
        Field::Namespace,
        Field::Service,
        Field::Port,
        Field::Address,
    ];

    fn group(self) -> Group {
        match self {
            Field::Context | Field::Talosconfig | Field::TalosContext => Group::Identity,
            _ => Group::Metrics,
        }
    }
}

/// The drafts of the selected entry's groups.
pub(super) struct EntryDraft {
    id: SharedString,
    /// The page's revision the baselines were taken from.
    synced: u64,
    identity: identity::Identity,
    metrics: metrics::Metrics,
    /// Why the setup refused a save that no field explains.
    error: Option<SharedString>,
    _subscriptions: Vec<Subscription>,
}

impl EntryDraft {
    fn new(
        entry: &Entry,
        revision: u64,
        window: &mut Window,
        cx: &mut Context<SettingsPage>,
    ) -> Self {
        let mut draft = Self {
            id: entry.id.clone().into(),
            synced: revision,
            identity: identity::Identity::new(entry, window, cx),
            metrics: metrics::Metrics::new(entry, window, cx),
            error: None,
            _subscriptions: Vec::new(),
        };
        for field in Field::ALL {
            let Some(input) = draft.input(field).cloned() else {
                continue;
            };
            draft.subscribe(field, &input, window, cx);
        }
        draft
    }

    fn subscribe(
        &mut self,
        field: Field,
        input: &Entity<InputState>,
        window: &mut Window,
        cx: &mut Context<SettingsPage>,
    ) {
        self._subscriptions.push(cx.subscribe_in(
            input,
            window,
            move |page, _, event: &InputEvent, window, cx| {
                page.entry_event(field, event, window, cx)
            },
        ));
    }

    fn input(&self, field: Field) -> Option<&Entity<InputState>> {
        self.identity
            .input(field)
            .or_else(|| self.metrics.input(field))
    }

    fn dirty(&self, group: Group, cx: &App) -> bool {
        match group {
            Group::Identity => self.identity.dirty(cx),
            Group::Metrics => self.metrics.dirty(cx),
        }
    }

    /// Takes what the file now holds for the entry as the baseline. A group
    /// with nothing typed shows it; one being edited keeps its edits, and
    /// stays dirty only if they still differ.
    fn refresh(&mut self, entry: &Entry, revision: u64, window: &mut Window, cx: &mut App) {
        self.synced = revision;
        let identity = identity::Values::of(entry);
        if identity != self.identity.baseline {
            let clean = !self.identity.dirty(cx);
            self.identity.baseline = identity.clone();
            if clean {
                self.identity.load(&identity, window, cx);
            }
        }
        let metrics = metrics::Values::of(entry);
        if metrics != self.metrics.baseline {
            let clean = !self.metrics.dirty(cx);
            self.metrics.baseline = metrics.clone();
            if clean {
                self.metrics.load(&metrics, window, cx);
            }
        }
        self.metrics.has_token = entry
            .metrics
            .as_deref()
            .and_then(|metrics| metrics.source.as_ref())
            .is_some_and(|source| source.token.is_some());
    }

    /// The group holding the keyboard, if any.
    fn focused(&self, window: &Window, cx: &App) -> Option<Group> {
        Field::ALL.into_iter().find_map(|field| {
            let input = self.input(field)?;
            input
                .read(cx)
                .focus_handle(cx)
                .contains_focused(window, cx)
                .then(|| field.group())
        })
    }
}

impl SettingsPage {
    /// Builds the drafts for the selected entry, or refreshes them when the
    /// file changed; nothing for no selection. Called as the page draws.
    pub(super) fn sync_entry(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let entry = self
            .selected
            .as_ref()
            .and_then(|key| {
                self.workspace
                    .clusters
                    .iter()
                    .find(|entry| entry.id.as_str() == key.as_ref())
            })
            .cloned();
        let Some(entry) = entry else {
            self.entry = None;
            return;
        };
        let revision = self.revision;
        match &mut self.entry {
            Some(draft) if draft.id.as_ref() == entry.id.as_str() => {
                if draft.synced != revision {
                    draft.refresh(&entry, revision, window, cx);
                }
            }
            _ => self.entry = Some(EntryDraft::new(&entry, revision, window, cx)),
        }
    }

    /// Whether the selected entry has a group with unsaved changes.
    pub(in crate::desktop) fn has_unsaved_edits(&self, cx: &App) -> bool {
        !self.dirty_groups(cx).is_empty()
    }

    fn dirty_groups(&self, cx: &App) -> Vec<Group> {
        let Some(draft) = &self.entry else {
            return Vec::new();
        };
        [Group::Identity, Group::Metrics]
            .into_iter()
            .filter(|group| draft.dirty(*group, cx))
            .collect()
    }

    /// Moves the selection to `target`. With unsaved changes in a group it
    /// first asks, through the platform's prompt, whether to Save them,
    /// Discard them or Stay.
    pub(super) fn request_select(
        &mut self,
        target: Option<SharedString>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if target == self.selected {
            return;
        }
        if !self.has_unsaved_edits(cx) {
            self.apply_selection(target, cx);
            return;
        }
        let name = self
            .entry
            .as_ref()
            .map(|draft| draft.id.clone())
            .unwrap_or_default();
        let answer = window.prompt(
            PromptLevel::Warning,
            &format!("Save the changes to {name}?"),
            Some("Discard leaves them unsaved; Stay keeps editing."),
            &["Save", "Discard", "Stay"],
            cx,
        );
        cx.spawn_in(window, async move |this, cx| {
            let Ok(choice) = answer.await else {
                return;
            };
            _ = this.update_in(cx, |page, window, cx| match choice {
                0 => {
                    let groups = page.dirty_groups(cx);
                    page.save_groups(groups, Some(target), window, cx);
                }
                1 => page.apply_selection(target, cx),
                _ => {}
            });
        })
        .detach();
    }

    fn apply_selection(&mut self, target: Option<SharedString>, cx: &mut Context<Self>) {
        match target {
            Some(key) => self.select(key, cx),
            None => {
                self.selected = None;
                cx.notify();
            }
        }
    }

    /// The error shown at `field`, if any.
    #[cfg(test)]
    pub(super) fn field_error(&self, field: Field) -> Option<SharedString> {
        let draft = self.entry.as_ref()?;
        draft
            .identity
            .errors
            .of(field)
            .or_else(|| draft.metrics.errors.of(field))
    }

    /// Edit: the keyboard goes to the Inspector's first field.
    pub(super) fn edit_selected(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.sync_entry(window, cx);
        if let Some(input) = self
            .entry
            .as_ref()
            .and_then(|draft| draft.input(Field::Context))
        {
            window.focus(&input.read(cx).focus_handle(cx), cx);
        }
    }

    pub(super) fn set_entry_role(&mut self, role: workspace::Role, cx: &mut Context<Self>) {
        if let Some(draft) = self.entry.as_mut() {
            draft.identity.role = role;
            draft.error = None;
            cx.notify();
        }
    }

    pub(super) fn set_entry_store(&mut self, kind: metrics::Kind, cx: &mut Context<Self>) {
        if let Some(draft) = self.entry.as_mut() {
            draft.metrics.kind = kind;
            draft.metrics.errors = metrics::Errors::default();
            draft.error = None;
            cx.notify();
        }
    }

    fn entry_event(
        &mut self,
        field: Field,
        event: &InputEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match event {
            InputEvent::PressEnter { .. } => self.save_group(field.group(), window, cx),
            InputEvent::Change => {
                if let Some(draft) = self.entry.as_mut() {
                    draft.error = None;
                    draft.identity.errors.set(field, None);
                    draft.metrics.errors.set(field, None);
                    cx.notify();
                }
            }
            InputEvent::Blur => {
                if let Some(draft) = self.entry.as_mut() {
                    // The field alone: the others have their own turn.
                    match field.group() {
                        Group::Identity => {
                            let errors = draft.identity.values(cx).check();
                            draft.identity.errors.set(field, errors.of(field));
                        }
                        Group::Metrics => {
                            let errors = draft.metrics.values(cx).check();
                            draft.metrics.errors.set(field, errors.of(field));
                        }
                    }
                    cx.notify();
                }
            }
            _ => {}
        }
    }

    pub(super) fn save_group(&mut self, group: Group, window: &mut Window, cx: &mut Context<Self>) {
        if self
            .entry
            .as_ref()
            .is_some_and(|draft| draft.dirty(group, cx))
        {
            self.save_groups(vec![group], None, window, cx);
        }
    }

    pub(super) fn revert_group(
        &mut self,
        group: Group,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(draft) = self.entry.as_mut() else {
            return;
        };
        draft.error = None;
        match group {
            Group::Identity => {
                let baseline = draft.identity.baseline.clone();
                draft.identity.load(&baseline, window, cx);
            }
            Group::Metrics => {
                let baseline = draft.metrics.baseline.clone();
                draft.metrics.load(&baseline, window, cx);
            }
        }
        cx.notify();
    }

    /// Checks `groups`, and if every field passes, writes them in one save
    /// of this entry's keys. `leave` says where the selection goes once the
    /// save has landed. A failing field shows its error and takes focus;
    /// nothing is written then.
    pub(super) fn save_groups(
        &mut self,
        groups: Vec<Group>,
        leave: Option<Option<SharedString>>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        if !self.editable() {
            return false;
        }
        let Some(draft) = self.entry.as_mut() else {
            return false;
        };
        let mut failing = None;
        let mut identity = None;
        let mut metrics = None;
        for group in groups {
            match group {
                Group::Identity => {
                    let values = draft.identity.values(cx);
                    let errors = values.check();
                    failing = failing.or(errors.first());
                    draft.identity.errors = errors;
                    identity = Some(values);
                }
                Group::Metrics => {
                    let values = draft.metrics.values(cx);
                    let errors = values.check();
                    failing = failing.or(errors.first());
                    draft.metrics.errors = errors;
                    metrics = Some(values);
                }
            }
        }
        if let Some(field) = failing {
            if let Some(input) = draft.input(field) {
                window.focus(&input.read(cx).focus_handle(cx), cx);
            }
            cx.notify();
            return false;
        }
        let id = draft.id.clone();
        let mut next = self.workspace.clone();
        let Some(entry) = next
            .clusters
            .iter_mut()
            .find(|entry| entry.id.as_str() == id.as_ref())
        else {
            return false;
        };
        if let Some(values) = &identity {
            values.apply(entry);
        }
        if let Some(values) = &metrics {
            values.apply(entry);
        }
        let saveable = next
            .check_saveable()
            .map_err(|why| SharedString::from(edit::capitalized(&why.to_string())));
        if let Some(draft) = self.entry.as_mut() {
            draft.error = saveable.as_ref().err().cloned();
        }
        if saveable.is_err() {
            cx.notify();
            return false;
        }
        match leave {
            Some(Some(key)) => self.select_on_save = Some(key),
            Some(None) => self.deselect_on_save = true,
            None => {}
        }
        self.commit(next, format!("Changed {id}."), cx);
        true
    }

    /// Browse… for the talosconfig: the chosen file fills the field, and
    /// Save writes it with the group.
    pub(super) fn browse_talosconfig(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.editable() {
            return;
        }
        let selection = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            prompt: Some("Use Talosconfig".into()),
        });
        cx.spawn_in(window, async move |this, cx| {
            let Ok(Ok(Some(paths))) = selection.await else {
                return;
            };
            let Some(path) = paths.into_iter().next() else {
                return;
            };
            _ = this.update_in(cx, |page, window, cx| {
                let Some(draft) = page.entry.as_mut() else {
                    return;
                };
                draft.identity.errors.talosconfig = None;
                draft.identity.talosconfig.update(cx, |input, cx| {
                    input.set_value(path.display().to_string(), window, cx)
                });
                cx.notify();
            });
        })
        .detach();
    }

    /// Escape in a group reverts it once; with nothing to revert it hands
    /// the keyboard back to the table.
    fn leave_inspector(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(draft) = &self.entry else {
            return;
        };
        match draft.focused(window, cx) {
            Some(group) if draft.dirty(group, cx) => self.revert_group(group, window, cx),
            _ => window.focus(&self.focus, cx),
        }
    }

    /// The Inspector for the selected cluster, if one is.
    pub(super) fn render_entry(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let draft = self.entry.as_ref()?;
        let p = crate::palette::palette(cx);
        let editable = self.editable();
        let id = draft.id.clone();
        let title = div()
            .id("settings-detail-title")
            .test_support()
            .aria_label(id.clone())
            .min_w_0()
            .text_size(dp(14.))
            .font_weight(FontWeight::SEMIBOLD)
            .truncate()
            .follow_tooltip(id.clone())
            .child(id);
        let why = (!editable && self.saving.is_none()).then(|| self.why_not_editable());
        let groups = v_flex()
            .id("settings-detail-groups")
            .test_support()
            .gap(dp(20.))
            .on_key_down(cx.listener(|page, event: &KeyDownEvent, window, cx| {
                if event.keystroke.key == "escape" {
                    page.leave_inspector(window, cx);
                    cx.stop_propagation();
                }
            }))
            .children(why.map(|why| {
                div()
                    .id("settings-detail-note")
                    .test_support()
                    .text_size(dp(12.))
                    .text_color(p.muted)
                    .child(why)
            }))
            .children(draft.error.clone().map(|why| {
                div()
                    .id("settings-detail-error")
                    .test_support()
                    .role(Role::Alert)
                    .aria_label(why.clone())
                    .child(ui::warning_banner(None, why, None, cx))
            }))
            .child(draft.identity.render(editable, cx))
            .child(draft.metrics.render(editable, cx));
        Some(
            Inspector::new("settings-detail")
                .heading(title)
                .child(groups)
                .render(cx)
                .into_any_element(),
        )
    }
}
