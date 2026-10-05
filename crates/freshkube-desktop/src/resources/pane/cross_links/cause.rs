//! What a pod's Overview opens on: why it fails, waits or isn't ready, and
//! its container's instances over time. Both are derived with the other
//! relationships when the document arrives.

use chrono::{DateTime, Local, Utc};
use freshkube_core::resources::{Diagnosis, Instance, PodStatus, Severity};
use gpui_kit::component::{
    Sizable,
    button::{Button, ButtonVariants},
    h_flex, v_flex,
};

use super::*;
use crate::palette::palette;
use crate::resources::model::format_age;
use crate::resources::rows::died;
use crate::ui::{self, MONO_FONT, Tone, dp};

/// The card above everything else when the status says something is wrong.
pub(in crate::resources::pane) struct CauseCard {
    title: SharedString,
    tone: Tone,
    state: SharedString,
    facts: Vec<(SharedString, SharedString)>,
    message: Option<SharedString>,
    /// The last instance's termination message, which often holds the error.
    output: Option<SharedString>,
    /// The container whose logs explain it, and whether it has a crashed
    /// instance to read.
    pub(in crate::resources::pane) container: Option<String>,
    pub(in crate::resources::pane) previous: bool,
}

/// One container's known instances along a line from the first to now, or
/// to the next restart.
pub(super) struct Timeline {
    title: SharedString,
    /// Start and width as fractions of the line, and how each run ended.
    runs: Vec<(f32, f32, Tone)>,
    next: Option<f32>,
    from: SharedString,
    to: SharedString,
    legend: Vec<(Tone, SharedString)>,
}

fn clock(time: DateTime<Utc>) -> String {
    time.with_timezone(&Local).format("%H:%M:%S").to_string()
}

fn ago(time: DateTime<Utc>, now: DateTime<Utc>) -> String {
    let seconds = (now - time).num_seconds();
    if seconds < 1 {
        "just now".into()
    } else {
        format!("{} ago", format_age(seconds as u64))
    }
}

fn ran(instance: &Instance) -> Option<String> {
    let (started, finished) = (instance.started?, instance.finished?);
    Some(format!(
        "ran {}",
        format_age((finished - started).num_seconds().max(0) as u64)
    ))
}

/// How an instance ended: `exit 1 · Error · 3m ago, ran 12s`.
fn ending(instance: &Instance, now: DateTime<Utc>) -> String {
    let mut parts = vec![format!("exit {}", instance.exit_code)];
    if !instance.reason.is_empty() {
        parts.push(instance.reason.clone());
    }
    if let Some(finished) = instance.finished {
        parts.push(ago(finished, now));
    }
    let mut text = parts.join(" · ");
    if let Some(ran) = ran(instance) {
        text.push_str(&format!(", {ran}"));
    }
    text
}

impl CauseCard {
    pub(super) fn new(
        diagnosis: &Diagnosis,
        containers: Option<&freshkube_core::resources::PodContainers>,
        now: DateTime<Utc>,
    ) -> Self {
        let (title, tone) = match diagnosis.severity {
            Severity::Failing if died(&diagnosis.state) => ("Why it's failing", Tone::Died),
            Severity::Failing => ("Why it's failing", Tone::Crit),
            Severity::Waiting => ("Why it's waiting", Tone::Unknown),
            Severity::NotReady => ("Why it isn't ready", Tone::Warn),
        };
        let mut facts = Vec::new();
        if let Some(container) = &diagnosis.container {
            facts.push(("Container".into(), container.clone().into()));
        }
        if let Some(last) = &diagnosis.last {
            facts.push(("Last exit".into(), ending(last, now).into()));
        }
        if diagnosis.restarts > 0 {
            facts.push(("Restarts".into(), diagnosis.restarts.to_string().into()));
        }
        if let Some(next) = diagnosis.next_restart {
            let when = if next > now {
                format!(
                    "in about {} ({})",
                    format_age((next - now).num_seconds() as u64),
                    clock(next)
                )
            } else {
                format!("due since {}", clock(next))
            };
            facts.push(("Next restart".into(), when.into()));
        }
        let output = diagnosis
            .last
            .as_ref()
            .map(|last| last.message.as_str())
            .filter(|message| !message.is_empty() && *message != diagnosis.message)
            .map(|message| message.to_owned().into());
        let previous = diagnosis.container.as_ref().is_some_and(|name| {
            containers
                .and_then(|containers| containers.get(name))
                .is_some_and(|container| container.has_previous())
        });
        Self {
            title: title.into(),
            tone,
            state: diagnosis.state.clone().into(),
            facts,
            message: (!diagnosis.message.is_empty()).then(|| diagnosis.message.clone().into()),
            output,
            container: diagnosis.container.clone(),
            previous,
        }
    }
}

