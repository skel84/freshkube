//! The access request a person copies to ask for what a row lacks.

use super::{Need, Who};

/// Plain text to paste into a ticket or a message: the cluster, who is
/// asking, where, and what is missing. It names no credential.
pub fn request_text(
    context: &str,
    who: Option<&Who>,
    namespace: Option<&str>,
    needs: &[Need],
) -> String {
    let mut lines = vec!["Access request".to_owned(), String::new()];
    lines.push(format!("Cluster context: {context}"));
    if let Some(who) = who.filter(|who| !who.username.is_empty()) {
        lines.push(format!("Who: {}", who.username));
    }
    lines.push(match namespace {
        Some(namespace) => format!("Namespace: {namespace}"),
        None => "Scope: cluster-wide".to_owned(),
    });
    lines.push(String::new());
    lines.push("Please grant:".to_owned());
    lines.extend(
        needs
            .iter()
            .map(|need| format!("- {} on {}", need.verb, need.qualified())),
    );
    lines.join("\n")
}
