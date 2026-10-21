//! A known workload's Pods when its destination could not be read. The
//! parent is supplied by delivery; no object or running image is guessed.

use super::*;

pub(super) fn pods(workload: &Hop, cluster: &str, why: &str) -> Hop {
    let reason = format!("Pods couldn't be read: {why}");
    // The entry didn't answer; which kind it would refuse is not known.
    let cause = Cause::cant_read(
        "the destination cluster's pods, to see which image they run",
        None,
        false,
    );
    Hop {
        cause: Some(cause.clone()),
        key: format!("{}-pods", workload.key),
        group: workload.group,
        state: HealthIndicator::Unknown,
        name: "Pods".into(),
        detail: reason.clone(),
        from: cluster.into(),
        at: None,
        link: Some(Confidence::Unknown),
        after: Some(workload.key.clone()),
        shows: Shows::Hop(Box::new(HopDetail {
            kind: "Pods".into(),
            title: "Pods".into(),
            state: "Unknown".into(),
            notice: Some(Notice {
                state: HealthIndicator::Warning,
                lead: "Not readable.".into(),
                body: sentence(&reason),
            }),
            fields: vec![Field {
                label: "Read from".into(),
                value: Value::Mono(cluster.into()),
            }],
            link: Some(LinkDetail {
                cause: Some(cause),
                confidence: Confidence::Unknown,
                by: "nothing".into(),
                before: workload.name.clone(),
                key: None,
                before_says: workload.detail.clone(),
                here_says: "not read".into(),
                why: sentence(&reason),
            }),
            unlinked: None,
            actions: Vec::new(),
        })),
    }
}
