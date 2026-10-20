//! The workspace's own kubeconfig: a row in the Clusters header's second
//! line. The file is chosen, never edited, and a choice stands alone, so it
//! saves at once.
use super::*;
use crate::ui::MONO_FONT;
use gpui_kit::assets::IconName;

impl SettingsPage {
    pub(super) fn render_kubeconfig(&self, cx: &mut Context<Self>) -> AnyElement {
        let p = crate::palette::palette(cx);
        let editable = self.editable();
        let path = self.workspace.kubeconfig.as_ref();
        h_flex()
            .id("settings-kubeconfig")
            .test_support()
            .gap(dp(8.))
            .h(dp(ui::CONTROL_HEIGHT))
            .min_w_0()
            .text_size(dp(12.))
            .child(div().flex_none().text_color(p.ink_2).child("Kubeconfig"))
            .child(
                div()
                    .id("settings-kubeconfig-path")
                    .test_support()
                    .aria_label("The workspace’s kubeconfig")
                    .min_w_0()
                    .truncate()
                    .when(path.is_some(), |this| this.font_family(MONO_FONT))
                    .when(path.is_none(), |this| this.text_color(p.muted))
                    .child(match path {
                        Some(path) => path.display().to_string(),
                        None => "Automatic: KUBECONFIG, then the home default".into(),
                    }),
            )
            .child(
                Button::new("settings-kubeconfig-browse")
                    .outline()
                    .small()
                    .h(dp(ui::CONTROL_HEIGHT))
                    .icon(IconName::FolderOpen)
                    .label("Browse…")
                    .disabled(!editable)
                    .tooltip("Choose the kubeconfig file the workspace opens clusters with")
                    .on_click(
                        cx.listener(|page, _, window, cx| page.browse_kubeconfig(window, cx)),
                    ),
            )
            .when(path.is_some(), |this| {
                this.child(
                    Button::new("settings-kubeconfig-clear")
                        .outline()
                        .small()
                        .h(dp(ui::CONTROL_HEIGHT))
                        .label("Use automatic")
                        .disabled(!editable)
                        .tooltip("Read KUBECONFIG, then the home default")
                        .on_click(cx.listener(|page, _, _, cx| page.set_kubeconfig(None, cx))),
                )
            })
            .into_any_element()
    }

    fn browse_kubeconfig(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.editable() {
            return;
        }
        let selection = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            prompt: Some("Use Kubeconfig".into()),
        });
        cx.spawn_in(window, async move |this, cx| {
            let Ok(Ok(Some(paths))) = selection.await else {
                return;
            };
            let Some(path) = paths.into_iter().next() else {
                return;
            };
            _ = this.update(cx, |page, cx| page.set_kubeconfig(Some(path), cx));
        })
        .detach();
    }
}
