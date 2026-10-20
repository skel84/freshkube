//! The Coroot section: one Connection group on `freshkube_ui::form`
//! (docs/DESIGN.md, "The Settings page"). Connect writes the `coroot` block
//! of `freshkube.yaml` through `SetupFile::save_with`, keeping the setup's
//! other parts, and then asks Observability to connect. The key itself goes
//! to the credential store when Observability connects; the file only names
//! it (`key: keys.json`). Disconnect asks Observability to disconnect and
//! keeps the block.
use super::{Origin, SettingsPage, SettingsSection};
use crate::observability::ConnectionState;
use crate::palette::palette;
use crate::ui::{self, dp};
use freshkube_core::setup::{Coroot, KeysJson, SignIn};
use freshkube_ui::{form, page};
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::{Disableable, Sizable, button::Button, h_flex};
use gpui_kit::prelude::*;
use gpui_kit::*;

/// A key just typed, on its way to Observability. It never prints.
#[derive(Clone, Default)]
pub(crate) struct TypedKey(String);

impl TypedKey {
    pub(crate) fn new(key: String) -> Self {
        Self(key)
    }

    pub(crate) fn into_string(self) -> String {
        self.0
    }
}

impl std::fmt::Debug for TypedKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("TypedKey(..)")
    }
}

/// What Connect asks of Observability once the block is saved.
#[derive(Clone, Debug)]
pub(crate) struct CorootRequest {
    pub(crate) server: String,
    /// 0 API key, 1 session, 2 anonymous.
    pub(crate) auth: usize,
    pub(crate) key: TypedKey,
}

/// The block the form was last filled from.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Baseline {
    server: String,
    sign_in: SignIn,
    project: String,
    /// The store keeps this server's key.
    kept: bool,
    /// A file holds the key instead; Settings only names it.
    key_file: bool,
}

impl Baseline {
    fn of(block: Option<&Coroot>) -> Self {
        block.map_or_else(
            || Self {
                server: String::new(),
                sign_in: SignIn::ApiKey,
                project: String::new(),
                kept: false,
                key_file: false,
            },
            |block| Self {
                server: block.server.clone(),
                sign_in: block.sign_in,
                project: block.project.clone().unwrap_or_default(),
                kept: block.key.is_some(),
                key_file: block.api_key_file.is_some(),
            },
        )
    }
}

/// The connection example data shows; nothing of it is saved.
fn example_block() -> Coroot {
    let mut block = Coroot::new("https://coroot.acme.example", SignIn::ApiKey);
    block.key = Some(KeysJson);
    block.project = Some("acme".into());
    block
}

pub(super) struct CorootForm {
    pub(super) server: Entity<InputState>,
    pub(super) project: Entity<InputState>,
    pub(super) credential: Entity<InputState>,
    sign_in: SignIn,
    baseline: Baseline,
    /// A new key is being typed although one is kept.
    replacing: bool,
    server_error: Option<SharedString>,
    credential_error: Option<SharedString>,
    /// Why the last save was refused or failed.
    save_error: Option<SharedString>,
    saving: bool,
    _subscriptions: Vec<Subscription>,
}

impl CorootForm {
    fn reset(&mut self, baseline: Baseline, window: &mut Window, cx: &mut Context<SettingsPage>) {
        self.server.update(cx, |input, cx| {
            input.set_value(baseline.server.clone(), window, cx)
        });
        self.project.update(cx, |input, cx| {
            input.set_value(baseline.project.clone(), window, cx)
        });
        self.credential
            .update(cx, |input, cx| input.set_value("", window, cx));
        self.sign_in = baseline.sign_in;
        self.baseline = baseline;
        self.replacing = false;
        self.server_error = None;
        self.credential_error = None;
    }

    fn server_text(&self, cx: &App) -> String {
        self.server.read(cx).value().trim().to_owned()
    }

    fn typed(&self, cx: &App) -> String {
        self.credential.read(cx).value().trim().to_owned()
    }

