//! Live profiles for one application: the profile types Coroot has, and one
//! flame graph, optionally compared with the window before. The frames to
//! draw are chosen when the answer, the zoom or the search changes.
use super::*;
use freshkube_core::coroot as api;
use gpui_kit::component::menu::{DropdownMenu, PopupMenuItem};
use std::time::Duration;

/// Frames narrower than this share of the zoomed frame are not drawn.
const MIN_DRAWN: f64 = 0.003;
/// Frames narrower than this share draw without a name: a few letters and an
/// ellipsis say less than the tooltip, and they bury the wide frames.
const MIN_LABELLED: f64 = 0.025;
/// Frames drawn at most.
const MAX_DRAWN: usize = 600;
/// Rows drawn below the zoomed frame.
const MAX_ROWS: usize = 64;
const ROW: f32 = 24.;
/// Increases listed beside the flame graph.
const INCREASES: usize = 6;

#[derive(Default)]
pub(super) struct Profiles {
    pub(super) query: api::ProfileQuery,
    app: Option<api::AppId>,
    kinds: Rc<[(String, String)]>,
    instances: Rc<[String]>,
    kind_label: String,
    instance_label: String,
    /// The type shown, for comparing it.
    shown_kind: Option<String>,
    note: String,
    graph: Option<api::FlameGraph>,
    unit: Option<api::ProfileUnit>,
    zoom: usize,
    selected: Option<usize>,
    search: String,
    drawn: Vec<Drawn>,
    rows: usize,
    hidden: usize,
    increases: Vec<(usize, String, String)>,
    detail: Option<FrameDetail>,
    legend: String,
    /// The example's answer arrived; a live page asks its snapshot.
    answered: bool,
}

struct Drawn {
    index: usize,
    left: f32,
    width: f32,
    row: usize,
    delta: i8,
    dim: bool,
    label: Option<SharedString>,
    tooltip: SharedString,
}

struct FrameDetail {
    name: String,
    facts: String,
}

impl Profiles {
    /// Another application or project: forget its types and choices.
    pub(super) fn reset_for(&mut self, app: Option<&api::AppId>) {
        if self.app.as_ref() != app {
            *self = Self {
                app: app.cloned(),
                search: std::mem::take(&mut self.search),
                ..Self::default()
            };
        }
    }

    fn prepare(&mut self, profiling: &api::Profiling) {
        self.answered = true;
        self.note = profiling.message.clone();
        self.kinds = profiling
            .kinds
            .iter()
            .map(|k| (k.id.clone(), k.name.clone()))
            .collect();
        self.instances = profiling.instances.iter().cloned().collect();
        self.graph = profiling.graph.clone();
        self.shown_kind = self
            .graph
            .as_ref()
            .map(|g| g.kind.clone())
            .or_else(|| self.query.kind.clone());
        self.unit = self.shown_kind.as_deref().map(api::ProfileKind::unit);
        self.kind_label = self
            .shown_kind
            .as_ref()
            .and_then(|id| self.kinds.iter().find(|(kind, _)| kind == id))
            .map_or_else(|| "Profile type".into(), |(_, name)| name.clone());
        self.instance_label = self
            .query
            .instance
            .clone()
            .unwrap_or_else(|| "All instances".into());
        self.legend = match self.unit {
            Some(api::ProfileUnit::Bytes) => "Width = bytes",
            Some(api::ProfileUnit::Count) => "Width = count",
            _ => "Width = CPU time",
        }
        .to_string();
        if self.graph.as_ref().is_some_and(|g| g.compared) {
            self.legend += " · colour = share change from the window before";
        }
        self.zoom = 0;
        self.selected = None;
        self.increases = self.graph.as_ref().map_or_else(Vec::new, |graph| {
            let mut rising: Vec<(usize, f64)> = graph
                .frames
                .iter()
                .enumerate()
                .skip(1)
                .filter_map(|(ix, frame)| frame.change.filter(|c| *c >= 0.1).map(|c| (ix, c)))
                .collect();
            rising.sort_by(|a, b| b.1.total_cmp(&a.1));
            rising
                .into_iter()
                .take(INCREASES)
                .map(|(ix, change)| {
                    (
                        ix,
                        graph.frames[ix].name.clone(),
                        format!("+{change:.1} pp"),
                    )
                })
                .collect()
        });
        self.layout();
    }

