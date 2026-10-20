//! The form in the dialog that maps an Argo CD destination to a workspace
//! cluster, or changes a mapping: what the Application names (its server
//! URL or Argo CD's name for the cluster) and the cluster it is. The page
//! checks and saves what it sends.
use super::*;
use freshkube_core::workspace::{Destination, Fault};
use freshkube_ui::form as fields;
use gpui_kit::component::{
    IndexPath,
    button::ButtonGroup,
    input::{Input, InputEvent, InputState},
    select::{Select, SelectEvent, SelectState},
};

/// What the form matches by, in its order.
const BY: [&str; 2] = ["Server URL", "Cluster name"];

/// The mapping a form changes, as it was when the form opened.
pub(super) struct Editing {
    /// Its place in the file, which the page checks still holds `key`.
    pub(super) at: usize,
    pub(super) key: Key,
    pub(super) entry: SharedString,
    /// Whether the workspace still lists `entry`; when it doesn't, the
    /// picker says so instead of showing nothing.
    pub(super) listed: bool,
}

pub(super) struct DestinationForm {
    page: Entity<SettingsPage>,
    list: WeakEntity<Destinations>,
    /// A save the form started is in flight, for the mapping it saves: the
    /// form closes and the list selects it when the page says it landed, and
    /// shows why when it did not.
    pending: Option<Key>,
    /// The row being changed and what it matched when the form opened; none
    /// when mapping a new one.
    editing: Option<(usize, Key)>,
    by_server: bool,
    value: Entity<InputState>,
    entry: Entity<SelectState<Vec<SharedString>>>,
    /// The picker's placeholder when the mapping's cluster is gone.
    gone: Option<SharedString>,
    value_error: Option<SharedString>,
    entry_error: Option<SharedString>,
    error: Option<SharedString>,
    _subscriptions: Vec<Subscription>,
}

pub(super) fn open(
    page: Entity<SettingsPage>,
    list: WeakEntity<Destinations>,
    editing: Option<Editing>,
    entries: Vec<SharedString>,
    window: &mut Window,
    cx: &mut App,
) -> Entity<DestinationForm> {
    let by_server = editing
        .as_ref()
        .is_some_and(|editing| matches!(editing.key, Key::Server(_)));
    let title: SharedString = match &editing {
        Some(Editing { key, .. }) => format!("Change the mapping for {}", key.describe()),
        None => "Map an Argo CD destination".to_owned(),
    }
    .into();
    let form = cx.new(|cx| DestinationForm::new(page, list, editing, entries, window, cx));
    let value = form.read(cx).value.clone();
    let shown = form.clone();
    window.open_dialog(cx, move |dialog, window, cx| {
        freshkube_ui::dialog::frame(dialog, 460., DestinationForm::actions(&shown, cx), window)
            .title(if by_server {
                // Kit's dialog close trigger is a small icon button, size_6.
                // Reserve the same rem width beside the flowing title.
                div()
                    .pr_6()
                    .child(freshkube_ui::url::url_text(
                        "settings-destination-edit-url",
                        &title,
                    ))
                    .into_any_element()
            } else {
                title.clone().into_any_element()
            })
            .child(shown.clone())
    });
    // A dialog remembers what had focus when it opened: focus its field after.
    value.update(cx, |state, cx| state.focus(window, cx));
    form
}

impl DestinationForm {
    fn actions(form: &Entity<Self>, cx: &App) -> impl IntoElement {
        let pending = form.read(cx).pending.is_some();
        let form = form.downgrade();
        fields::actions(
            fields::Action::new("settings-destination-save", "Save", move |_, window, cx| {
                _ = form.update(cx, |form, cx| form.submit(window, cx));
            })
            .disabled(pending),
            fields::Action::cancel("settings-destination-cancel"),
        )
    }