    /// The store's key still belongs to what the form shows.
    fn keeps_key(&self, cx: &App) -> bool {
        self.baseline.kept
            && self.sign_in != SignIn::Anonymous
            && self.sign_in == self.baseline.sign_in
            && self.server_text(cx) == self.baseline.server
    }

    fn dirty(&self, cx: &App) -> bool {
        self.server_text(cx) != self.baseline.server
            || self.sign_in != self.baseline.sign_in
            || self.project.read(cx).value().trim() != self.baseline.project
            || !self.typed(cx).is_empty()
    }

    fn auth(&self) -> usize {
        match self.sign_in {
            SignIn::ApiKey => 0,
            SignIn::Session => 1,
            SignIn::Anonymous => 2,
        }
    }

    /// The block to save: the file's own, with the form's fields over it, so
    /// the keys this version doesn't know come back as they were read.
    fn block(&self, current: Option<&Coroot>, cx: &App) -> Coroot {
        let mut block = current
            .cloned()
            .unwrap_or_else(|| Coroot::new(String::new(), self.sign_in));
        block.server = self.server_text(cx);
        block.sign_in = self.sign_in;
        let project = self.project.read(cx).value().trim().to_owned();
        block.project = (!project.is_empty()).then_some(project);
        // A key typed now is named in the block only once it is stored
        // (`coroot_key_kept`); until then the block keeps what it had.
        if self.sign_in == SignIn::Anonymous || !self.keeps_key(cx) {
            block.key = None;
        }
        block
    }

    /// The fields' errors; the first failing field takes focus.
    fn validate(&mut self, window: &mut Window, cx: &mut Context<SettingsPage>) -> bool {
        let server = self.server_text(cx);
        self.server_error = server_error(&server);
        self.credential_error = (self.sign_in != SignIn::Anonymous
            && !self.baseline.key_file
            && !self.keeps_key(cx)
            && self.typed(cx).is_empty())
        .then(|| {
            match self.sign_in {
                SignIn::Session => "Enter the coroot_session value.",
                _ => "Enter the API key.",
            }
            .into()
        });
        if self.server_error.is_some() {
            window.focus(&self.server.read(cx).focus_handle(cx), cx);
        } else if self.credential_error.is_some() {
            window.focus(&self.credential.read(cx).focus_handle(cx), cx);
        }
        self.server_error.is_none() && self.credential_error.is_none()
    }
}

fn server_error(server: &str) -> Option<SharedString> {
    let rest = server
        .strip_prefix("https://")
        .or_else(|| server.strip_prefix("http://"));
    match rest {
        _ if server.is_empty() => Some("Enter the Coroot server's address.".into()),
        Some(host) if !host.is_empty() && !server.contains(char::is_whitespace) => None,
        _ => Some("Use an address that starts with http:// or https://.".into()),
    }
}

impl SettingsPage {
    /// Takes the form's fields from the file's block. Prepared after the
    /// current update, since it creates inputs.
    pub(super) fn prepare_coroot(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.coroot.is_some() || self.section != SettingsSection::Coroot {
            return;
        }
        let baseline = Baseline::of(self.coroot_block(cx).as_ref());
        let server =
            cx.new(|cx| InputState::new(window, cx).placeholder("https://coroot.acme.example"));
        let project = cx.new(|cx| InputState::new(window, cx).placeholder("All projects"));
        let credential = cx.new(|cx| InputState::new(window, cx));
        let mut form = CorootForm {
            server: server.clone(),
            project: project.clone(),
            credential: credential.clone(),
            sign_in: baseline.sign_in,
            baseline: baseline.clone(),
            replacing: false,
            server_error: None,
            credential_error: None,
            save_error: None,
            saving: false,
            _subscriptions: Vec::new(),
        };
        form.reset(baseline, window, cx);
        for input in [&server, &project, &credential] {
            form._subscriptions.push(cx.subscribe_in(
                input,
                window,
                |page, _, event, window, cx| match event {
                    InputEvent::PressEnter { .. } => page.connect_coroot(window, cx),
                    InputEvent::Change => {
                        if let Some(form) = page.coroot.as_mut() {
                            form.save_error = None;
                        }
                        cx.notify();
                    }
                    InputEvent::Blur => {
                        if let Some(form) = page.coroot.as_mut() {
                            form.server_error = server_error_if_filled(&form.server_text(cx));
                        }
                        cx.notify();
                    }
                    _ => {}
                },
            ));
        }
        if let Some(file) = self.setup_file.clone() {
            form._subscriptions
                .push(cx.observe_in(&file, window, |page, _, window, cx| {
                    page.refill_coroot(window, cx)
                }));
        }
        self.coroot = Some(form);
        cx.notify();
    }

