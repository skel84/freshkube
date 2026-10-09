//! The form in the dialog that maps an Argo CD destination to a workspace
//! cluster, or changes a mapping: what the Application names (its server
//! URL or Argo CD's name for the cluster) and the cluster it is. The page
//! checks and saves what it sends.
use super::*;
use gpui_kit::component::{
    IndexPath,
    button::ButtonGroup,
    input::{Input, InputEvent, InputState},
    select::{Select, SelectState},
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
    let title = match &editing {
        Some(Editing { key, .. }) => format!("Change the mapping for {}", key.describe()),
        None => "Map an Argo CD destination".to_owned(),
    };
    let form = cx.new(|cx| DestinationForm::new(page, list, editing, entries, window, cx));
    let value = form.read(cx).value.clone();
    let shown = form.clone();
    window.open_dialog(cx, move |dialog, window, _| {
        dialog
            .w(ui::dp_px(460., window).min(window.viewport_size().width - ui::dp_px(32., window)))
            .overlay_closable(false)
            .title(title.clone())
            .child(shown.clone())
    });
    // A dialog remembers what had focus when it opened: focus its field after.
    value.update(cx, |state, cx| state.focus(window, cx));
    form
}

impl DestinationForm {
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
        let subscriptions = vec![
            cx.subscribe_in(&value, window, |this, _, event: &InputEvent, _, cx| {
                if matches!(event, InputEvent::PressEnter { .. }) {
                    this.submit(cx);
                }
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
            error: None,
            _subscriptions: subscriptions,
        }
    }

    fn submit(&mut self, cx: &mut Context<Self>) {
        if self.pending.is_some() {
            return;
        }
        let value = self.value.read(cx).value().to_string();
        let key = match self.by_server {
            true => Key::Server(value),
            false => Key::Name(value),
        };
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

    pub(super) fn gone(&self) -> Option<&str> {
        self.gone.as_deref()
    }
}

impl Render for DestinationForm {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let this = cx.entity().downgrade();
        let label = |text: &'static str| {
            div()
                .text_size(dp(12.5))
                .font_weight(FontWeight::SEMIBOLD)
                .child(text)
        };
        let selected = if self.by_server { 0 } else { 1 };
        let by = BY.iter().enumerate().fold(
            ButtonGroup::new("settings-destination-by")
                .outline()
                .small(),
            |group, (ix, by)| {
                group.child(ui::choice(
                    Button::new(("settings-destination-by", ix)).label(*by),
                    selected == ix,
                ))
            },
        );
        let hint = if self.by_server {
            "spec.destination.server, such as https://prod.example.test"
        } else {
            "spec.destination.name: Argo CD's name for the cluster"
        };
        v_flex()
            .id("settings-destination-form")
            .test_support()
            .role(Role::Dialog)
            .aria_label("Argo CD destination")
            .gap_3()
            .text_size(dp(13.))
            .child(
                v_flex()
                    .gap_1p5()
                    .child(label("Match by"))
                    .child(by.on_click(move |selected: &Vec<usize>, _, cx| {
                        let Some(ix) = selected.first().copied() else {
                            return;
                        };
                        _ = this.update(cx, |form, cx| {
                            form.by_server = ix == 0;
                            cx.notify();
                        });
                    })),
            )
            .child(
                v_flex()
                    .gap_1p5()
                    .child(label(BY[selected]))
                    .child(
                        Input::new(&self.value)
                            .id("settings-destination-value")
                            .aria_label(BY[selected])
                            .small(),
                    )
                    .child(
                        div()
                            .text_size(dp(11.5))
                            .text_color(crate::palette::palette(cx).muted)
                            .child(hint),
                    ),
            )
            .child(
                v_flex().gap_1p5().child(label("Workspace cluster")).child(
                    Select::new(&self.entry)
                        .id("settings-destination-entry")
                        .small()
                        .placeholder(self.gone.clone().unwrap_or("Pick a cluster".into()))
                        .search_placeholder("Find a cluster"),
                ),
            )
            .children(self.error.clone().map(|why| {
                div()
                    .id("settings-destination-error")
                    .test_support()
                    .role(Role::Alert)
                    .aria_label(why.clone())
                    .child(ui::warning_banner(None, why, None, cx))
            }))
            .child(
                h_flex()
                    .gap_2()
                    .justify_end()
                    .child(
                        Button::new("settings-destination-cancel")
                            .outline()
                            .label("Cancel")
                            .on_click(|_, window, cx| window.close_dialog(cx)),
                    )
                    .child(
                        Button::new("settings-destination-save")
                            .primary()
                            .label("Save")
                            .disabled(self.pending.is_some())
                            .on_click(cx.listener(|form, _, _, cx| form.submit(cx))),
                    ),
            )
    }
}
