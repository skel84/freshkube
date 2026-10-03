use std::rc::Rc;

use freshkube_core::monitoring::model::{
    Dashboard, PanelSpec,
    data::{Frame, Series},
    time::TimeWindow,
};
use serde_json::{Value, json};

use super::chart::{LEGEND_ROWS, clock, common_unit};
use super::ticks::{fitting, linear, log, nice_step, time_ticks};
use super::*;
use crate::monitoring::colors::{Ink, Tier};

const END: i64 = 1_700_003_600;

fn window() -> TimeWindow {
    TimeWindow::new(END, 3600, 7)
}

fn panel(panel: Value) -> PanelSpec {
    let mut panel = panel;
    panel["gridPos"] = json!({"x": 0, "y": 0, "w": 12, "h": 8});
    let dashboard = json!({"title": "Test", "panels": [panel]});
    Dashboard::parse(&dashboard.to_string())
        .unwrap()
        .panels
        .remove(0)
}

fn series(name: &str, values: &[f64]) -> Series {
    labelled(name, &[], values)
}

fn labelled(name: &str, labels: &[(&str, &str)], values: &[f64]) -> Series {
    Series {
        name: name.into(),
        query: "A".into(),
        field: None,
        labels: labels
            .iter()
            .map(|(key, value)| ((*key).into(), (*value).into()))
            .collect(),
        values: values.to_vec(),
    }
}

fn frame(series: Vec<Series>) -> Frame {
    let times = window().times().into_iter().map(|t| t as f64).collect();
    Frame { times, series }
}

fn chart_of(data: &PanelData) -> Rc<Chart> {
    match &data.body {
        Body::Chart(chart) => chart.clone(),
        _ => panic!("not a chart"),
    }
}

fn timeseries(defaults: Value, options: Value) -> PanelSpec {
    panel(json!({
        "type": "timeseries",
        "title": "Chart",
        "targets": [{"refId": "A", "expr": "up"}],
        "fieldConfig": {"defaults": defaults, "overrides": []},
        "options": options,
    }))
}

#[test]
fn a_few_series_get_slots_an_inline_legend_and_round_axis() {
    let spec = timeseries(json!({"unit": "percent"}), json!({}));
    let data = derive(
        &spec,
        frame(vec![
            series("cp-1", &[10., 20., 30., 40., 50., 60., 70.]),
            series("cp-2", &[5., 5., 5., 5., 5., 5., 5.]),
            series("cp-3", &[1., 2., 3., 4., 5., 6., 7.]),
        ]),
        window(),
    );
    let chart = chart_of(&data);
    let inks: Vec<Ink> = chart.series.iter().map(|series| series.ink).collect();
    assert_eq!(inks, [Ink::Slot(0), Ink::Slot(1), Ink::Slot(2)]);
    assert_eq!(chart.xs.first(), Some(&0.));
    assert_eq!(chart.xs.last(), Some(&1.));
    let axis = chart.axes[0].as_ref().unwrap();
    assert_eq!((axis.min, axis.max), (0., 80.));
    assert_eq!(axis.ticks.first().unwrap().label, "0%");
    assert!(chart.axes[1].is_none());
    assert_eq!(chart.legend.mode, LegendMode::Inline);
    assert_eq!(chart.legend.rows[0].values, ["70%"]);
    // The top of cp-1 is its last value over the axis's 80.
    assert_eq!(chart.series[0].tops.last(), Some(&(70. / 80.)));
}

#[test]
fn quantiles_take_the_blue_ramp_and_a_shared_unit_moves_to_the_title() {
    let spec = timeseries(json!({"unit": "ms"}), json!({}));
    let data = derive(
        &spec,
        frame(vec![
            series("p50", &[20., 25., 30., 22., 21., 24., 29.]),
            series("p95", &[120., 150., 140., 130., 151., 149., 151.]),
            series("p99", &[280., 300., 290., 310., 294., 280., 294.]),
        ]),
        window(),
    );
    let chart = chart_of(&data);
    let inks: Vec<Ink> = chart.series.iter().map(|series| series.ink).collect();
    assert_eq!(inks, [Ink::Level(0.), Ink::Level(0.5), Ink::Level(1.)]);
    assert_eq!(data.unit.as_deref(), Some("ms"));
    let labels: Vec<&str> = chart.axes[0]
        .as_ref()
        .unwrap()
        .ticks
        .iter()
        .map(|tick| tick.label.as_ref())
        .collect();
    assert!(
        labels.iter().all(|label| !label.contains(' ')),
        "{labels:?}"
    );
    assert_eq!(chart.legend.rows[2].values, ["294"]);
}

