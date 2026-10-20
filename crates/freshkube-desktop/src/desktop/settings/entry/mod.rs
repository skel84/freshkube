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
    /// Whether each group differs from the file, derived when a field, the
    /// choice of a group or the baseline changes, so a frame only reads it.
    dirty: [bool; 2],
    _subscriptions: Vec<Subscription>,
}

/// What a change of selection, or the Add dialog, waits for: the selected
/// cluster's unsaved changes saved, discarded or kept.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum Next {
    Select(Option<SharedString>),
    Add,
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
            dirty: [false; 2],
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

    fn dirty(&self, group: Group) -> bool {
        self.dirty[group as usize]
    }

    /// Derives which groups differ from the file.
    fn refresh_dirty(&mut self, cx: &App) {
        self.dirty = [self.identity.dirty(cx), self.metrics.dirty(cx)];
    }

    /// Takes what the file now holds for the entry as the baseline. A group
    /// with nothing typed shows it; one being edited keeps its edits, and
    /// stays dirty only if they still differ.
    fn refresh(&mut self, entry: &Entry, revision: u64, window: &mut Window, cx: &mut App) {
        self.synced = revision;
        // A group with nothing typed shows the file; so does one whose text
        // is what a save just wrote, trimmed as the file holds it.
        let identity = identity::Values::of(entry);
        if identity != self.identity.baseline {
            let was_clean = !self.identity.dirty(cx);
            self.identity.baseline = identity.clone();
            if was_clean || !self.identity.dirty(cx) {
                self.identity.load(&identity, window, cx);
            }
        }
        let metrics = metrics::Values::of(entry);
        if metrics != self.metrics.baseline {
            let was_clean = !self.metrics.dirty(cx);
            self.metrics.baseline = metrics.clone();
            if was_clean || !self.metrics.dirty(cx) {
                self.metrics.load(&metrics, window, cx);
            }
        }
        self.refresh_dirty(cx);
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
    /// Whether the drafts are those of the selection as the workspace
    /// stands.
    fn entry_is_current(&self) -> bool {
        self.entry_key.as_ref() == Some(&(self.selected.clone(), self.revision))
    }

    /// Asks for the drafts to be built after this frame, when the selection
    /// or the workspace has changed since they were. Building them makes
    /// inputs, which needs the window, so it waits for the frame to end.
    pub(super) fn sync_entry_after_frame(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.window = Some(window.window_handle());
        if !self.entry_is_current() {
            cx.defer_in(window, |page, window, cx| page.sync_entry(window, cx));
        }
    }

    /// Builds the drafts for the selected entry, or rebases them on what
    /// the file now holds; nothing for no selection.
    pub(super) fn sync_entry(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.entry_is_current() {
            return;
        }
        self.entry_key = Some((self.selected.clone(), self.revision));
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
            self.detail_scroll.set_offset(point(px(0.), px(0.)));
            cx.notify();
            return;
        };
        let revision = self.revision;
        match &mut self.entry {
            Some(draft) if draft.id.as_ref() == entry.id.as_str() => {
                draft.refresh(&entry, revision, window, cx);
            }
            _ => {
                // Another cluster opens at the top, not where the last one
                // was scrolled to.
                self.detail_scroll.set_offset(point(px(0.), px(0.)));
                self.entry = Some(EntryDraft::new(&entry, revision, window, cx));
            }
        }
        cx.notify();
    }

    /// Drops the drafts, to be built again from the file.
    pub(super) fn drop_entry(&mut self) {
        self.entry = None;
        self.entry_key = None;
    }

    pub(super) fn save_entry_unsaved(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.save_groups(self.dirty_groups(), None, window, cx);
    }

    pub(super) fn discard_entry_unsaved(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        for group in self.dirty_groups() {
            self.revert_group(group, window, cx);
        }
    }

    /// Whether the selected entry has a group with unsaved changes.
    pub(in crate::desktop) fn has_unsaved_edits(&self, _: &App) -> bool {
        !self.dirty_groups().is_empty()
    }

    fn dirty_groups(&self) -> Vec<Group> {
        let Some(draft) = &self.entry else {
            return Vec::new();
        };
        [Group::Identity, Group::Metrics]
            .into_iter()
            .filter(|group| draft.dirty(*group))
            .collect()
    }

    /// Settles the selected cluster's unsaved changes, then does `next`:
    /// moves the selection, or opens the Add dialog. With changes in a
    /// group it first asks, through the platform's prompt, whether to Save
    /// them, Discard them or Stay.
    pub(super) fn settle_edits(&mut self, next: Next, window: &mut Window, cx: &mut Context<Self>) {
        if let Next::Select(target) = &next
            && *target == self.selected
        {
            return;
        }
        if let Some(draft) = self.entry.as_mut() {
            draft.refresh_dirty(cx);
        }
        if self.dirty_groups().is_empty() {
            self.proceed(next, window, cx);
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
                    let groups = page.dirty_groups();
                    page.save_groups(groups, Some(next), window, cx);
                }
                1 => {
                    for group in page.dirty_groups() {
                        page.revert_group(group, window, cx);
                    }
                    page.proceed(next, window, cx);
                }
                _ => {}
            });
        })
        .detach();
    }

    /// Moves the selection to `target`, with the same prompt for changes.
    pub(super) fn request_select(
        &mut self,
        target: Option<SharedString>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.menu_holds_inspector = false;
        self.menu_had_focus = false;
        self.settle_edits(Next::Select(target), window, cx);
    }

    /// Add: its dialog, once the selected cluster's changes are settled.
    pub(super) fn request_add(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.editable() && !window.has_active_dialog(cx) {
            self.settle_edits(Next::Add, window, cx);
        }
    }

    fn proceed(&mut self, next: Next, window: &mut Window, cx: &mut Context<Self>) {
        match next {
            Next::Select(Some(key)) => {
                self.select(key, cx);
                self.sync_entry(window, cx);
            }
            Next::Select(None) => {
                self.selected = None;
                self.sync_entry(window, cx);
            }
            Next::Add => self.open_form(window, cx),
        }
        cx.notify();
    }

    /// Whether a change of selection is waiting on the prompt: the row the
    /// pointer is on isn't the selected one yet.
    pub(super) fn selection_pending(&self, key: &str) -> bool {
        self.selected.as_deref() != Some(key)
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
        self.menu_holds_inspector = false;
        self.menu_had_focus = false;
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
        // Locked fields take no choices either.
        if !self.editable() {
            return;
        }
        if let Some(draft) = self.entry.as_mut() {
            draft.identity.role = role;
            draft.error = None;
            draft.refresh_dirty(cx);
            cx.notify();
        }
    }

    pub(super) fn set_entry_store(&mut self, kind: metrics::Kind, cx: &mut Context<Self>) {
        if !self.editable() {
            return;
        }
        if let Some(draft) = self.entry.as_mut() {
            draft.metrics.kind = kind;
            draft.metrics.errors = metrics::Errors::default();
            draft.error = None;
            draft.refresh_dirty(cx);
            cx.notify();
        }
    }

    /// A field's text changed other than by typing, as a file chosen with
    /// Browse… does: the marks of the groups follow it.
    pub(super) fn entry_edited(&mut self, cx: &mut Context<Self>) {
        if let Some(draft) = self.entry.as_mut() {
            draft.refresh_dirty(cx);
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
                    draft.refresh_dirty(cx);
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
        // What was typed in this very update has not been heard of yet.
        if let Some(draft) = self.entry.as_mut() {
            draft.refresh_dirty(cx);
        }
        if self.entry.as_ref().is_some_and(|draft| draft.dirty(group)) {
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
        draft.refresh_dirty(cx);
        cx.notify();
    }

    /// Checks `groups`, and if every field passes, writes them in one save
    /// of this entry's keys. `next` is what follows once the save has
    /// landed. A failing field shows its error and takes focus; nothing is
    /// written then, and nothing follows. A page that can't save says why.
    pub(super) fn save_groups(
        &mut self,
        groups: Vec<Group>,
        next: Option<Next>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        if !self.editable() {
            self.notice = Some(Notice::Failed(self.why_not_editable().into()));
            cx.notify();
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
        let mut next_workspace = self.workspace.clone();
        let Some(entry) = next_workspace
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
        let saveable = next_workspace
            .check_saveable()
            .map_err(|why| SharedString::from(edit::capitalized(&why.to_string())));
        if let Some(draft) = self.entry.as_mut() {
            draft.error = saveable.as_ref().err().cloned();
        }
        if saveable.is_err() {
            cx.notify();
            return false;
        }
        match next {
            Some(Next::Select(Some(key))) => self.select_on_save = Some(key),
            Some(Next::Select(None)) => self.deselect_on_save = true,
            Some(Next::Add) => self.open_form_on_save = true,
            None => {}
        }
        self.commit(next_workspace, format!("Changed {id}."), cx);
        true
    }

    /// Opens the Add dialog once a save has landed, in the window the page
    /// was drawn in.
    pub(super) fn open_form_after_save(&mut self, cx: &mut Context<Self>) {
        let Some(handle) = self.window else {
            return;
        };
        let page = cx.entity();
        cx.defer(move |cx| {
            _ = handle.update(cx, |_, window, cx| {
                page.update(cx, |page, cx| page.open_form(window, cx))
            });
        });
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
                page.entry_edited(cx);
            });
        })
        .detach();
    }

    /// Escape in a group reverts it once. With nothing to revert it isn't
    /// handled here, and goes on to hand the keyboard to the column.
    fn leave_inspector(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        let Some(draft) = &self.entry else {
            return false;
        };
        match draft.focused(window, cx) {
            Some(group) if draft.dirty(group) => {
                self.revert_group(group, window, cx);
                true
            }
            _ => false,
        }
    }

    /// The Inspector for the selected cluster, if its drafts are built.
    pub(super) fn render_entry(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let draft = self.entry.as_ref()?;
        if self.selected.as_deref() != Some(draft.id.as_ref()) {
            return None;
        }
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
            .pb(dp(page::PANE_PADDING))
            .on_key_down(cx.listener(|page, event: &KeyDownEvent, window, cx| {
                if event.keystroke.key == "escape" && page.leave_inspector(window, cx) {
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
            .child(
                draft
                    .identity
                    .render(editable, draft.dirty(Group::Identity), cx),
            )
            .child(
                draft
                    .metrics
                    .render(editable, draft.dirty(Group::Metrics), cx),
            );
        // The body scrolls inside a padded frame, not padded itself, so a
        // scrolled line is cut at the gap under the heading instead of
        // showing in it (the shared body pads inside its scroll).
        let body = div()
            .flex_1()
            .min_h_0()
            .min_w_0()
            .px(dp(page::PANE_PADDING))
            .pt(dp(page::PANE_PADDING))
            .child(
                div()
                    .id("settings-detail-body")
                    .test_support()
                    .size_full()
                    .overflow_y_scroll()
                    .restrict_scroll_to_axis()
                    .track_scroll(&self.detail_scroll)
                    .child(groups),
            );
        Some(
            Inspector::new("settings-detail")
                .heading(title)
                .content(body)
                .render(cx)
                .into_any_element(),
        )
    }
}
