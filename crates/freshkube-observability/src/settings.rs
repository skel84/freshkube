use super::*;

impl ObservabilityPage {
    pub(super) fn read_placeholder(&self, cx: &Context<Self>) -> Option<AnyElement> {
        // A table shows its own loading rows.
        if self.fixture || self.first_read() {
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
            Destination::Deployments => (
                self.live.view.data().is_some(),
                self.live.view.is_loading(),
                self.live.view.error().or_else(|| self.live.apps.error()),
            ),
            _ => return None,
        };
        if has_data
            || (matches!(
                self.destination,
                Destination::Traces | Destination::Profiling | Destination::Deployments
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
        let refused = match self.destination {
            Destination::Applications => matches!(
                self.live.capabilities[0],
                freshkube_core::coroot::Capability::Unavailable(
                    freshkube_core::coroot::ReadError::Refused
                )
            ),
            // The revisions come from the application's page.
            Destination::Deployments => self.live.view_refused && self.live.view.error().is_some(),
            _ => false,
        };
        let title = if refused && self.destination == Destination::Deployments {
            "Not permitted to read this application".to_owned()
        } else if refused {
            "Not permitted to list applications".to_owned()
        } else {
            format!("Couldn't read {}", self.destination.label().to_lowercase())
        };
        Some(
            ui::empty_state(
                if refused { IconName::Shield } else { IconName::TriangleAlert },
                title,
                "No successful observation is available. A failed read does not establish that this project is empty.",
                error.cloned(),
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
    /// The header's Coroot connection button: the connected page's link
    /// panel, or Settings' Coroot section while nothing is connected.
    pub fn show_connection(&mut self, cx: &mut Context<Self>) {
        if self.live.provider.is_none() {
            cx.emit(ObservabilityEvent::OpenSettings);
            return;
        }
        self.settings_open = true;
        cx.notify();
    }
    pub(super) fn render_connection(&self, cx: &Context<Self>) -> AnyElement {
        if self.fixture {
            return div().into_any_element();
        }
        if self.live.provider.is_none() {
            return self.render_not_connected(cx);
        }
        let mut content = v_flex().gap(dp(12.));
        if !self.settings_open {
            if self.live.projects.is_empty() {
                content = content.child(text("No accessible projects were returned."));
            }
            return content.into_any_element();
        }
        content = content.child(self.render_entry_link(cx));
        if self.live.projects.is_empty() {
            content = content.child(text("No accessible projects were returned."));
        }
        content
            .child(
                line()
                    .child(
                        action("obs-connect-open-settings", "Open Settings").on_click(
                            cx.listener(|_, _, _, cx| cx.emit(ObservabilityEvent::OpenSettings)),
                        ),
                    )
                    .child(action("obs-connect-settings", "Done").on_click(cx.listener(
                        |this, _, _, cx| {
                            this.settings_open = false;
                            cx.notify();
                        },
                    ))),
            )
            .into_any_element()
    }
    /// The connection card when nothing is connected: one line that opens
    /// Settings' Coroot section, where the connection is edited.
    fn render_not_connected(&self, cx: &Context<Self>) -> AnyElement {
        let p = palette(cx);
        let lead = if self.live.connecting {
            "Connecting to Coroot…"
        } else {
            "Coroot isn't connected"
        };
        v_flex()
            .id("obs-not-connected")
            .test_support()
            .role(Role::Status)
            .gap(dp(6.))
            .child(
                line().child(muted(lead, cx)).child(muted("·", cx)).child(
                    action("obs-open-settings", "Open Settings").on_click(
                        cx.listener(|_, _, _, cx| cx.emit(ObservabilityEvent::OpenSettings)),
                    ),
                ),
            )
            .when_some(self.live.error.clone(), |this, error| {
                this.child(text(error).text_color(p.crit_ink))
            })
            .into_any_element()
    }
    /// The open entry's Coroot link: the cluster its Kubernetes links read,
    /// or why it has none. It is edited in the setup file.
    pub(super) fn render_entry_link(&self, cx: &Context<Self>) -> AnyElement {
        let note = self.link_note();
        let linked = match (&note, self.entry_link.link()) {
            (None, Some(link)) => Some(format!(
                "Coroot cluster {} is linked to “{}”, so reports open its objects.",
                link.cluster(),
                link.entry()
            )),
            _ => None,
        };
        line()
            .id("obs-entry-link")
            .test_support()
            .flex_wrap()
            .child(muted(note.or(linked).unwrap_or_default(), cx))
            .into_any_element()
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
            Destination::Application | Destination::Deployments => (
                self.live.view.is_loading(),
                self.live.view.is_stale(),
                self.live.view.error(),
            ),
        };
        if stale {
            let last = match self.destination {
                Destination::Applications => self.live.apps.last_successful(),
                Destination::ServiceMap => self.live.map.last_successful(),
                Destination::Incidents => self.live.incidents.last_successful(),
                Destination::Traces => self.live.tracing.last_successful(),
                Destination::Profiling => self.live.profiling.last_successful(),
                Destination::Application | Destination::Deployments => {
                    self.live.view.last_successful()
                }
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
                    error
                        .map(String::as_str)
                        .unwrap_or("This observation is out of date."),
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
}
