//! The rollback helper (#845): which earlier Freight of a Stage Kargo itself
//! recorded as verified, so the Inspector can offer to promote it again.
//!
//! Everything here is read from the Stage's own `status.freightHistory`, the
//! record Kargo keeps (10 entries by default), and its
//! `verificationHistory`. Nothing is guessed about Freight older than that
//! window, and nothing is read from logs.
//!
//! What "verified" means is Kargo's verification only. Argo CD's health and
//! Coroot's outcome are read live for the Freight a change runs and are not
//! recorded per earlier Freight, so no earlier Freight is ever called
//! healthy: the best it can be is `Verified` in Kargo, and the dialog says
//! what wasn't recorded.

use crate::kargo::{self, HistoryItem};

/// What Kargo recorded about an earlier Freight's verification in the Stage.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Evidence {
    /// Kargo's verification of it ended `Successful`; when it finished, as
    /// Kargo wrote it.
    Verified { finished: Option<String> },
    /// Not verified, or not known to be: why, in a sentence.
    Unknown(String),
}

impl Evidence {
    pub fn is_verified(&self) -> bool {
        matches!(self, Self::Verified { .. })
    }
}

/// One earlier Freight of the Stage's history that a Promotion could name.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Earlier {
    pub freight: String,
    /// Where the Stage names it, which the write checks is still true.
    pub pointer: String,
    pub evidence: Evidence,
    /// What it held, one line each (image and commit), from the history.
    pub would_run: Vec<String>,
}

/// Kargo's words for the end of a verification that wasn't a success.
fn ended(phase: &str) -> String {
    match phase {
        "Failed" | "Error" | "Aborted" | "Inconclusive" => {
            format!("Kargo's verification of it ended {phase}.")
        }
        "Pending" | "Running" => "Kargo's verification of it hadn't finished.".to_owned(),
        other => format!("Kargo recorded its verification as {other}."),
    }
}

/// The evidence for the history entry at `index`: its newest verification.
fn evidence(stage: &kargo::Stage, index: usize) -> Evidence {
    let when = |v: &kargo::Verification| {
        v.finished
            .as_deref()
            .or(v.started.as_deref())
            .and_then(|time| time.parse::<chrono::DateTime<chrono::Utc>>().ok())
    };
    let mut newest: Option<&kargo::Verification> = None;
    for run in stage.verifications.iter().filter(|v| v.collection == index) {
        if newest.is_none_or(|best| matches!((when(run), when(best)), (Some(a), Some(b)) if a > b))
        {
            newest = Some(run);
        }
    }
    match newest {
        None => Evidence::Unknown("No verification of it was recorded.".to_owned()),
        Some(run) => match run.phase.as_deref() {
            Some("Successful") => Evidence::Verified {
                finished: run.finished.clone(),
            },
            Some(phase) => Evidence::Unknown(ended(phase)),
            None => Evidence::Unknown("Its verification has no recorded outcome.".to_owned()),
        },
    }
}

/// The lines a history item holds, for the preview.
fn lines(item: &HistoryItem) -> Vec<String> {
    super::contents_lines(&item.images, &item.commits)
}

/// The earlier Freight of the Stage's history, newest first. The entry the
/// Stage runs now (the newest) is never one, nor is a Freight it runs again
/// further back; a Freight is listed once, at its newest entry. An entry
/// that holds Freight of several Warehouses can't be named by one Promotion
/// and is left out.
pub fn earlier(stage: &kargo::Stage) -> Vec<Earlier> {
    let running: Vec<&str> = stage
        .history_items
        .first()
        .into_iter()
        .flatten()
        .map(|item| item.name.as_str())
        .collect();
    let mut found: Vec<Earlier> = Vec::new();
    for (index, entry) in stage.history_items.iter().enumerate().skip(1) {
        let [item] = entry.as_slice() else {
            continue;
        };
        if running.contains(&item.name.as_str()) || found.iter().any(|e| e.freight == item.name) {
            continue;
        }
        found.push(Earlier {
            freight: item.name.clone(),
            pointer: item.pointer.clone(),
            evidence: evidence(stage, index),
            would_run: lines(item),
        });
    }
    found
}

/// The kargo CLI line that promotes `freight` to the Stage, to run by hand.
pub fn cli_line(project: &str, freight: &str, stage: &str) -> String {
    format!("kargo promote --project {project} --freight {freight} --stage {stage}")
}

