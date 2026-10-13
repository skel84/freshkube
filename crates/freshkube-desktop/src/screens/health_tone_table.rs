//! Every health variant's tone and icon, pinned as one table. It reads the
//! functions that map a health to its tag, so a refactor of them shows up as
//! a changed row, never as a changed glyph that no test noticed.

use freshkube_core::workloads::HealthState;
use freshkube_core::{HasHealth, HealthIndicator};

use crate::presentation::Health;
use crate::ui;

fn push(
    rows: &mut Vec<String>,
    enum_name: &str,
    variant: &str,
    tone: ui::Tone,
    icon: Option<String>,
) {
    rows.push(format!("{enum_name}::{variant} => {tone:?}, {icon:?}"));
}

/// (enum, variant, tone, icon) for every variant of every health enum that
/// reaches a tag or glyph.
fn table() -> Vec<String> {
    let mut rows = Vec::new();
    for (variant, health) in [
        ("Healthy", Health::Healthy),
        ("Unhealthy", Health::Unhealthy),
        ("Unknown", Health::Unknown),
    ] {
        push(
            &mut rows,
            "Health",
            variant,
            ui::health_tone(health.health()).0,
            None,
        );
    }
    for (variant, state) in [
        ("Failing", HealthState::Failing),
        ("Degraded", HealthState::Degraded),
        ("Pending", HealthState::Pending),
        ("Healthy", HealthState::Healthy),
    ] {
        push(
            &mut rows,
            "HealthState",
            variant,
            ui::health_tone(state.health()).0,
            None,
        );
    }
    for (variant, indicator) in [
        ("Healthy", HealthIndicator::Healthy),
        ("Warning", HealthIndicator::Warning),
        ("Error", HealthIndicator::Error),
        ("Pending", HealthIndicator::Pending),
        ("Info", HealthIndicator::Info),
        ("Unknown", HealthIndicator::Unknown),
    ] {
        let (tone, icon, label) = super::lifecycle::alert_look(&indicator);
        push(
            &mut rows,
            "HealthIndicator",
            variant,
            tone,
            icon.map(|icon| format!("{icon:?}")),
        );
        rows.push(format!("HealthIndicator::{variant} label {label}"));
    }
    rows
}

#[test]
fn every_health_variant_keeps_its_tone_and_icon() {
    let expected = [
        "Health::Healthy => Good, None",
        "Health::Unhealthy => Crit, None",
        "Health::Unknown => Unknown, None",
        "HealthState::Failing => Crit, None",
        "HealthState::Degraded => Warn, None",
        "HealthState::Pending => Unknown, None",
        "HealthState::Healthy => Good, None",
        "HealthIndicator::Healthy => Good, None",
        "HealthIndicator::Healthy label OK",
        "HealthIndicator::Warning => Warn, None",
        "HealthIndicator::Warning label Warning",
        "HealthIndicator::Error => Crit, None",
        "HealthIndicator::Error label Error",
        "HealthIndicator::Pending => Unknown, None",
        "HealthIndicator::Pending label Unknown",
        "HealthIndicator::Info => Accent, Some(\"Info\")",
        "HealthIndicator::Info label Info",
        "HealthIndicator::Unknown => Unknown, None",
        "HealthIndicator::Unknown label Unknown",
    ];
    assert_eq!(table(), expected);
}
