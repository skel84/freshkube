//! Remember a validated Talos selection without blocking the window.

use super::Pilot;
use crate::connection_preferences::Selection;
use gpui_kit::component::{WindowExt, notification::Notification};
use gpui_kit::{Context, Window};

struct ConnectionSaveError;

impl Pilot {
    pub(super) fn remember_connection(&self, window: &mut Window, cx: &mut Context<Self>) {
        if self.fixture || self.kubernetes_only.is_some() {
            return;
        }
        let (Some(store), Some(path), Some(context)) = (
            self.connection_store.clone(),
            self.loaded_config_path.clone(),
            self.applied.context.clone(),
        ) else {
            return;
        };
        store.remember(Selection { path, context });
        let saving = cx
            .background_executor()
            .spawn(async move { store.save_latest() });
        cx.spawn_in(window, async move |this, cx| {
            if let Err(error) = saving.await {
                let _ = this.update_in(cx, |_, window, cx| {
                    window.push_notification(
                        Notification::error(format!(
                            "Couldn't remember this connection for next launch: {error}"
                        ))
                        .id::<ConnectionSaveError>(),
                        cx,
                    );
                });
            }
        })
        .detach();
    }
}
