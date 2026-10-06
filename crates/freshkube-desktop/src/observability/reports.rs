//! The Application page's frame: its actions above Coroot's own view, and
//! the Kubernetes object the application links to.
use super::*;
use freshkube_core::coroot as api;

pub(super) struct ReportSnapshot {
    subject: Option<api::ObjectSubject>,
}

impl ObservabilityPage {
    pub(super) fn prepare_report(&mut self) {
        self.prepare_view();
        let Some(id) = &self.selected_app else {
            self.report_snapshot = None;
            return;
        };
        self.report_snapshot = Some(ReportSnapshot {
            subject: self
                .live
                .source
                .as_ref()
                .and_then(|s| api::ObjectSubject::for_app(s, id)),
        });
    }
    pub(super) fn render_report(&self, window: &Window, cx: &mut Context<Self>) -> AnyElement {
        let Some(id) = &self.selected_app else {
            return text("Select an application to inspect its reports").into_any_element();
        };
        let mut actions = line().flex_wrap();
        if self.fixture && *id == example::id(example::WORKER) {
            actions =
                actions
                    .child(action("obs-open-pod", "Example pod").on_click(cx.listener(
                        |_, _, _, cx| {
                            cx.emit(ObservabilityEvent::OpenExamplePod {
                                namespace: "payments".into(),
                                name: example::POD.into(),
                                logs: false,
                            })
                        },
                    )))
                    .child(
                        action("obs-open-logs", "Example pod logs").on_click(cx.listener(
                            |_, _, _, cx| {
                                cx.emit(ObservabilityEvent::OpenExamplePod {
                                    namespace: "payments".into(),
                                    name: example::POD.into(),
                                    logs: true,
                                })
                            },
                        )),
                    )
                    .child(action("obs-threshold", "Example threshold…").on_click(
                        cx.listener(|this, _, window, cx| this.edit_threshold(window, cx)),
                    ))
                    .child(action("obs-app-rollback", "Preview rollback…").on_click(
                        cx.listener(|this, _, window, cx| this.preview("Roll back", window, cx)),
                    ));
        }
        actions = actions
            .child(
                action("obs-app-traces", "Traces")
                    .on_click(cx.listener(|this, _, _, cx| this.open(Destination::Traces, cx))),
            )
            .child(
                action("obs-app-profiling", "Profiling")
                    .on_click(cx.listener(|this, _, _, cx| this.open(Destination::Profiling, cx))),
            );
        let mut content = v_flex().gap(dp(14.));
        let Some(snapshot) = &self.report_snapshot else {
            return content
                .child(line().flex_wrap().justify_end().child(actions))
                .child(muted("Reading application reports…", cx))
                .into_any_element();
        };
        if let Some(subject) = &snapshot.subject {
            let subject = subject.clone();
            let source = self
                .live
                .source
                .clone()
                .expect("a subject has an associated source");
            let app = id.clone();
            actions = actions.child(
                action("obs-open-object", "Open Kubernetes object").on_click(cx.listener(
                    move |_, _, _, cx| {
                        cx.emit(ObservabilityEvent::OpenObject {
                            source: source.clone(),
                            app: app.clone(),
                            subject: Box::new(subject.clone()),
                        });
                    },
                )),
            );
        }
        content = content
            .child(line().flex_wrap().justify_end().child(actions))
            .child(self.render_app_view(window, cx));
        if snapshot.subject.is_none() && !self.fixture {
            content = content.child(muted(
                "No Kubernetes link: the application is external, of an unsupported kind, or its cluster isn't associated.",
                cx,
            ));
        }
        content.into_any_element()
    }
}