impl Timeline {
    /// The diagnosed container's runs, else the one restarted most; none
    /// for a container that has run once and runs still.
    pub(super) fn new(
        status: &PodStatus,
        diagnosis: Option<&Diagnosis>,
        now: DateTime<Utc>,
    ) -> Option<Self> {
        let container = match diagnosis.and_then(|diagnosis| diagnosis.container.as_deref()) {
            Some(name) => status.container(name)?,
            None => status
                .containers
                .iter()
                .filter(|container| container.restarts > 0)
                .max_by_key(|container| container.restarts)?,
        };
        let last = container.last.as_ref();
        if container.restarts == 0 && last.is_none() {
            return None;
        }
        let next = diagnosis.and_then(|diagnosis| diagnosis.next_restart);
        // Each run as its start, its end, and how it ended.
        let mut runs: Vec<(DateTime<Utc>, DateTime<Utc>, Tone)> = Vec::new();
        let mut legend = Vec::new();
        if let Some(last) = last
            && let Some(finished) = last.finished
        {
            let started = last.started.unwrap_or(finished);
            let tone = if last.exit_code == 0 {
                Tone::Good
            } else {
                Tone::Crit
            };
            runs.push((started, finished, tone));
            legend.push((
                tone,
                format!(
                    "Last instance {}–{}: {}",
                    clock(started),
                    clock(finished),
                    ending(last, now)
                )
                .into(),
            ));
        }
        if let Some(since) = container.running_since {
            runs.push((since, now, Tone::Good));
            legend.push((Tone::Good, format!("Running since {}", clock(since)).into()));
        } else if let Some(ended) = &container.ended
            && let Some(finished) = ended.finished
        {
            let started = ended.started.unwrap_or(finished);
            runs.push((started, finished, Tone::Crit));
            legend.push((Tone::Crit, format!("Ended: {}", ending(ended, now)).into()));
        } else if let Some((reason, _)) = &container.waiting {
            let since = last.and_then(|last| last.finished);
            legend.push((
                Tone::Unknown,
                match since {
                    Some(since) => format!("{reason} since {}", clock(since)),
                    None => reason.clone(),
                }
                .into(),
            ));
        }
        if let Some(next) = next {
            legend.push((
                Tone::Unknown,
                format!("Next restart about {}", clock(next)).into(),
            ));
        }
        let from = runs.iter().map(|run| run.0).min()?;
        let to = next.map_or(now, |next| next.max(now));
        let span = (to - from).num_milliseconds().max(1) as f32;
        let at =
            |time: DateTime<Utc>| ((time - from).num_milliseconds() as f32 / span).clamp(0., 1.);
        let restarts = match container.restarts {
            1 => "1 restart".to_owned(),
            count => format!("{count} restarts"),
        };
        Some(Self {
            title: format!("{} · {restarts}", container.name).into(),
            runs: runs
                .iter()
                .map(|(start, end, tone)| {
                    let start = at(*start);
                    (start, (at(*end) - start).max(0.006), *tone)
                })
                .collect(),
            next: next.map(at),
            from: clock(from).into(),
            to: clock(to).into(),
            legend,
        })
    }
}

