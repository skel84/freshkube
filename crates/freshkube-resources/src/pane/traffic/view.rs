use super::*;
use crate::palette::palette;
use crate::ui::{self, MONO_FONT, dp};
use freshkube_ui::ui::field;
use gpui_kit::component::{Sizable, button::Button};

impl DetailPane {
    /// The Traffic block of Overview, for a Service or a workload.
    pub(in crate::pane) fn traffic_section(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let view = &self.traffic;
        let scope = view.scope?;
        let title = match scope {
            Scope::Service => "Published endpoints",
            Scope::Workload => "Traffic",
            Scope::Route => return Some(self.route_section(cx)),
        };
        let p = palette(cx);
        let step = |step: &StepLine, cx: &App| {
            v_flex()
                .id(step.id.clone())
                .test_support()
                .role(Role::Status)
                .aria_label(format!("{} {}: {}", step.kind, step.subject, step.summary))
                .gap_1()
                .child(
                    h_flex()
                        .gap_2()
                        .flex_wrap()
                        .child(ui::tag(step.tone, None, step.kind, cx))
                        .child(
                            div()
                                .text_size(dp(12.))
                                .text_color(palette(cx).muted)
                                .child(format!("{} · {}", step.evidence, step.subject)),
                        )
                        .child(div().min_w_0().child(step.summary.clone())),
                )
                .children(step.detail.clone().map(|detail| {
                    div()
                        .pl(dp(8.))
                        .text_size(dp(12.))
                        .text_color(palette(cx).muted)
                        .child(detail)
                }))
        };
        let group = |group: &GroupLine, cx: &App| {
            v_flex()
                .id(group.id.clone())
                .test_support()
                .gap_1()
                .child(
                    h_flex()
                        .gap_2()
                        .child(ui::tag(group.tone, None, group.title, cx))
                        .child(format!("{}", group.count)),
                )
                .children(group.hint.map(|hint| {
                    div()
                        .id(SharedString::from(format!("{}-hint", group.id)))
                        .test_support()
                        .pl(dp(8.))
                        .text_size(dp(12.))
                        .text_color(palette(cx).muted)
                        .child(hint)
                }))
                .children(group.rows.iter().map(|(id, text)| {
                    div()
                        .id(id.clone())
                        .test_support()
                        .pl(dp(8.))
                        .font_family(MONO_FONT)
                        .text_size(dp(12.))
                        .child(text.clone())
                }))
                .when(group.more > 0, |this| {
                    this.child(
                        div()
                            .pl(dp(8.))
                            .text_size(dp(12.))
                            .text_color(palette(cx).muted)
                            .child(format!("{} more not shown", group.more)),
                    )
                })
        };
        Some(
            v_flex()
                .id("traffic-section")
                .test_support()
                .gap_3()
                .child(ui::caption(title, cx))
                .when(!view.loaded, |this| {
                    this.child(
                        div()
                            .id("traffic-loading")
                            .test_support()
                            .child("Reading Services and endpoints…"),
                    )
                })
                .children(view.steps.iter().map(|line| step(line, cx)))
                .children(
                    view.source
                        .clone()
                        .map(|source| field("Answered by", source, cx)),
                )
                .children(view.groups.iter().map(|line| group(line, cx)))
                .when(!view.pods.is_empty(), |this| {
                    this.child(ui::caption("Selected but not published", cx))
                        .children(view.pods.iter().map(|pod| {
                            h_flex()
                                .id(pod.id.clone())
                                .test_support()
                                .gap_2()
                                .flex_wrap()
                                .child(ui::tag(Tone::Warn, None, pod.reason, cx))
                                .child(div().font_family(MONO_FONT).child(pod.pod.clone()))
                                .child(
                                    div()
                                        .text_size(dp(12.))
                                        .text_color(p.muted)
                                        .child(pod.detail.clone()),
                                )
                        }))
                })
                .children(view.pods_more.clone().map(|more| {
                    div()
                        .id("traffic-pods-more")
                        .test_support()
                        .pl(dp(8.))
                        .text_size(dp(12.))
                        .text_color(p.muted)
                        .child(more)
                }))
                .children(view.pods_note.clone().map(|note| {
                    div()
                        .id("traffic-pods-unknown")
                        .test_support()
                        .child(ui::warning_banner(None, note, None, cx))
                }))
                .when(view.has_unknown(), |this| {
                    this.child(
                        h_flex().child(
                            Button::new("traffic-retry")
                                .small()
                                .label("Retry traffic")
                                .on_click(cx.listener(|pane, _, _, cx| pane.retry_traffic(cx))),
                        ),
                    )
                })
                .into_any_element(),
        )
    }

