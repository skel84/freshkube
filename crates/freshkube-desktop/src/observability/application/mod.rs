//! The Application page as Coroot's own application view: the map strip,
//! the report tabs with their glyphs, and the chosen report's checks and
//! widgets. Everything shown is derived when Coroot answers or the tab
//! changes; render only reads it.
use super::*;
use connection::Subject;
use freshkube_core::coroot as api;

mod charts;
mod example;
mod strip;
mod table;
#[cfg(test)]
mod tests;

pub(super) use charts::{ChartKey, ShownChart};
pub(super) use table::ReportTable;

/// The narrowest a chart is drawn before it takes a row of its own.
const MIN_CHART_WIDTH: f32 = 360.;

/// What the page shows of one answer and the chosen report.
pub(super) struct AppPage {
    strip: strip::Strip,
    tabs: Vec<Tab>,
    checks: Vec<CheckLine>,
    blocks: Vec<Block>,
    tables: Vec<Rc<table::Prepared>>,
    /// Coroot sent no reports at all.
    empty: bool,
}

struct Tab {
    name: String,
    id: SharedString,
    /// Coroot shows a report's glyph only when it has something to judge by.
    status: Option<Status>,
    tooltip: SharedString,
}

struct CheckLine {
    /// `obs-app-check-<place>` on its verdict, `…-condition` on its condition.
    id: SharedString,
    condition_id: SharedString,
    status: Status,
    /// `Title: verdict`, as Coroot words it.
    title: SharedString,
    verdict: SharedString,
    /// The condition around its threshold, which is set apart.
    condition: Option<[SharedString; 3]>,
}

struct Block {
    /// The share of the row it takes.
    width: f32,
    kind: BlockKind,
}

enum BlockKind {
    Header(SharedString),
    /// The page's table, by its place in `AppPage::tables`.
    Table(usize),
    /// A chart, or a group of charts of which one shows.
    Charts(Rc<charts::Charts>),
    Heatmap(Box<charts::HeatBlock>),
    Profiling,
    Tracing,
}

fn check(ix: usize, check: &api::Check) -> CheckLine {
    let condition = check.condition.contains("<threshold>").then(|| {
        let (head, threshold, tail) = check.condition();
        [
            head.trim_end().to_string().into(),
            threshold.into(),
            tail.trim_start().to_string().into(),
        ]
    });
    CheckLine {
        id: format!("obs-app-check-{ix}").into(),
        condition_id: format!("obs-app-check-{ix}-condition").into(),
        status: check.status.into(),
        title: check.title.clone().into(),
        verdict: if check.message.is_empty() {
            "ok".into()
        } else {
            check.message.clone().into()
        },
        condition: condition.or_else(|| {
            (!check.condition.is_empty()).then(|| {
                [
                    check.condition.clone().into(),
                    SharedString::default(),
                    SharedString::default(),
                ]
            })
        }),
    }
}