    /// The file's block, or the example's.
    fn coroot_block(&self, cx: &App) -> Option<Coroot> {
        if self.origin == Origin::Example {
            return Some(example_block());
        }
        self.setup_file
            .as_ref()
            .and_then(|file| file.read(cx).setup.as_ref().and_then(|s| s.coroot.clone()))
    }

    /// The file changed: a form with nothing typed follows it.
    fn refill_coroot(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let baseline = Baseline::of(self.coroot_block(cx).as_ref());
        let Some(form) = self.coroot.as_mut() else {
            return;
        };
        if form.baseline == baseline {
            return;
        }
        if form.dirty(cx) {
            form.baseline = baseline;
        } else {
            form.reset(baseline, window, cx);
        }
        cx.notify();
    }

    /// Observability's connection moved on.
    pub(in crate::desktop) fn set_coroot_state(
        &mut self,
        state: ConnectionState,
        cx: &mut Context<Self>,
    ) {
        if self.coroot_state != state {
            self.coroot_state = state;
            cx.notify();
        }
    }

    fn coroot_editable(&self) -> bool {
        self.editable()
    }

    fn revert_coroot(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let baseline = Baseline::of(self.coroot_block(cx).as_ref());
        if let Some(form) = self.coroot.as_mut() {
            form.reset(baseline, window, cx);
            form.save_error = None;
            cx.notify();
        }
    }

    fn pick_sign_in(&mut self, sign_in: SignIn, cx: &mut Context<Self>) {
        if let Some(form) = self.coroot.as_mut() {
            form.sign_in = sign_in;
            form.credential_error = None;
            form.save_error = None;
            cx.notify();
        }
    }

    fn replace_coroot_key(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(form) = self.coroot.as_mut() else {
            return;
        };
        form.replacing = true;
        window.focus(&form.credential.read(cx).focus_handle(cx), cx);
        cx.notify();
    }

    /// Connect: saves the block when it changed, then asks Observability.
    pub(super) fn connect_coroot(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.origin == Origin::Example || self.coroot.as_ref().is_none_or(|form| form.saving) {
            return;
        }
        let editable = self.coroot_editable();
        let why_not = self.why_not_editable();
        let current = self.coroot_block(cx);
        let Some(form) = self.coroot.as_mut() else {
            return;
        };
        if !form.validate(window, cx) {
            cx.notify();
            return;
        }
        let request = CorootRequest {
            server: form.server_text(cx),
            auth: form.auth(),
            key: TypedKey::new(form.typed(cx)),
        };
        let block = form.block(current.as_ref(), cx);
        if current.as_ref() == Some(&block) {
            // Nothing to write: the file already says this.
            cx.emit(super::SettingsEvent::Connect(request));
            return;
        }
        let Some(file) = self.setup_file.clone().filter(|_| editable) else {
            form.save_error = Some(why_not.into());
            cx.notify();
            return;
        };
        form.saving = true;
        form.save_error = None;
        let seen = self.seen.clone();
        let save = file.update(cx, |file, cx| {
            file.save_with(seen, move |setup| setup.coroot = Some(block), cx)
        });
        self.saving = Some(cx.spawn_in(window, async move |this, cx| {
            let outcome = save.await;
            _ = this.update_in(cx, |page, window, cx| {
                page.coroot_saved(outcome, Some(request), window, cx)
            });
        }));
        cx.notify();
    }

