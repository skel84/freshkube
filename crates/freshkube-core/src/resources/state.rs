//! The meaning of a printed status, shared by the Resources page and the
//! command line.

/// The meaning of a printed status, for colouring; unknown values are neutral.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StatusTone {
    Neutral,
    Success,
    Warning,
    Danger,
}

pub fn status_tone(value: &str) -> StatusTone {
    let value = value.trim();
    match value {
        "Running" | "Ready" | "Active" | "Bound" | "Available" | "Complete" | "Completed"
        | "Succeeded" | "True" | "Healthy" | "Synced" | "Established" => StatusTone::Success,
        "Pending"
        | "ContainerCreating"
        | "PodInitializing"
        | "Terminating"
        | "Unknown"
        | "Released"
        | "Progressing"
        | "Suspended"
        | "NotReady"
        | "Ready,SchedulingDisabled"
        | "SchedulingDisabled"
        | "Waiting" => StatusTone::Warning,
        _ if value.starts_with("Init:")
            && !value.contains("Error")
            && !value.contains("CrashLoop") =>
        {
            StatusTone::Warning
        }
        _ if value.contains("BackOff")
            || value.contains("Err")
            || value.contains("Fail")
            || value.contains("OOMKilled")
            || value == "Lost"
            || value == "Evicted"
            || value == "False" =>
        {
            StatusTone::Danger
        }
        _ => StatusTone::Neutral,
    }
}

/// A workload's readiness from the `up/all` it prints: every replica ready,
/// some, or none of several. `None` for text that is not a ratio. A workload
/// scaled to zero is fine.
pub fn ready_tone(text: &str) -> Option<StatusTone> {
    let (up, all) = text.split_once('/')?;
    let (up, all): (u32, u32) = (up.trim().parse().ok()?, all.trim().parse().ok()?);
    Some(if up >= all {
        StatusTone::Success
    } else if up == 0 {
        StatusTone::Danger
    } else {
        StatusTone::Warning
    })
}

/// The printed column that gives a row its glyph, for kinds other than pods:
/// `Status` or `Phase`, else a `Ready` ratio. The Resources page and the
/// command line both read a row's state from it.
pub fn tone_column(columns: &[String]) -> Option<(usize, bool)> {
    let named = |wanted: &str| {
        columns
            .iter()
            .position(|name| name.eq_ignore_ascii_case(wanted))
    };
    named("status")
        .or_else(|| named("phase"))
        .map(|ix| (ix, false))
        .or_else(|| named("ready").map(|ix| (ix, true)))
}

/// A row's tone and the text that explains it, from its printed cells.
pub fn row_tone(columns: &[String], cells: &[String]) -> Option<(StatusTone, String)> {
    let (ix, ratio) = tone_column(columns)?;
    let text = cells.get(ix)?.trim();
    if ratio {
        ready_tone(text).map(|tone| (tone, format!("{text} ready")))
    } else {
        (!text.is_empty()).then(|| (status_tone(text), text.to_owned()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(names: &[&str]) -> Vec<String> {
        names.iter().map(|name| name.to_string()).collect()
    }

    #[test]
    fn a_ready_ratio_is_good_partial_or_none() {
        assert_eq!(ready_tone("2/2"), Some(StatusTone::Success));
        assert_eq!(ready_tone("0/0"), Some(StatusTone::Success));
        assert_eq!(ready_tone("1/2"), Some(StatusTone::Warning));
        assert_eq!(ready_tone("0/3"), Some(StatusTone::Danger));
        assert_eq!(ready_tone("Running"), None);
    }

    #[test]
    fn a_row_takes_its_status_before_its_readiness() {
        let columns = names(&["Name", "Ready", "Status"]);
        let cells = names(&["api", "0/1", "CrashLoopBackOff"]);
        assert_eq!(
            row_tone(&columns, &cells),
            Some((StatusTone::Danger, "CrashLoopBackOff".to_owned()))
        );
        let columns = names(&["Name", "Ready"]);
        assert_eq!(
            row_tone(&columns, &names(&["api", "1/2"])),
            Some((StatusTone::Warning, "1/2 ready".to_owned()))
        );
        assert_eq!(row_tone(&names(&["Name"]), &names(&["x"])), None);
    }
}