impl AppPage {
    fn new(view: &api::AppView, report: &str) -> Self {
        let tabs = view
            .reports
            .iter()
            .map(|r| {
                let status = r.judged().then(|| Status::from(r.status));
                Tab {
                    name: r.name.clone(),
                    id: format!("obs-report-{}", r.name.to_lowercase()).into(),
                    status,
                    tooltip: match status {
                        Some(status) => format!("{} · {}", r.name, status.label()),
                        None => r.name.clone(),
                    }
                    .into(),
                }
            })
            .collect();
        let mut page = Self {
            strip: strip::Strip::new(&view.map),
            tabs,
            checks: vec![],
            blocks: vec![],
            tables: vec![],
            empty: view.reports.is_empty(),
        };
        if let Some(report) = view.reports.iter().find(|r| r.name == report) {
            let slug = report.name.to_lowercase().replace(' ', "-");
            let key = |ix: usize| (view.map.app.id.clone(), report.name.clone(), ix);
            page.checks = report
                .checks
                .iter()
                .enumerate()
                .map(|(ix, c)| check(ix, c))
                .collect();
            for (ix, widget) in report.widgets.iter().enumerate() {
                let kind = match &widget.kind {
                    api::WidgetKind::Chart(chart) => {
                        BlockKind::Charts(Rc::new(charts::Charts::chart(key(ix), &slug, chart)))
                    }
                    api::WidgetKind::ChartGroup { title, charts } => BlockKind::Charts(Rc::new(
                        charts::Charts::group(key(ix), &slug, title, charts),
                    )),
                    api::WidgetKind::Heatmap(heatmap) => {
                        BlockKind::Heatmap(Box::new(charts::HeatBlock::new(&slug, ix, heatmap)))
                    }
                    api::WidgetKind::Header(title) => BlockKind::Header(title.clone().into()),
                    api::WidgetKind::Table(table) => {
                        page.tables.push(Rc::new(table::Prepared::new(table)));
                        BlockKind::Table(page.tables.len() - 1)
                    }
                    // Coroot judges logs in their widget; its check joins the others.
                    api::WidgetKind::Logs(logs) => {
                        let start = page.checks.len();
                        page.checks
                            .extend(logs.iter().enumerate().map(|(ix, c)| check(start + ix, c)));
                        continue;
                    }
                    api::WidgetKind::Profiling => BlockKind::Profiling,
                    api::WidgetKind::Tracing => BlockKind::Tracing,
                    _ => continue,
                };
                let width = match kind {
                    BlockKind::Table(_) | BlockKind::Charts(_) | BlockKind::Heatmap(_) => {
                        widget.width
                    }
                    _ => 1.,
                };
                page.blocks.push(Block { width, kind });
            }
        }
        page
    }

    fn embeds(&self, embedded: fn(&BlockKind) -> bool) -> bool {
        self.blocks.iter().any(|b| embedded(&b.kind))
    }
}

impl ObservabilityPage {
    /// Derives the page from Coroot's answer, or the example, and keeps the
    /// chosen report one the answer has.
    pub(super) fn prepare_view(&mut self) {
        let Some(app) = self.selected_app.clone() else {
            self.app_page = None;
            return;
        };
        let fixture;
        let view = if self.fixture {
            fixture = example::app_view(&app);
            Some(&fixture)
        } else {
            self.live.view.data().filter(|view| view.map.app.id == app)
        };
        let Some(view) = view else {
            self.app_page = None;
            return;
        };
        if let Some(first) = view.reports.first()
            && !view.reports.iter().any(|r| r.name == self.report_name)
        {
            self.report_name = first.name.clone();
        }
        self.app_page = Some(AppPage::new(view, &self.report_name));
    }

    /// Reads the selected application's view from Coroot.
    pub(super) fn read_view(
        &mut self,
        provider: api::Provider,
        source: api::Source,
        cx: &mut Context<Self>,
    ) {
        let Some(app) = self.selected_app.clone() else {
            return;
        };
        let Some(identity) = self.live.identity(Subject::View(app.clone())) else {
            return;
        };
        let range = self.live.range;
        let request = self.live.view.begin(identity);
        self.spawn_read(
            async move { provider.app_view(&source, range, &app).await },
            move |this, result, cx| {
                let refused = matches!(result, Err(api::ReadError::Refused));
                if this
                    .live
                    .view
                    .apply(&request, result.map_err(|e| e.to_string()))
                {
                    this.live.view_refused = refused;
                    this.prepare_report();
                    this.read_embedded(cx);
                    cx.notify();
                }
            },
            cx,
        );
    }

    /// Reads what the chosen report embeds: the traces or the profiles.
    pub(super) fn read_embedded(&mut self, cx: &mut Context<Self>) {
        if self.embeds_tracing() {
            self.read_traces(cx);
        } else if self.embeds_profiling() {
            self.read_profiling(cx);
        }
    }

    /// Whether the chosen report shows the Traces page's view, whose table
    /// is then the page's.
    pub(super) fn embeds_tracing(&self) -> bool {
        self.destination == Destination::Application
            && self
                .app_page
                .as_ref()
                .is_some_and(|page| page.embeds(|b| matches!(b, BlockKind::Tracing)))
    }

    fn embeds_profiling(&self) -> bool {
        self.app_page
            .as_ref()
            .is_some_and(|page| page.embeds(|b| matches!(b, BlockKind::Profiling)))
    }