impl DetailPane {
    pub(super) fn cause_card(&self, card: &CauseCard, cx: &Context<Self>) -> AnyElement {
        let p = palette(cx);
        let (ink, soft, line) = match card.tone {
            Tone::Crit | Tone::Died => (p.crit_ink, p.crit_soft, p.crit),
            Tone::Warn => (p.warn_ink, p.warn_soft, p.warn_line),
            _ => (p.unk_ink, p.unk_soft, p.line_strong),
        };
        let container = card.container.clone();
        let previous = card.container.clone();
        v_flex()
            .id("pod-cause")
            .test_support()
            .aria_label(format!("{} · {}", card.title, card.state))
            .gap(dp(10.))
            .p(dp(14.))
            .rounded(px(8.))
            .bg(soft)
            .border_1()
            .border_color(line)
            .child(
                h_flex()
                    .gap(dp(8.))
                    .children(ui::status_glyph(card.tone, cx))
                    .child(
                        div()
                            .font_weight(FontWeight::BOLD)
                            .text_size(dp(13.))
                            .child(card.title.clone()),
                    )
                    .child(
                        div()
                            .font_family(MONO_FONT)
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(ink)
                            .child(card.state.clone()),
                    ),
            )
            .children(
                card.message
                    .clone()
                    .map(|message| div().text_size(dp(12.5)).child(message)),
            )
            .child(
                v_flex()
                    .gap(dp(4.))
                    .children(card.facts.iter().map(|(label, value)| {
                        h_flex()
                            .gap(dp(10.))
                            .text_size(dp(12.5))
                            .child(
                                div()
                                    .w(dp(96.))
                                    .flex_none()
                                    .text_color(p.muted)
                                    .child(label.clone()),
                            )
                            .child(div().min_w_0().font_family(MONO_FONT).child(value.clone()))
                    })),
            )
            .children(card.output.clone().map(|output| {
                v_flex()
                    .gap(dp(4.))
                    .child(ui::caption("Termination message", cx))
                    .child(
                        div()
                            .id("pod-cause-output")
                            .test_support()
                            .max_h(dp(120.))
                            .overflow_y_scroll()
                            .px_2()
                            .py_1p5()
                            .rounded(px(8.))
                            .bg(p.surface)
                            .font_family(MONO_FONT)
                            .text_size(dp(12.))
                            .child(output),
                    )
            }))
            .when_some(container, |this, name| {
                this.child(
                    h_flex()
                        .gap(dp(8.))
                        .flex_wrap()
                        .when(card.previous, |this| {
                            let name = previous.clone().unwrap_or_default();
                            this.child(
                                Button::new("pod-cause-previous")
                                    .outline()
                                    .small()
                                    .label("Logs of the last instance")
                                    .on_click(cx.listener(move |pane, _, _, cx| {
                                        pane.container_logs(name.clone(), true, cx)
                                    })),
                            )
                        })
                        .child(
                            Button::new("pod-cause-logs")
                                .ghost()
                                .small()
                                .label(format!("Logs of {name}"))
                                .on_click(cx.listener(move |pane, _, _, cx| {
                                    pane.container_logs(name.clone(), false, cx)
                                })),
                        ),
                )
            })
            .into_any_element()
    }