/// The newest earlier Freight Kargo verified: the one the main action
/// offers. `None` when none was, which is never "the last healthy".
pub fn last_verified(earlier: &[Earlier]) -> Option<&Earlier> {
    earlier.iter().find(|e| e.evidence.is_verified())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{Value, json};

    fn item(name: &str) -> Value {
        json!({"name": name, "images": [{"repoURL": "registry.example/acme/api", "tag": "1.0",
            "digest": "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"}]})
    }

    fn stage(history: Vec<Value>) -> kargo::Stage {
        kargo::parse_stage(&json!({
            "metadata": {"name": "prod", "namespace": "shop", "uid": "u", "resourceVersion": "1"},
            "status": {"freightHistory": history}
        }))
        .unwrap()
    }

    fn entry(names: &[&str], verification: Option<(&str, &str)>) -> Value {
        let items: serde_json::Map<String, Value> = names
            .iter()
            .enumerate()
            .map(|(n, name)| (format!("Warehouse/w{n}"), item(name)))
            .collect();
        let mut entry = json!({ "items": items });
        if let Some((phase, finished)) = verification {
            entry["verificationHistory"] =
                json!([{"id": "v", "phase": phase, "finishTime": finished}]);
        }
        entry
    }

    const DAY1: &str = "2026-10-01T10:00:00Z";
    const DAY2: &str = "2026-10-02T10:00:00Z";

    #[test]
    fn safety_g26_the_newest_entry_is_what_runs_and_is_never_a_candidate() {
        let stage = stage(vec![entry(&["now"], Some(("Successful", DAY2)))]);
        assert!(earlier(&stage).is_empty());
        assert!(earlier(&stage::empty()).is_empty());
    }

    mod stage {
        pub fn empty() -> crate::kargo::Stage {
            super::stage(vec![])
        }
    }

    #[test]
    fn only_a_successful_verification_is_verified_and_everything_else_is_unknown() {
        let stage = stage(vec![
            entry(&["now"], None),
            entry(&["good"], Some(("Successful", DAY2))),
            entry(&["failed"], Some(("Failed", DAY1))),
            entry(&["running"], Some(("Running", DAY1))),
            entry(&["none"], None),
        ]);
        let found = earlier(&stage);
        let by = |name: &str| found.iter().find(|e| e.freight == name).unwrap();
        assert!(by("good").evidence.is_verified());
        for name in ["failed", "running", "none"] {
            assert!(
                matches!(by(name).evidence, Evidence::Unknown(_)),
                "{name} is not verified"
            );
        }
        assert_eq!(last_verified(&found).unwrap().freight, "good");
    }

    #[test]
    fn nothing_verified_means_no_last_verified_and_never_a_healthy_default() {
        let stage = stage(vec![
            entry(&["now"], Some(("Successful", DAY2))),
            entry(&["failed"], Some(("Error", DAY1))),
            entry(&["none"], None),
        ]);
        assert!(last_verified(&earlier(&stage)).is_none());
    }

    #[test]
    fn the_newest_verification_of_an_entry_decides() {
        let mut older_failed_then_passed = entry(&["x"], None);
        older_failed_then_passed["verificationHistory"] = json!([
            {"id": "a", "phase": "Successful", "finishTime": DAY1},
            {"id": "b", "phase": "Failed", "finishTime": DAY2},
        ]);
        let stage = stage(vec![entry(&["now"], None), older_failed_then_passed]);
        assert!(matches!(earlier(&stage)[0].evidence, Evidence::Unknown(_)));
    }

    #[test]
    fn safety_g26_freight_the_stage_runs_again_further_back_is_not_a_candidate_and_one_is_listed_once()
     {
        let stage = stage(vec![
            entry(&["now"], None),
            entry(&["other"], Some(("Successful", DAY2))),
            entry(&["now"], Some(("Successful", DAY1))),
            entry(&["other"], Some(("Failed", DAY1))),
        ]);
        let found = earlier(&stage);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].freight, "other");
        assert!(found[0].evidence.is_verified(), "the newest entry of it");
    }

    #[test]
    fn an_entry_of_several_warehouses_is_left_out_and_the_pointer_names_the_entry() {
        let stage = stage(vec![
            entry(&["now"], None),
            entry(&["a", "b"], Some(("Successful", DAY2))),
            entry(&["one"], Some(("Successful", DAY1))),
        ]);
        let found = earlier(&stage);
        assert_eq!(found.len(), 1);
        assert_eq!(
            found[0].pointer,
            "/status/freightHistory/2/items/Warehouse~1w0/name"
        );
        assert_eq!(found[0].would_run.len(), 1, "its image, from the history");
    }
}
