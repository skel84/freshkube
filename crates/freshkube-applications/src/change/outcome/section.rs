//! The **After the deploy** section of a Deployment hop's Inspector: what
//! Coroot found after the deploy, in every state of the outcome. Derived
//! when the selection or an answer changes; drawing only lays it out.
//!
//! No state reads as fine: an outcome that isn't a found revision is a
//! banner with its why (DESIGN.md, "After the deploy").
use std::rc::Rc;

use freshkube_core::coroot as api;
use freshkube_observability::After;
use freshkube_ui::palette::palette;
use freshkube_ui::platform::OpenSettings;
use freshkube_ui::ui::Tone;
use gpui_kit::component::{Sizable, button::Button, h_flex, v_flex};
use gpui_kit::*;

use super::{Coroot, Memo, ask_of};
use crate::change::{ChangePage, clock};
use crate::ui::{self, dp};

/// The look a banner's button leads to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::change) enum Act {
    Retry,
    Settings,
}

/// The section, as it is drawn.
pub(in crate::change) struct Section {
    state: State,
    /// `Read from Coroot · 14:07`, when something was read.
    footer: Option<SharedString>,
}

enum State {
    /// The first read is in flight: the section's heading over loading rows.
    Reading,
    Found {
        after: Rc<After>,
        /// A later read failed: the banner above the last answer.
        stale: Option<SharedString>,
    },
    Banner {
        tone: Tone,
        lead: SharedString,
        body: SharedString,
        act: Option<Act>,
    },
}

/// The names a sentence of the section uses.
struct Words {
    namespace: String,
    name: String,
    hash: String,
    /// The workspace entry the Deployment is on.
    entry: String,
    /// The Coroot project and cluster of its link, when it has one.
    project: Option<String>,
    cluster: Option<String>,
}

impl Section {
    /// The state in words, for tests.
    #[cfg(any(test, feature = "testing"))]
    pub(in crate::change) fn words(&self) -> String {
        match &self.state {
            State::Reading => "Reading".into(),
            State::Found { after, stale } => match stale {
                Some(stale) => format!("{} · {stale}", after.revision_line()),
                None => after.revision_line().to_owned(),
            },
            State::Banner { lead, body, .. } => format!("{lead} {body}"),
        }
    }
}

impl ChangePage {
    /// The section of the hop `hop`, when it is a Deployment's, derived from
    /// what was read.
    pub(in crate::change) fn section(&self, hop: &str) -> Option<Section> {
        let ask = ask_of(&self.change, hop)?;
        if matches!(self.outcomes.coroot, Coroot::None) {
            return None;
        }
        let plan = self.query_of(hop);
        let link = plan
            .as_ref()
            .and_then(|plan| plan.as_ref().ok())
            .map(|(query, _)| &query.link);
        let words = Words {
            namespace: ask.object.namespace.clone(),
            name: ask.object.name.clone(),
            hash: ask.hash.clone().unwrap_or_default(),
            entry: self.cluster_name(&ask.object.cluster).to_owned(),
            project: link.map(|link| link.project().to_owned()),
            cluster: link.map(|link| link.cluster().to_owned()),
        };
        // What is decided without asking Coroot.
        if let Some(Err((outcome, _))) = plan {
            return Some(banner(&outcome, &words, ask.why.as_deref()));
        }
        if let Some(memo) = self.outcomes.memo.get(hop) {
            return Some(self.remembered(memo, &words));
        }
        if self.outcomes.no_connection {
            return Some(banner(&api::Outcome::NoConnection, &words, None));
        }
        // Still reading, or not yet asked: loading rows either way, since
        // a read starts the moment a shown page selects the hop.
        Some(Section {
            state: State::Reading,
            footer: None,
        })
    }

    fn remembered(&self, memo: &Memo, words: &Words) -> Section {
        let footer = Some(SharedString::from(format!(
            "Read from Coroot · {}",
            clock(memo.read)
        )));
        let mut section = match &memo.after {
            Some(after) => Section {
                state: State::Found {
                    after: after.clone(),
                    stale: memo.stale.as_ref().map(|why| {
                        SharedString::from(format!(
                            "Showing the last read at {}. {why}",
                            clock(memo.read)
                        ))
                    }),
                },
                footer: None,
            },
            None => banner(&memo.outcome, words, None),
        };
        section.footer = footer;
        section
    }
}

