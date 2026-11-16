//! Joining on the Team setup page (#849 slice 4): the Join form, then what
//! the first fetch would set or replace, then "Map your contexts", then the
//! two-step review that applies. Words and rows are derived with the rest of
//! the page (`TeamState::derive`), never in `render`.

use std::sync::Arc;

use freshkube_core::team::{Choice, ContextMap, Membership};
use freshkube_ui::form;
use gpui_kit::component::input::Input;
use gpui_kit::component::{Disableable, Sizable, button::Button, menu::PopupMenuItem};
use gpui_kit::component::{h_flex, menu::DropdownMenu, v_flex};
use gpui_kit::prelude::*;
use gpui_kit::*;

use super::owner::Stage;
use super::{ChangeRow, SettingsPage, TeamState, change_line, contained, kind_word, line, plural};
use crate::ui::{self, Tone, dp};

/// One team cluster's row in "Map your contexts".
pub(super) struct MapRow {
    pub(super) id: String,
    /// What is chosen, worded: a context, "Left out" or "Choose a context".
    pub(super) shown: String,
    pub(super) chosen: Option<Choice>,
    /// A context with the cluster's own name, offered and never taken.
    pub(super) suggested: Option<String>,
}

/// What joining shows, derived from the membership and the contexts.
#[derive(Default)]
pub(super) struct JoinShown {
    pub(super) preview: Vec<ChangeRow>,
    pub(super) map: Vec<MapRow>,
    pub(super) map_note: String,
    /// The contexts to choose among, shared by every row's menu.
    pub(super) available: Arc<Vec<String>>,
    /// Why the last Join was refused, as a sentence.
    pub(super) error: Option<String>,
}

impl TeamState {
    /// Words the preview and the mapping from the membership.
    pub(super) fn derive_join(&self) -> JoinShown {
        let error = self.owner.join_error.as_deref().map(super::sentence);
        let Some(team) = self.membership.as_ref().filter(|m| m.applied().is_none()) else {
            return JoinShown {
                error,
                ..JoinShown::default()
            };
        };
        let preview = self
            .review
            .iter()
            .flat_map(|review| review.changes())
            .map(|change| ChangeRow {
                at: change.at.clone(),
                label: change
                    .trust
                    .map_or_else(|| kind_word(change).to_string(), |t| t.words().to_string()),
                line: change_line(change),
                trust: None,
            })
            .collect();
        let available = Arc::new(self.owner.available());
        let map = team
            .mapping()
            .clusters()
            .map(|(id, choice)| MapRow {
                id: id.to_string(),
                shown: match choice {
                    Some(Choice::Context(name)) => name.clone(),
                    Some(Choice::LeftOut) => "Left out".into(),
                    None => "Choose a context".into(),
                },
                chosen: choice.cloned(),
                suggested: ContextMap::suggested(id, available.as_slice()).map(str::to_string),
            })
            .collect();
        JoinShown {
            preview,
            map,
            map_note: map_note(team, self.owner.contexts.error()),
            available,
            error,
        }
    }
}

fn map_note(team: &Membership, read_error: Option<&str>) -> String {
    if let Some(error) = read_error {
        return format!("{error}. You can still leave a cluster out.");
    }
    match team.mapping().unchosen() {
        0 => "Every cluster is mapped. Continue to review.".into(),
        n => format!(
            "{n} {} still to map. The team file never names a context; yours stay yours.",
            plural(n, "cluster is", "clusters are")
        ),
    }
}