    fn new(
        page: Entity<SettingsPage>,
        list: WeakEntity<Destinations>,
        editing: Option<Editing>,
        entries: Vec<SharedString>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let value =
            cx.new(|cx| InputState::new(window, cx).placeholder("As the Application names it"));
        let picked = editing
            .as_ref()
            .and_then(|editing| entries.iter().position(|id| *id == editing.entry))
            .or((entries.len() == 1).then_some(0));
        let entry = cx.new(|cx| {
            SelectState::new(entries, picked.map(IndexPath::new), window, cx).searchable(true)
        });
        let mut by_server = true;
        if let Some(Editing { key, .. }) = &editing {
            by_server = matches!(key, Key::Server(_));
            value.update(cx, |input, cx| {
                input.set_value(key.value().to_owned(), window, cx)
            });
        }
        let gone = editing
            .as_ref()
            .filter(|editing| !editing.listed)
            .map(|editing| {
                format!(
                    "{} is gone, no longer in the workspace: pick a cluster",
                    editing.entry
                )
                .into()
            });
        let entry_focus = entry.focus_handle(cx);
        let subscriptions =
            vec![
                cx.subscribe_in(&value, window, |this, _, event: &InputEvent, window, cx| {
                    match event {
                        InputEvent::PressEnter { .. } => this.submit(window, cx),
                        InputEvent::Blur => {
                            this.value_error = this.inspect_value(cx);
                            cx.notify();
                        }
                        InputEvent::Change => {
                            this.value_error = None;
                            cx.notify();
                        }
                        _ => {}
                    }
                }),
                cx.subscribe_in(
                    &entry,
                    window,
                    |this, _, _: &SelectEvent<Vec<SharedString>>, _, cx| {
                        this.entry_error = this.inspect_entry(cx);
                        cx.notify();
                    },
                ),
                cx.on_blur(&entry_focus, window, |this, _, cx| {
                    this.entry_error = this.inspect_entry(cx);
                    cx.notify();
                }),
                cx.observe_in(&page, window, |this, page, window, cx| {
                    this.saved(&page, window, cx)
                }),
            ];
        Self {
            page,
            list,
            pending: None,
            gone,
            editing: editing.map(|editing| (editing.at, editing.key)),
            by_server,
            value,
            entry,
            value_error: None,
            entry_error: None,
            error: None,
            _subscriptions: subscriptions,
        }
    }

    fn key(&self, cx: &App) -> Key {
        let value = self.value.read(cx).value().trim().to_owned();
        if self.by_server {
            Key::Server(value)
        } else {
            Key::Name(value)
        }
    }

    /// Core decides what can match; the form only places its words at the field.
    fn inspect_value(&self, cx: &App) -> Option<SharedString> {
        let key = self.key(cx);
        if let Some(fault) = Destination::new(key.clone(), "entry").fault() {
            return Some(match fault {
                Fault::EmptyServer => "Enter the server URL the Application names".into(),
                Fault::EmptyName => "Enter Argo CD's name for the cluster".into(),
                Fault::NotHttp => "The server must be an http or https address".into(),
                Fault::ArgoCdOwn => {
                    "Argo CD knows its own cluster already; it needs no mapping".into()
                }
                fault => super::super::edit::capitalized(&format!("the mapping {}", fault.words()))
                    .into(),
            });
        }
        let workspace = self.page.read(cx).workspace();
        let other = workspace
            .destinations
            .iter()
            .enumerate()
            .find(|(at, row)| {
                self.editing
                    .as_ref()
                    .is_none_or(|(editing, _)| at != editing)
                    && row.key().same(&key)
            })?;
        let entry = &other.1.entry;
        let what = match key {
            Key::Server(server) => format!("Server {server}"),
            Key::Name(name) => format!("Argo CD's cluster {name}"),
        };
        Some(if workspace.clusters.iter().any(|cluster| cluster.id == *entry) {
            format!("{what} is already mapped to {entry}")
        } else {
            format!("{what} is already mapped to {entry}, which the workspace no longer lists: edit that mapping")
        }.into())
    }

    fn inspect_entry(&self, cx: &App) -> Option<SharedString> {
        let selected = self.entry.read(cx).selected_value();
        (!self
            .page
            .read(cx)
            .workspace()
            .clusters
            .iter()
            .any(|cluster| selected.is_some_and(|entry| cluster.id == entry.as_ref())))
        .then(|| "Pick a workspace cluster".into())
    }

    fn submit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.pending.is_some() {
            return;
        }
        self.error = None;
        self.value_error = self.inspect_value(cx);
        self.entry_error = self.inspect_entry(cx);
        if self.value_error.is_some() || self.entry_error.is_some() {
            if self.value_error.is_some() {
                self.value.update(cx, |value, cx| value.focus(window, cx));
            } else {
                self.entry.update(cx, |entry, cx| entry.focus(window, cx));
            }
            cx.notify();
            return;
        }
        let key = self.key(cx);
        let entry = self
            .entry
            .read(cx)
            .selected_value()
            .map(|id| id.to_string())
            .unwrap_or_default();
        let editing = self.editing.clone();
        let started = self.page.update(cx, |page, cx| {
            page.map_destination(
                editing.as_ref().map(|(at, was)| (*at, was)),
                key,
                &entry,
                cx,
            )
        });
        match started {
            // The save has started; the form stays until it answers, so a
            // failed write loses nothing that was typed.
            Ok(saved) => self.pending = Some(saved),
            Err(why) => self.error = Some(why),
        }
        cx.notify();
    }

    /// The page changed: if our save has answered, close on success, or show
    /// why not and keep what was typed.
    fn saved(&mut self, page: &Entity<SettingsPage>, window: &mut Window, cx: &mut Context<Self>) {
        if self.pending.is_none() {
            return;
        }
        let (saving, notice) = {
            let page = page.read(cx);
            (page.saving.is_some(), page.notice.clone())
        };
        if saving {
            return;
        }
        let saved = self.pending.take();
        match notice {
            Some(super::super::Notice::Failed(why)) => {
                self.error = Some(why);
                page.update(cx, |page, cx| {
                    page.notice = None;
                    cx.notify();
                });
            }
            // Landed: the mapping saved is the selected one.
            _ => {
                window.close_dialog(cx);
                if let Some(key) = saved {
                    _ = self.list.update(cx, |list, cx| list.select(key, cx));
                }
            }
        }
        cx.notify();
    }
}

