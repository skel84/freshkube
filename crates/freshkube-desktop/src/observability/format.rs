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
/// Requests a second, as Coroot writes them: `12 rps`, `0.25 rps`.
pub(super) fn requests(value: Option<f64>) -> String {
    reported(value, |v| format!("{} rps", significant(v)))
}
/// A latency in seconds, in the unit that reads best: `850 µs`, `3 ms`, `1.2 s`.
pub(super) fn seconds(value: Option<f64>) -> String {
    reported(value, |v| match v.abs() {
        // Bounds sit at the rounding, so 0.9996 s reads 1 s, not 1000 ms.
        s if s >= 0.9995 || s == 0. => format!("{} s", significant(v)),
        s if s >= 0.9995e-3 => format!("{} ms", significant(v * 1e3)),
        _ => format!("{} µs", significant(v * 1e6)),
    })
}
/// Bytes a second in decimal units, as Coroot shows traffic: `2.4 KB/s`.
pub(super) fn bytes_per_second(value: Option<f64>) -> String {
    reported(value, |v| {
        let units = ["B/s", "KB/s", "MB/s", "GB/s", "TB/s"];
        let (mut v, mut unit) = (v, 0);
        // Scaled before rounding, so 999.96 B/s reads 1 KB/s, not 1000 B/s.
        while unit + 1 < units.len() && v.abs() >= 999.95 {
            v /= 1000.;
            unit += 1;
        }
        format!("{} {}", significant(v), units[unit])
    })
}
fn reported(value: Option<f64>, format: impl Fn(f64) -> String) -> String {
    value
        .filter(|v| v.is_finite())
        .map_or_else(|| "Not reported".into(), format)
}
/// At most three significant digits, without trailing zeros: `12`, `2.4`, `0.25`.
fn significant(value: f64) -> String {
    let decimals = match value.abs() {
        v if v >= 99.95 => 0,
        v if v >= 9.995 => 1,
        _ => 2,
    };
    let text = format!("{value:.decimals$}");
    let text = if text.contains('.') {
        text.trim_end_matches('0').trim_end_matches('.')
    } else {
        &text
    };
    if text == "-0" {
        "0".into()
    } else {
        text.into()
    }
}

#[cfg(test)]
mod tests {
    use super::{bytes_per_second, requests, seconds};

    #[test]
    fn rates_drop_trailing_zeros() {
        for (value, expected) in [
            (12., "12 rps"),
            (0.25, "0.25 rps"),
            (0.004, "0 rps"),
            (12.34, "12.3 rps"),
            (1234.6, "1235 rps"),
            (0., "0 rps"),
        ] {
            assert_eq!(requests(Some(value)), expected, "{value}");
        }
    }

    #[test]
    fn latencies_take_the_unit_that_reads_best() {
        for (value, expected) in [
            (0.003, "3 ms"),
            (0.0125, "12.5 ms"),
            (0.00085, "850 µs"),
            (0.25, "250 ms"),
            (1., "1 s"),
            (0.9996, "1 s"),
            (0.0009996, "1 ms"),
            (1.25, "1.25 s"),
            (95., "95 s"),
            (0., "0 s"),
        ] {
            assert_eq!(seconds(Some(value)), expected, "{value}");
        }
    }

    #[test]
    fn traffic_scales_by_thousands() {
        for (value, expected) in [
            (0., "0 B/s"),
            (512., "512 B/s"),
            (2400., "2.4 KB/s"),
            (999.96, "1 KB/s"),
            (1_000_000., "1 MB/s"),
            (37_500_000., "37.5 MB/s"),
            (4.2e12, "4.2 TB/s"),
            (9e15, "9000 TB/s"),
        ] {
            assert_eq!(bytes_per_second(Some(value)), expected, "{value}");
        }
    }

    #[test]
    fn a_missing_or_broken_value_is_not_reported() {
        for value in [None, Some(f64::NAN), Some(f64::INFINITY)] {
            assert_eq!(requests(value), "Not reported");
            assert_eq!(seconds(value), "Not reported");
            assert_eq!(bytes_per_second(value), "Not reported");
        }
    }
}
