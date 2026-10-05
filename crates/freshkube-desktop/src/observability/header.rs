//! Source, project and time controls in the shared page header.
use super::*;
use freshkube_ui::page::{self, PageHeader};
use gpui_kit::component::menu::{DropdownMenu, PopupMenuItem};

impl ObservabilityPage {
    pub(super) fn page_header(&self, window: &Window) -> PageHeader {
        let title = if self.destination == Destination::Application {
            self.selected_app
                .as_ref()
                .map_or("Application", |app| app.name())
        } else {
            self.destination.label()
        };
        // Applications runs edge to edge, with its header inset; the other
        // destinations keep the padded frame.
        let width = if self.destination == Destination::Applications {
            crate::screens::inset_width(window)
        } else {
            crate::screens::content_width(window)
        };
        let narrow = width < page::HEADER_NARROW;
        PageHeader::new("obs", title.to_owned(), narrow)
    }

    pub(super) fn source_controls(&self, header: PageHeader, cx: &Context<Self>) -> PageHeader {
        self.time_controls(self.source_header(header, cx), cx)
    }

    pub(super) fn source_header(&self, header: PageHeader, cx: &Context<Self>) -> PageHeader {
        let p = palette(cx);
        let host = self.live.provider.as_ref().map_or_else(
            || {
                if self.fixture {
                    "Example data"
                } else {
                    "Not connected"
                }
                .to_owned()
            },
            |provider| {
                let url = provider.url();
                url.split_once("://")
                    .map_or(url, |(_, rest)| rest)
                    .trim_end_matches('/')
                    .to_owned()
            },
        );
        let source = line()
            .id("obs-source")
            .test_support()
            .role(Role::Status)
            .aria_label(host.clone())
            .flex_none()
            .text_size(dp(12.))
            .child(
                text(host.clone())
                    .max_w(dp(140.))
                    .truncate()
                    .text_color(p.muted),
            )
            .tooltip(move |window, cx| Tooltip::new(host.clone()).build(window, cx));
        let mut header = header.meta([source.into_any_element()]);
        if !self.fixture {
            header = header.control(self.project_picker(cx));
        }
        header
    }

    pub(super) fn time_controls(&self, header: PageHeader, cx: &Context<Self>) -> PageHeader {
        let hours = self.hours();
        let owner = cx.entity().downgrade();
        let time = Button::new(header.id("time"))
            .outline()
            .small()
            .icon(IconName::Clock)
            .label(if hours == 168 {
                "7d".into()
            } else {
                format!("{hours}h")
            })
            .dropdown_caret(true)
            .accessibility_label("Observability time range")
            .tooltip(self.live.range_label.clone())
            .dropdown_menu(move |mut menu, _, _| {
                for (span, label) in [
                    (1, "1 hour"),
                    (3, "3 hours"),
                    (24, "24 hours"),
                    (168, "7 days"),
                ] {
                    let owner = owner.clone();
                    menu = menu.item(PopupMenuItem::new(label).checked(hours == span).on_click(
                        move |_, _, cx| {
                            _ = owner.update(cx, |page, cx| page.set_range(span, cx));
                        },
                    ));
                }
                menu
            });
        let refresh = Button::new(header.id("refresh"))
            .ghost()
            .small()
            .icon(IconName::RefreshCw)
            .accessibility_label(format!(
                "Refresh {}",
                self.destination.label().to_lowercase()
            ))
            .tooltip(format!(
                "Refresh {}",
                self.destination.label().to_lowercase()
            ))
            .disabled(!self.fixture && self.live.source.is_none())
            .on_click(cx.listener(|this, _, _, cx| this.refresh_current(cx)));
        header.control(time).control(refresh)
    }

    fn project_picker(&self, cx: &Context<Self>) -> AnyElement {
        let owner = cx.entity().downgrade();
        let projects = self.live.projects.clone();
        let labels = self.live.project_labels.clone();
        let selected = self
            .live
            .source
            .as_ref()
            .map(|source| source.project().to_owned());
        action("obs-project", self.live.project_label.clone())
            .max_w(dp(170.))
            .overflow_hidden()
            .dropdown_caret(true)
            .accessibility_label("Coroot project")
            .disabled(projects.is_empty())
            .dropdown_menu(move |mut menu, _, _| {
                for (project, label) in projects.iter().zip(&labels) {
                    let (owner, project) = (owner.clone(), project.clone());
                    menu = menu.item(
                        PopupMenuItem::new(label.clone())
                            .checked(selected.as_deref() == Some(&project.id))
                            .on_click(move |_, _, cx| {
                                _ = owner.update(cx, |this, cx| this.select_project(&project, cx));
                            }),
                    );
                }
                menu
            })
            .into_any_element()
    }
}
