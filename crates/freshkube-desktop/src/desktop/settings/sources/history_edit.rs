//! The Supply chain and Results rows' edits: the verifying key and the
//! transparency log, saved to `supply_chain` through the setup file's save,
//! and where Tekton Results is reached, saved on the entry Builds is read
//! from. A key is a public key file, chosen and never read; a token or
//! certificate file Results already names stays as it is. Nothing here reads
//! a cluster: the page saves, and a check is Check sources' own.
use super::*;
use crate::desktop::settings::history;
use freshkube_core::setup::SupplyChain;
use freshkube_ui::form;
use gpui_kit::assets::IconName;
use gpui_kit::component::{button::ButtonGroup, h_flex};

impl SourcesList {
    /// Chooses a public key file, or `None` to stop verifying with one.
    pub(super) fn set_key_file(&mut self, path: Option<std::path::PathBuf>, cx: &mut Context<Self>) {
        if !self.editable(cx) {
            return;
        }
        let what = match &path {
            Some(_) => "Chose the verifying key.",
            None => "Stopped verifying with a key.",
        };
        self.save_chain(move |chain| chain.public_key_file = path, what, cx);
    }

    /// Turns reading the transparency log on or off.
    pub(super) fn set_transparency_log(&mut self, on: bool, cx: &mut Context<Self>) {
        if !self.editable(cx) || self.supply_chain.as_ref().is_some_and(|c| c.transparency_log == on)
        {
            return;
        }
        let what = if on {
            "Reading the transparency log."
        } else {
            "Not reading the transparency log."
        };
        self.save_chain(move |chain| chain.transparency_log = on, what, cx);
    }

    /// Saving synchronizes this list, so it waits for this update to end.
    fn save_chain(
        &mut self,
        edit: impl FnOnce(&mut SupplyChain) + Send + 'static,
        what: &'static str,
        cx: &mut Context<Self>,
    ) {
        let page = self.page.clone();
        cx.defer(move |cx| {
            _ = page.update(cx, |page, cx| page.save_supply_chain(edit, what.into(), cx));
        });
    }

