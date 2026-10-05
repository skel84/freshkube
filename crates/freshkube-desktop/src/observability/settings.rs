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
            Destination::Traces => (
                self.live.tracing.data().is_some(),
                self.live.tracing.is_loading(),
                self.live.tracing.error().or_else(|| self.live.apps.error()),
            ),
            Destination::Profiling => (
                self.live.profiling.data().is_some(),
                self.live.profiling.is_loading(),
                self.live
                    .profiling
                    .error()
                    .or_else(|| self.live.apps.error()),
            ),
            _ => return None,
        };
        if has_data
            || (matches!(
                self.destination,
                Destination::Traces | Destination::Profiling
            ) && !loading
                && error.is_none())
        {
            return None;
        }
        if loading {
            return Some(
                freshkube_ui::page::card(cx)
                    .id("obs-loading")
                    .test_support()
                    .role(Role::Status)
                    .aria_label("Reading Coroot observations")
                    .p(dp(14.))
                    .gap(dp(12.))
                    .children((0..9).map(|_| ui::skeleton(relative(0.7), dp(12.))))
                    .into_any_element(),
            );
        }
        let refused = self.destination == Destination::Applications
            && matches!(
                self.live.capabilities[0],
                freshkube_core::coroot::Capability::Unavailable(
                    freshkube_core::coroot::ReadError::Refused
                )
            );
        let title = if refused {
            "Not permitted to list applications".to_owned()
        } else {
            format!("Couldn't read {}", self.destination.label().to_lowercase())
        };
        Some(
            ui::empty_state(
                if refused { IconName::Shield } else { IconName::TriangleAlert },
                title,
                "No successful observation is available. A failed read does not establish that this project is empty.",
                error.map(str::to_owned),
                vec![action("obs-retry", "Retry")
                    .on_click(cx.listener(|this, _, _, cx| this.refresh(cx)))
                    .into_any_element()],
                cx,
            )
            .id(if refused { "obs-refused" } else { "obs-failed" })
            .test_support()
            .role(Role::Status)
            .h_auto()
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
        content
            .when(self.settings_open, |content| {
                content.child(action("obs-connect-settings", "Done").on_click(cx.listener(
                    |this, _, _, cx| {
                        this.settings_open = false;
                        cx.notify();
                    },
                )))
            })
            .into_any_element()
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
    pub(super) fn render_read_state(&self, cx: &Context<Self>) -> Option<AnyElement> {
        if self.fixture {
            return None;
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
            Destination::Application => (
                self.live.view.is_loading(),
                self.live.view.is_stale(),
                self.live.view.error(),
            ),
            _ => return None,
        };
        if stale {
            let last = match self.destination {
                Destination::Applications => self.live.apps.last_successful(),
                Destination::ServiceMap => self.live.map.last_successful(),
                Destination::Incidents => self.live.incidents.last_successful(),
                Destination::Traces => self.live.tracing.last_successful(),
                Destination::Profiling => self.live.profiling.last_successful(),
                Destination::Application => self.live.view.last_successful(),
                _ => None,
            };
            let lead = last.map(|time| {
                format!(
                    "Showing {} as last seen at {}.",
                    self.destination.label().to_lowercase(),
                    ui::clock(time)
                )
                .into()
            });
            return Some(
                ui::warning_banner(
                    lead,
                    error.unwrap_or("This observation is out of date."),
                    Some(
                        action("obs-retry", "Retry")
                            .on_click(cx.listener(|this, _, _, cx| this.refresh(cx)))
                            .into_any_element(),
                    ),
                    cx,
                )
                .id("obs-stale")
                .test_support()
                .role(Role::Status)
                .into_any_element(),
            );
        }
        if loading {
            return Some(
                text("Reading Coroot…")
                    .id("obs-refreshing")
                    .test_support()
                    .role(Role::Status)
                    .into_any_element(),
            );
        }
        None
    }
    pub(super) fn render_limited(&self, cx: &Context<Self>) -> AnyElement {
        v_flex().id("obs-capability-limited").test_support().gap(dp(12.))
            .child(text("This destination is not connected in this read-only Coroot slice."))
            .child(muted("Applications, the service map, incidents, traces, profiling and supported application reports are available. Full report histories and historical deployment comparisons require additional client APIs.",cx))
            .into_any_element()
    }
}
