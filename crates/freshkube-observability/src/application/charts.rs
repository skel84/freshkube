//! A report's charts and heatmaps. Each chart is Monitoring's panel, made
//! from the chart's history when Coroot answers, with its deployments
//! marked; a group shows one chart at a time, picked above it. Panels exist
//! only for the shown report, and only for the chart a group shows.
use super::super::traces::heat::{Heat, app_heat};
use super::super::*;
use super::{AppPage, BlockKind};
use crate::monitoring::panel::PanelView;
use freshkube_core::coroot::{self as api, ChartPanel, Coverage};

/// A chart's height, its legend included.
const HEIGHT: f32 = 240.;

/// Where a chart sits: the application, the report and the widget's place
/// in it.
pub(crate) type ChartKey = (api::AppId, String, usize);

/// A chart widget, or a group of charts of which one shows.
pub(crate) struct Charts {
    key: ChartKey,
    /// `obs-chart-<report>-<widget>`.
    pub(crate) id: SharedString,
    empty_id: SharedString,
    coverage_id: SharedString,
    sides_id: SharedString,
    choices: Vec<Choice>,
    /// The chart shown until the reader picks another.
    featured: usize,
    group: bool,
}

struct Choice {
    /// What the picker calls it.
    name: SharedString,
    pick_id: SharedString,
    title: SharedString,
    /// None when Coroot sent no points, or no history to place them by.
    panel: Option<Rc<ChartPanel>>,
    /// What of the window the history lacks, in one line; None when it
    /// covers all of it.
    coverage: Option<SharedString>,
    /// Around a revision, how many of its samples fall before the start
    /// and how many after, in one line.
    sides: Option<SharedString>,
    /// Why the card shows no chart.
    empty: SharedString,
}

/// What a chart is drawn with besides its own answer: the application's
/// deployments, and its namespace for their markers.
pub(super) struct Marks<'a> {
    pub revisions: &'a [api::DeploymentRevision],
    pub namespace: Option<&'a str>,
    /// The page's charts have no histories, so an empty card says so.
    pub no_history: bool,
    /// Around a revision, its start, where each chart is split.
    pub split_at: Option<chrono::DateTime<chrono::Utc>>,
}

fn choice(chart: &api::AppChart, title: String, pick_id: String, marks: &Marks) -> Choice {
    let panel = ChartPanel::from_history(&api::AppChart {
        title: title.clone(),
        ..chart.clone()
    })
    .map(|panel| panel.with_revisions(marks.revisions, marks.namespace));
    let coverage = panel.as_ref().and_then(|p| coverage_line(&p.coverage));
    let sides = marks
        .split_at
        .zip(chart.history.as_deref())
        .and_then(|(at, history)| sides_line(history, at));
    let empty = if chart.history.is_none() && marks.no_history {
        "Coroot's history for this chart couldn't be read."
    } else {
        "Coroot sent no points for this chart in this window."
    };
    Choice {
        name: chart.title.clone().into(),
        pick_id: pick_id.into(),
        title: title.into(),
        panel: panel.map(Rc::new),
        coverage,
        sides,
        empty: empty.into(),
    }
}

/// How a chart's series cover one side of a revision's start, together.
#[derive(Default)]
struct Side {
    /// The side has sample places in the window.
    places: bool,
    present: usize,
    expected: usize,
    /// Series whose samples Coroot didn't place in the window.
    unplaced: usize,
}

impl Side {
    fn add(&mut self, side: Option<api::SideCoverage>, places: usize) {
        let Some(side) = side else {
            return;
        };
        self.places = true;
        match side {
            api::SideCoverage::Full => {
                self.present += places;
                self.expected += places;
            }
            api::SideCoverage::Gaps { present, expected } => {
                self.present += present;
                self.expected += expected;
            }
            api::SideCoverage::Empty => self.expected += places,
            _ => self.unplaced += 1,
        }
    }

    fn words(&self) -> String {
        if !self.places {
            return "no samples in the window".into();
        }
        let mut words = match (self.present, self.expected) {
            (_, 0) => String::new(),
            (0, _) => "no samples".into(),
            (present, expected) if present == expected => format!("all {expected} samples"),
            (present, expected) => format!("{present} of {expected} samples"),
        };
        if self.unplaced > 0 {
            if !words.is_empty() {
                words.push_str(", ");
            }
            words.push_str(&match self.unplaced {
                1 => "1 series not placed".to_owned(),
                n => format!("{n} series not placed"),
            });
        }
        words
    }
}

