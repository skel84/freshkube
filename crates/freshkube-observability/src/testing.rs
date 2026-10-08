//! Read-outs and set-ups for tests in crates that embed the page, behind the
//! `testing` feature, which they turn on from their dev-dependencies.

use crate::{ObservabilityEvent, ObservabilityPage, Report};

impl ObservabilityPage {
    /// Configure a sanitized Coroot subject for the shell's navigation regression.
    pub fn fixture_link(
        &mut self,
        access: String,
        namespace: &str,
        name: &str,
        cx: &mut gpui_kit::Context<Self>,
    ) -> ObservabilityEvent {
        use freshkube_core::coroot as api;
        let provider =
            api::Provider::new("https://coroot.example.com", api::Credentials::None).unwrap();
        let source = provider
            .source(&api::ProjectInfo {
                id: "fixture-project".into(),
                name: "Fixture".into(),
            })
            .with_association(Some(api::Association::new(
                access.clone(),
                "fixture".into(),
            )));
        let app = api::AppId::new(format!("fixture:{namespace}:Pod:{name}"));
        let subject = api::ObjectSubject::for_app(&source, &app).unwrap();
        self.live.access = Some(access);
        self.live.source = Some(source.clone());
        self.live.provider = Some(provider);
        self.open_app(app.clone(), Report::Net, cx);
        ObservabilityEvent::OpenObject {
            source,
            app,
            subject: Box::new(subject),
        }
    }
}
