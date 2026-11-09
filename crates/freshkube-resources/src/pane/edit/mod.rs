//! Edit YAML in the detail pane (#858). The YAML tab gets an Edit button; it
//! reads the object once through core (`EditSession`, which fixes the UID and
//! resourceVersion the edit is based on) and swaps the YAML view for Kit's code
//! editor. The cluster's own fields and a Secret's values are not in the text.
//! Changed lines are marked as the text changes. Review (⌘S) asks core to
//! describe the text; the pane only emits that description, and the shell
//! hands it to the write flow, where the level, the dry-run diff and the send
//! belong. Nothing here builds a request.

mod marks;

use std::sync::Arc;

use freshkube_core::access::Offer;
use freshkube_core::write::AccessQuestion;
use freshkube_core::write::edit_yaml::EditSession;
use freshkube_core::write::everyday::Row;
use gpui_kit::component::{
    ActiveTheme, Disableable, Sizable,
    button::{Button, ButtonVariants},
    h_flex,
    input::{Editor, EditorState, InputEvent, TextDecoration, TextDecorationCollection},
    v_flex,
};
use gpui_kit::prelude::*;
use gpui_kit::*;

use super::{DetailEvent, DetailPane, Job, READ_DEADLINE};
use crate::backend;
use crate::detail::DocumentRead;
use crate::model::ResourceIdentity;
use crate::palette::palette;
use crate::screen::ClusterAccess;
use crate::ui::dp;
use crate::{example, live};
use freshkube_core::pluralize;
use freshkube_ui::platform::primary_modifier;

actions!(kube_detail_edit, [ReviewEdit]);

pub(super) fn bind(cx: &mut App, context: &'static str) {
    cx.bind_keys([KeyBinding::new("secondary-s", ReviewEdit, Some(context))]);
}

/// One edit of the open object, from the press on Edit until Cancel or until
/// the pane shows another object.
pub(super) struct Edit {
    identity: ResourceIdentity,
    phase: Phase,
}

enum Phase {
    /// The object is being read; the YAML view stays.
    Reading {
        _job: Job,
    },
    /// The read failed; the YAML view stays and says why.
    Failed(SharedString),
    Editing(Editing),
}

struct Editing {
    session: Arc<EditSession>,
    state: Entity<EditorState>,
    marks: TextDecorationCollection,
    /// The text the editor started from.
    original: String,
    /// The locked fields, as the list under the toolbar opens to show them.
    locked: String,
    /// How many there are, for the toolbar's disclosure.
    locked_count: usize,
    /// A Secret's `data` and `stringData`, as the locked block's lines.
    locked_values: Vec<String>,
    /// Whether the locked list is open or one line.
    locked_open: bool,
    changed: usize,
    _subscriptions: Vec<Subscription>,
}

/// What the view needs of an edit under way, with no borrow of the pane.
pub(super) struct EditView {
    state: Entity<EditorState>,
    locked: String,
    locked_count: usize,
    locked_values: Vec<String>,
    locked_open: bool,
    changed: usize,
    stale: bool,
}

impl DetailPane {
    /// The row the open object is, as core names what an edit is about.
    fn edit_row(&self) -> Option<Row> {
        let target = &self.detail.as_ref()?.target;
        let (kind, identity) = (&target.kind, &target.identity);
        Some(Row {
            group: kind.group.clone(),
            version: kind.version.clone(),
            kind: kind.kind.clone(),
            resource: kind.plural.clone(),
            namespace: Some(identity.namespace.clone()).filter(|ns| !ns.is_empty()),
            name: identity.name.clone(),
            uid: identity.uid.clone(),
        })
    }

    /// What an edit of the open object needs, which core declares.
    fn edit_needs(&self) -> Vec<AccessQuestion> {
        if self.embedded_node {
            return Vec::new();
        }
        self.edit_row()
            .map(|row| freshkube_core::write::edit_yaml::needs(&row))
            .unwrap_or_default()
    }

    /// Asks the cluster whether Edit may be offered for the open object, if
    /// it isn't known. Called when an object opens, never while drawing.
    pub(crate) fn want_edit(&mut self, cx: &mut Context<Self>) {
        let needs = self.edit_needs();
        if !self.active || needs.is_empty() {
            return;
        }
        self.offers
            .update(cx, |offers, cx| offers.want(&needs, cx));
    }

    /// What the cluster said of Edit on the open object.
    pub(crate) fn edit_offer(&self, cx: &App) -> Offer {
        let needs = self.edit_needs();
        if needs.is_empty() {
            return Offer::Pending;
        }
        self.offers.read(cx).offer(&needs, cx)
    }

