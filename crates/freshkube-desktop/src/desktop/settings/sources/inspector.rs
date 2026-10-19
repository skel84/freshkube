//! Selection, its explanation and the immediate pin edits.
use super::*;
use freshkube_ui::{form, inspector::Inspector};
use gpui_kit::component::{h_flex, menu::DropdownMenu};

#[derive(Clone, Debug, PartialEq, gpui_kit::Action)]
#[action(namespace = settings, no_json)]
pub(super) struct PinSource {
    pub(super) entry: Option<String>,
}

impl SourcesList {
    pub(super) fn select(&mut self, key: Read, window: &mut Window, cx: &mut Context<Self>) {
        self.selected = Some(key);
        table::reveal_when_settled(
            cx.entity().downgrade(),
            |list| list.selected.is_some(),
            window,
        );
        cx.notify();
    }

    pub(super) fn step(&mut self, delta: isize, cx: &mut Context<Self>) {
        if let Some(key) = table::step(self, delta, cx) {
            self.selected = Some(key);
            table::reveal(self, ScrollStrategy::Nearest);
            cx.notify();
        }
    }

    fn editable(&self, cx: &App) -> bool {
        self.page
            .upgrade()
            .is_some_and(|page| page.read(cx).editable())
    }

    fn pinned(&self, cx: &App) -> bool {
        match self.selected {
            Some(Read::Part(part)) => self
                .page
                .upgrade()
                .is_some_and(|page| page.read(cx).workspace.sources.pin(part).is_some()),
            _ => false,
        }
    }

    pub(super) fn pin_entries(&self, cx: &App) -> Vec<menu::MenuAction> {
        let Some(page) = self.page.upgrade() else {
            return Vec::new();
        };
        let page = page.read(cx);
        page.workspace
            .clusters
            .iter()
            .map(|entry| {
                let pinned = match self.selected {
                    Some(Read::Part(part)) => {
                        page.workspace.sources.pin(part) == Some(entry.id.as_str())
                    }
                    _ => false,
                };
                menu::MenuAction::new(
                    entry.id.clone(),
                    PinSource {
                        entry: Some(entry.id.clone()),
                    },
                )
                .checked(pinned)
                .enabled(page.editable())
            })
            .collect()
    }

    pub(super) fn follow_roles(&self, cx: &App) -> menu::MenuAction {
        menu::MenuAction::new("Follow roles", PinSource { entry: None })
            .enabled(self.pinned(cx) && self.editable(cx))
    }

    /// Defer until the list's update ends: saving synchronizes this child.
    pub(super) fn pin(&mut self, entry: Option<String>, cx: &mut Context<Self>) {
        let Some(Read::Part(part)) = self.selected else {
            return;
        };
        let page = self.page.clone();
        cx.defer(move |cx| {
            _ = page.update(cx, |page, cx| {
                if !page.editable()
                    || entry
                        .as_ref()
                        .is_some_and(|id| !page.workspace.clusters.iter().any(|e| &e.id == id))
                    || page.workspace.sources.pin(part) == entry.as_deref()
                {
                    return;
                }
                let mut next = page.workspace.clone();
                next.sources.set(part, entry);
                page.commit(next, format!("Updated {} source.", part.label()), cx);
            });
        });
    }

    pub(super) fn render_inspector(
        &self,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let row = self
            .rows
            .iter()
            .find(|row| Some(row.read) == self.selected)?;
        let width = crate::screens::page_width(window);
        let pane_width = if width >= freshkube_ui::inspector::SPLIT_WIDTH {
            self.split.live_width(cx)
        } else {
            width
        };
        let narrow = pane_width - 2. * page::PANE_PADDING < form::NARROW;
        let description = form::group("settings-source-details", "Source")
            .narrow(narrow)
            .row(form::row(
                "settings-source-choice",
                "Chosen by",
                row.explanation.clone(),
            ))
            .row(
                form::row("settings-source-state", "State", row.state.clone())
                    .hint(row.detail.clone()),
            )
            .row(form::row(
                "settings-source-reads",
                "Reads",
                row.reads.clone(),
            ));
        let mut detail = Inspector::new("settings-source-inspector")
            .heading(
                div()
                    .id("settings-source-inspector-title")
                    .test_support()
                    .aria_label(row.label.clone())
                    .text_size(dp(14.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(row.label.clone()),
            )
            .child(description.render(cx));
        if matches!(row.read, Read::Part(_)) {
            let editable = self.editable(cx);
            let reason = self
                .page
                .upgrade()
                .map(|page| page.read(cx).why_not_editable());
            let source = cx.entity().downgrade();
            let selected = self.selected;
            let focus = self.focus.clone();
            let pin = Button::new("settings-source-pin")
                .outline()
                .small()
                .h(dp(ui::CONTROL_HEIGHT))
                .label("Pin to entry…")
                .disabled(!editable)
                .when(!editable, |button| {
                    button.tooltip(reason.unwrap_or("Unavailable"))
                })
                .dropdown_menu(move |menu, window, cx| {
                    let entries = source
                        .upgrade()
                        .map(|source| source.read(cx).pin_entries(cx))
                        .unwrap_or_default();
                    let source = source.clone();
                    menu::actions(
                        menu,
                        entries,
                        &focus,
                        move |cx| {
                            source
                                .upgrade()
                                .is_some_and(|source| source.read(cx).selected == selected)
                        },
                        window,
                        cx,
                    )
                });
            let follow = Button::new("settings-source-follow-roles")
                .outline()
                .small()
                .h(dp(ui::CONTROL_HEIGHT))
                .label("Follow roles")
                .disabled(!editable || !self.pinned(cx))
                .on_click(cx.listener(|this, _, _, cx| this.pin(None, cx)));
            detail = detail.child(
                form::group("settings-source-routing", "Routing")
                    .narrow(narrow)
                    .row(form::row(
                        "settings-source-pin-row",
                        "Pin",
                        h_flex().gap(dp(8.)).flex_wrap().child(pin).child(follow),
                    ))
                    .saved_in(form::SavedIn::file("freshkube.yaml · sources"))
                    .render(cx),
            );
        }
        Some(detail.render(cx).into_any_element())
    }
}
