//! Units are formatted once when observations are projected.
use std::time::Duration;
pub(super) fn percent(value: Option<f64>) -> String {
    value
        .filter(|v| v.is_finite())
        .map_or_else(|| "Not reported".into(), |v| format!("{v:.1}%"))
}
pub(super) fn multiplier(value: Option<f64>) -> String {
    value
        .filter(|v| v.is_finite())
        .map_or_else(|| "Not reported".into(), |v| format!("{v:.1}×"))
}
pub(super) fn duration(value: Duration) -> String {
    let secs = value.as_secs();
    if secs >= 3600 {
        format!("{}h {}m", secs / 3600, (secs % 3600) / 60)
    } else if secs >= 60 {
        format!("{}m {}s", secs / 60, secs % 60)
    } else {
        format!("{secs}s")
    }
}
/// A moment in the viewer's own time zone, day and minute.
pub(super) fn local_time(value: chrono::DateTime<chrono::Utc>) -> String {
    value
        .with_timezone(&chrono::Local)
        .format("%d %b %H:%M")
        .to_string()
}
