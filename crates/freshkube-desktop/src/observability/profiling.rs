use super::*;
impl ObservabilityPage {
    fn zoom_frame(&mut self, index: usize, cx: &mut Context<Self>) {
        self.frame_root = Some(index);
        self.selected_frame = Some(index);
        self.visible_frames = (0..self.frames.len())
            .filter(|ix| {
                let mut at = Some(*ix);
                while let Some(i) = at {
                    if i == index {
                        return true;
                    }
                    at = self.frames[i].parent;
                }
                false
            })
            .collect();
        cx.notify();
    }
    pub(super) fn render_profiling(&self, cx: &Context<Self>) -> AnyElement {
        let p = palette(cx);
        let base = self.frame_root.map(|i| &self.frames[i]);
        let start = base.map_or(0., |f| f.x);
        let width = base.map_or(1., |f| f.width);
        let depth = base.map_or(0, |f| f.depth);
        let toolbar = line()
            .flex_wrap()
            .child(mono("argocd/argocd-application-controller"))
            .child(div().flex_1())
            .child(
                action(
                    "obs-profile-compare",
                    if self.compare_profile {
                        "Comparing previous period"
                    } else {
                        "Compare previous period"
                    },
                )
                .selected(self.compare_profile)
                .on_click(cx.listener(|this, _, _, cx| {
                    this.compare_profile = !this.compare_profile;
                    this.rebuild_charts();
                    cx.notify();
                })),
            );
        let increases = card("Biggest increases · self time", cx).child(body().children(
            [14, 15, 9, 16].into_iter().map(|ix| {
                let frame = &self.frames[ix];
                Button::new(SharedString::from(format!("obs-profile-increase-{ix}")))
                    .ghost()
                    .group("fog-control")
                    .small()
                    .justify_start()
                    .w_full()
                    .child(mono(frame.name).truncate().flex_1())
                    .child(mono(frame.cpu))
                    .child(mono(format!("+{}%", frame.delta)).text_color(p.crit_ink))
                    .on_click(cx.listener(move |this, _, _, cx| this.zoom_frame(ix, cx)))
            }),
        ));
        let flame = div()
            .id("obs-flame")
            .test_support()
            .relative()
            .h(dp(244.))
            .w_full()
            .children(self.visible_frames.iter().map(|ix| {
                let index = *ix;
                let frame = &self.frames[index];
                let color =
                    crate::palette::flame_color(if self.compare_profile { frame.delta } else { 0 });
                let matches = self.flame_matches[index];
                Button::new(SharedString::from(format!("obs-flame-{index}")))
                    .ghost()
                    .group("fog-control")
                    .absolute()
                    .left(relative((frame.x - start) / width))
                    .top(dp((frame.depth - depth) as f32 * 38.))
                    .w(relative(frame.width / width))
                    .h(dp(28.))
                    .px(dp(6.))
                    .rounded(px(3.))
                    .justify_start()
                    .bg(color)
                    .text_color(gpui_kit::white())
                    .opacity(if matches { 1. } else { 0.3 })
                    .overflow_hidden()
                    .child(mono(frame.name).truncate())
                    .tooltip(format!(
                        "{} · {} CPU · {:+}% self time · Click to zoom",
                        frame.name, frame.cpu, frame.delta
                    ))
                    .on_click(cx.listener(move |this, _, _, cx| this.zoom_frame(index, cx)))
            }));
        let details = if let Some(index) = self.selected_frame {
            let frame = &self.frames[index];
            line().flex_wrap().child(mono(frame.name)).child(muted(
                format!("{} CPU · {:+}% vs previous period", frame.cpu, frame.delta),
                cx,
            ))
        } else {
            line().child(status(Status::LogError,cx)).child(text("Most of the extra CPU is spent comparing application state. Select a frame to inspect and zoom."))
        };
        v_flex()
            .gap(dp(12.))
            .child(toolbar)
            .child(
                h_flex()
                    .items_stretch()
                    .flex_wrap()
                    .gap(dp(12.))
                    .child(div().flex_1().min_w(dp(320.)).child(plots::chart(
                        &self.charts[2],
                        "obs-profile-chart",
                        cx,
                    )))
                    .child(div().flex_1().min_w(dp(320.)).child(increases)),
            )
            .child(
                card("Flame graph", cx).child(
                    body()
                        .pt_0()
                        .child(
                            line()
                                .flex_wrap()
                                .child(muted("Width = CPU time · colour = change", cx))
                                .child(div().flex_1())
                                .children(
                                    [("less", -10), ("same", 0), ("more", 18)].into_iter().map(
                                        |(label, delta)| {
                                            line()
                                                .child(
                                                    div()
                                                        .size(dp(10.))
                                                        .rounded(px(3.))
                                                        .bg(crate::palette::flame_color(delta)),
                                                )
                                                .child(muted(label, cx))
                                        },
                                    ),
                                )
                                .child(
                                    div().w(dp(190.)).child(
                                        Input::new(&self.flame_query)
                                            .id("obs-flame-search")
                                            .small()
                                            .cleanable(true),
                                    ),
                                )
                                .child(action("obs-profile-reset", "Reset zoom").on_click(
                                    cx.listener(|this, _, _, cx| {
                                        this.frame_root = None;
                                        this.selected_frame = None;
                                        this.visible_frames = (0..this.frames.len()).collect();
                                        cx.notify();
                                    }),
                                )),
                        )
                        .child(flame)
                        .child(
                            div()
                                .p(dp(12.))
                                .rounded(px(8.))
                                .bg(p.surface_2)
                                .child(details),
                        ),
                ),
            )
            .into_any_element()
    }
}
