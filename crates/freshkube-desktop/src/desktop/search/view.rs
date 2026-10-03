use super::*;
use gpui_kit::component::command::Command;
#[derive(IntoElement)]
pub(super) struct SearchView {
    pub(super) search: Entity<Search>,
    pub(super) pilot: WeakEntity<Pilot>,
}
impl RenderOnce for SearchView {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let state = self.search.read(cx);
        let command = state.command.clone();
        let groups = state.groups.clone();
        let destinations = state.destinations.clone();
        let search = self.search.downgrade();
        let query = self.search.downgrade();
        let cancel = self.search.downgrade();
        let pilot = self.pilot;
        let notes = state.notes.clone();
        groups
            .into_iter()
            .fold(Command::new(&command), Command::group)
            .bordered(false)
            .filterable(false)
            .placeholder("Search everything…")
            .max_h(
                crate::ui::dp_px(360., window).min(
                    (window.viewport_size().height - crate::ui::dp_px(230., window))
                        .max(crate::ui::dp_px(96., window)),
                ),
            )
            .on_query(move |text, _, cx| {
                _ = query.update(cx, |search, cx| {
                    search.query = text.into();
                    search.rebuild();
                    cx.notify();
                });
            })
            .on_confirm(move |index, window, cx| {
                let destination = destinations
                    .get(index.section)
                    .and_then(|group| group.get(index.row))
                    .cloned()
                    .flatten();
                let Some(destination) = destination else {
                    return;
                };
                _ = search.update(cx, |search, cx| search.close(cx));
                window.close_dialog(cx);
                _ = pilot.update(cx, |pilot, cx| {
                    pilot.search_destination(destination, window, cx)
                });
            })
            .on_cancel(move |_, cx| {
                _ = cancel.update(cx, |search, cx| search.close(cx));
            })
            .footer(move |_, _, cx| {
                gpui_kit::component::v_flex()
                    .p(dp(10.))
                    .gap(dp(3.))
                    .text_size(dp(11.))
                    .text_color(crate::palette::palette(cx).muted)
                    .children(notes.iter().map(|(id, note)| {
                        div()
                            .id(id.clone())
                            .test_support()
                            .role(Role::Status)
                            .aria_label(note.clone())
                            .child(note.clone())
                    }))
                    .child("↑ ↓ choose · Enter opens · Escape closes")
            })
    }
}
