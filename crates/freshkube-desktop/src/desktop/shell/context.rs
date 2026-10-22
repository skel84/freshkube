//! Labels are prepared on source and text-size changes, never while drawing.
use super::*;
use freshkube_core::pluralize;
use freshkube_core::write::Protection;
use freshkube_ui::level::Mark;

/// Where the applied context's connection stands, as the header's switcher
/// and Overview's heading show it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(in crate::desktop) enum Connection {
    #[default]
    Connecting,
    Connected,
    Failed,
}

#[derive(Default)]
pub(in crate::desktop) struct ContextDisplay {
    /// The context's whole name, for the label and tooltip.
    pub(in crate::desktop) full: SharedString,
    pub(in crate::desktop) name: SharedString,
    pub(in crate::desktop) detail: SharedString,
    pub(in crate::desktop) status: SharedString,
    pub(in crate::desktop) state: Connection,
    /// The protection level of the context shown (#852), prepared with the
    /// rest so the header and the status bar only draw it.
    pub(in crate::desktop) level: Protection,
    /// What the level's tag says in a tooltip and to a screen reader,
    /// prepared with the level.
    pub(in crate::desktop) level_words: SharedString,
    pub(in crate::desktop) open: bool,
    pub(in crate::desktop) counts: std::collections::BTreeMap<String, SharedString>,
}

/// The look of a protection level.
pub(in crate::desktop) fn mark(level: Protection) -> Mark {
    match level {
        Protection::ReadOnly => Mark::ReadOnly,
        Protection::Protected => Mark::Protected,
        Protection::Guarded => Mark::Guarded,
        Protection::Open => Mark::Open,
    }
}

/// What a level's tag says in a tooltip and to a screen reader.
pub(in crate::desktop) fn level_words(level: Protection) -> String {
    format!("Protection: {}. {}", level.label(), level.meaning())
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
    /// The context the header names: the applied one, else the context the
    /// active workspace entry asks for while its connection has not named
    /// one (its kubeconfig can't be read yet, or at all), else none.
    pub(in crate::desktop) fn display_context(&self, cx: &App) -> String {
        if let Some(context) = &self.applied.context {
            return context.clone();
        }
        let workspace = self.settings_page.read(cx).workspace();
        self.switch
            .active_cluster()
            .and_then(|id| workspace.clusters.iter().find(|entry| entry.id == id))
            .map(|entry| entry.context.clone())
            .unwrap_or_else(|| "No context".into())
    }

    /// The level the window works under right now, from the entry and the
    /// context as they stand: see [`Self::prepare_protection`], which keeps
    /// it for drawing. The app's follows are offered this and the
    /// connection in one call, so a level never reaches another cluster's
    /// follows.
    pub(in crate::desktop) fn current_level(&self, cx: &App) -> Protection {
        let shown = self.display_context(cx);
        let settings = self.settings_page.read(cx);
        let workspace = settings.workspace();
        let active = self
            .switch
            .active_cluster()
            .and_then(|id| workspace.clusters.iter().find(|entry| entry.id == id));
        match active {
            Some(entry) => workspace
                .level_of(&entry.context, &settings.source_recovery)
                .stricter(entry.level()),
            None => workspace.level_of(&shown, &settings.source_recovery),
        }
    }

    /// The level the window works under: the active workspace entry's, with
    /// the stricter of any entry naming the same kubeconfig context, else
    /// guarded, else what a skipped row locked it to. The entry is the
    /// source, not the name the header shows: the applied context can be a
    /// Talos context that no entry's `context` names. With no entry active,
    /// the context shown decides. Prepared on a source change and when the
    /// setup changes, never while drawing.
    pub(in crate::desktop) fn prepare_protection(&mut self, cx: &mut App) -> bool {
        let level = self.current_level(cx);
        if level == self.context_display.level && !self.context_display.level_words.is_empty() {
            return false;
        }
        self.context_display.level = level;
        self.context_display.level_words = level_words(level).into();
        // What writes ask: the slot for the operations, the dock's shells.
        crate::mutation::set_protection(level, cx);
        self.pages
            .dock
            .update(cx, |dock, cx| dock.set_read_only(level.is_read_only(), cx));
        // Hide details defaults on for a protected or read-only cluster.
        self.offer_followed(cx);
        true
    }

    pub(in crate::desktop) fn prepare_context_display(
        &mut self,
        window: &mut Window,
        cx: &mut App,
    ) {
        let full = self.display_context(cx);
        self.prepare_protection(cx);
        self.context_display.full = full.clone().into();
        let full = full.as_str();
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
        let kube = self.switch.registry.active().kubernetes_summary.data();
        let kubernetes = kube
            .and_then(|summary| summary.version.loaded())
            .map(|version| version.as_str())
            .unwrap_or("unknown");
        let count = self.pages.node_workspace.read(cx).rows.len();
        let connected = if let Some(kube) = self.mode().kubernetes_only() {
            matches!(kube.connection, KubeConnection::Connected { .. })
        } else {
            self.overview.data().is_some()
        };
        let failed = self.config_error.is_some()
            || self
                .mode()
                .kubernetes_only()
                .is_some_and(|kube| matches!(kube.connection, KubeConnection::Failed(_)))
            || (!connected && self.overview.error().is_some());
        self.context_display.state = if failed {
            Connection::Failed
        } else if connected {
            Connection::Connected
        } else {
            Connection::Connecting
        };
        self.context_display.detail = if failed {
            "Couldn't connect".into()
        } else if connected {
            format!(
                "{} · {count} nodes",
                if !self.mode().has_talos() {
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
            .target
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
            .map(|(name, count)| (name.clone(), pluralize(*count, "node", "nodes").into()))
            .collect();
        self.context_display.status = if !self.mode().has_talos() {
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