/// Which of the chart's samples fall before `at` and which at or after it,
/// in one line; never their values.
fn sides_line(
    history: &api::ChartHistory,
    at: chrono::DateTime<chrono::Utc>,
) -> Option<SharedString> {
    if history.series.is_empty() {
        return None;
    }
    let split = history.split_at(at);
    let (mut before, mut after) = (Side::default(), Side::default());
    for series in &history.series {
        let (b, a) = series.sides(&split);
        before.add(b, split.before);
        after.add(a, split.after);
    }
    Some(
        format!(
            "Before the start: {} · after: {}",
            before.words(),
            after.words()
        )
        .into(),
    )
}

/// The chart's coverage in Coroot's terms, as one muted line under it.
fn coverage_line(coverage: &[Coverage]) -> Option<SharedString> {
    let words: Vec<String> = coverage
        .iter()
        .map(|c| match c {
            Coverage::Truncated => "Truncated range".to_owned(),
            Coverage::Partial {
                series,
                samples,
                expected,
            } => format!("{series} covers {samples} of {expected} points"),
            Coverage::Empty { series } => format!("{series}: no data"),
        })
        .collect();
    (!words.is_empty()).then(|| words.join(" · ").into())
}

impl Charts {
    pub(super) fn chart(key: ChartKey, slug: &str, chart: &api::AppChart, marks: &Marks) -> Self {
        let id = format!("obs-chart-{slug}-{}", key.2);
        Self {
            choices: vec![choice(
                chart,
                chart.title.clone(),
                format!("{id}-pick-0"),
                marks,
            )],
            empty_id: format!("{id}-empty").into(),
            coverage_id: format!("{id}-coverage").into(),
            sides_id: format!("{id}-sides").into(),
            id: id.into(),
            key,
            featured: 0,
            group: false,
        }
    }

    /// A group's charts under one title, with `<selector>` naming each.
    pub(super) fn group(
        key: ChartKey,
        slug: &str,
        title: &str,
        charts: &[api::AppChart],
        marks: &Marks,
    ) -> Self {
        let id = format!("obs-chart-{slug}-{}", key.2);
        Self {
            choices: charts
                .iter()
                .enumerate()
                .map(|(ix, chart)| {
                    let title = title.replace("<selector>", &chart.title);
                    choice(chart, title, format!("{id}-pick-{ix}"), marks)
                })
                .collect(),
            featured: charts.iter().position(|c| c.featured).unwrap_or(0),
            empty_id: format!("{id}-empty").into(),
            coverage_id: format!("{id}-coverage").into(),
            sides_id: format!("{id}-sides").into(),
            id: id.into(),
            key,
            group: true,
        }
    }

    /// The chart that shows: the reader's pick, else the featured one.
    fn shown(&self, picks: &BTreeMap<ChartKey, SharedString>) -> usize {
        picks
            .get(&self.key)
            .and_then(|name| self.choices.iter().position(|c| c.name == *name))
            .unwrap_or(self.featured)
    }
}

/// A heatmap, prepared when Coroot answers.
pub(crate) struct HeatBlock {
    id: SharedString,
    title: SharedString,
    heat: Heat,
}

impl HeatBlock {
    pub(super) fn new(slug: &str, widget: usize, heatmap: &api::AppHeatmap) -> Self {
        Self {
            id: format!("obs-heatmap-{slug}-{widget}").into(),
            title: heatmap.title.clone().into(),
            heat: app_heat(heatmap),
        }
    }
}

/// A chart's panel, kept while its chart shows.
pub(crate) struct ShownChart {
    charts: Rc<Charts>,
    shown: usize,
    view: Entity<PanelView>,
}

impl ShownChart {
    /// Where the chart sits, for the page's tests.
    #[cfg(test)]
    pub(crate) fn key(&self) -> &ChartKey {
        &self.charts.key
    }

    /// The shown chart's title, for the page's tests.
    #[cfg(test)]
    pub(crate) fn title(&self) -> &SharedString {
        &self.charts.choices[self.shown].title
    }

    /// The panel, for the page's tests.
    #[cfg(test)]
    pub(crate) fn view(&self) -> &Entity<PanelView> {
        &self.view
    }
}

impl AppPage {
    /// Each chart's title, coverage line and empty card's words, in page
    /// order, for the page's tests.
    #[cfg(test)]
    pub(crate) fn chart_words(&self) -> Vec<(String, Option<String>, String)> {
        self.charts()
            .flat_map(|charts| &charts.choices)
            .map(|c| {
                (
                    c.title.to_string(),
                    c.coverage.as_ref().map(ToString::to_string),
                    c.empty.to_string(),
                )
            })
            .collect()
    }

    pub(super) fn charts(&self) -> impl Iterator<Item = &Rc<Charts>> {
        self.blocks.iter().filter_map(|b| match &b.kind {
            BlockKind::Charts(charts) => Some(charts),
            _ => None,
        })
    }
}

