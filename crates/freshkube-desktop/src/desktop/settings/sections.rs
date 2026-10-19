//! Settings' sections (docs/DESIGN.md, "The Settings page"): what the
//! column lists, in its two groups, and the page each opens. The form
//! sections point to where their controls are until #697's later slices
//! build them on `freshkube_ui::form`.
use super::SettingsPage;
use crate::desktop::{Appearance, app_menu, shell::Chrome};
use crate::text_size;
use crate::ui::{self, dp};
use freshkube_ui::{form, page};
use gpui_kit::assets::IconName;
use gpui_kit::component::{Disableable, button::ButtonGroup, switch::Switch};
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
            Self::General => None,
            Self::Connection => None,
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
        matches!(self, Self::Clusters | Self::Destinations | Self::Sources)
    }
}

impl SettingsPage {
    pub(super) fn render_general(&self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let (appearance, automatic, has_talos, saved) =
            Chrome::general_values(window, cx).unwrap_or((Appearance::System, true, true, false));
        let narrow = page::content_width(window) < form::NARROW;
        let appearance_control = ButtonGroup::new("general-appearance")
            .outline()
            .small()
            .child(ui::choice(
                Button::new("general-appearance-system")
                    .label("System")
                    .h(dp(ui::CONTROL_HEIGHT)),
                appearance == Appearance::System,
            ))
            .child(ui::choice(
                Button::new("general-appearance-light")
                    .label("Light")
                    .h(dp(ui::CONTROL_HEIGHT)),
                appearance == Appearance::Light,
            ))
            .child(ui::choice(
                Button::new("general-appearance-dark")
                    .label("Dark")
                    .h(dp(ui::CONTROL_HEIGHT)),
                appearance == Appearance::Dark,
            ))
            .on_click(|selected: &Vec<usize>, window, cx| {
                let action: Box<dyn Action> = match selected.first() {
                    Some(0) => Box::new(app_menu::UseSystemAppearance),
                    Some(1) => Box::new(app_menu::UseLightAppearance),
                    Some(_) => Box::new(app_menu::UseDarkAppearance),
                    None => return,
                };
                window.dispatch_action(action, cx);
            });
        let size = text_size::current(cx);
        let text_control = div()
            .id("general-text-size")
            .flex()
            .flex_wrap()
            .gap(dp(4.))
            .children(text_size::STEPS.iter().map(|step| {
                let step = *step;
                ui::choice(
                    Button::new(("general-text-size", step as usize))
                        .outline()
                        .small()
                        .label(format!("{step:.0}"))
                        .h(dp(ui::CONTROL_HEIGHT)),
                    step == size,
                )
                .toggled(step == size)
                .on_click(move |_, _, cx| text_size::set(step, cx))
            }));
        let refresh_control = Switch::new("general-auto-refresh")
            .small()
            .checked(automatic)
            .disabled(!has_talos)
            .accessibility_label("Refresh Talos every 15 seconds")
            .on_click(|automatic, window, cx| Chrome::change_auto_refresh(*automatic, window, cx));
        let saved_in = || {
            if saved {
                form::SavedIn::file("preferences.json")
            } else {
                form::SavedIn::session()
            }
        };
        let header = page::PageHeader::new("settings-general", "General").render(window, cx);
        page::padded("settings-page")
            .track_focus(&self.focus)
            .overflow_y_scroll()
            .restrict_scroll_to_axis()
            .child(header)
            .child(
                div()
                    .id("settings-general-form")
                    .test_support()
                    .w_full()
                    .flex()
                    .flex_col()
                    .gap(dp(20.))
                    .flex_none()
                    .child(
                        form::group("general-display", "Display")
                            .narrow(narrow)
                            .row(form::row(
                                "general-appearance-row",
                                "Appearance",
                                appearance_control,
                            ))
                            .row(
                                form::row("general-text-size-row", "Text size", text_control)
                                    .hint("Size in pixels; keyboard shortcuts also change it."),
                            )
                            .saved_in(saved_in())
                            .render(cx),
                    )
                    .child(
                        form::group("general-refresh", "Refresh")
                            .narrow(narrow)
                            .row(
                                form::row(
                                    "general-auto-refresh-row",
                                    "Talos auto-refresh",
                                    div()
                                        .h(dp(ui::CONTROL_HEIGHT))
                                        .flex()
                                        .items_center()
                                        .child(refresh_control),
                                )
                                .hint(
                                    crate::desktop::kubernetes_only::refresh_hint(
                                        has_talos, automatic,
                                    ),
                                ),
                            )
                            .saved_in(saved_in())
                            .render(cx),
                    ),
            )
            .into_any_element()
    }

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

    #[gpui_kit::test]
    fn general_is_a_settings_form_at_each_window_and_text_size(cx: &mut gpui_kit::TestAppContext) {
        use crate::desktop::{layout_check, testing::fixture};
        use crate::{text_size, ui};
        use freshkube_ui::{form, page};
        use gpui_kit::{AppContext, px, test::TestWindowExt};

        for (width, height) in [(1280., 880.), (760., 560.)] {
            let (_runtime, handle, pilot) = fixture(cx, width, height);
            for size in [13., 20.] {
                cx.update_window(handle, |_, window, cx| {
                    text_size::set(size, cx);
                    pilot.update(cx, |pilot, cx| {
                        pilot.show_settings(SettingsSection::General, window, cx)
                    });
                    window.render_frame(cx);
                    layout_check::assert_page_frame(
                        window,
                        cx,
                        &layout_check::PageFrame {
                            page: "settings-page",
                            title: "settings-general-title",
                            title_text: "General",
                            content: "settings-general-form",
                        },
                    );
                    let dp = |n| ui::dp_px(n, window);
                    let display = window.find("general-display").bounds();
                    let refresh = window.find("general-refresh").bounds();
                    assert!(display.size.width <= dp(form::MAX_WIDTH) + px(1.));
                    assert!((refresh.top() - display.bottom() - dp(20.)).abs() < px(1.));
                    let control = window.find("general-appearance-system").bounds();
                    let label = window.find("general-appearance-row-label").bounds();
                    assert!((control.size.height - dp(ui::CONTROL_HEIGHT)).abs() < px(1.));
                    if page::content_width(window) < form::NARROW {
                        assert!((control.left() - label.left()).abs() < px(1.));
                        assert!(control.top() >= label.bottom() + dp(4.) - px(1.));
                    } else {
                        assert!((control.left() - label.right() - dp(12.)).abs() < px(1.));
                        assert!((label.size.width - dp(form::LABEL_WIDTH)).abs() < px(1.));
                    }
                    let text_row = window.find("general-text-size-row").bounds();
                    for size in text_size::STEPS {
                        let button = window.find(("general-text-size", size as usize)).bounds();
                        assert!(button.left() >= display.left() - px(1.));
                        assert!(
                            button.right() <= display.right() + px(1.),
                            "{width} x {height}: {button:?} outside {display:?}"
                        );
                        assert!(button.bottom() <= text_row.bottom() + px(1.));
                        assert!((button.size.height - dp(ui::CONTROL_HEIGHT)).abs() < px(1.));
                    }
                    assert!(window.try_find("settings-scope").is_none());
                })
                .unwrap();
            }
        }
    }
}
