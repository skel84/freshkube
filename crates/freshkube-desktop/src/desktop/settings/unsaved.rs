//! The section hooks used by the shell's single navigation guard.
use super::*;

impl SettingsPage {
    pub(in crate::desktop) fn has_unsaved(&self, section: SettingsSection, cx: &App) -> bool {
        match section {
            SettingsSection::Connection => self.connection_has_unsaved(cx),
            SettingsSection::Monitoring => self.monitoring_has_unsaved(cx),
            SettingsSection::Coroot => self.coroot_has_unsaved(cx),
            SettingsSection::Clusters => self.has_unsaved_edits(cx),
            _ => false,
        }
    }

    pub(in crate::desktop) fn unsaved_save_in_progress(&self) -> bool {
        self.save_in_progress()
            || self.connection_is_saving()
            || self.monitoring_is_saving()
            || self.coroot_is_saving()
    }

    pub(in crate::desktop) fn save_unsaved(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        match self.section {
            SettingsSection::Connection => self.save_connection(window, cx),
            SettingsSection::Monitoring => {
                self.save_monitoring(window, cx);
                self.save_monitoring_source(window, cx);
            }
            SettingsSection::Coroot => self.save_coroot_unsaved(window, cx),
            SettingsSection::Clusters => self.save_entry_unsaved(window, cx),
            _ => {}
        }
    }

    pub(in crate::desktop) fn discard_unsaved(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match self.section {
            SettingsSection::Connection => self.revert_connection(window, cx),
            SettingsSection::Monitoring => self.revert_monitoring(window, cx),
            SettingsSection::Coroot => self.revert_coroot(window, cx),
            SettingsSection::Clusters => self.discard_entry_unsaved(window, cx),
            _ => {}
        }
    }
}
