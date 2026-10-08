//! Bounded charts: samples are prepared on range changes, never in render.
use super::*;
use crate::monitoring::colors::{AREA_OPACITY, Ink};
pub(super) fn chart(chart: &Chart, id: &'static str, cx: &App) -> AnyElement {
    let p = palette(cx);
    let plot = chart.clone();
    let legend = line().children(chart.series.iter().enumerate().map(|(ix, series)| {
        line()
            .child(
                div()
                    .w(dp(12.))
                    .h(px(2.))
                    .bg(Ink::Slot(ix).color(&p, false)),
            )
            .child(mono(series.label).text_color(p.ink_2))
    }));
    card(chart.title, cx)
        .child(
            body()
                .pt_0()
                .child(
                    line()
                        .child(muted(chart.unit, cx))
                        .child(div().flex_1())
                        .child(legend),
                )
                .child(
                    div()
                        .id(id)
                        .test_support()
                        .relative()
                        .h(dp(160.))
                        .w_full()
                        .children(chart.y_ticks.iter().enumerate().map(|(i, label)| {
                            muted(label.clone(), cx)
                                .absolute()
                                .left_0()
                                .top(dp(2. + i as f32 * 43.3))
                                .text_size(dp(10.))
                                .w(dp(26.))
                                .text_right()
                        }))
                        .child(
                            canvas(
                                |_, _, _| {},
                                move |bounds, _, window, _| {
                                    let scale = ui::dp_px(1., window);
                                    let left = bounds.left() + scale * 30.;
                                    let width = (bounds.size.width - scale * 36.).max(px(1.));
                                    let top = bounds.top() + scale * 8.;
                                    let height = (bounds.size.height - scale * 30.).max(px(1.));
                                    let xy = |x: f32, y: f32| {
                                        point(
                                            left + width * x,
                                            top + height * (1. - y / plot.maximum),
                                        )
                                    };
                                    let mut grid = PathBuilder::stroke(px(1.));
                                    for row in 0..=3 {
                                        let y = top + height * (row as f32 / 3.);
                                        grid.move_to(point(left, y));
                                        grid.line_to(point(left + width, y));
                                    }
                                    if let Ok(path) = grid.build() {
                                        window.paint_path(path, p.line);
                                    }
                                    for (ix, series) in plot.series.iter().enumerate() {
                                        let color = Ink::Slot(ix).color(&p, false);
                                        let count =
                                            series.values.len().saturating_sub(1).max(1) as f32;
                                        let mut path = PathBuilder::stroke(px(2.));
                                        let mut area = PathBuilder::fill();
                                        area.move_to(xy(0., 0.));
                                        for (i, value) in series.values.iter().enumerate() {
                                            let pt = xy(i as f32 / count, *value);
                                            if i == 0 {
                                                path.move_to(pt)
                                            } else {
                                                path.line_to(pt)
                                            }
                                            area.line_to(pt);
                                        }
                                        area.line_to(xy(1., 0.));
                                        area.close();
                                        if let Ok(path) = area.build() {
                                            window.paint_path(path, color.opacity(AREA_OPACITY));
                                        }
                                        if let Ok(path) = path.build() {
                                            window.paint_path(path, color);
                                        }
                                    }
                                    for (x, color) in
                                        [(plot.deploy, p.accent), (plot.event, p.crit)]
                                    {
                                        if !(0. ..=1.).contains(&x) {
                                            continue;
                                        }
                                        let mut marker = PathBuilder::stroke(px(1.));
                                        marker.move_to(xy(x, 0.));
                                        marker.line_to(xy(x, plot.maximum));
                                        if let Ok(path) = marker.build() {
                                            window.paint_path(path, color.opacity(0.6));
                                        }
                                        let pt = xy(x, 0.);
                                        let mut triangle = PathBuilder::fill();
                                        triangle.move_to(point(pt.x - px(4.), pt.y));
                                        triangle.line_to(point(pt.x + px(4.), pt.y));
                                        triangle.line_to(point(pt.x, pt.y - px(6.)));
                                        triangle.close();
                                        if let Ok(path) = triangle.build() {
                                            window.paint_path(path, color);
                                        }
                                    }
                                },
                            )
                            .size_full(),
                        ),
                )
                .child(
                    h_flex()
                        .justify_between()
                        .children(chart.ticks.iter().map(|tick| muted(tick.clone(), cx))),
                ),
        )
        .into_any_element()
}