impl ObservabilityPage {
    /// Keeps a panel for each chart the shown report shows, and drops the
    /// rest. Runs from render, and makes panels only after the report or a
    /// pick changes.
    pub(crate) fn sync_app_charts(&mut self, cx: &mut Context<Self>) {
        // Deployments shows the charts around a revision; the other
        // destinations the application page's.
        let shown: Vec<Rc<Charts>> = if self.destination == Destination::Deployments {
            self.revision_charts().to_vec()
        } else {
            match &self.app_page {
                Some(page) => page.charts().cloned().collect(),
                None => vec![],
            }
        };
        let picks = &self.app_picks;
        let wanted = || {
            shown.iter().filter_map(|charts| {
                let shown = charts.shown(picks);
                charts.choices[shown]
                    .panel
                    .as_ref()
                    .map(|_| (charts, shown))
            })
        };
        let same = wanted().count() == self.app_charts.len()
            && wanted()
                .zip(&self.app_charts)
                .all(|((charts, shown), kept)| {
                    Rc::ptr_eq(charts, &kept.charts) && shown == kept.shown
                });
        if same {
            return;
        }
        let mut kept = std::mem::take(&mut self.app_charts);
        let mut next = vec![];
        for (charts, shown) in wanted() {
            if let Some(ix) = kept
                .iter()
                .position(|k| Rc::ptr_eq(&k.charts, charts) && k.shown == shown)
            {
                next.push(kept.swap_remove(ix));
                continue;
            }
            let Some(panel) = charts.choices[shown].panel.clone() else {
                continue;
            };
            let id = charts.id.clone();
            let view = cx.new(|cx| {
                let mut view = PanelView::new(id, Rc::new(panel.spec.clone()), cx);
                view.set_result(panel.result.clone(), panel.window, cx);
                view.set_markers(panel.markers.clone().into(), cx);
                view
            });
            next.push(ShownChart {
                charts: charts.clone(),
                shown,
                view,
            });
        }
        self.app_charts = next;
    }

    pub(super) fn pick_chart(&mut self, charts: &Charts, ix: usize, cx: &mut Context<Self>) {
        let name = charts.choices[ix].name.clone();
        self.app_picks.insert(charts.key.clone(), name);
        cx.notify();
    }

    pub(crate) fn render_charts(&self, charts: &Rc<Charts>, cx: &mut Context<Self>) -> AnyElement {
        let shown = charts.shown(&self.app_picks);
        let choice = &charts.choices[shown];
        let chart = match self
            .app_charts
            .iter()
            .find(|k| Rc::ptr_eq(&k.charts, charts) && k.shown == shown)
        {
            Some(kept) => div()
                .id(charts.id.clone())
                .test_support()
                .relative()
                .h(dp(HEIGHT))
                .child(
                    kept.view
                        .clone()
                        .cached(StyleRefinement::default().size_full()),
                )
                .children(kept.view.read(cx).cursor_overlay())
                .into_any_element(),
            None => card(choice.title.clone(), cx)
                .id(charts.empty_id.clone())
                .test_support()
                .h(dp(HEIGHT))
                .child(body().child(muted(choice.empty.clone(), cx)))
                .into_any_element(),
        };
        let note = |words: &SharedString, id: &SharedString| {
            muted(words.clone(), cx)
                .id(id.clone())
                .test_support()
                .text_size(dp(12.))
                .whitespace_normal()
        };
        let chart = if choice.coverage.is_none() && choice.sides.is_none() {
            chart
        } else {
            v_flex()
                .gap(dp(4.))
                .child(chart)
                .children(
                    choice
                        .coverage
                        .as_ref()
                        .map(|c| note(c, &charts.coverage_id)),
                )
                .children(choice.sides.as_ref().map(|s| note(s, &charts.sides_id)))
                .into_any_element()
        };
        if !charts.group {
            return chart;
        }
        let picker =
            line()
                .flex_wrap()
                .gap(dp(2.))
                .children(charts.choices.iter().enumerate().map(|(ix, choice)| {
                    let charts = charts.clone();
                    ui::segment(Button::new(choice.pick_id.clone()), ix == shown, cx)
                        .small()
                        .child(text(choice.name.clone()))
                        .on_click(
                            cx.listener(move |this, _, _, cx| this.pick_chart(&charts, ix, cx)),
                        )
                }));
        v_flex()
            .gap(dp(6.))
            .child(picker)
            .child(chart)
            .into_any_element()
    }

    pub(super) fn render_heat_block(&self, block: &HeatBlock, cx: &Context<Self>) -> AnyElement {
        self.static_heatmap(block.id.clone(), block.title.clone(), &block.heat, cx)
    }
}