#[test]
fn a_zero_in_another_unit_does_not_keep_the_unit_on_the_axis() {
    assert_eq!(
        common_unit(["0 s", "200 ms", "400 ms"].into_iter()),
        Some("ms".into())
    );
    assert_eq!(common_unit(["0 s", "1 s", "500 ms"].into_iter()), None);
    assert_eq!(common_unit(["0", "50%"].into_iter()), None);
    assert_eq!(common_unit(["0 B"].into_iter()), None);
}

#[test]
fn many_series_get_a_table_legend_with_grey_overflow() {
    let spec = timeseries(json!({}), json!({}));
    let many: Vec<Series> = (0..LEGEND_ROWS + 3)
        .map(|n| series(&format!("pod-{n}"), &[n as f64; 7]))
        .collect();
    let chart = chart_of(&derive(&spec, frame(many), window()));
    assert_eq!(chart.legend.mode, LegendMode::Table);
    assert_eq!(chart.legend.headings, ["last", "max"]);
    assert_eq!(chart.legend.rows.len(), LEGEND_ROWS);
    assert_eq!(chart.legend.more, 3);
    assert_eq!(chart.series[5].ink, Ink::Slot(5));
    assert_eq!(chart.series[6].ink, Ink::Overflow(6));
}

#[test]
fn the_dashboards_calcs_pick_the_legend_columns_and_a_stopped_series_is_dated() {
    let spec = timeseries(
        json!({}),
        json!({"legend": {"displayMode": "table", "calcs": ["mean", "max"]}}),
    );
    let gap = f64::NAN;
    let chart = chart_of(&derive(
        &spec,
        frame(vec![series("a", &[1., 2., 3., 4., 5., gap, gap])]),
        window(),
    ));
    assert_eq!(chart.legend.mode, LegendMode::Table);
    assert_eq!(chart.legend.headings, ["mean", "max"]);
    assert_eq!(chart.legend.rows[0].values, ["3", "5"]);
    // The stale time shows only for a last-value column.
    assert_eq!(chart.legend.rows[0].stale, None);

    let spec = timeseries(json!({}), json!({}));
    let chart = chart_of(&derive(
        &spec,
        frame(vec![series("a", &[1., 2., 3., 4., 5., gap, gap])]),
        window(),
    ));
    let stopped = window().times()[4] as f64;
    assert_eq!(
        chart.legend.rows[0].stale.as_deref(),
        Some(clock(stopped).as_str())
    );
    assert!(chart.series[0].tops[5].is_nan());
}

#[test]
fn hidden_legends_stay_hidden() {
    let spec = timeseries(json!({}), json!({"legend": {"showLegend": false}}));
    let chart = chart_of(&derive(&spec, frame(vec![series("a", &[1.; 7])]), window()));
    assert_eq!(chart.legend.mode, LegendMode::Hidden);
}

#[test]
fn a_second_unit_goes_to_the_right_axis() {
    let spec = panel(json!({
        "type": "timeseries",
        "title": "Two units",
        "targets": [{"refId": "A", "expr": "a"}, {"refId": "B", "expr": "b"}],
        "fieldConfig": {
            "defaults": {"unit": "bytes"},
            "overrides": [{
                "matcher": {"id": "byFrameRefID", "options": "B"},
                "properties": [{"id": "unit", "value": "percent"}],
            }],
        },
    }));
    let mut b = series("b", &[50.; 7]);
    b.query = "B".into();
    let chart = chart_of(&derive(
        &spec,
        frame(vec![series("a", &[1e9; 7]), b]),
        window(),
    ));
    let right = chart.axes[1].as_ref().unwrap();
    assert!(right.ticks.iter().all(|tick| tick.label.ends_with('%')));
    assert_eq!(chart.series[1].tops[0], (50. / right.max) as f32);
    let left = chart.axes[0].as_ref().unwrap();
    assert_eq!(chart.series[0].tops[0], (1e9 / left.max) as f32);
}

#[test]
fn percent_stacking_fills_to_a_hundred() {
    let spec = timeseries(
        json!({"custom": {"stacking": {"mode": "percent"}, "fillOpacity": 30}}),
        json!({}),
    );
    let chart = chart_of(&derive(
        &spec,
        frame(vec![series("a", &[1.; 7]), series("b", &[3.; 7])]),
        window(),
    ));
    let axis = chart.axes[0].as_ref().unwrap();
    assert_eq!((axis.min, axis.max), (0., 100.));
    assert_eq!(chart.series[0].tops[0], 0.25);
    assert_eq!(chart.series[1].bases.as_ref().unwrap()[0], 0.25);
    assert_eq!(chart.series[1].tops[0], 1.);
    assert!(chart.series[1].fill > 0.);
}

