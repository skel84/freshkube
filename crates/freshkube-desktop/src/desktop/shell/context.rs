//! Labels are prepared on source and text-size changes, never while drawing.
use super::*;

#[derive(Default)]
pub(in crate::desktop) struct ContextDisplay {
    pub(in crate::desktop) name: SharedString,
    pub(in crate::desktop) detail: SharedString,
    pub(in crate::desktop) status: SharedString,
    pub(in crate::desktop) state: u8,
    pub(in crate::desktop) open: bool,
    pub(in crate::desktop) counts: std::collections::BTreeMap<String, SharedString>,
}

/// Room for the context's name in the header's switcher, in dp.
const CONTEXT_NAME_WIDTH: f32 = 170.;

fn middle(name: &str, max: usize) -> String {
    let chars: Vec<_> = name.chars().collect();
    if chars.len() <= max {
        return name.into();
    }
    let keep = max.saturating_sub(1);
    chars[..keep.div_ceil(2)]
        .iter()
        .chain(std::iter::once(&'…'))
        .chain(chars[chars.len() - keep / 2..].iter())
        .collect()
}

impl Pilot {
    pub(in crate::desktop) fn prepare_context_display(&mut self, window: &mut Window, cx: &App) {
        let full = self.applied.context.as_deref().unwrap_or("No context");
        let font = window.text_system().resolve_font(&Font {
            family: cx.theme().font_family.clone(),
            weight: ui::HEADING_WEIGHT,
            ..Default::default()
        });
        let advance = window
            .text_system()
            .advance(font, ui::dp_px(13., window), 'm')
            .map(|size| f32::from(size.width))
            .unwrap_or(8.);
        // The header gives the name this much room beside the status glyph.
        let width = f32::from(ui::dp_px(CONTEXT_NAME_WIDTH, window));
        self.context_display.name = middle(full, (width / advance).floor().max(5.) as usize).into();
        let kube = self.kubernetes_summary.data();
        let kubernetes = kube
            .and_then(|summary| summary.version.loaded())
            .map(|version| version.as_str())
            .unwrap_or("unknown");
        let count = self.node_workspace.rows.len();
        let connected = if let Some(kube) = &self.kubernetes_only {
            matches!(kube.connection, KubeConnection::Connected { .. })
        } else {
            self.overview.data().is_some()
        };
        let failed = self.config_error.is_some()
            || self
                .kubernetes_only
                .as_ref()
                .is_some_and(|kube| matches!(kube.connection, KubeConnection::Failed(_)))
            || (!connected && self.overview.error().is_some());
        self.context_display.state = if failed {
            2
        } else if connected {
            1
        } else {
            0
        };
        self.context_display.detail = if failed {
            "Couldn't connect".into()
        } else if connected {
            format!(
                "{} · {count} nodes",
                if self.kubernetes_only.is_some() {
                    "Kubernetes"
                } else {
                    "Talos + Kubernetes"
                }
            )
            .into()
        } else {
            "Connecting…".into()
        };
        let mut versions: Vec<_> = self
            .nodes
            .iter()
            .filter_map(|node| node.version.as_deref())
            .collect();
        versions.sort_unstable();
        versions.dedup();
        let talos = match (versions.first(), versions.last()) {
            (Some(first), Some(last)) if first != last => format!("{first}–{last}"),
            (Some(first), _) => (*first).into(),
            _ => "unknown".into(),
        };
        self.context_display.counts = self
            .context_nodes
            .iter()
            .map(|(name, count)| {
                (
                    name.clone(),
                    format!("{count} {}", if *count == 1 { "node" } else { "nodes" }).into(),
                )
            })
            .collect();
        self.context_display.status = if self.kubernetes_only.is_some() {
            format!("Connected to {full} · Kubernetes {kubernetes}")
        } else {
            format!("Connected to {full} · Talos {talos} · Kubernetes {kubernetes}")
        }
        .into();
    }
}

#[cfg(test)]
mod tests {
    use super::middle;
    #[test]
    fn middle_keeps_both_ends_and_unicode() {
        assert_eq!(middle("abcdefghij", 7), "abc…hij");
        assert_eq!(middle("αβγδεζηθ", 5), "αβ…ηθ");
        assert_eq!(middle("short", 8), "short");
    }
}