    /// A route's Overview: the verdict and where each parent stops, with a
    /// way to the full trace.
    fn route_section(&self, cx: &mut Context<Self>) -> AnyElement {
        let body = match &self.traffic.route {
            None if !self.traffic.loaded => div()
                .id("traffic-loading")
                .test_support()
                .child("Reading the route's gateways and backends…")
                .into_any_element(),
            None => div().into_any_element(),
            Some(RouteView::Message { tone, text }) => self.route_message(*tone, text, cx),
            Some(RouteView::Trace { model, notes }) => v_flex()
                .gap_2()
                .child(route_trace::compact(model, cx))
                .children(self.route_notes(notes, cx))
                .child(
                    h_flex()
                        .gap_2()
                        .child(
                            Button::new("route-open-trace")
                                .small()
                                .label("Open full trace")
                                .on_click(cx.listener(|pane, _, _, cx| {
                                    pane.set_tab(Tab::Trace, cx);
                                })),
                        )
                        .when(self.traffic.has_unknown(), |this| {
                            this.child(self.retry_button(cx))
                        }),
                )
                .into_any_element(),
        };
        v_flex()
            .id("traffic-section")
            .test_support()
            .gap_3()
            .child(ui::caption("Route trace", cx))
            .child(body)
            .into_any_element()
    }

    fn retry_button(&self, cx: &mut Context<Self>) -> Button {
        Button::new("traffic-retry")
            .small()
            .label("Retry traffic")
            .on_click(cx.listener(|pane, _, _, cx| pane.retry_traffic(cx)))
    }

    fn route_message(&self, tone: Tone, text: &SharedString, cx: &mut Context<Self>) -> AnyElement {
        v_flex()
            .id("route-trace-message")
            .test_support()
            .role(Role::Status)
            .aria_label(text.clone())
            .gap_2()
            .child(
                h_flex()
                    .gap(dp(8.))
                    .items_start()
                    .children(ui::status_glyph(tone, cx))
                    .child(
                        div()
                            .min_w_0()
                            .flex_1()
                            .whitespace_normal()
                            .child(text.clone()),
                    ),
            )
            .when(tone == Tone::Unknown, |this| {
                this.child(h_flex().child(self.retry_button(cx)))
            })
            .into_any_element()
    }

    fn route_notes(&self, notes: &[SharedString], cx: &App) -> Option<AnyElement> {
        (!notes.is_empty()).then(|| {
            v_flex()
                .id("route-trace-notes")
                .test_support()
                .gap(dp(2.))
                .text_size(dp(12.))
                .text_color(palette(cx).muted)
                .children(notes.iter().map(|note| div().child(note.clone())))
                .into_any_element()
        })
    }

    /// The Trace tab: every step of every parent and backend.
    pub(in crate::pane) fn trace_page(
        &self,
        _detail: &Detail,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let pad = dp(freshkube_ui::page::PANE_PADDING);
        let body = match &self.traffic.route {
            None => div()
                .id("traffic-loading")
                .test_support()
                .child("Reading the route's gateways and backends…")
                .into_any_element(),
            Some(RouteView::Message { tone, text }) => self.route_message(*tone, text, cx),
            Some(RouteView::Trace { model, notes }) => {
                let this = cx.entity().downgrade();
                let on_link: route_trace::OnLink = std::rc::Rc::new(move |link, _, cx| {
                    _ = this.update(cx, |pane, cx| pane.open_trace_link(link, cx));
                });
                v_flex()
                    .gap_3()
                    .child(route_trace::full(model, Some(on_link), cx))
                    .children(self.route_notes(notes, cx))
                    .when(self.traffic.has_unknown(), |this| {
                        this.child(h_flex().child(self.retry_button(cx)))
                    })
                    .into_any_element()
            }
        };
        div()
            .id("detail-trace-page")
            .test_support()
            .size_full()
            .overflow_y_scroll()
            .restrict_scroll_to_axis()
            .px(pad)
            .py_3()
            .child(body)
            .into_any_element()
    }

    /// A link from the trace goes through the normal object entry point.
    pub(super) fn open_trace_link(&mut self, link: &TraceLink, cx: &mut Context<Self>) {
        let Some(detail) = self.detail.as_ref() else {
            return;
        };
        let connection = Some(detail.target.identity.connection.clone());
        let object = |namespace: &str, name: &str| crate::model::ObjectRef {
            namespace: namespace.to_owned(),
            name: name.to_owned(),
            uid: String::new(),
            connection: connection.clone(),
        };
        let link = match link {
            TraceLink::Service { namespace, name } => ResourceLink::Object(
                freshkube_core::resources::builtin("services").expect("services is built in"),
                object(namespace, name),
                Tab::Overview,
            ),
            TraceLink::Gateway { namespace, name } => ResourceLink::Owner {
                api_version: format!("{}/{}", routes::GATEWAY_GROUP, detail.target.kind.version),
                kind: "Gateway".to_owned(),
                object: object(namespace, name),
            },
        };
        cx.emit(DetailEvent::Link(link));
    }
}