/// A banner for an outcome that isn't a found revision.
fn banner(outcome: &api::Outcome, words: &Words, why: Option<&str>) -> Section {
    let ns_name = format!("{}/{}", words.namespace, words.name);
    let (tone, lead, body, act): (Tone, String, String, Option<Act>) = match outcome {
        api::Outcome::Found(_) | api::Outcome::EndsEarly(..) => {
            unreachable!("a found revision has its content, not a banner")
        }
        api::Outcome::NoRevision => (
            Tone::Warn,
            format!("Coroot keeps no revision {} of {ns_name}.", words.hash),
            "It keeps a Deployment's last 100.".into(),
            None,
        ),
        api::Outcome::NoApplication => (
            Tone::Warn,
            format!(
                "Coroot lists no application {ns_name} in {}.",
                words.cluster.as_deref().unwrap_or("its cluster")
            ),
            "The application may not be observed, or its name differs in Coroot.".into(),
            None,
        ),
        api::Outcome::NoProject => (
            Tone::Warn,
            match &words.project {
                Some(project) => format!("Coroot answered not found for project {project}."),
                None => "Coroot answered not found for the entry's project.".into(),
            },
            format!(
                "The project is named by the coroot key of {}, else by the connection's. \
                 A wrong proxy path or an old Coroot answers the same.",
                words.entry
            ),
            Some(Act::Settings),
        ),
        api::Outcome::NoConnection => (
            Tone::Warn,
            "Coroot isn't connected.".into(),
            "Connect it in Settings to read what Coroot found after this deploy.".into(),
            Some(Act::Settings),
        ),
        api::Outcome::NotLinked => (
            Tone::Warn,
            format!("No Coroot cluster is linked to {}.", words.entry),
            "Name its Coroot cluster in the entry's coroot key.".into(),
            Some(Act::Settings),
        ),
        api::Outcome::NoProjectForEntry => (
            Tone::Warn,
            format!(
                "{} names no Coroot project, and the connection names none.",
                words.entry
            ),
            "Name a project in the entry's coroot key or in the connection's.".into(),
            Some(Act::Settings),
        ),
        api::Outcome::Refused(read) => (
            Tone::Crit,
            match read {
                api::Refused::Application => "Not permitted to read this application.".into(),
                api::Refused::Window => "Not permitted to read this application's window.".into(),
            },
            "Coroot refused the read. Check this user's project permissions.".into(),
            Some(Act::Retry),
        ),
        api::Outcome::NoData => (
            Tone::Info,
            "Coroot has no data for this window.".into(),
            "Coroot keeps no metrics of this application from then: the window is older than \
             Coroot keeps, newer than its latest data, or the application sent none in it."
                .into(),
            None,
        ),
        api::Outcome::Failed(error) => (
            Tone::Crit,
            "Couldn't read Coroot.".into(),
            error.to_string(),
            Some(Act::Retry),
        ),
        api::Outcome::NotDeployment => (
            Tone::Info,
            "Coroot keeps revisions of Deployments only.".into(),
            format!("{} is an Argo Rollout.", words.name),
            None,
        ),
        api::Outcome::NotLooked => (
            Tone::Warn,
            "Not looked for.".into(),
            why.map(str::to_owned).unwrap_or_else(|| {
                "The Deployment's current ReplicaSet isn't known, so its revision may be another change's."
                    .into()
            }),
            None,
        ),
    };
    Section {
        state: State::Banner {
            tone,
            lead: lead.into(),
            body: body.into(),
            act,
        },
        footer: None,
    }
}

impl Section {
    pub(in crate::change) fn render(&self, cx: &mut Context<ChangePage>) -> AnyElement {
        let heading = ui::caption("After the deploy", cx);
        let body = match &self.state {
            State::Reading => v_flex()
                .id("change-after-loading")
                .test_support()
                .role(Role::Status)
                .aria_label("Reading what Coroot found after the deploy")
                .gap(dp(8.))
                .child(ui::skeleton(relative(0.4), dp(12.)))
                .child(ui::skeleton(relative(0.7), dp(12.)))
                .into_any_element(),
            State::Found { after, stale } => v_flex()
                .gap(dp(12.))
                .children(stale.as_ref().map(|why| {
                    ui::banner(
                        Tone::Warn,
                        Some("Couldn't read again.".into()),
                        why.clone(),
                        Some(act_button(Act::Retry, cx)),
                        cx,
                    )
                    .id("change-after-stale")
                    .test_support()
                }))
                .child(after.render("change-after", cx))
                .child(
                    muted(
                        "Another window, or the other reports: Compare in Observability.",
                        cx,
                    )
                    .whitespace_normal(),
                )
                .into_any_element(),
            State::Banner {
                tone,
                lead,
                body,
                act,
            } => v_flex()
                .child(
                    ui::banner(
                        *tone,
                        Some(lead.clone()),
                        body.clone(),
                        act.map(|act| act_button(act, cx)),
                        cx,
                    )
                    .id("change-after-banner")
                    .test_support()
                    .role(Role::Status),
                )
                .into_any_element(),
        };
        v_flex()
            .id("change-after")
            .test_support()
            .gap(dp(7.))
            .text_size(dp(12.))
            .child(heading)
            .child(body)
            .children(self.footer.as_ref().map(|footer| {
                muted(footer.clone(), cx)
                    .id("change-after-footer")
                    .test_support()
            }))
            .into_any_element()
    }
}

fn act_button(act: Act, cx: &mut Context<ChangePage>) -> AnyElement {
    let (id, label) = match act {
        Act::Retry => ("change-after-retry", "Retry"),
        Act::Settings => ("change-after-settings", "Open Settings"),
    };
    h_flex()
        .child(
            Button::new(id)
                .outline()
                .xsmall()
                .label(label)
                .on_click(cx.listener(move |this, _, window, cx| match act {
                    Act::Retry => this.retry_outcome(cx),
                    Act::Settings => window.dispatch_action(Box::new(OpenSettings), cx),
                })),
        )
        .into_any_element()
}

fn muted(text: impl Into<SharedString>, cx: &App) -> Div {
    div().text_color(palette(cx).muted).child(text.into())
}
