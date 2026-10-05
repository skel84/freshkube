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
            Destination::Incidents => (
                self.live.incidents.data().is_some(),
                self.live.incidents.is_loading(),
                self.live.incidents.error(),
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
        let connected = self.live.provider.is_some();
        if connected {
            content = content.child(self.connection_line(cx));
        }
        if connected && !self.settings_open {
            if self.live.projects.is_empty() {
                content = content.child(text("No accessible projects were returned."));
            }
            return content.into_any_element();
        }
        if let Some(source) = &self.live.source {
            let linked = source.association().map(|association| {
                format!(
                    "Coroot cluster {} is linked to the selected Kubernetes access, so reports open its objects.",
                    association.cluster()
                )
            });
            content = content.child(
                line()
                    .flex_wrap()
                    .child(muted(
                        linked.unwrap_or_else(|| {
                            "Kubernetes links are unmapped: reports can't open objects.".into()
                        }),
                        cx,
                    ))
                    .when(source.association().is_some(), |row| {
                        row.child(
                            action("obs-unmap", "Remove link")
                                .on_click(cx.listener(|this, _, _, cx| this.associate(None, cx))),
                        )
                    })
                    .children(self.associate_menu(cx)),
            );
        }
        if self.settings_open || self.live.provider.is_none() {
            let p = palette(cx);
            let auth = line()
                .gap(dp(2.))
                .p(dp(3.))
                .rounded(px(8.))
                .bg(p.surface_2)
                .children(
                    ["API key", "Session cookie", "Anonymous"]
                        .into_iter()
                        .enumerate()
                        .map(|(ix, label)| {
                            ui::segment(
                                Button::new(SharedString::from(format!("obs-auth-{ix}"))),
                                self.auth == ix,
                                cx,
                            )
                            .small()
                            .label(label)
                            .on_click(cx.listener(
                                move |this, _, _, cx| {
                                    this.auth = ix;
                                    this.invalidate_connection();
                                    cx.notify();
                                },
                            ))
                        }),
                );
            let field = |label: &'static str| ui::caption(label, cx);
            content = content.child(
                card("Coroot connection", cx).max_w(dp(560.)).child(
                    body()
                        .child(field("Server URL"))
                        .child(
                            Input::new(&self.url)
                                .id("obs-url")
                                .aria_label("Coroot server URL"),
                        )
                        .child(field("Sign in with"))
                        .child(line().child(auth))
                        .when(self.auth != 2, |body| {
                            body.child(field(if self.auth == 0 {
                                "API key"
                            } else {
                                "coroot_session value"
                            }))
                            .child(
                                Input::new(&self.secret)
                                    .id("obs-credential")
                                    .aria_label("Coroot credential"),
                            )
                        })
                        .children(self.render_memory(cx))
                        .when_some(self.live.error.clone(), |body, error| {
                            body.child(text(error).text_color(p.crit_ink))
                        })
                        .child(
                            line()
                                .pt(dp(4.))
                                .child(
                                    Button::new("obs-connect")
                                        .primary()
                                        .small()
                                        .label(if self.live.connecting {
                                            "Connecting…"
                                        } else {
                                            "Connect"
                                        })
                                        .disabled(self.live.connecting)
                                        .on_click(cx.listener(|this, _, _, cx| this.connect(cx))),
                                )
                                .child(
                                    Button::new("obs-disconnect")
                                        .ghost()
                                        .small()
                                        .label(if connected { "Disconnect" } else { "Clear" })
                                        .on_click(cx.listener(|this, _, window, cx| {
                                            this.disconnect(window, cx)
                                        })),
                                )
                                .child(muted(self.credential_note(), cx)),
                        ),
                ),
            );
        }
        if self.live.provider.is_some() && self.live.projects.is_empty() {
            content = content.child(text("No accessible projects were returned."));
        }
        content.into_any_element()
    }
    /// One line while connected: where the data comes from, the window it
    /// covers, and the two things people do often. The rest of the connection
    /// waits behind Connection….
    fn connection_line(&self, cx: &Context<Self>) -> Div {
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
        let host = self.live.provider.as_ref().map_or("Coroot".into(), |p| {
            let url = p.url();
            let url = url.split_once("://").map_or(url, |(_, rest)| rest);
            url.trim_end_matches('/').to_owned()
        });
        let unmapped = self.live.access.is_some()
            && self
                .live
                .source
                .as_ref()
                .is_some_and(|s| s.association().is_none());
        line()
            .flex_wrap()
            .child(status(Status::Integration, cx))
            .child(text(host).font_weight(FontWeight::SEMIBOLD))
            .child(picker)
            .child(muted(self.live.range_label.clone(), cx))
            .child(div().flex_1())
            .when(unmapped && !self.settings_open, |row| {
                row.children(self.associate_menu(cx))
            })
            .child(
                action("obs-refresh", "Refresh")
                    .disabled(self.live.source.is_none())
                    .on_click(cx.listener(|this, _, _, cx| this.refresh_current(cx))),
            )
            .child(
                action(
                    "obs-connect-settings",
                    if self.settings_open {
                        "Done"
                    } else {
                        "Connection…"
                    },
                )
                .on_click(cx.listener(|this, _, _, cx| {
                    this.settings_open = !this.settings_open;
                    cx.notify();
                })),
            )
    }
    /// Links a Coroot cluster to the Kubernetes access Freshkube uses.
    fn associate_menu(&self, cx: &Context<Self>) -> Option<AnyElement> {
        self.live.access.as_ref()?;
        let owner = cx.entity().downgrade();
        let clusters = self.cluster_ids.clone();
        Some(
            action("obs-associate", "Link Kubernetes cluster…")
                .disabled(clusters.is_empty())
                .dropdown_caret(true)
                .dropdown_menu(move |mut menu, _, _| {
                    for cluster in &clusters {
                        let (owner, cluster) = (owner.clone(), cluster.clone());
                        menu = menu.item(
                            PopupMenuItem::new(format!("{cluster} → selected Freshkube cluster"))
                                .on_click(move |_, _, cx| {
                                    _ = owner.update(cx, |this, cx| {
                                        this.associate(Some(cluster.clone()), cx)
                                    });
                                }),
                        );
                    }
                    menu
                })
                .into_any_element(),
        )
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
            Destination::Incidents => (
                self.live.incidents.is_loading(),
                self.live.incidents.is_stale(),
                self.live.incidents.error(),
            ),
            Destination::Traces => (
                self.live.tracing.is_loading(),
                self.live.tracing.is_stale(),
                self.live.tracing.error(),
            ),
            Destination::Profiling => (
                self.live.profiling.is_loading(),
                self.live.profiling.is_stale(),
                self.live.profiling.error(),
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
            .child(muted("Applications, the service map, incidents, traces, profiling and supported application reports are available. Full report histories and historical deployment comparisons require additional client APIs.",cx))
            .into_any_element()
    }
}