    /// The frames' names, root first, for tests.
    #[cfg(test)]
    pub(super) fn frame_names(&self) -> Vec<&str> {
        self.graph
            .iter()
            .flat_map(|g| &g.frames)
            .map(|f| f.name.as_str())
            .collect()
    }

    pub(super) fn set_search(&mut self, search: &str) {
        if self.search != search {
            self.search = search.to_owned();
            self.layout();
        }
    }

    fn zoom(&mut self, index: usize) {
        self.zoom = index;
        self.selected = Some(index);
        self.layout();
    }

    fn select(&mut self, index: usize) {
        self.selected = Some(index);
        self.describe();
    }

    /// Chooses the frames to draw: the zoomed frame's ancestors at full
    /// width, then its descendants scaled to it.
    fn layout(&mut self) {
        self.drawn.clear();
        self.rows = 0;
        self.hidden = 0;
        let Some(graph) = &self.graph else {
            self.detail = None;
            return;
        };
        let frames = &graph.frames;
        let zoom = self.zoom.min(frames.len().saturating_sub(1));
        let Some(base) = frames.get(zoom) else {
            self.detail = None;
            return;
        };
        let mut inside = vec![false; frames.len()];
        inside[zoom] = true;
        let mut path = vec![];
        let mut at = base.parent;
        while let Some(ix) = at {
            path.push(ix);
            at = frames[ix].parent;
        }
        let search = self.search.to_lowercase();
        let drawn = |ix: usize, left: f64, width: f64, row: usize| {
            let frame = &frames[ix];
            Drawn {
                index: ix,
                left: left as f32,
                width: width as f32,
                row,
                delta: frame
                    .change
                    .map_or(0, |c| c.round().clamp(-100., 100.) as i8),
                dim: !search.is_empty() && !frame.name.to_lowercase().contains(&search),
                label: (width >= MIN_LABELLED).then(|| frame.name.clone().into()),
                tooltip: format!("{} · {}", frame.name, share(frame.width)).into(),
            }
        };
        for (row, &ix) in path.iter().rev().enumerate() {
            self.drawn.push(drawn(ix, 0., 1., row));
        }
        let top = path.len();
        for (ix, frame) in frames.iter().enumerate().skip(zoom) {
            if ix != zoom && !frame.parent.is_some_and(|parent| inside[parent]) {
                continue;
            }
            let width = frame.width / base.width;
            let depth = frame.depth - base.depth;
            if width < MIN_DRAWN || depth >= MAX_ROWS || self.drawn.len() >= MAX_DRAWN {
                self.hidden += 1;
                continue;
            }
            inside[ix] = true;
            self.drawn.push(drawn(
                ix,
                (frame.x - base.x) / base.width,
                width,
                top + depth,
            ));
        }
        self.hidden += graph.omitted;
        self.rows = self.drawn.iter().map(|d| d.row + 1).max().unwrap_or(0);
        self.describe();
    }

    fn describe(&mut self) {
        let Some(graph) = &self.graph else {
            return;
        };
        let Some(frame) = graph.frames.get(self.selected.unwrap_or(self.zoom)) else {
            self.detail = None;
            return;
        };
        let whole = graph.frames[0].total.max(1) as f64;
        let mut facts = vec![
            format!("{} of the total", share(frame.width)),
            format!(
                "{} in the frame itself",
                share(frame.self_value as f64 / whole)
            ),
        ];
        if !graph.compared
            && let Some(unit) = self.unit
        {
            facts.push(amount(frame.total, unit));
        }
        if let Some(change) = frame.change {
            facts.push(format!("{change:+.1} pp vs the window before"));
        }
        if self.selected.is_some_and(|ix| ix != self.zoom) {
            facts.push("click it again to zoom in".into());
        }
        self.detail = Some(FrameDetail {
            name: frame.name.clone(),
            facts: facts.join(" · "),
        });
    }
}

fn share(value: f64) -> String {
    match value * 100. {
        v if v >= 10. => format!("{v:.0}%"),
        v if v >= 0.1 => format!("{v:.1}%"),
        v => format!("{v:.2}%"),
    }
}