    pub(super) fn select_report(&mut self, name: String, cx: &mut Context<Self>) {
        self.report_name = name;
        self.prepare_report();
        self.read_embedded(cx);
        cx.notify();
    }

    /// Opens another application on the same report, as Coroot's links do.
    pub(super) fn open_linked_app(&mut self, id: api::AppId, cx: &mut Context<Self>) {
        self.selected_app = Some(id);
        self.report_snapshot = None;
        self.app_page = None;
        self.open(Destination::Application, cx);
    }

    /// Keeps a table entity for each of the report's tables. Runs from
    /// render, and does work only after the tables change.
    pub(super) fn sync_app_tables(&mut self, cx: &mut Context<Self>) {
        let prepared: Vec<_> = self
            .app_page
            .as_ref()
            .map(|page| page.tables.clone())
            .unwrap_or_default();
        let same = self.app_tables.len() == prepared.len()
            && self
                .app_tables
                .iter()
                .zip(&prepared)
                .all(|(entity, table)| Rc::ptr_eq(entity.read(cx).table(), table));
        if same {
            return;
        }
        self.app_tables.truncate(prepared.len());
        let page = cx.entity().downgrade();
        for (ix, table) in prepared.into_iter().enumerate() {
            match self.app_tables.get(ix) {
                Some(entity) => entity.update(cx, |entity, cx| entity.set(table, cx)),
                None => {
                    let page = page.clone();
                    let entity =
                        cx.new(|_| ReportTable::new(&format!("obs-app-table-{ix}"), table, page));
                    self.app_tables.push(entity);
                }
            }
        }
    }

    /// The strip, the tabs and the chosen report, or the read's state.
    pub(super) fn render_app_view(&self, window: &Window, cx: &mut Context<Self>) -> Div {
        let Some(page) = &self.app_page else {
            return v_flex().child(self.render_view_state(cx));
        };
        if page.empty {
            return v_flex().child(
                muted(
                    "Coroot reports nothing for this application in this window.",
                    cx,
                )
                .id("obs-app-empty")
                .test_support()
                .role(Role::Status),
            );
        }
        v_flex()
            .gap(dp(14.))
            .child(self.render_strip(&page.strip, cx))
            .child(self.render_tabs(page, cx))
            .when(!page.checks.is_empty(), |this| {
                this.child(self.render_checks(page, cx))
            })
            .children(self.render_blocks(page, window, cx))
    }

    fn render_view_state(&self, cx: &Context<Self>) -> AnyElement {
        if self.fixture || self.live.view.is_loading() || self.live.view.error().is_none() {
            return freshkube_ui::page::card(cx)
                .id("obs-app-loading")
                .test_support()
                .role(Role::Status)
                .aria_label("Reading the application from Coroot")
                .p(dp(14.))
                .gap(dp(12.))
                .child(ui::skeleton(relative(0.4), dp(12.)))
                .child(ui::skeleton(relative(0.7), dp(12.)))
                .into_any_element();
        }
        let refused = self.live.view_refused;
        ui::empty_state(
            if refused {
                IconName::Shield
            } else {
                IconName::TriangleAlert
            },
            if refused {
                "Not permitted to read this application"
            } else {
                "Couldn't read this application"
            },
            "Nothing is shown as missing: a failed read says nothing about the application's health.",
            self.live.view.error().map(str::to_owned),
            vec![
                action("obs-app-retry", "Retry")
                    .on_click(cx.listener(|this, _, _, cx| this.refresh(cx)))
                    .into_any_element(),
            ],
            cx,
        )
        .id(if refused {
            "obs-app-refused"
        } else {
            "obs-app-failed"
        })
        .test_support()
        .role(Role::Status)
        .h_auto()
        .into_any_element()
    }