    /// Forget: the block no longer names a key, and the store drops it.
    fn forget_coroot_key(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.origin == Origin::Example || self.coroot.as_ref().is_none_or(|form| form.saving) {
            return;
        }
        let Some(server) = self.coroot_block(cx).map(|block| block.server) else {
            return;
        };
        let Some(file) = self.setup_file.clone().filter(|_| self.coroot_editable()) else {
            let why = self.why_not_editable();
            if let Some(form) = self.coroot.as_mut() {
                form.save_error = Some(why.into());
            }
            cx.notify();
            return;
        };
        if let Some(form) = self.coroot.as_mut() {
            form.saving = true;
            form.save_error = None;
        }
        let seen = self.seen.clone();
        let save = file.update(cx, |file, cx| {
            file.save_with(
                seen,
                |setup| {
                    if let Some(block) = &mut setup.coroot {
                        block.key = None;
                    }
                },
                cx,
            )
        });
        self.saving = Some(cx.spawn_in(window, async move |this, cx| {
            let outcome = save.await;
            _ = this.update_in(cx, |page, window, cx| {
                let forgotten = outcome.is_ok();
                page.coroot_saved(outcome, None, window, cx);
                if forgotten {
                    cx.emit(super::SettingsEvent::Forget(server));
                }
            });
        }));
        cx.notify();
    }

    fn coroot_saved(
        &mut self,
        outcome: Result<(), crate::setup_file::SaveError>,
        request: Option<CorootRequest>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.saving = None;
        let Some(file) = self.setup_file.clone() else {
            return;
        };
        match outcome {
            Ok(()) => {
                self.sync_setup(file, false, cx);
                let baseline = Baseline::of(self.coroot_block(cx).as_ref());
                if let Some(form) = self.coroot.as_mut() {
                    form.saving = false;
                    let typed = request.as_ref().map(|r| r.key.0.clone());
                    if let Some(typed) = typed {
                        // A key typed now stays in its field until it is
                        // stored and the block names it.
                        form.reset(baseline, window, cx);
                        if !typed.is_empty() {
                            form.credential
                                .update(cx, |input, cx| input.set_value(typed, window, cx));
                            form.replacing = true;
                        }
                    } else {
                        form.baseline = baseline;
                    }
                }
                if let Some(request) = request {
                    cx.emit(super::SettingsEvent::Connect(request));
                }
            }
            Err(error) => {
                self.pending_setup |= matches!(error, crate::setup_file::SaveError::Changed);
                self.sync_setup(file, self.pending_setup, cx);
                if let Some(form) = self.coroot.as_mut() {
                    form.saving = false;
                    form.save_error = Some(error.to_string().into());
                }
            }
        }
        cx.notify();
    }

    /// The credential store holds the key just typed for `server`: the block
    /// names it now, and the field lets go of the text.
    pub(in crate::desktop) fn coroot_key_kept(
        &mut self,
        server: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.origin == Origin::Example || self.coroot.as_ref().is_none_or(|form| form.saving) {
            return;
        }
        let Some(block) = self.coroot_block(cx) else {
            return;
        };
        if block.server != server
            || block.key.is_some()
            || block.sign_in == SignIn::Anonymous
            || block.api_key_file.is_some()
        {
            return;
        }
        let why_not = self.why_not_editable();
        let Some(file) = self.setup_file.clone().filter(|_| self.coroot_editable()) else {
            if let Some(form) = self.coroot.as_mut() {
                form.save_error = Some(why_not.into());
            }
            cx.notify();
            return;
        };
        if let Some(form) = self.coroot.as_mut() {
            form.saving = true;
        }
        let seen = self.seen.clone();
        let save = file.update(cx, |file, cx| {
            file.save_with(
                seen,
                |setup| {
                    if let Some(block) = &mut setup.coroot {
                        block.key.get_or_insert(KeysJson);
                    }
                },
                cx,
            )
        });
        self.saving = Some(cx.spawn_in(window, async move |this, cx| {
            let outcome = save.await;
            _ = this.update_in(cx, |page, window, cx| {
                let kept = outcome.is_ok();
                page.coroot_saved(outcome, None, window, cx);
                if kept && let Some(form) = page.coroot.as_mut() {
                    form.credential
                        .update(cx, |input, cx| input.set_value("", window, cx));
                    form.replacing = false;
                }
                cx.notify();
            });
        }));
        cx.notify();
    }