fn amount(value: i64, unit: api::ProfileUnit) -> String {
    let value = value as f64;
    match unit {
        api::ProfileUnit::Nanoseconds => {
            format::duration(Duration::from_secs_f64(value.max(0.) / 1e9))
        }
        api::ProfileUnit::Bytes => {
            const UNITS: [&str; 5] = ["B", "KiB", "MiB", "GiB", "TiB"];
            let mut value = value;
            let mut unit = 0;
            while value >= 1024. && unit < UNITS.len() - 1 {
                value /= 1024.;
                unit += 1;
            }
            format!("{value:.1} {}", UNITS[unit])
        }
        api::ProfileUnit::Count => format!("{value:.0}"),
    }
}

impl ObservabilityPage {
    pub(super) fn read_profiling(&mut self, cx: &mut Context<Self>) {
        let Some(app) = self.selected_app.clone() else {
            return;
        };
        self.live_profiles.reset_for(Some(&app));
        let query = self.live_profiles.query.clone();
        if self.fixture {
            // The example answers for the application asked, as Coroot would.
            let profiling = example::profiling(&app, &query);
            self.live_profiles.prepare(&profiling);
            cx.notify();
            return;
        }
        let (Some(provider), Some(source)) = (self.live.provider.clone(), self.live.source.clone())
        else {
            return;
        };
        let Some(identity) = self
            .live
            .identity(connection::Subject::Profiling(app.clone(), query.clone()))
        else {
            return;
        };
        let request = self.live.profiling.begin(identity);
        let range = self.live.range;
        self.live.profile_job = Some(self.spawn_owned_read(
            async move { provider.profiling(&source, range, &app, &query).await },
            move |this, result, cx| {
                if this
                    .live
                    .profiling
                    .apply(&request, result.map_err(|e| e.to_string()))
                {
                    if let Some(profiling) = this.live.profiling.data() {
                        this.live_profiles.prepare(profiling);
                    }
                    cx.notify();
                }
            },
            cx,
        ));
    }

    fn change_profile(
        &mut self,
        change: impl FnOnce(&mut api::ProfileQuery),
        cx: &mut Context<Self>,
    ) {
        let profiles = &mut self.live_profiles;
        if profiles.query.kind.is_none() {
            profiles.query.kind = profiles.shown_kind.clone();
        }
        change(&mut profiles.query);
        self.read_profiling(cx);
        cx.notify();
    }

    pub(super) fn render_live_profiling(&self, cx: &Context<Self>) -> AnyElement {
        let profiles = &self.live_profiles;
        let mut page = v_flex()
            .id("obs-live-profiling")
            .test_support()
            .gap(dp(12.))
            .child(self.evidence_header("obs-profile-app", cx));
        if self.selected_app.is_none() {
            return page
                .child(muted(
                    "Choose an application to read its profiles from Coroot.",
                    cx,
                ))
                .into_any_element();
        }
        let answered = if self.fixture {
            profiles.answered
        } else {
            self.live.profiling.data().is_some()
        };
        if !answered {
            return page.into_any_element();
        }
        page = page.child(self.profile_controls(cx));
        if !profiles.note.is_empty() && profiles.note != "OK" {
            page = page.child(muted(profiles.note.clone(), cx).whitespace_normal());
        }
        if profiles.graph.is_none() {
            return page
                .child(card("Flame graph", cx).child(body().pt_0().child(muted(
                    "Coroot has no profile of this type for this application in the window.",
                    cx,
                ))))
                .into_any_element();
        }
        if !profiles.increases.is_empty() {
            page = page.child(self.profile_increases(cx));
        }
        page.child(self.live_flame(cx)).into_any_element()
    }