#[test]
fn threshold_lines_carry_their_meaning_not_their_colour() {
    let spec = timeseries(
        json!({
            "unit": "percentunit",
            "min": 0, "max": 1,
            "thresholds": {"mode": "absolute", "steps": [
                {"color": "green", "value": null},
                {"color": "#EAB839", "value": 0.8},
                {"color": "purple", "value": 0.9},
            ]},
            "custom": {"thresholdsStyle": {"mode": "line+area"}},
        }),
        json!({}),
    );
    let chart = chart_of(&derive(
        &spec,
        frame(vec![series("a", &[0.5; 7])]),
        window(),
    ));
    let lines: Vec<(f32, Tier, &str)> = chart
        .thresholds
        .iter()
        .map(|line| (line.at, line.tier, line.label.as_ref()))
        .collect();
    assert_eq!(
        lines,
        [
            (0.8, Tier::Warn, "80% threshold"),
            (0.9, Tier::Crit, "90% threshold"),
        ]
    );
    let bands: Vec<(f32, f32, Tier)> = chart
        .bands
        .iter()
        .map(|band| (band.from, band.to, band.tier))
        .collect();
    assert_eq!(bands, [(0.8, 0.9, Tier::Warn), (0.9, 1., Tier::Crit)]);
}

#[test]
fn thresholds_are_not_drawn_unless_asked() {
    let spec = timeseries(
        json!({"thresholds": {"mode": "absolute", "steps": [
            {"color": "green", "value": null},
            {"color": "red", "value": 80},
        ]}}),
        json!({}),
    );
    let chart = chart_of(&derive(
        &spec,
        frame(vec![series("a", &[50.; 7])]),
        window(),
    ));
    assert!(chart.thresholds.is_empty());
    assert!(chart.bands.is_empty());
}

#[test]
fn no_values_is_no_data_and_unknown_kinds_are_named() {
    let spec = timeseries(json!({}), json!({}));
    let data = derive(&spec, frame(vec![series("a", &[f64::NAN; 7])]), window());
    assert!(matches!(data.body, Body::NoData));
    let data = derive(&spec, frame(Vec::new()), window());
    assert!(matches!(data.body, Body::NoData));

    let spec = panel(json!({"type": "geomap", "title": "Map"}));
    let data = derive(&spec, frame(Vec::new()), window());
    match data.body {
        Body::NotDrawn(kind) => assert_eq!(not_drawn(&kind), "Not drawn here: geomap"),
        _ => panic!("drawn"),
    }
}

fn stats_of(data: PanelData) -> Rc<[Stat]> {
    match data.body {
        Body::Stats(stats) => stats,
        _ => panic!("not stats"),
    }
}

#[test]
fn a_stat_says_which_threshold_it_passed() {
    let spec = panel(json!({
        "type": "stat",
        "title": "Restarts",
        "targets": [{"refId": "A", "expr": "restarts"}],
        "fieldConfig": {"defaults": {
            "decimals": 0,
            "thresholds": {"mode": "absolute", "steps": [
                {"color": "green", "value": null},
                {"color": "orange", "value": 5},
                {"color": "red", "value": 20},
            ]},
        }},
        "options": {"graphMode": "area", "colorMode": "value", "reduceOptions": {"calcs": ["lastNotNull"]}},
    }));
    let values: Vec<f64> = (0..12).map(f64::from).chain([50.]).collect();
    let stats = stats_of(derive(&spec, frame(vec![series("a", &values)]), window()));
    assert_eq!(stats.len(), 1);
    let stat = &stats[0];
    assert_eq!(stat.value, "50");
    assert_eq!(stat.tier, Some(Tier::Crit));
    assert_eq!(stat.note.as_deref(), Some("above 20"));
    assert_eq!(stat.name, None);
    let spark = stat.spark.as_ref().unwrap();
    assert_eq!(spark.ys.len(), 13);
    assert_eq!(spark.ys[12], 1.);
    assert_eq!(spark.tail, 13 - 3);

    let calm = stats_of(derive(&spec, frame(vec![series("a", &[3.; 7])]), window()));
    assert_eq!(calm[0].tier, None);
    assert_eq!(calm[0].note, None);
}

