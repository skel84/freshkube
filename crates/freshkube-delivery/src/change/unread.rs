//! A known workload's Pods when its destination could not be read. The
//! parent is supplied by delivery; no object or running image is guessed.

use super::*;
use crate::words;

pub(super) fn pods(workload: &Hop, cluster: &str, why: &str) -> Hop {
    let reason = words::unreadable("Pods", cluster, why);
    Hop {
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
                body: reason.clone(),
            }),
            fields: vec![Field {
                label: "Read from".into(),
                value: Value::Mono(cluster.into()),
            }],
            link: Some(LinkDetail {
                confidence: Confidence::Unknown,
                by: "nothing".into(),
                before: workload.name.clone(),
                key: None,
                before_says: workload.detail.clone(),
                here_says: "not read".into(),
                why: reason,
            }),
            unlinked: None,
            actions: Vec::new(),
        })),
    }
}
