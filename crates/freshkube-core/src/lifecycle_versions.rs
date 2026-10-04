//! Pure version interpretation used by Lifecycle.
//! The support table preserves the desktop rules; unknown versions stay unknown.

/// `v1.34.3`, `1.13.2-rc1` or `v1.30.0+k3s1` as numbers.
pub fn parse_version(version: &str) -> Option<(u32, u32, u32)> {
    let mut parts = version.trim().trim_start_matches('v').split('.');
    let number = |part: Option<&str>| -> Option<u32> {
        let part = part?;
        let digits: String = part.chars().take_while(char::is_ascii_digit).collect();
        digits.parse().ok()
    };
    let major = number(parts.next())?;
    let minor = number(parts.next())?;
    let patch = number(parts.next()).unwrap_or(0);
    Some((major, minor, patch))
}

/// The Kubernetes minors a Talos minor supports, from the support matrix at
/// <https://www.talos.dev/latest/introduction/support-matrix/>. Versions not
/// listed are unknown rather than unsupported.
pub fn kubernetes_support(talos: &str) -> Option<(u32, u32)> {
    match parse_version(talos)? {
        (1, 12, _) => Some((30, 35)),
        (1, 11, _) => Some((29, 34)),
        (1, 10, _) => Some((28, 33)),
        (1, 9, _) => Some((27, 32)),
        (1, 8, _) => Some((26, 31)),
        (1, 7, _) => Some((25, 30)),
        (1, 6, _) => Some((24, 29)),
        _ => None,
    }
}