    fn profile_controls(&self, cx: &Context<Self>) -> Div {
        let profiles = &self.live_profiles;
        let owner = cx.entity().downgrade();
        let kinds = profiles.kinds.clone();
        let current = profiles.shown_kind.clone();
        let kind_picker = action("obs-profile-kind", profiles.kind_label.clone())
            .dropdown_caret(true)
            .disabled(kinds.is_empty())
            .dropdown_menu(move |mut menu, _, _| {
                for (id, name) in kinds.iter() {
                    let (owner, id) = (owner.clone(), id.clone());
                    menu = menu.item(
                        PopupMenuItem::new(name.clone())
                            .checked(current.as_ref() == Some(&id))
                            .on_click(move |_, _, cx| {
                                _ = owner.update(cx, |this, cx| {
                                    let id = id.clone();
                                    this.change_profile(|query| query.kind = Some(id), cx)
                                });
                            }),
                    );
                }
                menu.scrollable(true).max_h(px(420.))
            });
        let owner = cx.entity().downgrade();
        let instances = profiles.instances.clone();
        let chosen = profiles.query.instance.clone();
        let instance_picker = action("obs-profile-instance", profiles.instance_label.clone())
            .dropdown_caret(true)
            .disabled(instances.is_empty())
            .dropdown_menu(move |menu, _, _| {
                let all = owner.clone();
                let mut menu = menu.item(
                    PopupMenuItem::new("All instances")
                        .checked(chosen.is_none())
                        .on_click(move |_, _, cx| {
                            _ = all.update(cx, |this, cx| {
                                this.change_profile(|query| query.instance = None, cx)
                            });
                        }),
                );
                for instance in instances.iter() {
                    let (owner, instance) = (owner.clone(), instance.clone());
                    menu = menu.item(
                        PopupMenuItem::new(instance.clone())
                            .checked(chosen.as_ref() == Some(&instance))
                            .on_click(move |_, _, cx| {
                                _ = owner.update(cx, |this, cx| {
                                    let instance = instance.clone();
                                    this.change_profile(|query| query.instance = Some(instance), cx)
                                });
                            }),
                    );
                }
                menu.scrollable(true).max_h(px(420.))
            });
        let compare = profiles.query.compare;
        line()
            .flex_wrap()
            .child(kind_picker)
            .child(instance_picker)
            .child(
                action(
                    "obs-live-profile-compare",
                    if compare {
                        "Comparing with the window before"
                    } else {
                        "Compare with the window before"
                    },
                )
                .selected(compare)
                .disabled(profiles.shown_kind.is_none())
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.change_profile(|query| query.compare = !compare, cx)
                })),
            )
    }

    fn profile_increases(&self, cx: &Context<Self>) -> Div {
        let p = palette(cx);
        card("Biggest increases · share of the total", cx).child(
            body()
                .pt_0()
                .children(
                    self.live_profiles
                        .increases
                        .iter()
                        .map(|(ix, name, change)| {
                            let index = *ix;
                            Button::new(SharedString::from(format!("obs-live-increase-{index}")))
                                .ghost()
                                .group("fog-control")
                                .small()
                                .justify_start()
                                .w_full()
                                .child(mono(name.clone()).truncate().flex_1())
                                .child(mono(change.clone()).flex_none().text_color(p.crit_ink))
                                .tooltip(name.clone())
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.live_profiles.zoom(index);
                                    cx.notify();
                                }))
                        }),
                ),
        )
    }

    fn live_flame(&self, cx: &Context<Self>) -> Div {
        let p = palette(cx);
        let profiles = &self.live_profiles;
        let compared = profiles.graph.as_ref().is_some_and(|g| g.compared);
        let selected = profiles.selected;
        let flame = div()
            .id("obs-live-flame")
            .test_support()
            .relative()
            .w_full()
            .h(dp(profiles.rows as f32 * ROW))
            .children(profiles.drawn.iter().map(|frame| {
                let index = frame.index;
                let chosen = selected == Some(index);
                Button::new(SharedString::from(format!("obs-live-flame-{index}")))
                    .ghost()
                    .group("fog-control")
                    .absolute()
                    .left(relative(frame.left))
                    .top(dp(frame.row as f32 * ROW))
                    .w(relative(frame.width))
                    .h(dp(ROW - 2.))
                    .px(dp(4.))
                    .rounded(px(3.))
                    .justify_start()
                    .bg(crate::palette::flame_color(if compared {
                        frame.delta
                    } else {
                        0
                    }))
                    .border(if chosen { px(2.) } else { px(1.) })
                    .border_color(if chosen {
                        p.ink
                    } else {
                        gpui_kit::transparent_black()
                    })
                    .when(chosen, |button| button.shadow_md())
                    .text_color(gpui_kit::white())
                    .opacity(if frame.dim { 0.3 } else { 1. })
                    .overflow_hidden()
                    .when_some(frame.label.clone(), |button, label| {
                        button.child(mono(label).text_size(dp(11.5)).truncate())
                    })
                    .tooltip(frame.tooltip.clone())
                    .on_click(cx.listener(move |this, event: &ClickEvent, _, cx| {
                        if event.click_count() > 1 || this.live_profiles.selected == Some(index) {
                            this.live_profiles.zoom(index);
                        } else {
                            this.live_profiles.select(index);
                        }
                        cx.notify();
                    }))
            }));
        card("Flame graph", cx).child(
            body()
                .pt_0()
                .child(
                    line()
                        .flex_wrap()
                        .child(muted(profiles.legend.clone(), cx))
                        .child(div().flex_1())
                        .child(
                            div().w(dp(190.)).child(
                                Input::new(&self.flame_query)
                                    .id("obs-live-flame-search")
                                    .small()
                                    .cleanable(true),
                            ),
                        )
                        .child(
                            action("obs-live-profile-reset", "Reset zoom")
                                .disabled(profiles.zoom == 0)
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.live_profiles.zoom(0);
                                    this.live_profiles.selected = None;
                                    this.live_profiles.describe();
                                    cx.notify();
                                })),
                        ),
                )
                .when_some(profiles.detail.as_ref(), |this, detail| {
                    this.child(
                        v_flex()
                            .id("obs-live-frame-detail")
                            .test_support()
                            .px(dp(12.))
                            .py(dp(8.))
                            .gap(dp(2.))
                            .rounded(px(8.))
                            .bg(p.surface_2)
                            .child(mono(detail.name.clone()).truncate())
                            .child(muted(detail.facts.clone(), cx).truncate()),
                    )
                })
                .child(flame)
                .when(profiles.hidden > 0, |this| {
                    this.child(muted(
                        format!(
                            "{} frames too narrow to draw · zoom in to see them",
                            profiles.hidden
                        ),
                        cx,
                    ))
                }),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::{Profiles, api};

    fn frame(name: &str, parent: Option<usize>, depth: usize, x: f64, width: f64) -> api::Frame {
        api::Frame {
            name: name.into(),
            parent,
            depth,
            x,
            width,
            total: (width * 1000.) as i64,
            self_value: 0,
            change: None,
        }
    }

    fn profiles() -> Profiles {
        let mut profiles = Profiles::default();
        profiles.prepare(&api::Profiling {
            graph: Some(api::FlameGraph {
                kind: "go:profile_cpu:nanoseconds".into(),
                compared: false,
                frames: vec![
                    frame("total", None, 0, 0., 1.),
                    frame("main", Some(0), 1, 0., 0.6),
                    frame("gc", Some(0), 1, 0.6, 0.4),
                    frame("handle", Some(1), 2, 0., 0.5),
                    frame("sliver", Some(1), 2, 0.5, 0.001),
                ],
                omitted: 3,
            }),
            ..Default::default()
        });
        profiles
    }

    #[test]
    fn a_flame_graph_draws_wide_frames_and_counts_what_it_leaves_out() {
        let profiles = profiles();
        let drawn: Vec<_> = profiles.drawn.iter().map(|d| d.index).collect();
        assert_eq!(drawn, vec![0, 1, 2, 3]);
        assert_eq!(profiles.hidden, 4);
        assert_eq!(profiles.rows, 3);
        assert_eq!(profiles.legend, "Width = CPU time");
    }

    #[test]
    fn zooming_keeps_the_path_and_scales_the_subtree() {
        let mut profiles = profiles();
        profiles.zoom(1);
        let drawn: Vec<_> = profiles
            .drawn
            .iter()
            .map(|d| (d.index, d.row, (d.width * 100.).round() as i32))
            .collect();
        // `sliver` is 0.17% of `main`: still too narrow.
        assert_eq!(drawn, vec![(0, 0, 100), (1, 1, 100), (3, 2, 83)]);
        profiles.set_search("hand");
        assert!(profiles.drawn.iter().all(|d| d.dim == (d.index != 3)));
    }
}
