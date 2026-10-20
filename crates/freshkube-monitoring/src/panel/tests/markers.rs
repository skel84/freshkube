use super::*;
use crate::panel::markers;

#[gpui_kit::test]
fn markers_on_the_window_draw_and_the_readout_names_the_one_under_the_pointer(
    cx: &mut TestAppContext,
) {
    use freshkube_core::monitoring::markers::{Marker, MarkerKind};
    let (dashboard, specs) = cluster();
    let cpu = index_of(&specs, "CPU usage by node");
    let stat = index_of(&specs, "Ready nodes");
    let (handle, panels) = mount(cx, specs);
    answer(cx, &dashboard, &panels);
    let marker = |kind, at: i64, label: &str| Marker {
        kind,
        at,
        namespace: None,
        node: None,
        label: label.into(),
    };
    let markers: Rc<[Marker]> = Rc::from(vec![
        marker(MarkerKind::Deploy, END - 7 * 3600, "deploy/old → 3"),
        marker(MarkerKind::Deploy, END - 3 * 3600, "deploy/api → 1.8.2"),
        marker(MarkerKind::NodeNotReady, END - 600, "worker-2 NotReady"),
    ]);
    for panel in &panels {
        let markers = markers.clone();
        cx.update(|cx| panel.update(cx, |panel, cx| panel.set_markers(markers, cx)));
    }
    let labels = cx.read(|cx| panels[cpu].read(cx).marker_labels());
    assert_eq!(labels, ["deploy/api → 1.8.2", "worker-2 NotReady"]);
    assert!(cx.read(|cx| panels[stat].read(cx).marker_labels().is_empty()));

    cx.update_window(handle, |_, window, cx| window.draw(cx).clear(cx))
        .unwrap();
    let geometry = cx.read(|cx| panels[cpu].read(cx).geometry.get());
    let move_to = |cx: &mut TestAppContext, fraction: f32| {
        cx.update_window(handle, |_, window, cx| {
            let position = geometry.origin
                + gpui_kit::point(
                    geometry.left + geometry.width * fraction,
                    geometry.top + geometry.height / 2.,
                );
            window.dispatch_event(
                gpui_kit::PlatformInput::MouseMove(gpui_kit::MouseMoveEvent {
                    position,
                    pressed_button: None,
                    modifiers: Default::default(),
                }),
                cx,
            );
            window.draw(cx).clear(cx);
        })
        .unwrap();
    };
    let marker_at = |cx: &mut TestAppContext| {
        cx.read(|cx| panels[cpu].read(cx).cursor.clone())
            .and_then(|cursor| cursor.marker)
    };
    // Three hours before the end of six is halfway.
    move_to(cx, 0.5);
    assert_eq!(marker_at(cx), Some(0));
    assert!(shown(
        cx,
        handle,
        &format!("monitoring-panel-{cpu}-readout")
    ));
    move_to(cx, 0.25);
    assert_eq!(marker_at(cx), None);

    // A new answer over a later window places them again.
    let later = TimeWindow::new(END + 4 * 3600, 6 * 3600, 72);
    let source = ExampleSource::new(BUILTINS[0].uid);
    let (variables, _) = source
        .resolve_variables(&dashboard.variables, &[], later)
        .unwrap();
    let context = QueryContext {
        window: later,
        variables: variables.context_values(),
    };
    cx.update(|cx| {
        panels[cpu].update(cx, |panel, cx| {
            let result = source
                .query_panel(&panel.spec.clone(), &context, &variables)
                .unwrap();
            panel.set_result(result, later, cx);
        })
    });
    let labels = cx.read(|cx| panels[cpu].read(cx).marker_labels());
    assert_eq!(labels, ["worker-2 NotReady"]);
}

#[test]
fn deploy_marker_ink_is_separate_from_every_series_ink_in_both_themes() {
    use crate::colors::Ink;
    use freshkube_core::{coroot::SeriesColor, monitoring::markers::MarkerKind};
    for p in [crate::palette::light(), crate::palette::dark()] {
        let deploy = markers::ink(MarkerKind::Deploy, &p).0;
        assert_eq!(deploy, p.ink_2);
        let mut inks = vec![
            Ink::Slot(0),
            Ink::Slot(1),
            Ink::Overflow(2),
            Ink::Overflow(3),
        ];
        inks.extend([0., 0.25, 0.5, 0.75, 1.].map(Ink::Level));
        inks.extend(
            [
                SeriesColor::Critical,
                SeriesColor::Warning,
                SeriesColor::Ok,
                SeriesColor::Blue,
                SeriesColor::Orange,
                SeriesColor::Purple,
                SeriesColor::Grey,
            ]
            .map(Ink::Named),
        );
        for ink in inks {
            for focused in [false, true] {
                assert_ne!(deploy, ink.color(&p, focused), "{ink:?}, focused {focused}");
            }
        }
        assert_eq!(
            markers::ink(MarkerKind::NodeNotReady, &p),
            (p.crit, 0.55, 1.)
        );
        assert_eq!(markers::ink(MarkerKind::NodeReboot, &p), (p.crit, 0.55, 1.));
        assert_eq!(
            markers::ink(MarkerKind::NodeReady, &p),
            (p.crit, 0.25, 0.45)
        );
    }
}

