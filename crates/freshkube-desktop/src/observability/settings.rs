use super::*;
use gpui_kit::component::menu::{DropdownMenu, PopupMenuItem};

impl ObservabilityPage {
    pub(super) fn read_placeholder(&self, cx: &Context<Self>) -> Option<AnyElement> {
        if self.fixture {
            return None;
        }
        let (has_data, loading, error) = match self.destination {
            Destination::Applications => (
                self.live.apps.data().is_some(),
                self.live.apps.is_loading(),
                self.live.apps.error(),
            ),
            Destination::ServiceMap => (
                self.live.map.data().is_some(),
                self.live.map.is_loading(),
                self.live.map.error(),
            ),
            _ => return None,
        };
        if has_data {
            return None;
        }
        Some(
            v_flex()
                .id("obs-read-placeholder")
                .test_support()
                .gap(dp(12.))
                .child(section(self.destination.label()))
                .child(muted(
                    if loading {
                        "Waiting for Coroot"
                    } else if error.is_some() {
                        "No successful observation is available. Use Refresh to retry."
                    } else {
                        "Choose a project to read its observations"
                    },
                    cx,
                ))
                .into_any_element(),
        )
    }
    pub(crate) fn show_connection(&mut self, cx: &mut Context<Self>) {
        self.settings_open = true;
        cx.notify();
    }
    pub(super) fn render_connection(&self, cx: &Context<Self>) -> AnyElement {
        if self.fixture {
            return div().into_any_element();
        }
        let mut content = v_flex().gap(dp(12.));
        let owner = cx.entity().downgrade();
        let projects = self.live.projects.clone();
        let labels = self.live.project_labels.clone();
        let picker = action("obs-project", self.live.project_label.clone())
            .dropdown_caret(true)
            .disabled(projects.is_empty())
            .dropdown_menu(move |mut menu, _, _| {
                for (project, label) in projects.iter().zip(&labels) {
                    let (owner, project) = (owner.clone(), project.clone());
                    menu =
                        menu.item(PopupMenuItem::new(label.clone()).on_click(move |_, _, cx| {
                            _ = owner.update(cx, |this, cx| this.select_project(&project, cx));
                        }));
                }
                menu
            });
        content = content.child(
            line()
                .flex_wrap()
                .child(text(
                    self.live
                        .provider
                        .as_ref()
                        .map_or("Coroot".into(), |p| p.url().to_string()),
                ))
                .child(picker)
                .child(
                    action("obs-connect-settings", "Connection…")
                        .on_click(cx.listener(|this, _, _, cx| this.show_connection(cx))),
                )
                .child(
                    action("obs-refresh", "Refresh")
                        .disabled(self.live.source.is_none())
                        .on_click(cx.listener(|this, _, _, cx| this.refresh_current(cx))),
                )
                .child(
                    action("obs-disconnect", "Disconnect")
                        .disabled(self.live.provider.is_none())
                        .on_click(cx.listener(|this, _, window, cx| this.disconnect(window, cx))),
                ),
        );
        content = content.child(muted(self.live.range_label.clone(), cx));
        if let Some(source) = &self.live.source {
            if let Some(association) = source.association() {
                content = content.child(
                    action("obs-unmap", "Remove Kubernetes association")
                        .on_click(cx.listener(|this, _, _, cx| this.associate(None, cx))),
                );
                content=content.child(muted(format!("Kubernetes links: Coroot cluster {} is associated with the selected Freshkube access",association.cluster()),cx));
            }
            if self.live.access.is_some() {
                let owner = cx.entity().downgrade();
                let clusters = self.cluster_ids.clone();
                content = content.child(
                    line()
                        .flex_wrap()
                        .when(source.association().is_none(), |row| {
                            row.child(muted("Kubernetes links are unmapped", cx))
                        })
                        .child(
                            action("obs-associate", "Associate cluster…")
                                .disabled(clusters.is_empty())
                                .dropdown_caret(true)
                                .dropdown_menu(move |mut menu, _, _| {
                                    for cluster in &clusters {
                                        let (owner, cluster) = (owner.clone(), cluster.clone());
                                        menu = menu.item(
                                            PopupMenuItem::new(format!(
                                                "{cluster} → selected Freshkube cluster"
                                            ))
                                            .on_click(move |_, _, cx| {
                                                _ = owner.update(cx, |this, cx| {
                                                    this.associate(Some(cluster.clone()), cx)
                                                });
                                            }),
                                        );
                                    }
                                    menu
                                }),
                        ),
                );
            }
        }
        if self.settings_open || self.live.provider.is_none() {
            content=content.child(card("Coroot connection",cx).child(body()
                .child(muted("Reachable HTTP(S) URL. Credentials stay in memory until Disconnect or the window closes.",cx))
                .child(text("Server URL"))
                .child(Input::new(&self.url).id("obs-url").aria_label("Coroot server URL"))
                .child(line().flex_wrap().children(["API key","Session cookie","Anonymous"].into_iter().enumerate().map(|(ix,label)| {
                    action(SharedString::from(format!("obs-auth-{ix}")),label).selected(self.auth==ix).on_click(cx.listener(move |this,_,_,cx|{this.auth=ix;this.invalidate_connection();cx.notify();}))
                })))
                .when(self.auth!=2,|body| body.child(text(if self.auth==0 {"API key"} else {"coroot_session value"}))
                    .child(Input::new(&self.secret).id("obs-credential").aria_label("Coroot credential")))
                .child(action("obs-connect",if self.live.connecting {"Connecting…"} else {"Connect"})
                    .disabled(self.live.connecting)
                    .on_click(cx.listener(|this,_,_,cx|this.connect(cx))))
                .when_some(self.live.error.clone(),|body,error| body.child(text(error).text_color(palette(cx).crit_ink)))));
        }
        if self.live.provider.is_some() && self.live.projects.is_empty() {
            content = content.child(text("No accessible projects were returned."));
        }
        content.into_any_element()
    }
    pub(super) fn render_read_state(&self, cx: &Context<Self>) -> AnyElement {
        if self.fixture {
            return div().into_any_element();
        }
        let (loading, stale, error) = match self.destination {
            Destination::Applications => (
                self.live.apps.is_loading(),
                self.live.apps.is_stale(),
                self.live.apps.error(),
            ),
            Destination::ServiceMap => (
                self.live.map.is_loading(),
                self.live.map.is_stale(),
                self.live.map.error(),
            ),
            _ => return div().into_any_element(),
        };
        line()
            .flex_wrap()
            .when(loading, |row| row.child(muted("Reading Coroot…", cx)))
            .when(stale, |row| row.child(text("Last known data · stale")))
            .when_some(error, |row, error| {
                row.child(text(error.to_string()).text_color(palette(cx).crit_ink))
                    .child(
                        action("obs-retry", "Retry this window")
                            .on_click(cx.listener(|this, _, _, cx| this.refresh(cx))),
                    )
            })
            .into_any_element()
    }
    pub(super) fn render_limited(&self, cx: &Context<Self>) -> AnyElement {
        v_flex().id("obs-capability-limited").test_support().gap(dp(12.))
            .child(section(self.destination.label()))
            .child(text("This destination is not connected in this read-only Coroot slice."))
            .child(muted("Applications, the service map and supported application reports are available. Full report histories, CPU profiling, historical deployment comparisons and missing trace detail require additional client APIs.",cx))
            .into_any_element()
    }
}