impl SettingsPage {
    /// The Join group, drawn while no team is joined.
    pub(super) fn team_join_group(&self, narrow: bool, cx: &mut Context<Self>) -> AnyElement {
        let owner = &self.team.owner;
        let busy = owner.is_fetching();
        let mut column = v_flex().gap(dp(12.));
        if let Some(why) = &self.team.shown.join.error {
            column = column.child(div().id("team-join-error").test_support().child(contained(
                ui::banner(
                    Tone::Warn,
                    Some("Couldn’t join".into()),
                    why.clone(),
                    None,
                    cx,
                ),
            )));
        }
        let mut group = form::group("team-join", "Join a team").narrow(narrow);
        if let Some(inputs) = owner.form.as_ref() {
            let field =
                |id: &'static str, state: &Entity<gpui_kit::component::input::InputState>| {
                    Input::new(state)
                        .id(id)
                        .small()
                        .disabled(busy)
                        .map(|input| Styled::h(input, dp(ui::CONTROL_HEIGHT)))
                };
            group = group
                .row(form::row(
                    "team-join-address",
                    "Address",
                    field("team-join-address-input", &inputs.address),
                ).hint("A git repository (https:// or ssh), or an https:// address of the file. Your own git sign-in is used; no password goes in an address."))
                .row(form::row(
                    "team-join-ref",
                    "Branch or tag",
                    field("team-join-ref-input", &inputs.git_ref),
                ))
                .row(form::row(
                    "team-join-path",
                    "Path",
                    field("team-join-path-input", &inputs.path),
                ))
                .row(
                    form::row(
                        "team-join-token",
                        "Token",
                        field("team-join-token-input", &inputs.token),
                    )
                    .hint("Optional, for an https:// file that needs one. Kept in your key store, never in the address or in a file."),
                );
        }
        let this = cx.entity();
        let label = if busy {
            "Fetching…"
        } else {
            "Fetch and preview"
        };
        let go = Button::new("team-join-start")
            .small()
            .outline()
            .label(label)
            .disabled(busy)
            .on_click(move |_, window, cx| this.update(cx, |page, cx| page.team_join(window, cx)));
        group = group.row(form::row(
            "team-join-row",
            "Join",
            h_flex().w_full().child(go),
        ));
        column.child(group.render(cx)).into_any_element()
    }

    /// The page while a first version is fetched and not yet applied.
    pub(super) fn team_joining(&self, narrow: bool, cx: &mut Context<Self>) -> AnyElement {
        let shown = &self.team.shown;
        let stage = self.team.owner.stage;
        let mut column = v_flex().id("team-joining").test_support().gap(dp(12.));
        if let Some(failure) = &shown.failure {
            column = column.child(div().id("team-failure").test_support().child(contained(
                ui::banner(
                    Tone::Warn,
                    Some(failure.lead.clone().into()),
                    failure.body.clone(),
                    None,
                    cx,
                ),
            )));
        }
        if let Some(why) = &shown.refusal {
            column = column.child(div().id("team-refusal").test_support().child(contained(
                ui::banner(
                    Tone::Warn,
                    Some("The update wasn’t applied".into()),
                    why.clone(),
                    None,
                    cx,
                ),
            )));
        }
        match stage {
            Stage::Preview => column.child(self.team_preview(narrow, cx)),
            Stage::Map => column.child(self.team_map(narrow, cx)),
            Stage::Review => column.child(self.team_review(narrow, cx)),
        }
        .into_any_element()
    }

    fn team_continue_actions(
        &self,
        id: &'static str,
        label: &'static str,
        ready: bool,
        cx: &mut Context<Self>,
    ) -> form::Actions {
        let go = cx.entity();
        let cancel = cx.entity();
        form::actions(
            form::Action::new(id, label, move |_, _, cx| {
                go.update(cx, |page, cx| page.team_continue(cx));
            })
            .disabled(!ready),
            form::Action::new("team-cancel-join", "Cancel", move |_, _, cx| {
                cancel.update(cx, |page, cx| page.team_cancel_join(cx));
            }),
        )
    }

    /// Everything the team would set or replace, before anything is asked.
    fn team_preview(&self, narrow: bool, cx: &mut Context<Self>) -> AnyElement {
        let shown = &self.team.shown;
        let rows = &shown.join.preview;
        let mut group = form::group("team-preview", "What joining sets").narrow(narrow);
        if rows.is_empty() {
            group = group.row(form::row(
                "team-preview-none",
                "Settings",
                "The team file sets nothing.",
            ));
        }
        for row in rows {
            group = group.row(form::row(
                format!("team-preview-{}", row.at),
                row.label.clone(),
                line(format!("team-preview-{}-line", row.at), row.line.clone()),
            ));
        }
        let n = rows.len();
        let banner = ui::banner(
            Tone::Info,
            Some("Joining the team".into()),
            format!(
                "Version {} would set or replace {n} {}. Nothing applies until you have mapped \
                 your contexts and reviewed it.",
                self.team.review.as_ref().map_or("", |r| r.target_id()),
                plural(n, "setting", "settings")
            ),
            None,
            cx,
        );
        v_flex()
            .id("team-preview-step")
            .test_support()
            .gap(dp(8.))
            .child(contained(banner))
            .child(
                group
                    .actions(self.team_continue_actions(
                        "team-preview-continue",
                        "Continue",
                        true,
                        cx,
                    ))
                    .render(cx),
            )
            .into_any_element()
    }

    /// "Map your contexts": which of your kubeconfig's contexts each team
    /// cluster is, or to leave it out.
    fn team_map(&self, narrow: bool, cx: &mut Context<Self>) -> AnyElement {
        let shown = &self.team.shown;
        let ready = self
            .team
            .membership
            .as_ref()
            .is_some_and(|m| m.mapping().is_complete());
        let available = shown.join.available.clone();
        let mut group = form::group("team-map", "Map your contexts").narrow(narrow);
        for row in &shown.join.map {
            let this = cx.entity();
            let id = row.id.clone();
            let chosen = row.chosen.clone();
            let available = available.clone();
            let suggested = row.suggested.clone();
            let picker = Button::new(SharedString::from(format!("team-map-{}-pick", row.id)))
                .outline()
                .small()
                .h(dp(ui::CONTROL_HEIGHT))
                .label(row.shown.clone())
                .dropdown_caret(true)
                .dropdown_menu(move |mut menu, _, _| {
                    for name in available.iter().take(MOST_CONTEXTS) {
                        let label: SharedString = if suggested.as_deref() == Some(name.as_str()) {
                            format!("{name} · same name").into()
                        } else {
                            name.clone().into()
                        };
                        let (this, id, pick) = (this.clone(), id.clone(), name.clone());
                        menu = menu.item(
                            PopupMenuItem::new(label)
                                .checked(chosen == Some(Choice::Context(name.clone())))
                                .on_click(move |_, _, cx| {
                                    let (id, pick) = (id.clone(), pick.clone());
                                    this.update(cx, |page, cx| {
                                        page.team_map_choose(&id, Choice::Context(pick), cx)
                                    });
                                }),
                        );
                    }
                    let (this, id) = (this.clone(), id.clone());
                    menu.item(
                        PopupMenuItem::new("Leave out")
                            .checked(chosen == Some(Choice::LeftOut))
                            .on_click(move |_, _, cx| {
                                let id = id.clone();
                                this.update(cx, |page, cx| {
                                    page.team_map_choose(&id, Choice::LeftOut, cx)
                                });
                            }),
                    )
                });
            let hint = match (&row.suggested, &row.chosen) {
                (Some(name), None) => Some(format!("Your kubeconfig has a context named {name}.")),
                _ => None,
            };
            let mut form_row = form::row(
                format!("team-map-{}", row.id),
                row.id.clone(),
                h_flex().w_full().child(picker),
            );
            if let Some(hint) = hint {
                form_row = form_row.hint(hint);
            }
            group = group.row(form_row);
        }
        group = group.row(form::row(
            "team-map-note",
            "Status",
            line("team-map-note-value", shown.join.map_note.clone()),
        ));
        let this = cx.entity();
        let match_names = Button::new("team-map-suggested")
            .small()
            .outline()
            .label("Use matching names")
            .on_click(move |_, _, cx| this.update(cx, |page, cx| page.team_map_suggested(cx)));
        v_flex()
            .id("team-map-step")
            .test_support()
            .gap(dp(8.))
            .child(contained(ui::banner(
                Tone::Info,
                Some("Map your contexts".into()),
                "The team file never names a context. Say which of yours each of its clusters is.",
                Some(match_names.into_any_element()),
                cx,
            )))
            .child(
                group
                    .actions(self.team_continue_actions("team-map-continue", "Continue", ready, cx))
                    .render(cx),
            )
            .into_any_element()
    }
}

/// The most contexts a picker lists.
const MOST_CONTEXTS: usize = 100;
