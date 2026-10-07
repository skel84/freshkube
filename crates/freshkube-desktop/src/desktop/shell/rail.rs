//! The icon rail: one button per area, with a dot where the overview
//! found a problem in it.
use super::*;
use crate::presentation::overview::Card;
use std::collections::HashMap;

/// The rail's problem dots and what each one is about, derived with the
/// overview's cards so the rail only reads them.
#[derive(Default)]
pub(in crate::desktop) struct RailMarks(HashMap<Area, (Tone, SharedString)>);

impl RailMarks {
    /// A warning or critical card marks its area; critical wins.
    pub(in crate::desktop) fn from_cards(cards: &[Card], fixture: bool) -> Self {
        let mut marks: HashMap<Area, (Tone, Vec<String>)> = HashMap::new();
        for card in cards {
            if !matches!(card.tone, Tone::Warn | Tone::Crit) {
                continue;
            }
            let area = match card.id {
                "tile-nodes" | "tile-memory" => Area::Nodes,
                "tile-etcd" | "tile-services" => Area::ControlPlane,
                "tile-workloads" | "tile-pods" => Area::Group("workloads"),
                "tile-events" => Area::Events,
                "tile-storage" => Area::Group("storage"),
                _ => continue,
            };
            let (tone, lines) = marks.entry(area).or_insert((card.tone, Vec::new()));
            if card.tone == Tone::Crit {
                *tone = Tone::Crit;
            }
            lines.push(format!("{}: {} · {}", card.label, card.figure, card.detail));
        }
        if fixture {
            marks.insert(
                Area::Observability,
                (Tone::Crit, vec!["Example data · 2 open incidents".into()]),
            );
        }
        Self(
            marks
                .into_iter()
                .map(|(area, (tone, lines))| (area, (tone, lines.join("\n").into())))
                .collect(),
        )
    }

    #[cfg(test)]
    pub(in crate::desktop) fn tone(&self, area: Area) -> Option<Tone> {
        self.0.get(&area).map(|(tone, _)| *tone)
    }
}

fn icon(area: Area) -> IconName {
    match area {
        Area::Overview => IconName::LayoutDashboard,
        Area::Nodes => IconName::Server,
        Area::Namespaces => IconName::Folders,
        Area::Events => IconName::Activity,
        Area::Monitoring => IconName::ChartLine,
        Area::Observability => IconName::Radar,
        Area::Group("workloads") => IconName::Boxes,
        Area::Group("networking") => IconName::Network,
        Area::Group("configuration") => IconName::SlidersHorizontal,
        Area::Group("storage") => IconName::HardDrive,
        Area::Group("access-control") => IconName::KeyRound,
        Area::Group(_) => IconName::Cog,
        Area::Custom => IconName::Puzzle,
        Area::ControlPlane => IconName::ServerCog,
    }
}

impl Pilot {
    pub(in crate::desktop) fn render_rail(&self, cx: &mut Context<Self>) -> AnyElement {
        let p = palette(cx);
        let divider = || div().w(dp(28.)).h(px(1.)).my(dp(6.)).flex_none().bg(p.line);
        let mut rail = v_flex()
            .id("nav-rail-list")
            .size_full()
            .overflow_y_scroll()
            .restrict_scroll_to_axis()
            .track_scroll(&self.rail_scroll)
            .items_center()
            .py(dp(12.))
            .gap(dp(4.));
        for (ix, section) in Area::RAIL.iter().enumerate() {
            if ix > 0 {
                rail = rail.child(divider());
            }
            rail = rail.children(section.iter().map(|area| self.rail_button(*area, cx)));
        }
        super::column::icon_strip(
            rail,
            &self.rail_scroll,
            "nav-rail-scrollbar",
            cx.theme().sidebar,
        )
        .id("nav-rail")
        .test_support()
        .aria_label("Areas")
        .w(dp(RAIL_WIDTH))
        .flex_none()
        .h_full()
        .bg(cx.theme().sidebar)
        .border_r_1()
        .border_color(cx.theme().sidebar_border)
        .into_any_element()
    }

    fn rail_button(&self, area: Area, cx: &Context<Self>) -> AnyElement {
        let p = palette(cx);
        let active = self.area == area;
        let mark = self.rail_marks.0.get(&area).cloned();
        let why = mark.as_ref().map(|(_, why)| why.clone());
        let tone = mark.map(|(tone, _)| {
            if tone == Tone::Crit {
                Tone::Crit
            } else {
                Tone::Warn
            }
        });
        h_flex()
            .id(area.id())
            .test_support()
            .role(Role::Tab)
            .aria_selected(active)
            .aria_label(area.label())
            .tab_index(0)
            .relative()
            .size(dp(44.))
            .flex_none()
            .justify_center()
            .rounded(px(10.))
            .cursor_pointer()
            .when(active, |this| this.bg(p.accent_soft))
            .when(!active, |this| this.hover(|style| style.bg(p.hover)))
            .child(Icon::new(icon(area)).size(dp(20.)).text_color(if active {
                p.accent
            } else {
                p.muted
            }))
            .children(tone.map(|tone| {
                ui::badge_dot(tone, Some(cx.theme().sidebar), cx)
                    .id("rail-mark")
                    .test_support()
                    .absolute()
                    .top(dp(8.))
                    .right(dp(8.))
            }))
            // Built on hover, so the rail formats nothing while drawing.
            .tooltip(move |window, cx| {
                let mut text = area.label().to_owned();
                if let Some(key) = area.shortcut() {
                    text.push_str(&format!("  {}{key}", ui::modifier()));
                }
                if let Some(why) = &why {
                    text.push('\n');
                    text.push_str(why);
                }
                Tooltip::new(text).build(window, cx)
            })
            .on_click(cx.listener(move |view, _, window, cx| view.show_area(area, window, cx)))
            .into_any_element()
    }
}