#[test]
fn deploy_marker_line_and_triangle_reach_three_to_one_on_both_cards() {
    use freshkube_core::monitoring::markers::MarkerKind;
    let luminance = |color: [f32; 3]| {
        let [r, g, b] = color.map(|c| {
            if c <= 0.04045 {
                c / 12.92
            } else {
                ((c + 0.055) / 1.055).powf(2.4)
            }
        });
        0.2126 * r + 0.7152 * g + 0.0722 * b
    };
    for (theme, p) in [
        ("light", crate::palette::light()),
        ("dark", crate::palette::dark()),
    ] {
        let (ink, line, triangle) = markers::ink(MarkerKind::Deploy, &p);
        let (ink, card) = (ink.to_rgb(), p.surface.to_rgb());
        let ground = luminance([card.r, card.g, card.b]);
        for opacity in [line, triangle] {
            let mix = |ink, card| ink * opacity + card * (1. - opacity);
            let foreground =
                luminance([mix(ink.r, card.r), mix(ink.g, card.g), mix(ink.b, card.b)]);
            let ratio = (foreground.max(ground) + 0.05) / (foreground.min(ground) + 0.05);
            assert!(ratio >= 3., "{theme}, opacity {opacity}: {ratio:.2}");
        }
    }
}

#[gpui_kit::test]
fn a_marker_readout_keeps_its_whole_label_at_half_width(cx: &mut TestAppContext) {
    use freshkube_core::monitoring::markers::{Marker, MarkerKind};
    use gpui_kit::{Font, FontWeight, Hsla, TextRun};
    let (handle, panel) = narrow_chart(cx, 360.);
    let label: gpui_kit::SharedString = "4f2a9c · worker:1.8.2".into();
    for theme in [ThemeMode::Dark, ThemeMode::Light] {
        cx.update(|cx| Theme::change(theme, None, cx));
        for percent in [38, 62] {
            let marker = Marker {
                at: END - 6 * 3600 * (100 - percent) / 100,
                kind: MarkerKind::Deploy,
                label: label.to_string(),
                namespace: None,
                node: None,
            };
            cx.update(|cx| panel.update(cx, |panel, cx| panel.set_markers(Rc::from([marker]), cx)));
            frame(cx, handle);
            let geometry = cx.read(|cx| panel.read(cx).geometry.get());
            let counts = || {
                [
                    "monitoring-panel",
                    "monitoring-plot",
                    "monitoring-plot-paint",
                    "monitoring-legend",
                    "monitoring-path",
                ]
                .map(probe::count)
            };
            let before = counts();
            cx.update_window(handle, |_, window, cx| {
                window.dispatch_event(
                    gpui_kit::PlatformInput::MouseMove(gpui_kit::MouseMoveEvent {
                        position: geometry.origin
                            + point(
                                geometry.left + geometry.width * percent as f32 / 100.,
                                geometry.top + geometry.height / 2.,
                            ),
                        pressed_button: None,
                        modifiers: Default::default(),
                    }),
                    cx,
                );
                window.draw(cx).clear(cx);
                let readout = window.find("monitoring-panel-0-readout").bounds();
                let name = window
                    .find("monitoring-panel-0-readout-marker-label")
                    .bounds();
                let date = window
                    .find("monitoring-panel-0-readout-marker-time")
                    .bounds();
                let run = TextRun {
                    len: label.len(),
                    font: Font {
                        family: crate::ui::MONO_FONT.into(),
                        weight: FontWeight::NORMAL,
                        ..window.text_style().font()
                    },
                    color: Hsla::default(),
                    background_color: None,
                    underline: None,
                    strikethrough: None,
                };
                let lines = window
                    .text_system()
                    .shape_text(
                        label.clone(),
                        crate::ui::dp_px(12., window),
                        &[run],
                        Some(name.size.width),
                        None,
                    )
                    .unwrap();
                let whole_height = lines.iter().fold(px(0.), |height, line| {
                    height + line.size(crate::ui::dp_px(18., window)).height
                });
                assert!(
                    whole_height >= crate::ui::dp_px(36., window),
                    "the label should wrap"
                );
                assert!(
                    (name.size.height - whole_height).abs() <= px(1.),
                    "{name:?}, whole label {whole_height:?}"
                );
                assert!(date.top() >= name.bottom(), "{date:?} above {name:?}");
                for text in [name, date] {
                    assert!(
                        text.left() >= readout.left() && text.right() <= readout.right() + px(0.5)
                    );
                    assert!(text.bottom() <= readout.bottom());
                }
                let plot = window.find("monitoring-panel-0-plot").bounds();
                assert!(readout.left() >= plot.left() && readout.right() <= plot.right() + px(0.5));
                assert!(readout.bottom() <= plot.bottom(), "{readout:?} in {plot:?}");
            })
            .unwrap();
            assert_eq!(counts(), before, "a marker hover redrew the cached chart");
            assert_eq!(
                cx.read(|cx| panel.read(cx).cursor.as_ref().unwrap().marker),
                Some(0)
            );
        }
    }
}