    fn render_tabs(&self, page: &AppPage, cx: &Context<Self>) -> Div {
        let p = palette(cx);
        line()
            .flex_wrap()
            .gap(dp(2.))
            .p(dp(3.))
            .rounded(px(8.))
            .bg(p.surface_2)
            .children(page.tabs.iter().map(|tab| {
                let report = tab.name.clone();
                ui::segment(
                    Button::new(tab.id.clone()),
                    self.report_name == tab.name,
                    cx,
                )
                .group("fog-control")
                .small()
                .gap(dp(6.))
                .children(tab.status.map(|s| status(s, cx)))
                .child(text(tab.name.clone()))
                .tooltip(tab.tooltip.clone())
                .on_click(cx.listener(move |this, _, _, cx| this.select_report(report.clone(), cx)))
            }))
    }

    fn render_checks(&self, page: &AppPage, cx: &Context<Self>) -> impl IntoElement + use<> {
        let p = palette(cx);
        card("Checks", cx)
            .id("obs-app-checks")
            .test_support()
            .child(body().pt_0().children(page.checks.iter().map(|check| {
                // Coroot's words wrap inside the card, however narrow it is.
                let words = |words: SharedString| div().min_w_0().whitespace_normal().child(words);
                v_flex()
                    .gap(dp(2.))
                    .child(
                        line()
                            .items_start()
                            .child(
                                h_flex()
                                    .h(dp(20.))
                                    .flex_none()
                                    .child(status(check.status, cx)),
                            )
                            .child(
                                text(format!("{}:", check.title))
                                    .flex_none()
                                    .font_weight(ui::HEADING_WEIGHT),
                            )
                            .child(
                                words(check.verdict.clone())
                                    .id(check.id.clone())
                                    .test_support(),
                            ),
                    )
                    .children(check.condition.as_ref().map(|[head, threshold, tail]| {
                        line()
                            .flex_wrap()
                            .gap(dp(4.))
                            .pl(dp(18.))
                            .text_size(dp(12.))
                            .text_color(p.muted)
                            .child(
                                words(format!("Condition: {head}").into())
                                    .id(check.condition_id.clone())
                                    .test_support(),
                            )
                            .when(!threshold.is_empty(), |this| {
                                this.child(
                                    div()
                                        .font_family(MONO_FONT)
                                        .text_color(p.ink_2)
                                        .child(threshold.clone()),
                                )
                            })
                            .when(!tail.is_empty(), |this| this.child(words(tail.clone())))
                    }))
            })))
    }

    fn render_blocks(
        &self,
        page: &AppPage,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        if page.blocks.is_empty() {
            return None;
        }
        // A chart narrower than this is hard to read, so it takes a row
        // alone; a page narrower than it gives each chart the whole row.
        let roomy = crate::screens::content_width(window) >= MIN_CHART_WIDTH;
        // Coroot's widths are shares of a row; the margins make the gaps.
        let mut grid = div().flex().flex_wrap().mx(dp(-4.)).my(dp(-4.));
        for block in &page.blocks {
            let content = match &block.kind {
                BlockKind::Header(title) => text(title.clone())
                    .pt(dp(6.))
                    .font_weight(ui::HEADING_WEIGHT)
                    .text_size(dp(14.))
                    .into_any_element(),
                BlockKind::Table(ix) => match self.app_tables.get(*ix) {
                    Some(table) => table.clone().into_any_element(),
                    None => continue,
                },
                BlockKind::Charts(charts) => self.render_charts(charts, cx),
                BlockKind::Heatmap(heat) => self.render_heat_block(heat, cx),
                BlockKind::Profiling => self.render_live_profiling(cx),
                BlockKind::Tracing => self.render_live_traces(window, cx),
            };
            let cell = div().min_w_0().p(dp(4.));
            // A half-width chart pairs with the next, and takes the row
            // alone when the page is too narrow for two.
            let cell = match block.kind {
                BlockKind::Charts(_) | BlockKind::Heatmap(_) if roomy => cell
                    .flex_grow(1.)
                    .flex_basis(relative(block.width.clamp(0.1, 1.)))
                    .min_w(dp(MIN_CHART_WIDTH)),
                BlockKind::Charts(_) | BlockKind::Heatmap(_) => cell.w_full(),
                _ => cell.w(relative(block.width.clamp(0.1, 1.))),
            };
            grid = grid.child(cell.child(content));
        }
        Some(grid.into_any_element())
    }
}
