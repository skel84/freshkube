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
    id: SharedString,
    empty_id: SharedString,
    coverage_id: SharedString,
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
}

fn choice(chart: &api::AppChart, title: String, pick_id: String, marks: &Marks) -> Choice {
    let panel = ChartPanel::from_history(&api::AppChart {
        title: title.clone(),
        ..chart.clone()
    })
    .map(|panel| panel.with_revisions(marks.revisions, marks.namespace));
    let coverage = panel.as_ref().and_then(|p| coverage_line(&p.coverage));
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
        empty: empty.into(),
    }
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
    pub(super) fn chart_words(&self) -> Vec<(String, Option<String>, String)> {
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

    fn charts(&self) -> impl Iterator<Item = &Rc<Charts>> {
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
        let Some(page) = &self.app_page else {
            self.app_charts.clear();
            return;
        };
        let picks = &self.app_picks;
        let wanted = || {
            page.charts().filter_map(|charts| {
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

    pub(super) fn render_charts(&self, charts: &Rc<Charts>, cx: &mut Context<Self>) -> AnyElement {
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
        let chart = match &choice.coverage {
            Some(coverage) => v_flex()
                .gap(dp(4.))
                .child(chart)
                .child(
                    muted(coverage.clone(), cx)
                        .id(charts.coverage_id.clone())
                        .test_support()
                        .text_size(dp(12.))
                        .whitespace_normal(),
                )
                .into_any_element(),
            None => chart,
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
