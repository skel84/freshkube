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
                        || this.settings_page.read(cx).setup_is_pending();
                    this.settings_page
                        .update(cx, |page, cx| page.sync_setup(file, defer, cx));
                    if *setup_changed {
                        this.rebuild_switcher(cx);
                        this.sync_setup_connection(window, cx);
                    }
                    cx.notify();
                }
            },
        ));
    }

    pub(super) fn sync_setup_connection(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.settings_page.read(cx).setup_is_pending() {
            return;
        }
        let Some(setup) = self.setup_file.read(cx).setup.as_ref() else {
            return;
        };
        let connection = setup.coroot.clone();
        let setup = setup.clone();
        self.pages.set_setup(setup, cx);
        self.pages.observability.update(cx, |page, cx| {
            page.set_setup_connection(&connection, window, cx)
        });
    }
}