    /// Chooses the key file with the system's picker.
    fn browse_key(&mut self, cx: &mut Context<Self>) {
        if !self.editable(cx) {
            return;
        }
        let selection = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            prompt: Some("Use public key".into()),
        });
        cx.spawn(async move |this, cx| {
            let Ok(Ok(Some(paths))) = selection.await else {
                return;
            };
            if let Some(path) = paths.into_iter().next() {
                _ = this.update(cx, |list, cx| list.set_key_file(Some(path), cx));
            }
        })
        .detach();
    }

    /// The Results setting of the entry Builds is read from.
    fn current_results(&self) -> Option<freshkube_core::setup::Results> {
        self.builds_entry()
            .and_then(|entry| entry.results.as_deref().cloned())
    }

    /// Makes the input, and fills it again when the file's setting changed.
    pub(super) fn sync_results_input(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.selected != Some(Read::Results) {
            return;
        }
        let input = self
            .results_input
            .get_or_insert_with(|| {
                cx.new(|cx| {
                    InputState::new(window, cx)
                        .placeholder("https://results.acme.example or namespace/service:port")
                })
            })
            .clone();
        let text = history::route_text(self.current_results().as_ref());
        if self.results_loaded.as_ref() != Some(&text) {
            self.results_loaded = Some(text.clone());
            self.results_error = None;
            input.update(cx, |input, cx| input.set_value(text, window, cx));
        }
    }

    fn typed_results(&self, cx: &App) -> String {
        self.results_input
            .as_ref()
            .map(|input| input.read(cx).value().trim().to_owned())
            .unwrap_or_default()
    }

    fn results_dirty(&self, cx: &App) -> bool {
        self.results_loaded
            .as_ref()
            .is_some_and(|loaded| *loaded != self.typed_results(cx))
    }

    /// Saves the typed address or forward on the Builds entry; empty text is
    /// Off.
    pub(super) fn save_results(&mut self, cx: &mut Context<Self>) {
        if !self.editable(cx) {
            return;
        }
        let text = self.typed_results(cx);
        let Some(id) = self.builds_entry().map(|entry| entry.id.clone()) else {
            return;
        };
        let results = if text.is_empty() {
            None
        } else {
            match history::parse(&text, self.current_results().as_ref()) {
                Ok(results) => Some(results),
                Err(why) => {
                    self.results_error = Some(why.into());
                    cx.notify();
                    return;
                }
            }
        };
        self.results_error = None;
        let page = self.page.clone();
        cx.defer(move |cx| {
            _ = page.update(cx, |page, cx| page.save_results(&id, results, cx));
        });
        cx.notify();
    }

    /// Off: the entry reads no Tekton Results.
    pub(super) fn turn_results_off(&mut self, cx: &mut Context<Self>) {
        if let Some(input) = &self.results_input {
            self.results_loaded = None;
            // The input follows the file once it has saved.
            _ = input;
        }
        self.results_error = None;
        let Some(id) = self.builds_entry().map(|entry| entry.id.clone()) else {
            return;
        };
        let page = self.page.clone();
        cx.defer(move |cx| {
            _ = page.update(cx, |page, cx| page.save_results(&id, None, cx));
        });
    }

    pub(super) fn render_supply_chain_edit(&self, narrow: bool, cx: &mut Context<Self>) -> AnyElement {
        let editable = self.editable(cx);
        let chain = self.supply_chain.as_ref();
        let key = chain.and_then(|chain| chain.public_key_file.clone());
        let p = crate::palette::palette(cx);
        let path = h_flex()
            .gap(dp(8.))
            .child(
                div()
                    .id("settings-source-key-path")
                    .test_support()
                    .aria_label(
                        key.as_ref()
                            .map(|key| key.display().to_string())
                            .unwrap_or_else(|| "No public key".into()),
                    )
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .text_size(dp(12.))
                    .when(key.is_some(), |this| this.font_family(ui::MONO_FONT))
                    .when(key.is_none(), |this| this.text_color(p.muted))
                    .child(
                        key.as_ref()
                            .map(|key| key.display().to_string())
                            .unwrap_or_else(|| "No public key".into()),
                    ),
            )
            .child(
                Button::new("settings-source-key-browse")
                    .outline()
                    .small()
                    .h(dp(ui::CONTROL_HEIGHT))
                    .icon(IconName::FolderOpen)
                    .label("Browse…")
                    .disabled(!editable)
                    .on_click(cx.listener(|this, _, _, cx| this.browse_key(cx))),
            )
            .child(
                Button::new("settings-source-key-remove")
                    .outline()
                    .small()
                    .h(dp(ui::CONTROL_HEIGHT))
                    .label("Remove")
                    .disabled(!editable || key.is_none())
                    .on_click(cx.listener(|this, _, _, cx| this.set_key_file(None, cx))),
            );
        let on = chain.is_some_and(|chain| chain.transparency_log);
        let log = ButtonGroup::new("settings-source-log")
            .outline()
            .small()
            .disabled(!editable)
            .child(ui::choice(
                Button::new("settings-source-log-off").label("Off").disabled(!editable),
                !on,
            ))
            .child(ui::choice(
                Button::new("settings-source-log-on").label("On").disabled(!editable),
                on,
            ))
            .on_click(cx.listener(|this, selected: &Vec<usize>, _, cx| {
                if let Some(index) = selected.first() {
                    this.set_transparency_log(*index == 1, cx);
                }
            }));
        form::group("settings-source-trust", "Trust")
            .narrow(narrow)
            .row(
                form::row("settings-source-key-row", "Public key", path)
                    .hint("A public key file; Freshkube never reads a private key."),
            )
            .row(
                form::row("settings-source-log-row", "Transparency log", log)
                    .hint("Off, no log is asked for an attestation’s entry."),
            )
            .saved_in(form::SavedIn::file("freshkube.yaml").and(" · supply_chain"))
            .render(cx)
            .into_any_element()
    }

    pub(super) fn render_results_edit(&self, narrow: bool, cx: &mut Context<Self>) -> AnyElement {
        let editable = self.editable(cx);
        let dirty = self.results_dirty(cx);
        let off = history::route(self.current_results().as_ref()).0 == "Off";
        let Some(input) = &self.results_input else {
            return div().into_any_element();
        };
        let field = Input::new(input)
            .id("settings-source-results-input")
            .aria_label("Where Results is reached")
            .small()
            .map(|input| Styled::h(input, dp(ui::CONTROL_HEIGHT)))
            .disabled(!editable);
        let field = match self.results_error {
            Some(_) => form::invalid(field, cx),
            None => field,
        };
        form::group("settings-source-reach", "Where Results is reached")
            .narrow(narrow)
            .dirty(dirty)
            .row(
                form::row("settings-source-results-row", "Address or Service", field)
                    .hint("An https:// address, or namespace/service:port to forward to when you press Forward. Its token stays where it is kept.")
                    .error(self.results_error.clone()),
            )
            .saved_in(form::SavedIn::file("freshkube.yaml").and(" · the Builds entry’s results"))
            .actions(form::actions(
                form::Action::new(
                    "settings-source-results-save",
                    "Save",
                    cx.listener(|this, _, _, cx| this.save_results(cx)),
                )
                .disabled(!editable || !dirty),
                form::Action::new(
                    "settings-source-results-off",
                    "Turn off",
                    cx.listener(|this, _, _, cx| this.turn_results_off(cx)),
                )
                .disabled(!editable || off),
            ))
            .render(cx)
            .into_any_element()
    }
}
