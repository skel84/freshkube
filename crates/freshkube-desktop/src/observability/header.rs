//! Source, project and time controls in the shared page header.
use super::*;
use freshkube_ui::page::{self, PageHeader};
use gpui_kit::component::menu::{DropdownMenu, PopupMenuItem};
use std::rc::Rc;

impl ObservabilityPage {
    pub(super) fn page_header(&self) -> PageHeader {
        let title = if self.destination == Destination::Application {
            self.selected_app
                .as_ref()
                .map_or("Application", |app| app.name())
        } else {
            self.destination.label()
        };
        PageHeader::new("obs", title.to_owned())
    }

    pub(super) fn source_controls(&self, header: PageHeader, cx: &Context<Self>) -> PageHeader {
        self.time_controls(self.source_header(header, cx), cx)
    }

    pub(super) fn source_header(&self, header: PageHeader, cx: &Context<Self>) -> PageHeader {
        let mut header = header;
        if !self.fixture {
            let items = self.project_items(cx);
            let folded = if self.live.projects.is_empty() {
                page::disabled_item("Project")
            } else {
                page::submenu_value("Project", self.live.project_label.clone(), items.clone())
            };
            header = header.foldable(self.project_picker(items), folded);
        }
        header
    }

    pub(super) fn time_controls(&self, header: PageHeader, cx: &Context<Self>) -> PageHeader {
        let hours = self.hours();
        let owner = cx.entity().downgrade();
        let ranges: page::MenuItems = Rc::new(move |mut menu, _, _| {
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
        let span = if hours == 168 {
            "7d".to_string()
        } else {
            format!("{hours}h")
        };
        let range = page::Fold::from(page::submenu_value(
            "Time range",
            span.clone(),
            ranges.clone(),
        ))
        .changed((hours != super::DEFAULT_HOURS).then(|| format!("Time range {span}").into()));
        let time = Button::new(header.id("time"))
            .outline()
            .small()
            .h(dp(crate::ui::CONTROL_HEIGHT))
            .icon(IconName::Clock)
            .label(span.clone())
            .dropdown_caret(true)
            .accessibility_label("Observability time range")
            .tooltip(self.live.range_label.clone())
            .dropdown_menu({
                let ranges = ranges.clone();
                move |menu, window, cx| ranges(menu, window, cx)
            });
        let enabled = self.fixture || self.live.source.is_some();
        let refresh = page::handler(cx, |this: &mut Self, _, cx| this.refresh_current(cx));
        let button = Button::new(header.id("refresh"))
            .ghost()
            .small()
            .size(dp(crate::ui::CONTROL_HEIGHT))
            .icon(IconName::RefreshCw)
            .accessibility_label(format!(
                "Refresh {}",
                self.destination.label().to_lowercase()
            ))
            .tooltip(format!(
                "Refresh {}",
                self.destination.label().to_lowercase()
            ))
            .disabled(!enabled)
            .on_click({
                let refresh = refresh.clone();
                move |_, window, cx| refresh(window, cx)
            });
        let folded_refresh = if enabled {
            page::item("Refresh", refresh)
        } else {
            page::disabled_item("Refresh")
        };
        header
            .foldable(time, range)
            .foldable(button, folded_refresh)
    }

    /// The Coroot projects as checked items: the project picker's menu, and
    /// its folded form.
    fn project_items(&self, cx: &Context<Self>) -> page::MenuItems {
        let owner = cx.entity().downgrade();
        let projects = self.live.projects.clone();
        let labels = self.live.project_labels.clone();
        let selected = self
            .live
            .source
            .as_ref()
            .map(|source| source.project().to_owned());
        Rc::new(move |mut menu, _, _| {
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
    }

    fn project_picker(&self, items: page::MenuItems) -> AnyElement {
        action("obs-project", self.live.project_label.clone())
            .h(dp(crate::ui::CONTROL_HEIGHT))
            .max_w(dp(170.))
            .overflow_hidden()
            .dropdown_caret(true)
            .accessibility_label("Coroot project")
            .disabled(self.live.projects.is_empty())
            .dropdown_menu(move |menu, window, cx| items(menu, window, cx))
            .into_any_element()
    }
}
