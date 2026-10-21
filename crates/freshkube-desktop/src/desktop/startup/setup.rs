//! The setup read precedes source selection; subsequent changes flow through
//! Settings' workspace to the existing switcher and delivery hand-offs.
use super::{GpuiOptions, Pilot};
use gpui_kit::component::WindowExt;
use gpui_kit::{Context, Window};

impl Pilot {
    pub(super) fn start_with_setup(
        &mut self,
        options: GpuiOptions,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if options.starts_with_examples() || options.preferences.is_none() {
            self.start(options, window, cx);
            return;
        }
        let mut pending = Some(options);
        self._subscriptions.push(cx.subscribe_in(
            &self.setup_file,
            window,
            move |this, _, event: &crate::setup_file::Event, window, cx| match event {
                crate::setup_file::Event::Ready => {
                    if let Some(options) = pending.take() {
                        this.start(options, window, cx);
                        this.sync_setup_connection(window, cx);
                    }
                }
                crate::setup_file::Event::Changed { setup_changed } => {
                    let file = this.setup_file.clone();
                    let defer = window.has_active_dialog(cx)
                        || this.settings_page.read(cx).save_in_progress()
                        || this.settings_page.read(cx).has_unsaved_edits(cx)
                        || this.settings_page.read(cx).setup_is_pending();
                    this.settings_page
                        .update(cx, |page, cx| page.sync_setup(file, defer, cx));
                    if *setup_changed {
                        this.rebuild_switcher(cx);
                    }
                    this.prepare_protection(cx);
                    this.sync_setup_connection(window, cx);
                    cx.notify();
                }
            },
        ));
    }

    pub(super) fn sync_setup_connection(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.settings_page.read(cx).setup_is_pending() {
            return;
        }
        let file = self.setup_file.read(cx);
        let setup = file.setup.clone();
        let problem = file.notice();
        let revision = file.setup_revision();
        let reconnect = file.coroot_reconnect;
        self.pages.set_setup(setup.clone(), problem, revision, cx);
        let Some(setup) = setup else {
            return;
        };
        let connection = setup.coroot;
        self.pages.observability.update(cx, |page, cx| {
            page.set_setup_connection(&connection, reconnect, window, cx)
        });
        // The connection's project is the entries' when they name none.
        self.sync_applications_coroot(cx);
    }
}