    fn disconnect_coroot(&mut self, cx: &mut Context<Self>) {
        cx.emit(super::SettingsEvent::Disconnect);
    }

    pub(super) fn render_coroot(&self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let header = page::PageHeader::new("settings-coroot", "Coroot").render(window, cx);
        let mut content = div()
            .id("settings-coroot-form")
            .test_support()
            .w_full()
            .flex()
            .flex_col()
            .gap(dp(20.))
            .flex_none();
        if let Some(form) = &self.coroot {
            let narrow = page::content_width(window) < form::NARROW;
            content = content.child(self.render_coroot_group(form, narrow, cx));
        }
        page::padded("settings-page")
            .track_focus(&self.focus)
            .on_key_down(cx.listener(|page, event: &KeyDownEvent, window, cx| {
                if event.keystroke.key == "escape"
                    && page.coroot.as_ref().is_some_and(|form| form.dirty(cx))
                {
                    page.revert_coroot(window, cx);
                    cx.stop_propagation();
                }
            }))
            .overflow_y_scroll()
            .restrict_scroll_to_axis()
            .child(header)
            .child(
                div()
                    .w_full()
                    .max_w(dp(form::MAX_WIDTH))
                    .flex()
                    .flex_col()
                    .children(self.render_banners(cx)),
            )
            .child(content)
            .into_any_element()
    }

