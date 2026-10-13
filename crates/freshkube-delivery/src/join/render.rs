use super::*;

/// A trail as text, one link per line with an indented `from …` line for
/// each observation behind it, then its summary.
pub fn render(trail: &Trail) -> String {
    let mut out = format!(
        "change {} (read {})\n",
        trail.sha,
        trail
            .observed_at
            .to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
    );
    for link in &trail.links {
        out.push_str(&format!(
            "  {:<9} {} -> {}  [{}]  {}  ({})\n",
            link.confidence.word(),
            link.from.word(),
            link.to.word(),
            link.key,
            link.subject,
            link.reason
        ));
        for seen in &link.evidence {
            out.push_str(&format!("      from {}\n", seen.describe()));
        }
    }
    out.push_str(&format!("summary: {}\n", trail.summary()));
    out
}