    /// Why the cluster says Edit is not allowed, when it did.
    pub(crate) fn access_refusal(&self, cx: &App) -> Option<String> {
        self.edit_offer(cx).refusal().map(str::to_owned)
    }

    /// Why Edit is off, or `None`: the shell sets it from the cluster's level,
    /// as it does for the list's actions.
    pub fn set_write_block(&mut self, block: Option<&'static str>, cx: &mut Context<Self>) {
        if self.write_block != block {
            self.write_block = block;
            cx.notify();
        }
    }

    /// The editor and what is said around it, while the text is edited.
    pub(super) fn edit_view(&self) -> Option<EditView> {
        let detail = self.detail.as_ref()?;
        let Edit {
            phase: Phase::Editing(editing),
            identity,
        } = self.edit.as_ref()?
        else {
            return None;
        };
        if *identity != detail.target.identity {
            return None;
        }
        let version = &editing.session.target().resource_version;
        let stale = detail
            .view
            .as_ref()
            .is_some_and(|view| view.document.resource_version != *version);
        Some(EditView {
            state: editing.state.clone(),
            locked: editing.locked.clone(),
            locked_count: editing.locked_count,
            locked_values: editing.locked_values.clone(),
            locked_open: editing.locked_open,
            changed: editing.changed,
            stale,
        })
    }

    /// Whether a text is being edited: the pane's footer strip stays out of
    /// the way then, as the editor says what it has to say itself.
    pub(super) fn is_editing(&self) -> bool {
        matches!(
            &self.edit,
            Some(Edit {
                phase: Phase::Editing(_),
                ..
            })
        )
    }

    /// Opens or closes the locked list.
    fn toggle_locked(&mut self, cx: &mut Context<Self>) {
        if let Some(Edit {
            phase: Phase::Editing(editing),
            ..
        }) = self.edit.as_mut()
        {
            editing.locked_open = !editing.locked_open;
            cx.notify();
        }
    }

    /// The editor's state, while a text is edited.
    #[cfg(test)]
    pub(super) fn edit_state(&self) -> Option<Entity<EditorState>> {
        match &self.edit {
            Some(Edit {
                phase: Phase::Editing(editing),
                ..
            }) => Some(editing.state.clone()),
            _ => None,
        }
    }

    /// Drops an edit, with its text: another object, or the pane closing.
    pub(super) fn end_edit(&mut self) {
        self.edit = None;
    }