#[cfg(test)]
impl DestinationForm {
    /// Picks the workspace cluster, as choosing it in the picker does.
    pub(super) fn pick(&mut self, entry: &str, window: &mut Window, cx: &mut Context<Self>) {
        let entry = SharedString::from(entry.to_owned());
        self.entry.update(cx, |select, cx| {
            select.set_selected_value(&entry, window, cx)
        });
    }

    pub(super) fn field_error(&self, field: &str) -> Option<&str> {
        match field {
            "value" => self.value_error.as_deref(),
            "entry" => self.entry_error.as_deref(),
            _ => None,
        }
    }

    pub(super) fn focused_field(&self, window: &Window, cx: &App) -> Option<&'static str> {
        if self.value.focus_handle(cx).is_focused(window) {
            Some("value")
        } else if self.entry.focus_handle(cx).is_focused(window) {
            Some("entry")
        } else {
            None
        }
    }

    pub(super) fn gone(&self) -> Option<&str> {
        self.gone.as_deref()
    }
}

impl Render for DestinationForm {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let this = cx.entity().downgrade();
        let selected = if self.by_server { 0 } else { 1 };
        let by = BY
            .iter()
            .enumerate()
            .fold(
                ButtonGroup::new("settings-destination-by")
                    .outline()
                    .small(),
                |group, (ix, by)| {
                    group.child(ui::choice(
                        Button::new(("settings-destination-by", ix)).label(*by),
                        selected == ix,
                    ))
                },
            )
            .on_click(move |selected: &Vec<usize>, _, cx| {
                let Some(ix) = selected.first().copied() else {
                    return;
                };
                _ = this.update(cx, |form, cx| {
                    form.by_server = ix == 0;
                    form.value_error = None;
                    cx.notify();
                });
            });
        let hint = if self.by_server {
            "spec.destination.server, such as https://prod.example.test"
        } else {
            "spec.destination.name: Argo CD's name for the cluster"
        };
        let value = Input::new(&self.value)
            .id("settings-destination-value")
            .aria_label(BY[selected])
            .small()
            .map(|input| gpui_kit::Styled::h(input, dp(ui::CONTROL_HEIGHT)))
            .when(self.value_error.is_some(), |input| {
                fields::invalid(input, cx)
            });
        let entry = Select::new(&self.entry)
            .id("settings-destination-entry")
            .small()
            .h(dp(ui::CONTROL_HEIGHT))
            .placeholder(self.gone.clone().unwrap_or("Pick a cluster".into()))
            .search_placeholder("Find a cluster");
        // The select paints its focus border after its style, so paint the
        // invalid border over it; focus must not hide the field's error.
        let entry = div()
            .id("settings-destination-entry-control")
            .test_support()
            .relative()
            .child(entry)
            .when(self.entry_error.is_some(), |this| {
                this.child(
                    div()
                        .absolute()
                        .inset_0()
                        .rounded(px(3.))
                        .border_1()
                        .border_color(crate::palette::palette(cx).crit),
                )
            });
        v_flex()
            .id("settings-destination-form")
            .test_support()
            .role(Role::Dialog)
            .aria_label("Argo CD destination")
            .gap(dp(12.))
            .text_size(dp(13.))
            .child(
                fields::group("settings-destination-fields", "Mapping")
                    .narrow(true)
                    .row(fields::row(
                        "settings-destination-match-field",
                        "Match by",
                        by,
                    ))
                    .row(
                        fields::row("settings-destination-value-field", BY[selected], value)
                            .hint(hint)
                            .error(self.value_error.clone()),
                    )
                    .row(
                        fields::row(
                            "settings-destination-entry-field",
                            "Workspace cluster",
                            entry,
                        )
                        .hint("The workspace cluster this destination points to.")
                        .error(self.entry_error.clone()),
                    )
                    .saved_in(fields::SavedIn::file("freshkube.yaml"))
                    .render(cx),
            )
            .children(self.error.clone().map(|why| {
                div()
                    .id("settings-destination-error")
                    .test_support()
                    .role(Role::Alert)
                    .aria_label(why.clone())
                    .child(ui::warning_banner(None, why, None, cx))
            }))
    }
}
