//! Source, project and time controls in the shared page header.
use super::*;
use freshkube_ui::page::{self, PageHeader};
use gpui_kit::component::menu::{DropdownMenu, PopupMenuItem};
use std::rc::Rc;

impl ObservabilityPage {
    /// The page's title. An application's report leads with its
    /// breadcrumb, "Applications / namespace / name" with the application's
    /// status glyph before its name, as a dashboard leads with its own.
    pub(super) fn page_header(&self, cx: &Context<Self>) -> PageHeader {
        if self.destination != Destination::Application {
            return PageHeader::new("obs", self.destination.label());
        }
        let Some(app) = &self.selected_app else {
            return PageHeader::new("obs", "Application");
        };
        let mut header = PageHeader::new("obs", app.name().to_owned()).parent(
            "breadcrumb",
            "Applications",
            cx.listener(|this, _, _, cx| this.open(Destination::Applications, cx)),
        );
        header = header.crumb(app.namespace().unwrap_or("External / unmapped").to_owned());
        let state = self
            .selected_application()
            .map_or(Status::Unknown, |a| a.status);
        match tone(state) {
            Some(tone) => header.glyph(tone),
            None => header,
        }
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
        let what = format!("Refresh {}", self.destination.label().to_lowercase());
        let button = Button::new(header.id("refresh"))
            .ghost()
            .small()
            .size(dp(crate::ui::CONTROL_HEIGHT))
            .icon(IconName::RefreshCw)
            .accessibility_label(what.clone())
            .tooltip_with_action(what, &page::Refresh, Some(page::SHELL_CONTEXT))
            .disabled(!enabled)
            .on_click(page::dispatch(page::Refresh, &self.focus));
        let folded_refresh = page::action_entry(
            freshkube_ui::menu::MenuAction::new("Refresh", page::Refresh).enabled(enabled),
            &self.focus,
        );
        // Coroot keeps an application's revisions whatever the range, and
        // the inspector picks its own window around one.
        let header = if self.destination == Destination::Deployments {
            header
        } else {
            header.foldable(time, range)
        };
        header.foldable(button, folded_refresh)
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
