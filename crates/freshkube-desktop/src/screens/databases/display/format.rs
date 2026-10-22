use super::*;
use chrono::{DateTime, Duration};
use freshkube_core::databases::Engine;

pub(super) fn fact<T: std::fmt::Display>(value: &Fact<T>) -> String {
    value
        .known()
        .map(ToString::to_string)
        .unwrap_or_else(|| "Unknown".into())
}

pub(super) fn time(value: Option<DateTime<Utc>>) -> String {
    value
        .map(|at| {
            format!(
                "{} {}",
                at.with_timezone(&chrono::Local).format("%Y-%m-%d"),
                ui::clock(at.into())
            )
        })
        .unwrap_or_else(|| "Unknown".into())
}

/// "1 cluster", "2 clusters".
pub(super) fn plural(count: usize, noun: &str) -> String {
    format!("{count} {noun}{}", if count == 1 { "" } else { "s" })
}

/// How long ago, in the unit that reads best: "12 min ago", "3 h ago".
pub(super) fn age(now: DateTime<Utc>, at: DateTime<Utc>) -> String {
    let elapsed = now - at;
    if elapsed < Duration::zero() {
        return "in the future".into();
    }
    match elapsed {
        e if e < Duration::minutes(1) => "just now".into(),
        e if e < Duration::hours(1) => format!("{} min ago", e.num_minutes()),
        e if e < Duration::days(2) => format!("{} h ago", e.num_hours()),
        e => format!("{} d ago", e.num_days()),
    }
}

/// A span in the words an Inspector line wants: "9 days", "5 hours".
pub(super) fn span(duration: Duration) -> String {
    if duration >= Duration::days(1) {
        plural(duration.num_days() as usize, "day")
    } else {
        plural(duration.num_hours().max(1) as usize, "hour")
    }
}

/// Seconds of lag: "0.4 s", "74 s".
pub(super) fn seconds(value: f64) -> String {
    if value < 10. {
        format!("{value:.1} s")
    } else {
        format!("{value:.0} s")
    }
}

pub(super) fn tone_of(health: freshkube_core::HealthIndicator) -> ui::Tone {
    ui::health_tone(health).0
}

pub(super) fn health_word(health: freshkube_core::HealthIndicator) -> &'static str {
    use freshkube_core::HealthIndicator::*;
    match health {
        Healthy | Info => "Healthy",
        Warning => "Warning",
        Error => "Error",
        Pending => "Pending",
        Unknown => "Unknown",
    }
}

/// A schedule in words, and the expression as the operator wrote it with
/// what is wrong with it, if anything. Nothing is guessed: what is not
/// recognised stays as written.
pub(super) fn cron(engine: Engine, expression: &Fact<String>) -> (String, Option<String>) {
    use freshkube_core::databases::cron::{Cron, CronError};
    let Some(raw) = expression.known() else {
        return ("Unknown".into(), None);
    };
    match Cron::parse(engine, raw) {
        Ok(cron) => (cron.words(), Some(raw.clone())),
        Err(CronError::Fields { found, expected }) => (
            raw.clone(),
            Some(format!(
                "{raw} · {found} fields where {} takes {expected}",
                engine.product()
            )),
        ),
        Err(CronError::Empty) => ("Unknown".into(), None),
    }
}
