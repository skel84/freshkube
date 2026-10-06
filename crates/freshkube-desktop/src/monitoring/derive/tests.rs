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

/// [`super::derive`] with the cap a panel starts with.
fn derive(spec: &PanelSpec, frame: Frame, window: TimeWindow) -> PanelData {
    super::derive(spec, frame, window, SeriesCap::Top)
}

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
    assert_eq!(inks, [Ink::Slot(0), Ink::Slot(1), Ink::Overflow(2)]);
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
    let chart = chart_of(&super::derive(&spec, frame(many), window(), SeriesCap::All));
    assert_eq!(chart.legend.mode, LegendMode::Table);
    assert_eq!(chart.legend.headings, ["last", "max"]);
    assert_eq!(chart.legend.rows.len(), LEGEND_ROWS);
    assert_eq!(chart.legend.more, 3);
    assert_eq!(chart.series[5].ink, Ink::Overflow(5));
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
        Some(format!("Last value at {}", clock(stopped)).as_str())
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
fn legend_values_take_the_titles_unit_and_one_column_width() {
    const GIB: f64 = 1024. * 1024. * 1024.;
    let spec = timeseries(json!({"unit": "bytes"}), json!({}));
    let names = ["a", "b", "c", "d", "e"];
    let levels = [6. * GIB, 3. * GIB, 0.5 * GIB, 929. / 1024. * GIB, 2. * GIB];
    let all = names
        .iter()
        .zip(levels)
        .map(|(name, level)| series(name, &[level; 7]))
        .collect();
    let data = derive(&spec, frame(all), window());
    assert_eq!(data.unit.as_deref(), Some("GiB"));
    let chart = chart_of(&data);
    assert_eq!(chart.legend.mode, LegendMode::Table);
    // Under a GiB title, 512 MiB reads 0.5 and no value names a second unit.
    assert_eq!(chart.legend.rows[2].values, ["0.5", "0.5"]);
    assert_eq!(chart.legend.rows[3].values, ["0.907", "0.907"]);
    let values = chart.legend.rows.iter().flat_map(|row| &row.values);
    assert!(
        values.clone().all(|value| !value.contains(' ')),
        "{:?}",
        chart.legend
    );
    // One width for every value column, room for the widest value.
    let widest = values.map(|value| value.chars().count()).max().unwrap();
    assert!(chart.legend.column >= widest as f32 * 7.);
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

/// `n` series named `pod-<i>`, each flat at `peak(i)`.
fn flat(n: usize, peak: impl Fn(usize) -> f64) -> Vec<Series> {
    (0..n)
        .map(|i| series(&format!("pod-{i}"), &[peak(i); 7]))
        .collect()
}

fn names(chart: &Chart) -> Vec<&str> {
    chart.series.iter().map(|s| s.name.as_ref()).collect()
}

#[test]
fn many_series_draw_the_highest_peaks_in_their_own_order() {
    let spec = timeseries(json!({}), json!({}));
    // pod-0 is low but for one spike, pod-1 is high all along, and the
    // rest rise with their number.
    let mut many = flat(67, |i| i as f64);
    many[0].values = vec![0., 0., 0., 500., 0., 0., 0.];
    many[1].values = vec![400.; 7];
    let chart = chart_of(&derive(&spec, frame(many), window()));
    assert_eq!(chart.capped, Some(Capped { of: 67, all: false }));
    let mut kept = vec!["pod-0".to_owned(), "pod-1".to_owned()];
    kept.extend((39..67).map(|i| format!("pod-{i}")));
    assert_eq!(names(&chart), kept);
    // The legend lists what is drawn, with nothing more to count, and the
    // first two keep the colour slots.
    assert_eq!(chart.legend.rows.len(), MOST_SERIES);
    assert_eq!(chart.legend.more, 0);
    assert_eq!(chart.series[0].ink, Ink::Slot(0));
    assert_eq!(chart.series[2].ink, Ink::Overflow(2));
}

#[test]
fn showing_all_draws_every_series_as_before() {
    let spec = timeseries(json!({}), json!({}));
    let all = || frame(flat(67, |i| i as f64));
    let chart = chart_of(&super::derive(&spec, all(), window(), SeriesCap::All));
    assert_eq!(chart.capped, Some(Capped { of: 67, all: true }));
    assert_eq!(chart.series.len(), 67);
    assert_eq!(chart.legend.more, 67 - LEGEND_ROWS);
    let off = chart_of(&super::derive(&spec, all(), window(), SeriesCap::Off));
    assert_eq!((off.series.len(), off.capped), (67, None));
}

#[test]
fn thirty_series_are_not_capped() {
    let spec = timeseries(json!({}), json!({}));
    let chart = chart_of(&derive(
        &spec,
        frame(flat(MOST_SERIES, |i| i as f64)),
        window(),
    ));
    assert_eq!((chart.series.len(), chart.capped), (MOST_SERIES, None));
}

#[test]
fn a_peak_below_zero_ranks_by_its_size_and_no_values_rank_last() {
    let spec = timeseries(json!({}), json!({}));
    let mut many = flat(31, |_| 1.);
    many[0].values = vec![-90.; 7];
    many[1].values = vec![f64::NAN; 7];
    let chart = chart_of(&derive(&spec, frame(many), window()));
    let names = names(&chart);
    assert_eq!(names.len(), MOST_SERIES);
    assert!(names.contains(&"pod-0"));
    assert!(!names.contains(&"pod-1"));
}

#[test]
fn a_stacked_chart_is_never_capped() {
    let spec = timeseries(
        json!({"custom": {"stacking": {"mode": "normal"}, "fillOpacity": 30}}),
        json!({}),
    );
    let chart = chart_of(&derive(&spec, frame(flat(67, |i| i as f64)), window()));
    assert_eq!((chart.series.len(), chart.capped), (67, None));
}

#[test]
fn a_series_hidden_from_the_legend_always_draws_and_is_not_counted() {
    let spec = panel(json!({
        "type": "timeseries",
        "title": "Chart",
        "targets": [{"refId": "A", "expr": "up"}],
        "fieldConfig": {"defaults": {}, "overrides": [{
            "matcher": {"id": "byName", "options": "limit"},
            "properties": [{"id": "custom.hideFrom",
                "value": {"legend": true, "tooltip": false, "viz": false}}],
        }]},
        "options": {},
    }));
    // The limit is the lowest series, so only its being hidden keeps it.
    let mut many = flat(MOST_SERIES, |i| 10. + i as f64);
    many.push(series("limit", &[1.; 7]));
    let chart = chart_of(&derive(&spec, frame(many.clone()), window()));
    assert_eq!((chart.series.len(), chart.capped), (MOST_SERIES + 1, None));
    many.push(series("pod-extra", &[2.; 7]));
    let chart = chart_of(&derive(&spec, frame(many), window()));
    assert_eq!(chart.capped, Some(Capped { of: 31, all: false }));
    let names = names(&chart);
    assert!(names.contains(&"limit"));
    assert!(!names.contains(&"pod-extra"));
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

fn bar_places(chart: &Chart) -> Vec<(usize, usize)> {
    chart
        .series
        .iter()
        .map(|s| {
            let bar = s.bar.expect("a bar series");
            (bar.index, bar.count)
        })
        .collect()
}

fn bars(defaults: Value, overrides: Value, names: &[&str]) -> Rc<Chart> {
    let mut spec = json!({
        "type": "timeseries",
        "title": "Chart",
        "targets": [{"refId": "A", "expr": "up"}],
        "fieldConfig": {"defaults": defaults, "overrides": overrides},
        "options": {},
    });
    spec["fieldConfig"]["defaults"]["custom"]["drawStyle"] = json!("bars");
    let spec = panel(spec);
    let series = names.iter().map(|name| series(name, &[1.; 7])).collect();
    chart_of(&derive(&spec, frame(series), window()))
}

#[test]
fn bars_stand_side_by_side_unless_they_stack() {
    // Side by side, the last series first.
    let chart = bars(json!({"custom": {}}), json!([]), &["a", "b", "c"]);
    assert_eq!(bar_places(&chart), [(2, 3), (1, 3), (0, 3)]);
    assert!(chart.groups.is_empty());

    // Stacked, they share the whole column.
    let stacked = json!({"custom": {"stacking": {"mode": "normal"}}});
    let chart = bars(stacked.clone(), json!([]), &["a", "b", "c"]);
    assert_eq!(bar_places(&chart), [(0, 1), (0, 1), (0, 1)]);

    // One series out of the stack stands beside it.
    let alone = json!([{
        "matcher": {"id": "byName", "options": "c"},
        "properties": [{"id": "custom.stacking", "value": {"mode": "none"}}],
    }]);
    let chart = bars(stacked, alone, &["a", "b", "c"]);
    assert_eq!(bar_places(&chart), [(1, 2), (1, 2), (0, 2)]);
}

#[test]
fn lines_that_look_alike_are_one_group_derived_with_the_chart() {
    let spec = timeseries(json!({}), json!({}));
    let names: Vec<String> = (0..10).map(|i| format!("pod-{i}")).collect();
    let chart = chart_of(&derive(
        &spec,
        frame(names.iter().map(|name| series(name, &[1.; 7])).collect()),
        window(),
    ));
    assert!(chart.series.iter().all(|s| s.bar.is_none()));
    let mut members: Vec<usize> = chart.groups.iter().flatten().copied().collect();
    members.sort_unstable();
    assert_eq!(members, (0..10).collect::<Vec<_>>(), "{:?}", chart.groups);
    // Past the sixth series the lines are one grey, so fewer groups than
    // series; every member draws as its group's first.
    assert!(chart.groups.len() < 10, "{:?}", chart.groups);
    for group in &chart.groups {
        let first = &chart.series[group[0]];
        for &i in group {
            assert_eq!(chart.series[i].ink.color(false), first.ink.color(false));
        }
    }
    // The last series' group draws first, so the first series ends on top.
    assert!(chart.groups[0].contains(&9));
    assert!(chart.groups.last().unwrap().contains(&0));
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
    assert_eq!(table.rows[0].cells[0], "alert-0");
    assert_eq!(summary::MAX_ROWS, 1_000, "MONITORING.md names the cap");
}

fn alerts(rows: &[(&str, &str)]) -> Rc<summary::TableData> {
    let spec = panel(json!({
        "type": "table",
        "title": "Firing alerts",
        "targets": [{"refId": "A", "expr": "ALERTS", "format": "table", "instant": true}],
        "transformations": [{"id": "organize", "options": {
            "excludeByName": {"Time": true},
            "renameByName": {"alertname": "Alert"},
        }}],
    }));
    let series = rows
        .iter()
        .enumerate()
        .map(|(n, (name, severity))| {
            labelled(
                "ALERTS",
                &[("alertname", name), ("severity", severity)],
                &[n as f64; 7],
            )
        })
        .collect();
    let Body::Table(table) = derive(&spec, frame(series), window()).body else {
        panic!("not a table")
    };
    table
}

#[test]
fn table_rows_are_keyed_by_their_labels_and_which_of_the_same_they_are() {
    let table = alerts(&[
        ("disk", "warning"),
        ("cpu", "critical"),
        ("disk", "warning"),
    ]);
    let keys: Vec<_> = table.rows.iter().map(|row| row.key.clone()).collect();
    // Two rows share every label: the second is told apart by its turn.
    assert_eq!(keys[0].labels, keys[2].labels);
    assert_eq!((keys[0].occurrence, keys[2].occurrence), (0, 1));
    assert_ne!(keys[0], keys[2]);
    // The values aren't part of the key, and another order keeps each key.
    assert_eq!(&*keys[1].labels, ["cpu", "critical"]);
    let moved = alerts(&[
        ("cpu", "critical"),
        ("disk", "warning"),
        ("disk", "warning"),
    ]);
    assert_eq!(moved.rows[0].key, keys[1]);
    assert_eq!(moved.rows[1].key, keys[0]);
    assert_eq!(moved.rows[2].key, keys[2]);
}

#[test]
fn a_severity_column_gives_rows_a_status_only_when_it_is_a_problem() {
    let table = alerts(&[
        ("a", "critical"),
        ("b", "Error"),
        ("c", "warning"),
        ("d", "WARN"),
        ("e", "info"),
        ("f", "none"),
        ("g", "debug"),
        ("h", "page-me"),
    ]);
    assert!(table.severity);
    let tiers: Vec<_> = table.rows.iter().map(|row| row.tier).collect();
    use crate::monitoring::colors::Tier::{Crit, Warn};
    assert_eq!(
        tiers,
        [
            Some(Crit),
            Some(Crit),
            Some(Warn),
            Some(Warn),
            None,
            None,
            None,
            None
        ]
    );
    // Without a severity column no row carries one.
    let spec = panel(json!({
        "type": "table",
        "title": "Pods",
        "targets": [{"refId": "A", "expr": "up", "format": "table", "instant": true}],
    }));
    let Body::Table(plain) = derive(
        &spec,
        frame(vec![labelled("up", &[("pod", "critical")], &[1.; 7])]),
        window(),
    )
    .body
    else {
        panic!("not a table")
    };
    assert!(!plain.severity);
    assert!(plain.rows.iter().all(|row| row.tier.is_none()));
}

#[test]
fn value_ticks_are_round() {
    assert_eq!(nice_step(100., 4.), 25.);
    assert_eq!(nice_step(7., 4.), 2.);
    assert_eq!(nice_step(0., 4.), 1.);
    assert_eq!(
        linear(3., 97., None, None, false),
        (0., 100., vec![0., 25., 50., 75., 100.])
    );
    let (min, max, values) = linear(-0.3, 0.2, None, None, false);
    assert_eq!((min, max), (-0.4, 0.2));
    assert!(values.contains(&0.) && values.iter().all(|v| !v.is_sign_negative() || *v < 0.));
    assert_eq!(linear(10., 20., Some(0.), Some(1.), false).1, 1.);
    assert_eq!(
        log(10., 3., 2000.),
        (1., 10000., vec![1., 10., 100., 1000., 10000.])
    );
}

#[test]
fn whole_values_take_whole_ticks() {
    assert_eq!(
        linear(0., 1., None, None, false).2,
        [0., 0.25, 0.5, 0.75, 1.]
    );
    assert_eq!(linear(0., 1., None, None, true).2, [0., 1.]);
    assert_eq!(linear(0., 2., None, None, true).2, [0., 1., 2.]);
    assert_eq!(
        linear(0., 10., None, None, true).2,
        [0., 2., 4., 6., 8., 10.]
    );
    assert_eq!(
        linear(3., 97., None, None, true).2,
        [0., 25., 50., 75., 100.]
    );

    let labels = |defaults: Value, values: &[f64]| -> Vec<String> {
        let spec = timeseries(defaults, json!({}));
        let chart = chart_of(&derive(&spec, frame(vec![series("a", values)]), window()));
        let axis = chart.axes[0].as_ref().unwrap();
        axis.ticks
            .iter()
            .map(|tick| tick.label.to_string())
            .collect()
    };
    // A count of one or two messages a column.
    assert_eq!(
        labels(json!({}), &[0., 1., 2., 1., 0., 1., 2.]),
        ["0", "1", "2"]
    );
    // A fraction of one stays a fraction, and so does 100% of one.
    assert_eq!(labels(json!({}), &[0., 0.5, 1., 0.5, 0., 0.5, 1.]).len(), 5);
    assert_eq!(
        labels(
            json!({"unit": "percentunit"}),
            &[0., 1., 1., 0., 1., 0., 1.]
        )
        .len(),
        5
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