#[gpui_kit::test]
fn a_wrapped_marker_leaves_room_for_the_series_in_a_short_plot(cx: &mut TestAppContext) {
    use freshkube_core::monitoring::markers::{Marker, MarkerKind};
    let (handle, panels, result) = crowded_at(cx, 360.);
    let panel = &panels[0];
    cx.update(|cx| {
        panel.update(cx, |panel, cx| {
            panel.set_result(result, window_range(), cx);
            panel.set_markers(
                Rc::from([Marker {
                    at: END - 3 * 3600,
                    kind: MarkerKind::Deploy,
                    label: "4f2a9c · worker:1.8.2".into(),
                    namespace: None,
                    node: None,
                }]),
                cx,
            );
            panel.toggle_picked(0, cx);
        })
    });
    frame(cx, handle);
    cx.update_window(handle, |_, window, cx| {
        let geometry = panel.read(cx).geometry.get();
        window.dispatch_event(
            gpui_kit::PlatformInput::MouseMove(gpui_kit::MouseMoveEvent {
                position: geometry.origin
                    + point(
                        geometry.left + geometry.width / 2.,
                        geometry.top + geometry.height / 2.,
                    ),
                pressed_button: None,
                modifiers: Default::default(),
            }),
            cx,
        );
        window.draw(cx).clear(cx);
        let plot = window.find("monitoring-panel-0-plot").bounds();
        let readout = window.find("monitoring-panel-0-readout").bounds();
        assert!(readout.bottom() <= plot.bottom(), "{readout:?} in {plot:?}");
        let cursor = panel.read(cx).cursor.as_ref().unwrap();
        assert_eq!(cursor.marker, Some(0));
        assert!(cursor.more > 0);
        assert!(cursor.rows.iter().any(|row| row.series == 0));
    })
    .unwrap();
}

#[gpui_kit::test]
fn marker_focus_draws_heavier_and_keeps_its_labels_until_cleared(cx: &mut TestAppContext) {
    use freshkube_core::monitoring::markers::{Marker, MarkerKind};
    let (dashboard, specs) = cluster();
    let cpu = index_of(&specs, "CPU usage by node");
    let (handle, panels) = mount(cx, specs);
    answer(cx, &dashboard, &panels);
    let panel = &panels[cpu];
    let at = END - 3 * 3600;
    let marker = |at, label: &str| Marker {
        kind: MarkerKind::Deploy,
        at,
        label: label.into(),
        namespace: None,
        node: None,
    };
    cx.update(|cx| {
        panel.update(cx, |panel, cx| {
            panel.set_markers(
                Rc::from([
                    marker(at, "checkout · 1.2.0"),
                    marker(at, "worker · 1.2.0"),
                    marker(at + 600, "api · 1.3.0"),
                    marker(END - 7 * 3600, "old · 1.1.0"),
                ]),
                cx,
            );
        })
    });
    let label_id = format!("monitoring-panel-{cpu}-focused-marker-0");
    let draw = |cx: &mut TestAppContext, labels: usize| {
        let before = probe::count("monitoring-focused-marker");
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            assert_eq!(window.try_find(label_id.clone()).is_some(), labels > 0);
        })
        .unwrap();
        assert_eq!(probe::count("monitoring-focused-marker") - before, labels);
    };
    let chart = cx.read(|cx| panel.read(cx).chart().unwrap());
    draw(cx, 0);
    cx.update(|cx| panel.update(cx, |panel, cx| panel.set_focused_marker(Some(at), cx)));
    draw(cx, 2);
    cx.read(|cx| {
        let panel = panel.read(cx);
        assert_eq!(
            panel.focused_marker_labels(),
            ["checkout · 1.2.0", "worker · 1.2.0"]
        );
        assert!(
            Rc::ptr_eq(&chart, &panel.chart().unwrap()),
            "focus never changes the chart"
        );
        assert!(panel.cursor.is_none(), "no hover is needed for the label");
    });
    cx.update(|cx| panel.update(cx, |panel, cx| panel.set_focused_marker(None, cx)));
    draw(cx, 0);
    cx.update(|cx| {
        panel.update(cx, |panel, cx| {
            panel.set_focused_marker(Some(END - 7 * 3600), cx)
        })
    });
    draw(cx, 0);
    assert!(cx.read(|cx| Rc::ptr_eq(&chart, &panel.read(cx).chart().unwrap())));
}