    pub(super) fn timeline(&self, timeline: &Timeline, cx: &Context<Self>) -> AnyElement {
        let p = palette(cx);
        let fill = |tone: Tone| match tone {
            Tone::Crit => p.crit,
            Tone::Good => p.good,
            _ => p.unk,
        };
        let track = div()
            .relative()
            .h(dp(10.))
            .w_full()
            .rounded(px(3.))
            .bg(p.track)
            .children(timeline.runs.iter().map(|(start, width, tone)| {
                div()
                    .absolute()
                    .top_0()
                    .bottom_0()
                    .left(relative(*start))
                    .w(relative(*width))
                    .min_w(px(3.))
                    .rounded(px(3.))
                    .bg(fill(*tone))
            }))
            .children(timeline.next.map(|at| {
                div()
                    .absolute()
                    .top(px(-3.))
                    .bottom(px(-3.))
                    .left(relative(at))
                    .w(px(2.))
                    .bg(p.ink_2)
            }));
        v_flex()
            .id("pod-timeline")
            .test_support()
            .gap(dp(7.))
            .child(ui::caption("Restarts", cx))
            .child(
                div()
                    .font_family(MONO_FONT)
                    .text_size(dp(12.))
                    .child(timeline.title.clone()),
            )
            .child(track)
            .child(
                h_flex()
                    .justify_between()
                    .font_family(MONO_FONT)
                    .text_size(dp(11.))
                    .text_color(p.muted)
                    .child(timeline.from.clone())
                    .child(timeline.to.clone()),
            )
            .children(timeline.legend.iter().map(|(tone, text)| {
                h_flex()
                    .gap(dp(7.))
                    .text_size(dp(12.))
                    .children(ui::status_glyph(*tone, cx))
                    .child(div().min_w_0().child(text.clone()))
            }))
            .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::{CauseCard, DateTime, PodStatus, Timeline, Tone, Utc};

    fn at(text: &str) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339(text)
            .unwrap()
            .with_timezone(&Utc)
    }

    fn crashing() -> PodStatus {
        PodStatus::new(
            &serde_yaml::from_str(
                "
status:
  phase: Running
  containerStatuses:
  - name: api
    ready: false
    restartCount: 3
    state:
      waiting: {reason: CrashLoopBackOff, message: back-off 40s restarting failed container}
    lastState:
      terminated:
        exitCode: 1
        reason: Error
        message: 'panic: no config'
        startedAt: '2026-10-01T10:00:00Z'
        finishedAt: '2026-10-01T10:00:30Z'
",
            )
            .unwrap(),
        )
    }

    #[test]
    fn a_crash_loop_card_names_the_exit_the_restarts_and_the_next_try() {
        let status = crashing();
        let diagnosis = status.diagnose().unwrap();
        let card = CauseCard::new(&diagnosis, None, at("2026-10-01T10:00:50Z"));
        assert_eq!(card.title.as_ref(), "Why it's failing");
        assert_eq!(card.state.as_ref(), "CrashLoopBackOff");
        let facts: Vec<(&str, &str)> = card
            .facts
            .iter()
            .map(|(label, value)| (label.as_ref(), value.as_ref()))
            .collect();
        assert_eq!(facts[0], ("Container", "api"));
        assert_eq!(facts[1], ("Last exit", "exit 1 · Error · 20s ago, ran 30s"));
        assert_eq!(facts[2], ("Restarts", "3"));
        assert!(facts[3].1.starts_with("in about 20s ("), "{facts:?}");
        assert_eq!(
            card.output.as_ref().map(|output| output.as_ref()),
            Some("panic: no config")
        );
        assert_eq!(card.container.as_deref(), Some("api"));
    }

    #[test]
    fn the_timeline_runs_from_the_last_instance_to_the_next_try() {
        let status = crashing();
        let diagnosis = status.diagnose().unwrap();
        let timeline =
            Timeline::new(&status, Some(&diagnosis), at("2026-10-01T10:00:50Z")).unwrap();
        assert_eq!(timeline.title.as_ref(), "api · 3 restarts");
        // 10:00:00 to the next try at 10:01:10: the run is the first 30 s.
        assert_eq!(timeline.runs.len(), 1);
        let (start, width, tone) = timeline.runs[0];
        assert_eq!((start, tone), (0., Tone::Crit));
        assert!((width - 30. / 70.).abs() < 0.001, "{width}");
        assert_eq!(timeline.next, Some(1.));
        assert_eq!(timeline.legend.len(), 3);

        // A container that never restarted has no timeline.
        let fresh = PodStatus::new(
            &serde_yaml::from_str(
                "
status:
  phase: Running
  containerStatuses:
  - name: web
    ready: true
    restartCount: 0
    state:
      running: {startedAt: '2026-10-01T10:00:00Z'}
",
            )
            .unwrap(),
        );
        assert!(Timeline::new(&fresh, None, at("2026-10-01T10:00:50Z")).is_none());
    }
}
