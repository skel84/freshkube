//! Knowing a dialog is gone, however it went. Cancel, Close and Escape pop the
//! dialog from the window's stack without telling its owner, so the owner
//! hands the dialog a guard: when the dialog's builder is dropped, the guard
//! runs `run` once, after the update that closed the dialog has ended.

use std::rc::Rc;

use gpui_kit::{App, AsyncApp};

pub(crate) struct OnDismiss {
    cx: AsyncApp,
    run: Option<Box<dyn FnOnce(&mut App)>>,
}

impl OnDismiss {
    /// Keep the guard in the dialog's builder closure.
    pub(crate) fn new(cx: &App, run: impl FnOnce(&mut App) + 'static) -> Rc<Self> {
        Rc::new(Self {
            cx: cx.to_async(),
            run: Some(Box::new(run)),
        })
    }
}

impl Drop for OnDismiss {
    fn drop(&mut self) {
        let Some(run) = self.run.take() else {
            return;
        };
        let cx = self.cx.clone();
        // Not now: the dialog is being closed inside an update of the window.
        self.cx
            .foreground_executor()
            .spawn(async move { cx.update(run) })
            .detach();
    }
}