    fn render_coroot_group(
        &self,
        form: &CorootForm,
        narrow: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let example = self.origin == Origin::Example;
        let dirty = form.dirty(cx);
        let busy = form.saving || self.coroot_state == ConnectionState::Connecting;
        let p = palette(cx);
        let anonymous = form.sign_in == SignIn::Anonymous;

        let track = h_flex()
            .id("coroot-sign-in")
            .test_support()
            .gap(dp(2.))
            .p(dp(3.))
            .rounded(px(8.))
            .bg(p.surface_2)
            .children(
                [
                    (SignIn::ApiKey, "API key"),
                    (SignIn::Session, "Session"),
                    (SignIn::Anonymous, "Anonymous"),
                ]
                .into_iter()
                .enumerate()
                .map(|(ix, (sign_in, label))| {
                    ui::segment(
                        Button::new(("coroot-sign-in", ix)),
                        form.sign_in == sign_in,
                        cx,
                    )
                    .small()
                    .label(label)
                    .disabled(example)
                    .on_click(cx.listener(move |page, _, _, cx| page.pick_sign_in(sign_in, cx)))
                }),
            );

        let server = Input::new(&form.server)
            .id("coroot-server")
            .small()
            .map(|input| Styled::h(input, dp(ui::CONTROL_HEIGHT)))
            .disabled(example);
        let server = if form.server_error.is_some() {
            form::invalid(server, cx)
        } else {
            server
        };
        let project = Input::new(&form.project)
            .id("coroot-project")
            .small()
            .map(|input| Styled::h(input, dp(ui::CONTROL_HEIGHT)))
            .disabled(example);

        let (credential_label, credential_hint) = if form.sign_in == SignIn::Session {
            ("Session value", "The value of the coroot_session cookie.")
        } else {
            (
                "API key",
                "Create one under the project's settings in Coroot.",
            )
        };
        let kept = form.keeps_key(cx) && !form.replacing;
        let credential_control: AnyElement = if form.baseline.key_file {
            div()
                .id("coroot-key-file")
                .test_support()
                .h(dp(ui::CONTROL_HEIGHT))
                .flex()
                .items_center()
                .text_size(dp(12.))
                .text_color(p.muted)
                .child("Read from the key file named in freshkube.yaml")
                .into_any_element()
        } else if kept {
            form::secret("coroot-credential", form::Secret::Kept, cx)
                .store_name(freshkube_core::secrets::STORE_NAME)
                .on_replace(cx.listener(|page, _, window, cx| page.replace_coroot_key(window, cx)))
                .on_forget(cx.listener(|page, _, window, cx| page.forget_coroot_key(window, cx)))
                .into_any_element()
        } else {
            form::secret(
                "coroot-credential",
                form::Secret::Typing(form.credential.clone()),
                cx,
            )
            .into_any_element()
        };

        let (state_text, state_error): (SharedString, bool) =
            match (&form.save_error, &self.coroot_state) {
                (Some(error), _) => (error.clone(), true),
                (None, ConnectionState::Failed(error)) => (error.clone().into(), true),
                (None, ConnectionState::Connecting) => ("Connecting…".into(), false),
                (None, ConnectionState::Connected) => (
                    format!("Connected to {}", form.baseline.server).into(),
                    false,
                ),
                (None, ConnectionState::Disconnected) => ("Not connected".into(), false),
            };
        let connected = self.coroot_state == ConnectionState::Connected;
        let state = h_flex()
            .gap(dp(8.))
            .min_h(dp(ui::CONTROL_HEIGHT))
            .child(
                div()
                    .id("coroot-state-text")
                    .test_support()
                    .role(if state_error {
                        Role::Alert
                    } else {
                        Role::Status
                    })
                    .aria_label(state_text.clone())
                    .flex_1()
                    .min_w_0()
                    .text_size(dp(12.))
                    .text_color(if state_error { p.crit_ink } else { p.muted })
                    .child(state_text),
            )
            .when(connected, |row| {
                row.child(
                    Button::new("coroot-disconnect")
                        .outline()
                        .xsmall()
                        .label("Disconnect")
                        .disabled(example)
                        .on_click(cx.listener(|page, _, _, cx| page.disconnect_coroot(cx))),
                )
            });

        let saved_in = if example {
            form::SavedIn::session()
        } else if anonymous || form.baseline.key_file {
            form::SavedIn::file("freshkube.yaml")
        } else {
            // The key is in `keys.json`, beside the file.
            form::SavedIn::file(format!(
                "freshkube.yaml, the key in {}",
                freshkube_core::secrets::STORE_NAME
            ))
        };

        let mut group = form::group("coroot-connection", "Connection")
            .narrow(narrow)
            .dirty(dirty)
            .row(
                form::row("coroot-server-row", "Server", server)
                    .hint("The address of your Coroot, such as https://coroot.acme.example.")
                    .error(form.server_error.clone()),
            )
            .row(form::row("coroot-sign-in-row", "Sign in with", track));
        if !anonymous {
            group = group.row(
                form::row(
                    "coroot-credential-row",
                    credential_label,
                    credential_control,
                )
                .hint(credential_hint)
                .error(form.credential_error.clone()),
            );
        }
        group
            .row(
                form::row("coroot-project-row", "Project", project)
                    .hint("Optional: the project to open when more than one is offered."),
            )
            .row(form::row("coroot-state", "Status", state))
            .saved_in(saved_in)
            .actions(form::actions(
                form::Action::new(
                    "coroot-connect",
                    if busy { "Connecting…" } else { "Connect" },
                    cx.listener(|page, _, window, cx| page.connect_coroot(window, cx)),
                )
                .disabled(example || busy),
                form::Action::new(
                    "coroot-revert",
                    "Revert",
                    cx.listener(|page, _, window, cx| page.revert_coroot(window, cx)),
                )
                .disabled(example || !dirty),
            ))
            .render(cx)
            .into_any_element()
    }
}

fn server_error_if_filled(server: &str) -> Option<SharedString> {
    (!server.is_empty()).then(|| server_error(server)).flatten()
}