    /// Edit was pressed.
    pub(super) fn start_edit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let (Some(detail), Some(access)) = (self.detail.as_ref(), self.access.clone()) else {
            return;
        };
        if self.embedded_node
            || self.write_block.is_some()
            || self.access_refusal(cx).is_some()
            || self.edit.is_some()
            || detail.read == DocumentRead::Deleted
        {
            return;
        }
        let identity = detail.target.identity.clone();
        let Some(row) = self.edit_row() else {
            return;
        };
        self.feedback = None;
        if let ClusterAccess::Example(_) = access {
            let session = example::object_json(&identity, live::now())
                .and_then(|object| EditSession::from_row(&row, &object))
                .ok_or_else(|| "Example data has no such object.".to_owned());
            self.begin_editing(identity, session, window, cx);
            return;
        }
        let (job, receiver) = backend::spawn_job(
            &self.runtime,
            READ_DEADLINE,
            format!("No answer within {} seconds", READ_DEADLINE.as_secs()),
            async move {
                let client = access.client().await.map_err(|failure| failure.message)?;
                EditSession::read(&client, &row).await
            },
        );
        let task = cx.spawn_in(window, {
            let identity = identity.clone();
            async move |this, cx| {
                let session = receiver
                    .await
                    .unwrap_or_else(|_| Err("Reading stopped unexpectedly.".to_owned()));
                _ = this.update_in(cx, |pane, window, cx| {
                    // Only the edit this read was for: another object, or
                    // Cancel, has dropped it since.
                    let waiting = matches!(
                        &pane.edit,
                        Some(Edit { identity: at, phase: Phase::Reading { .. } }) if *at == identity
                    );
                    if waiting {
                        pane.begin_editing(identity, session, window, cx)
                    }
                });
            }
        });
        self.edit = Some(Edit {
            identity,
            phase: Phase::Reading { _job: (job, task) },
        });
        cx.notify();
    }

    fn begin_editing(
        &mut self,
        identity: ResourceIdentity,
        session: Result<EditSession, String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let phase = match session {
            Ok(session) => Phase::Editing(self.editing(session, window, cx)),
            Err(why) => Phase::Failed(format!("Couldn't open the YAML for editing: {why}").into()),
        };
        self.edit = Some(Edit { identity, phase });
        cx.notify();
    }

    fn editing(
        &mut self,
        session: EditSession,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Editing {
        let original = session.editable_text();
        let state = cx.new(|cx| {
            EditorState::new(window, cx)
                .language("yaml")
                .line_number(true)
                .searchable(true)
        });
        state.update(cx, |state, cx| {
            state.set_value(original.clone(), window, cx)
        });
        let marks = state.update(cx, |state, cx| {
            state.create_decorations_collection(vec![], cx)
        });
        let subscriptions = vec![
            cx.subscribe_in(&state, window, |this, _, event: &InputEvent, _, cx| {
                if let InputEvent::Change = event {
                    this.edited(cx);
                }
            }),
            // The caret and the selection redraw the editor; this view is cached.
            cx.observe(&state, |_, _, cx| cx.notify()),
        ];
        window.focus(&state.read(cx).focus_handle(cx).clone(), cx);
        Editing {
            locked: session.locked().join(", "),
            locked_count: session.locked().len(),
            locked_values: session.locked_values(),
            locked_open: false,
            session: Arc::new(session),
            state,
            marks,
            original,
            changed: 0,
            _subscriptions: subscriptions,
        }
    }

    /// The text changed: mark the lines that differ from where it started.
    pub(super) fn edited(&mut self, cx: &mut Context<Self>) {
        let mark = palette(cx).accent_soft;
        let Some(Edit {
            phase: Phase::Editing(editing),
            ..
        }) = self.edit.as_mut()
        else {
            return;
        };
        let text = editing.state.read(cx).value();
        let now: Vec<&str> = text.lines().collect();
        let was: Vec<&str> = editing.original.lines().collect();
        let marks = marks::compare(&was, &now);
        let mut starts = Vec::with_capacity(now.len());
        let mut at = 0;
        for line in text.split_inclusive('\n') {
            starts.push((at, line.trim_end_matches('\n').len()));
            at += line.len();
        }
        let decorations = marks
            .added
            .iter()
            .filter_map(|ix| starts.get(*ix))
            .filter(|(_, len)| *len > 0)
            .map(|(start, len)| {
                TextDecoration::new(
                    *start..*start + *len,
                    HighlightStyle {
                        background_color: Some(mark),
                        ..HighlightStyle::default()
                    },
                )
            })
            .collect();
        editing.marks.set(decorations, cx);
        editing.changed = marks.count();
        cx.notify();
    }

    /// Cancel was pressed: the text is dropped and the YAML view is back.
    pub(super) fn cancel_edit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.end_edit();
        window.focus(&self.focus, cx);
        cx.notify();
    }

    /// ⌘S or Review changes: core describes the text; a text that is not an
    /// edit says why and stays in the editor.
    pub(super) fn review_edit(&mut self, cx: &mut Context<Self>) {
        let Some(Edit {
            phase: Phase::Editing(editing),
            ..
        }) = self.edit.as_ref()
        else {
            return;
        };
        let text = editing.state.read(cx).value();
        match editing.session.describe(&text) {
            Ok(action) => {
                self.feedback = None;
                cx.emit(DetailEvent::EditYaml(Box::new(action)));
            }
            Err(why) => self.feedback = Some(why.text().into()),
        }
        cx.notify();
    }

    /// The YAML view's Edit button, or what stands in its place.
    pub(super) fn edit_button(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        if self.embedded_node || self.edit_view().is_some() {
            return None;
        }
        let reading = matches!(
            &self.edit,
            Some(Edit {
                phase: Phase::Reading { .. },
                ..
            })
        );
        let reason = self
            .write_block
            .map(str::to_owned)
            .or_else(|| match &self.edit {
                Some(Edit {
                    phase: Phase::Failed(why),
                    ..
                }) => Some(why.to_string()),
                _ => None,
            });
        // The level's reason wins over the cluster's.
        let offer = self.edit_offer(cx);
        let refusal = offer.refusal().map(str::to_owned);
        let reason = reason.or_else(|| refusal.clone());
        let tooltip = reason.clone().unwrap_or_else(|| match offer.note() {
            Some(note) => format!("Edit this object's YAML. {note}"),
            None => "Edit this object's YAML".to_owned(),
        });
        Some(
            Button::new("detail-edit")
                .outline()
                .xsmall()
                .label(if reading { "Opening…" } else { "Edit" })
                .disabled(self.write_block.is_some() || refusal.is_some() || reading)
                .tooltip(tooltip)
                .on_click(cx.listener(|pane, _, window, cx| {
                    pane.end_edit();
                    pane.start_edit(window, cx);
                }))
                .into_any_element(),
        )
    }

    /// The editor, its buttons and the notes around it.
    pub(super) fn editor(&self, view: EditView, cx: &mut Context<Self>) -> AnyElement {
        let p = palette(cx);
        let note = |id: &'static str, text: String, color: Hsla| {
            div()
                .id(id)
                .test_support()
                .aria_label(text.clone())
                .px(dp(freshkube_ui::page::PANE_PADDING))
                .py(dp(3.))
                .text_size(dp(11.5))
                .text_color(color)
                .child(text)
        };
        let counter = if view.changed == 0 {
            "No changes yet".to_owned()
        } else {
            format!("{} changed", pluralize(view.changed, "line", "lines"))
        };
        let toolbar = h_flex()
            .gap_2()
            .items_center()
            .px(dp(freshkube_ui::page::PANE_PADDING))
            .py_2()
            .child(
                div()
                    .id("detail-edit-count")
                    .test_support()
                    .aria_label(counter.clone())
                    .flex_1()
                    .text_size(dp(12.))
                    .text_color(p.muted)
                    .child(counter),
            )
            .child(self.locked_toggle(&view, cx))
            .child(
                Button::new("detail-edit-cancel")
                    .ghost()
                    .xsmall()
                    .label("Cancel")
                    .on_click(cx.listener(|pane, _, window, cx| pane.cancel_edit(window, cx))),
            )
            .child(
                Button::new("detail-edit-review")
                    .primary()
                    .xsmall()
                    .label("Review changes")
                    .tooltip(format!("Review changes ({}S)", primary_modifier()))
                    .on_click(cx.listener(|pane, _, _, cx| pane.review_edit(cx))),
            );
        v_flex()
            .id("detail-edit-view")
            .test_support()
            .size_full()
            .child(toolbar)
            .when(view.locked_open, |this| this.child(self.locked_list(&view, cx)))
            .when(!view.locked_values.is_empty(), |this| {
                this.child(self.locked_block(&view, cx))
            })
            .children(self.feedback.as_ref().map(|why| {
                note("detail-edit-error", why.to_string(), p.warn_ink)
            }))
            .when(view.stale, |this| {
                this.child(note(
                    "detail-edit-stale",
                    "This object changed since you opened it. Review checks it again before anything is sent."
                        .to_owned(),
                    p.warn_ink,
                ))
            })
            .child(
                div()
                    .id("detail-edit-editor")
                    .test_support()
                    .flex_1()
                    .min_h_0()
                    .border_t_1()
                    .border_color(p.line)
                    .font_family(cx.theme().mono_font_family.clone())
                    .child(Editor::new(&view.state).size_full()),
            )
            .into_any_element()
    }

    /// The disclosure in the toolbar that opens the list of locked fields.
    fn locked_toggle(&self, view: &EditView, cx: &mut Context<Self>) -> AnyElement {
        let label = format!(
            "{} Locked ({})",
            if view.locked_open { "▾" } else { "▸" },
            view.locked_count
        );
        Button::new("detail-edit-locked")
            .ghost()
            .xsmall()
            .label(label.clone())
            .accessibility_label(label)
            .tooltip("Fields an edit can't change")
            .on_click(cx.listener(|pane, _, _, cx| pane.toggle_locked(cx)))
            .into_any_element()
    }

    /// The locked fields, listed while the disclosure is open.
    fn locked_list(&self, view: &EditView, cx: &mut Context<Self>) -> AnyElement {
        let p = palette(cx);
        let text = format!("Locked, not editable here: {}.", view.locked);
        div()
            .id("detail-edit-locked-list")
            .test_support()
            .aria_label(text.clone())
            .px(dp(freshkube_ui::page::PANE_PADDING))
            .py(dp(3.))
            .text_size(dp(11.5))
            .text_color(p.muted)
            .child(text)
            .into_any_element()
    }

    /// A Secret's `data` and `stringData`, drawn outside the text so the
    /// placeholder can't be edited.
    fn locked_block(&self, view: &EditView, cx: &mut Context<Self>) -> AnyElement {
        let p = palette(cx);
        v_flex()
            .id("detail-edit-locked-values")
            .test_support()
            .aria_label(format!(
                "A Secret's values are not read: only its metadata can be edited. {}",
                view.locked_values.join("; ")
            ))
            .px(dp(freshkube_ui::page::PANE_PADDING))
            .py(dp(3.))
            .border_t_1()
            .border_color(p.line)
            .font_family(cx.theme().mono_font_family.clone())
            .text_size(dp(11.5))
            .text_color(p.muted)
            .child(view.locked_values.join(" · "))
            .into_any_element()
    }
}
