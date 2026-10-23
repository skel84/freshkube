//! The Protection group: how much the app asks before it writes to this
//! cluster (#852, `docs/WRITE_MODEL.md`). One choice of four, saved with
//! the group's Save like the others, so a click never loosens a guard.
use super::{Group, SettingsPage, saved_in};
use crate::ui;
use freshkube_core::team::{ClusterView, LevelSource};
use freshkube_core::workspace::Entry;
use freshkube_core::write::Protection;
use freshkube_ui::form;
use gpui_kit::component::{
    Disableable, Sizable,
    button::{Button, ButtonGroup},
    h_flex, v_flex,
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
    fn hint(&self, shown: Protection, source: LevelSource) -> String {
        let unset = if source == LevelSource::Default {
            "Not set in freshkube.yaml: guarded. "
        } else {
            ""
        };
        format!(
            "{unset}{} Read-only disables a shell's Start, Operations, Diagnostics' Apply fix, a service's Restart and every change to an object; the other levels ask first, as above.",
            shown.meaning()
        )
    }

    pub(super) fn render(
        &self,
        editable: bool,
        dirty: bool,
        team: Option<&ClusterView>,
        cx: &Context<SettingsPage>,
    ) -> AnyElement {
        // Never looser than the team's floor, whatever the draft holds.
        let (shown, source) = match team {
            Some(view) => view.effective(self.values.level),
            None => (
                self.values.shown(),
                self.values
                    .level
                    .map_or(LevelSource::Default, |_| LevelSource::Own),
            ),
        };
        // The team's level is a floor: the looser levels are off, and the
        // group says why.
        let allowed = team.map(ClusterView::allowed);
        let off = |level: &Protection| allowed.as_ref().is_some_and(|all| !all.contains(level));
        let levels = Protection::ALL.iter().enumerate().fold(
            ButtonGroup::new("settings-protection-level")
                .outline()
                .small()
                .disabled(!editable),
            |group, (ix, level)| {
                group.child(ui::choice(
                    Button::new(("settings-protection-level", ix))
                        .label(level.label())
                        .disabled(!editable || off(level)),
                    shown == *level,
                ))
            },
        );
        let chip = team.and_then(|view| {
            super::super::team::origin_chip(
                "settings-protection-origin".to_string(),
                view.level,
                cx,
            )
        });
        let floor = team.and_then(ClusterView::floor_words).map(|words| {
            div()
                .id("settings-protection-floor")
                .test_support()
                .aria_label(words.clone())
                .child(words)
        });
        let from = super::super::team::level_note(shown, source)
            .filter(|(kind, ..)| *kind != "default")
            .map(|(_, _, whole)| {
                div()
                    .id("settings-protection-source")
                    .test_support()
                    .aria_label(whole.clone())
                    .child(whole)
            });
        let hint = v_flex()
            .gap(ui::dp(4.))
            .child({
                let hint = self.hint(shown, source);
                div()
                    .id("settings-protection-hint")
                    .test_support()
                    .aria_label(hint.clone())
                    .child(hint)
            })
            .children(from)
            .children(floor);
        let page = cx.weak_entity();
        form::group("settings-protection", "Protection")
            .narrow(true)
            .dirty(dirty)
            .row(
                form::row(
                    "settings-protection-level-row",
                    "Level",
                    h_flex()
                        .gap(ui::dp(8.))
                        .items_center()
                        .child(levels.on_click(move |selected: &Vec<usize>, _, cx| {
                            let Some(level) =
                                selected.first().and_then(|ix| Protection::ALL.get(*ix))
                            else {
                                return;
                            };
                            let level = *level;
                            _ = page.update(cx, |page, cx| page.set_entry_protection(level, cx));
                        }))
                        .children(chip),
                )
                .hint_element(hint),
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
