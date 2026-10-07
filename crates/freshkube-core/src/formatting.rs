//! Formatting utilities for consistent display across the application
//!
//! Provides functions for formatting bytes and percentages, and for pluralizing
//! a count.

// Byte size constants
const KB: u64 = 1024;
const MB: u64 = KB * 1024;
const GB: u64 = MB * 1024;
const TB: u64 = GB * 1024;

/// Format bytes into a human-readable string
///
/// Uses binary units (KiB-style but labeled as KB for familiarity).
///
/// # Examples
///
/// ```
/// use freshkube_core::formatting::format_bytes;
///
/// assert_eq!(format_bytes(500), "500 B");
/// assert_eq!(format_bytes(1536), "1.5 KB");
/// assert_eq!(format_bytes(1_572_864), "1.5 MB");
/// ```
pub fn format_bytes(bytes: u64) -> String {
    if bytes >= TB {
        format!("{:.1} TB", bytes as f64 / TB as f64)
    } else if bytes >= GB {
        format!("{:.1} GB", bytes as f64 / GB as f64)
    } else if bytes >= MB {
        format!("{:.1} MB", bytes as f64 / MB as f64)
    } else if bytes >= KB {
        format!("{:.1} KB", bytes as f64 / KB as f64)
    } else {
        format!("{} B", bytes)
    }
}

/// Format signed bytes (for etcd and other APIs that use i64)
///
/// Handles negative values by showing "0 B".
pub fn format_bytes_signed(bytes: i64) -> String {
    if bytes < 0 {
        "0 B".to_string()
    } else {
        format_bytes(bytes as u64)
    }
}

/// Format a percentage value
///
/// # Examples
///
/// ```
/// use freshkube_core::formatting::format_percent;
///
/// assert_eq!(format_percent(75.5), "75.5%");
/// assert_eq!(format_percent(100.0), "100.0%");
/// ```
pub fn format_percent(value: f64) -> String {
    format!("{:.1}%", value)
}

/// Format a count with singular/plural form
///
/// # Examples
///
/// ```
/// use freshkube_core::formatting::pluralize;
///
/// assert_eq!(pluralize(1, "node", "nodes"), "1 node");
/// assert_eq!(pluralize(5, "node", "nodes"), "5 nodes");
/// assert_eq!(pluralize(0, "item", "items"), "0 items");
/// ```
pub fn pluralize(count: usize, singular: &str, plural: &str) -> String {
    if count == 1 {
        format!("{} {}", count, singular)
    } else {
        format!("{} {}", count, plural)
    }
}

/// Groups digits in threes: 455555555 becomes 455,555,555.
///
/// ```
/// use freshkube_core::group_digits;
/// assert_eq!(group_digits(7), "7");
/// assert_eq!(group_digits(1_234), "1,234");
/// assert_eq!(group_digits(1_234_567), "1,234,567");
/// ```
pub fn group_digits(value: u64) -> String {
    let digits = value.to_string();
    let mut grouped = String::with_capacity(digits.len() + digits.len() / 3);
    for (ix, digit) in digits.chars().enumerate() {
        if ix > 0 && (digits.len() - ix).is_multiple_of(3) {
            grouped.push(',');
        }
        grouped.push(digit);
    }
    grouped
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn digits_are_grouped_in_threes() {
        assert_eq!(group_digits(0), "0");
        assert_eq!(group_digits(999), "999");
        assert_eq!(group_digits(1_000), "1,000");
        assert_eq!(group_digits(455_555_555), "455,555,555");
    }

    #[test]
    fn test_format_bytes() {
        assert_eq!(format_bytes(0), "0 B");
        assert_eq!(format_bytes(500), "500 B");
        assert_eq!(format_bytes(1024), "1.0 KB");
        assert_eq!(format_bytes(1536), "1.5 KB");
        assert_eq!(format_bytes(1_048_576), "1.0 MB");
        assert_eq!(format_bytes(1_073_741_824), "1.0 GB");
        assert_eq!(format_bytes(1_099_511_627_776), "1.0 TB");
    }

    #[test]
    fn test_format_bytes_signed() {
        assert_eq!(format_bytes_signed(-100), "0 B");
        assert_eq!(format_bytes_signed(0), "0 B");
        assert_eq!(format_bytes_signed(1024), "1.0 KB");
    }

    #[test]
    fn test_format_percent() {
        assert_eq!(format_percent(0.0), "0.0%");
        assert_eq!(format_percent(50.5), "50.5%");
        assert_eq!(format_percent(100.0), "100.0%");
    }

    #[test]
    fn test_pluralize() {
        assert_eq!(pluralize(0, "node", "nodes"), "0 nodes");
        assert_eq!(pluralize(1, "node", "nodes"), "1 node");
        assert_eq!(pluralize(5, "node", "nodes"), "5 nodes");
    }
}