#[test]
fn a_stat_without_colour_carries_no_status_and_a_gauge_fills() {
    let defaults = json!({
        "unit": "percent",
        "thresholds": {"mode": "absolute", "steps": [
            {"color": "green", "value": null},
            {"color": "red", "value": 50},
        ]},
    });
    let spec = panel(json!({
        "type": "stat",
        "title": "Plain",
        "fieldConfig": {"defaults": defaults},
        "options": {"colorMode": "none", "graphMode": "none"},
    }));
    let stats = stats_of(derive(&spec, frame(vec![series("a", &[75.; 7])]), window()));
    assert_eq!((stats[0].tier, stats[0].spark.is_none()), (None, true));

    let spec = panel(json!({
        "type": "gauge",
        "title": "Gauge",
        "fieldConfig": {"defaults": defaults},
    }));
    let stats = stats_of(derive(
        &spec,
        frame(vec![series("a", &[75.; 7]), series("b", &[25.; 7])]),
        window(),
    ));
    assert_eq!(stats[0].gauge, Some(0.75));
    assert_eq!(stats[0].tier, Some(Tier::Crit));
    assert_eq!(stats[1].tier, None);
    assert_eq!(stats[1].name.as_deref(), Some("b"));
}

#[test]
fn bars_split_the_namespace_and_measure_against_the_peak() {
    let spec = panel(json!({
        "type": "barchart",
        "title": "Top pods",
    }));
    let data = derive(
        &spec,
        frame(vec![
            series("payments/api-6c4f8", &[2.; 7]),
            series("node exporter / cpu", &[1.; 7]),
            series("gone", &[f64::NAN; 7]),
        ]),
        window(),
    );
    let Body::Bars(rows) = data.body else {
        panic!("not bars")
    };
    assert_eq!(rows[0].prefix.as_deref(), Some("payments/"));
    assert_eq!(rows[0].name, "api-6c4f8");
    assert_eq!(rows[0].fraction, 1.);
    assert_eq!(rows[1].prefix, None);
    assert_eq!(rows[1].fraction, 0.5);
    assert!(rows[2].missing);
    assert_eq!(rows[2].value, "–");
}

#[test]
fn a_table_drops_hidden_columns_and_counts_rows_past_the_cap() {
    let spec = panel(json!({
        "type": "table",
        "title": "Alerts",
        "targets": [{"refId": "A", "expr": "ALERTS", "format": "table", "instant": true}],
        "transformations": [{"id": "organize", "options": {
            "excludeByName": {"Time": true},
            "renameByName": {"alertname": "Alert"},
        }}],
    }));
    let rows: Vec<Series> = (0..summary::MAX_ROWS + 5)
        .map(|n| labelled("ALERTS", &[("alertname", &format!("alert-{n}"))], &[1.; 7]))
        .collect();
    let Body::Table(table) = derive(&spec, frame(rows), window()).body else {
        panic!("not a table")
    };
    let names: Vec<&str> = table.columns.iter().map(|c| c.name.as_ref()).collect();
    assert_eq!(names, ["Alert", "Value"]);
    assert!(!table.columns[0].numeric);
    assert!(table.columns[1].numeric);
    assert_eq!(table.rows.len(), summary::MAX_ROWS);
    assert_eq!(table.total, summary::MAX_ROWS + 5);
    assert_eq!(table.rows[0][0], "alert-0");
}

#[test]
fn value_ticks_are_round() {
    assert_eq!(nice_step(100., 4.), 25.);
    assert_eq!(nice_step(7., 4.), 2.);
    assert_eq!(nice_step(0., 4.), 1.);
    assert_eq!(
        linear(3., 97., None, None),
        (0., 100., vec![0., 25., 50., 75., 100.])
    );
    let (min, max, values) = linear(-0.3, 0.2, None, None);
    assert_eq!((min, max), (-0.4, 0.2));
    assert!(values.contains(&0.) && values.iter().all(|v| !v.is_sign_negative() || *v < 0.));
    assert_eq!(linear(10., 20., Some(0.), Some(1.)).1, 1.);
    assert_eq!(
        log(10., 3., 2000.),
        (1., 10000., vec![1., 10., 100., 1000., 10000.])
    );
}

#[test]
fn time_ticks_land_on_the_local_clock_and_fit_the_width() {
    let sets = time_ticks(END as f64 - 6. * 3600., END as f64);
    assert!(!sets.is_empty());
    for set in &sets {
        assert!(set.ticks.iter().all(|tick| (0.0..=1.0).contains(&tick.at)));
        assert!(set.ticks.windows(2).all(|pair| pair[0].at < pair[1].at));
    }
    // An hour apart over six hours, at 600 px, leaves 100 px a tick.
    let hourly = fitting(&sets, 600., 90.).unwrap();
    assert!((hourly.spacing - 1. / 6.).abs() < 1e-6);
    assert!(hourly.ticks.iter().all(|tick| tick.label.ends_with(":00")));
    // Too narrow for any: the coarsest.
    assert_eq!(
        fitting(&sets, 10., 90.).unwrap().spacing,
        sets.last().unwrap().spacing
    );
    assert!(time_ticks(10., 10.).is_empty());
}
