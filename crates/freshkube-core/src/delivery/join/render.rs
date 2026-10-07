use super::*;

/// A trail as text, one link per line, then its summary.
pub fn render(trail: &Trail) -> String {
    let mut out = format!("change {}\n", trail.sha);
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
    }
    out.push_str(&format!("summary: {}\n", trail.summary()));
    out
}
