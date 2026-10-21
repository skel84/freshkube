//! The Protection group: how much the app asks before it writes to this
//! cluster (#852, `docs/WRITE_MODEL.md`). One choice of four, saved with
//! the group's Save like the others, so a click never loosens a guard.
use super::{Group, SettingsPage, saved_in};
use crate::ui;
use freshkube_core::workspace::Entry;
use freshkube_core::write::Protection;
use freshkube_ui::form;
use gpui_kit::component::{
    Disableable, Sizable,
    button::{Button, ButtonGroup},
};
use gpui_kit::prelude::*;
use gpui_kit::*;

/// What the group holds. `None` is the file's no key, which reads as
/// guarded; choosing Guarded is `Some`, and writes the key.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Values {
    pub(super) level: Option<Protection>,
}

impl Values {
    pub(super) fn of(entry: &Entry) -> Self {
        Self {
            level: entry.protection,
        }
    }

    /// Writes the group's key into `entry` and nothing else of it.
    pub(super) fn apply(&self, entry: &mut Entry) {
        entry.protection = self.level;
    }

    /// The level the choice stands for.
    pub(super) fn shown(&self) -> Protection {
        self.level.unwrap_or_default()
    }
}

pub(super) struct Level {
    /// The draft; the group is dirty when it differs from the baseline.
    pub(super) values: Values,
    /// What the file holds for the entry.
    pub(super) baseline: Values,
}

impl Level {
    pub(super) fn new(entry: &Entry) -> Self {
        let baseline = Values::of(entry);
        Self {
            values: baseline,
            baseline,
        }
    }

    pub(super) fn dirty(&self) -> bool {
        self.values != self.baseline
    }

    /// What the group says under the choice: the level's meaning, that an
    /// unset level is guarded, and what enforces it today.
    fn hint(&self) -> String {
        let shown = self.values.shown();
        let unset = (self.values.level.is_none())
            .then_some("Not set in freshkube.yaml, so Guarded. ")
            .unwrap_or_default();
        format!(
            "{unset}{} Today only Read-only does anything: it disables a shell's Start and Operations; the other levels change nothing yet.",
            shown.meaning()
        )
    }

    pub(super) fn render(
        &self,
        editable: bool,
        dirty: bool,
        cx: &Context<SettingsPage>,
    ) -> AnyElement {
        let shown = self.values.shown();
        let levels = Protection::ALL.iter().enumerate().fold(
            ButtonGroup::new("settings-protection-level")
                .outline()
                .small()
                .disabled(!editable),
            |group, (ix, level)| {
                group.child(ui::choice(
                    Button::new(("settings-protection-level", ix))
                        .label(level.label())
                        .disabled(!editable),
                    shown == *level,
                ))
            },
        );
        let page = cx.weak_entity();
        form::group("settings-protection", "Protection")
            .narrow(true)
            .dirty(dirty)
            .row(
                form::row(
                    "settings-protection-level-row",
                    "Level",
                    levels.on_click(move |selected: &Vec<usize>, _, cx| {
                        let Some(level) = selected.first().and_then(|ix| Protection::ALL.get(*ix))
                        else {
                            return;
                        };
                        let level = *level;
                        _ = page.update(cx, |page, cx| page.set_entry_protection(level, cx));
                    }),
                )
                .hint(self.hint()),
            )
            .saved_in(saved_in())
            .actions(form::actions(
                form::Action::new(
                    "settings-protection-save",
                    "Save",
                    cx.listener(|page, _, window, cx| {
                        page.save_group(Group::Protection, window, cx)
                    }),
                )
                .disabled(!editable || !dirty),
                form::Action::new(
                    "settings-protection-revert",
                    "Revert",
                    cx.listener(|page, _, window, cx| {
                        page.revert_group(Group::Protection, window, cx);
                        window.focus(&page.focus, cx);
                    }),
                )
                .disabled(!dirty),
            ))
            .render(cx)
            .into_any_element()
    }
}
