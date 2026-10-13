//! The shared building blocks from `freshkube-ui`, under their old path,
//! with the tones and marks for the app's own health and memory levels.
use gpui_kit::prelude::*;
use gpui_kit::{AnyElement, App, Div, ElementId, div, px, relative};

pub(crate) use freshkube_ui::ui::*;

use freshkube_core::{HasHealth, HealthIndicator};
use gpui_kit::assets::IconName;

use crate::palette::palette;
use crate::presentation::{Health, MemoryLevel};

/// The tone and icon a health draws with, for tags, glyphs and group rows.
/// Only informational health has an icon, which is why it is an `Option`:
/// every caller but Lifecycle's alerts takes the tone alone. Pending reads
/// as unknown.
pub(crate) fn health_tone(health: HealthIndicator) -> (Tone, Option<IconName>) {
    match health {
        HealthIndicator::Healthy => (Tone::Good, None),
        HealthIndicator::Warning => (Tone::Warn, None),
        HealthIndicator::Error => (Tone::Crit, None),
        HealthIndicator::Info => (Tone::Accent, Some(IconName::Info)),
        HealthIndicator::Pending | HealthIndicator::Unknown => (Tone::Unknown, None),
    }
}

pub(crate) fn memory_tone(level: MemoryLevel) -> Option<(Tone, &'static str)> {
    match level {
        MemoryLevel::Normal => None,
        MemoryLevel::High => Some((Tone::Warn, "High")),
        MemoryLevel::Critical => Some((Tone::Crit, "Critical")),
    }
}

/// The status mark for a health, with its label as the tooltip.
pub(crate) fn health_mark(id: impl Into<ElementId>, health: Health, cx: &App) -> AnyElement {
    let label = crate::presentation::health_text(&health);
    status_mark(id, health_tone(health.health()).0, label, cx)
}

pub(crate) fn meter(percent: f64, level: MemoryLevel, cx: &App) -> Div {
    let p = palette(cx);
    let fill_color = match level {
        MemoryLevel::Normal => p.accent,
        MemoryLevel::High => p.warn,
        MemoryLevel::Critical => p.crit,
    };
    div()
        .w_full()
        .h(dp(6.))
        .rounded(px(3.))
        .bg(p.track)
        .overflow_hidden()
        .child(
            div()
                .h_full()
                .w(relative((percent / 100.).clamp(0., 1.) as f32))
                .rounded(px(3.))
                .bg(fill_color),
        )
}

#[cfg(test)]
mod tests {
    use freshkube_core::workloads::HealthState;

    use super::*;

    fn look(health: impl HasHealth) -> (Tone, Option<IconName>) {
        health_tone(health.health())
    }

    /// Every health enum that reaches a tag keeps the tone and icon it drew
    /// before the three maps became one.
    #[test]
    fn every_health_variant_keeps_its_tone_and_icon() {
        let rows = [
            (Health::Healthy, Tone::Good),
            (Health::Unhealthy, Tone::Crit),
            (Health::Unknown, Tone::Unknown),
        ];
        for (health, tone) in rows {
            assert_eq!(look(health), (tone, None), "{health:?}");
        }
        let rows = [
            (HealthState::Failing, Tone::Crit),
            (HealthState::Degraded, Tone::Warn),
            (HealthState::Pending, Tone::Unknown),
            (HealthState::Healthy, Tone::Good),
        ];
        for (state, tone) in rows {
            assert_eq!(look(state), (tone, None), "{state:?}");
        }
        let rows = [
            (HealthIndicator::Healthy, Tone::Good, None),
            (HealthIndicator::Warning, Tone::Warn, None),
            (HealthIndicator::Error, Tone::Crit, None),
            (HealthIndicator::Pending, Tone::Unknown, None),
            (HealthIndicator::Info, Tone::Accent, Some(IconName::Info)),
            (HealthIndicator::Unknown, Tone::Unknown, None),
        ];
        for (indicator, tone, icon) in rows {
            let (got_tone, got_icon) = health_tone(indicator);
            assert_eq!(got_tone, tone, "{indicator:?}");
            assert_eq!(
                got_icon.map(|icon| format!("{icon:?}")),
                icon.map(|icon| format!("{icon:?}")),
                "{indicator:?}"
            );
        }
    }
}
