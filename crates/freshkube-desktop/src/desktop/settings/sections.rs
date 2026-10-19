//! Settings' sections (docs/DESIGN.md, "The Settings page"): what the
//! column lists, in its two groups, and the page each opens. The form
//! sections point to where their controls are until #697's later slices
//! build them on `freshkube_ui::form`.
use super::SettingsPage;
use crate::ui::{self, dp};
use freshkube_ui::page;
use gpui_kit::assets::IconName;
use gpui_kit::component::{Sizable, button::Button};
use gpui_kit::prelude::*;
use gpui_kit::*;

/// A section of Settings, each a page of its own.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub(crate) enum SettingsSection {
    General,
    Connection,
    Monitoring,
    Coroot,
    /// The workspace's clusters; `settings` opens it.
    #[default]
    Clusters,
    Destinations,
    Sources,
}

/// What a section that points elsewhere asks of the shell.
#[derive(Clone, Debug)]
pub(crate) enum SettingsEvent {
    /// Open the header's Settings menu, where the section's controls are.
    OpenMenu,
    /// Open Observability's Coroot connection.
    Coroot,
}

impl EventEmitter<SettingsEvent> for SettingsPage {}

impl SettingsSection {
    /// The column's groups, each a label and its sections in order.
    pub(crate) const GROUPS: [(&'static str, &'static [Self]); 2] = [
        (
            "App",
            &[
                Self::General,
                Self::Connection,
                Self::Monitoring,
                Self::Coroot,
            ],
        ),
        (
            "Workspace",
            &[Self::Clusters, Self::Destinations, Self::Sources],
        ),
    ];

    pub(crate) fn slug(self) -> &'static str {
        match self {
            Self::General => "general",
            Self::Connection => "connection",
            Self::Monitoring => "monitoring",
            Self::Coroot => "coroot",
            Self::Clusters => "clusters",
            Self::Destinations => "destinations",
            Self::Sources => "sources",
        }
    }

    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::General => "General",
            Self::Connection => "Connection",
            Self::Monitoring => "Monitoring",
            Self::Coroot => "Coroot",
            Self::Clusters => "Clusters",
            Self::Destinations => "Argo CD destinations",
            Self::Sources => "Delivery sources",
        }
    }

    pub(crate) fn icon(self) -> IconName {
        match self {
            Self::General => IconName::SlidersHorizontal,
            Self::Connection => IconName::KeyRound,
            Self::Monitoring => IconName::ChartLine,
            Self::Coroot => IconName::Radar,
            Self::Clusters => IconName::Server,
            Self::Destinations => IconName::Waypoints,
            Self::Sources => IconName::Database,
        }
    }

    /// The section a `FRESHKUBE_PAGE` value names: `settings-<slug>`.
    pub(crate) fn from_page(page: &str) -> Option<Self> {
        let slug = page.strip_prefix("settings-")?;
        Self::GROUPS
            .into_iter()
            .flat_map(|(_, sections)| sections.iter().copied())
            .find(|section| section.slug() == slug)
    }

    /// What the section's page says while its controls are elsewhere, and
    /// its button; none for a list section.
    fn pointer(self) -> Option<(&'static str, &'static str, &'static str, SettingsEvent)> {
        const MENU: &str = "In the Settings menu for now";
        const OPEN_MENU: &str = "Open the Settings menu";
        match self {
            Self::General => Some((
                MENU,
                "Appearance, text size and Talos auto-refresh are in the header’s Settings menu \
                 until this section holds them.",
                OPEN_MENU,
                SettingsEvent::OpenMenu,
            )),
            Self::Connection => Some((
                MENU,
                "The talosconfig and how the Kubernetes API is reached are in the header’s \
                 Settings menu until this section holds them.",
                OPEN_MENU,
                SettingsEvent::OpenMenu,
            )),
            Self::Monitoring => Some((
                MENU,
                "The metrics source and the dashboards folder are in the header’s Settings menu \
                 until this section holds them.",
                OPEN_MENU,
                SettingsEvent::OpenMenu,
            )),
            Self::Coroot => Some((
                "On Observability for now",
                "The Coroot connection is on Observability until this section holds it.",
                "Open the Coroot connection",
                SettingsEvent::Coroot,
            )),
            Self::Clusters | Self::Destinations | Self::Sources => None,
        }
    }

    /// Whether the section is one of the workspace's lists.
    pub(crate) fn is_list(self) -> bool {
        self.pointer().is_none()
    }
}

impl SettingsPage {
    /// A section whose controls are elsewhere: its title, and where they
    /// are with a button that goes there.
    pub(super) fn render_pointer(
        &self,
        section: SettingsSection,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let Some((title, description, action, event)) = section.pointer() else {
            return page::page("settings-page").into_any_element();
        };
        let prefix = format!("settings-{}", section.slug());
        let header = page::PageHeader::new(prefix.clone(), section.label()).render(window, cx);
        let button = Button::new(SharedString::from(format!("{prefix}-open")))
            .outline()
            .small()
            .h(dp(ui::CONTROL_HEIGHT))
            .label(action)
            .on_click(cx.listener(move |_, _, _, cx| cx.emit(event.clone())));
        page::padded("settings-page")
            .child(header)
            .child(
                div()
                    .id(SharedString::from(format!("{prefix}-pointer")))
                    .test_support()
                    .flex_1()
                    .min_h_0()
                    .child(ui::empty_state(
                        section.icon(),
                        title,
                        description,
                        None,
                        vec![button.into_any_element()],
                        cx,
                    )),
            )
            .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::SettingsSection;

    #[test]
    fn every_section_has_its_page_slug() {
        for (_, sections) in SettingsSection::GROUPS {
            for section in sections {
                let page = format!("settings-{}", section.slug());
                assert_eq!(SettingsSection::from_page(&page), Some(*section));
            }
        }
        assert_eq!(SettingsSection::from_page("settings"), None);
        assert_eq!(SettingsSection::from_page("settings-nowhere"), None);
        assert_eq!(SettingsSection::from_page("general"), None);
    }
}
