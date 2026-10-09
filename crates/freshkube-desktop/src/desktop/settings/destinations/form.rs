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

pub(super) struct DestinationForm {
    page: Entity<SettingsPage>,
    list: WeakEntity<Destinations>,
    /// A save the form started is in flight: it closes when the page says it
    /// landed, and shows why when it did not.
    pending: bool,
    /// The row being changed and what it matched when the form opened; none
    /// when mapping a new one.
    editing: Option<(usize, Key)>,
    by_server: bool,
    value: Entity<InputState>,
    entry: Entity<SelectState<Vec<SharedString>>>,
    error: Option<SharedString>,
    _subscriptions: Vec<Subscription>,
}

pub(super) fn open(
    page: Entity<SettingsPage>,
    list: WeakEntity<Destinations>,
    editing: Option<(usize, Key, SharedString)>,
    entries: Vec<SharedString>,
    window: &mut Window,
    cx: &mut App,
) {
    let title = match &editing {
        Some((_, key, _)) => format!("Change the mapping for {}", key.describe()),
        None => "Map an Argo CD destination".to_owned(),
    };
    let form = cx.new(|cx| DestinationForm::new(page, list, editing, entries, window, cx));
    let value = form.read(cx).value.clone();
    window.open_dialog(cx, move |dialog, window, _| {
        dialog
            .w(ui::dp_px(460., window).min(window.viewport_size().width - ui::dp_px(32., window)))
            .overlay_closable(false)
            .title(title.clone())
            .child(form.clone())
    });
    // A dialog remembers what had focus when it opened: focus its field after.
    value.update(cx, |state, cx| state.focus(window, cx));
}

impl DestinationForm {
    fn new(
        page: Entity<SettingsPage>,
        list: WeakEntity<Destinations>,
        editing: Option<(usize, Key, SharedString)>,
        entries: Vec<SharedString>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let value =
            cx.new(|cx| InputState::new(window, cx).placeholder("As the Application names it"));
        let picked = editing
            .as_ref()
            .and_then(|(_, _, entry)| entries.iter().position(|id| id == entry))
            .or((entries.len() == 1).then_some(0));
        let entry = cx.new(|cx| {
            SelectState::new(entries, picked.map(IndexPath::new), window, cx).searchable(true)
        });
        let mut by_server = true;
        if let Some((_, key, _)) = &editing {
            by_server = matches!(key, Key::Server(_));
            value.update(cx, |input, cx| {
                input.set_value(key.value().to_owned(), window, cx)
            });
        }
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
            pending: false,
            editing: editing.map(|(at, key, _)| (at, key)),
            by_server,
            value,
            entry,
            error: None,
            _subscriptions: subscriptions,
        }
    }

    fn submit(&mut self, cx: &mut Context<Self>) {
        if self.pending {
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
            // failed write loses nothing that was typed. The row is selected
            // now and stays so once the save lands.
            Ok(at) => {
                self.pending = true;
                _ = self.list.update(cx, |list, cx| list.select(at, cx));
            }
            Err(why) => self.error = Some(why),
        }
        cx.notify();
    }

    /// The page changed: if our save has answered, close on success, or show
    /// why not and keep what was typed.
    fn saved(&mut self, page: &Entity<SettingsPage>, window: &mut Window, cx: &mut Context<Self>) {
        if !self.pending {
            return;
        }
        let (saving, notice) = {
            let page = page.read(cx);
            (page.saving.is_some(), page.notice.clone())
        };
        if saving {
            return;
        }
        self.pending = false;
        match notice {
            Some(super::super::Notice::Failed(why)) => {
                self.error = Some(why);
                page.update(cx, |page, cx| {
                    page.notice = None;
                    cx.notify();
                });
            }
            _ => window.close_dialog(cx),
        }
        cx.notify();
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
            "spec.destination.name: Argo CD’s name for the cluster"
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
                        .placeholder("Pick a cluster")
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
                            .disabled(self.pending)
                            .on_click(cx.listener(|form, _, _, cx| form.submit(cx))),
                    ),
            )
    }
}
