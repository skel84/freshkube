//! The stepper cell and the rail (`freshkube_ui::table::{stepper_cell,
//! rail}`): the Overview matrix's cells and its pinned Stage column, on
//! invented Stages. Each case isolates one state of the track: confirmed
//! (solid), a Stage that waits (dotted, and the clock), the causes of an
//! Unknown, and a Stage the change hasn't reached (a hairline). The rows
//! declare the 40 dp height the cells need.

use std::rc::Rc;

use super::motion::{option, segments};
use freshkube_ui::page::{self, PageHeader};
use freshkube_ui::palette::palette;
use freshkube_ui::table::{
    self, Line, RailAt, RowStyle, SortOrder, Step, TableColumn, TableRow, TableSource, TableState,
    Track,
};
use freshkube_ui::ui::{Tone, dp};
use gpui_kit::component::v_flex;
use gpui_kit::prelude::*;
use gpui_kit::{
    AnyElement, AnyView, App, Context, FontWeight, Render, SharedString, TestSupportExt, Window,
    div,
};

/// The id prefix of everything the story draws.
const PREFIX: &str = "stepper";
const STEPS: [&str; 4] = ["Eligible", "Promoted", "Verified", "Running"];

pub fn build(_: &mut Window, cx: &mut App) -> AnyView {
    cx.new(|_| StepperStory {
        case: Case::Confirmed,
        picked: None,
        table: TableState::new(PREFIX),
        columns: columns(),
    })
    .into()
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Case {
    Confirmed,
    Waiting,
    Causes,
    Unreached,
}

impl Case {
    const ALL: [(Self, &'static str, &'static str); 4] = [
        (Self::Confirmed, "confirmed", "Confirmed"),
        (Self::Waiting, "waiting", "Waiting"),
        (Self::Causes, "causes", "Causes"),
        (Self::Unreached, "unreached", "Not reached"),
    ];

    /// Each Stage's cells: tone and word.
    fn stages(self) -> Vec<Stage> {
        use Tone::*;
        let done = [
            (Good, "Eligible"),
            (Good, "Promoted"),
            (Good, "Verified"),
            (Good, "Running"),
        ];
        match self {
            Self::Confirmed => vec![("dev", done, false), ("stage", done, false)],
            Self::Waiting => vec![
                ("dev", done, false),
                (
                    "prod-a",
                    [
                        (Good, "Eligible"),
                        (Waiting, "Waiting"),
                        (Waiting, "Waiting"),
                        (Outline, "Not reached"),
                    ],
                    true,
                ),
            ],
            Self::Causes => vec![
                ("dev", done, false),
                (
                    "prod-a",
                    [
                        (Good, "Eligible"),
                        (Good, "Promoted"),
                        (Unknown, "Not found"),
                        (Unknown, "Can't read"),
                    ],
                    false,
                ),
                (
                    "prod-b",
                    [
                        (Good, "Eligible"),
                        (Good, "Promoted"),
                        (Good, "Verified"),
                        (Unknown, "Not set up"),
                    ],
                    false,
                ),
            ],
            Self::Unreached => vec![
                ("dev", done, false),
                ("prod-a", [(Outline, "Not reached"); 4], true),
            ],
        }
    }
}

pub struct Column {
    label: SharedString,
    width: f32,
    stage: bool,
}

impl TableColumn for Column {
    fn label(&self) -> &SharedString {
        &self.label
    }
    fn width(&self) -> f32 {
        self.width
    }
    fn flexible(&self) -> bool {
        false
    }
    fn pinned(&self) -> bool {
        self.stage
    }
}

fn columns() -> Vec<Column> {
    std::iter::once(Column {
        label: "Stage".into(),
        width: 180.,
        stage: true,
    })
    .chain(STEPS.map(|label| Column {
        label: label.into(),
        width: 112.,
        stage: false,
    }))
    .collect()
}

pub struct StepperStory {
    case: Case,
    picked: Option<SharedString>,
    table: TableState,
    columns: Vec<Column>,
}

impl StepperStory {
    fn set(&mut self, case: Case, cx: &mut Context<Self>) {
        self.case = case;
        self.picked = None;
        cx.notify();
    }

    pub fn case(&self) -> Case {
        self.case
    }

    pub fn picked(&self) -> Option<&SharedString> {
        self.picked.as_ref()
    }
}

/// A Stage's name, its cells (tone and word) and whether it waits.
type Stage = (&'static str, [(Tone, &'static str); 4], bool);

type Row = (usize, &'static str, [(Tone, &'static str); 4], bool);

fn track(tone: Tone, reached: bool) -> Track {
    if reached {
        Track::into_tone(tone)
    } else {
        Track::Hairline
    }
}

impl TableSource for StepperStory {
    type Key = SharedString;
    type Sort = ();
    type Column = Column;
    type Row<'a> = Rc<Row>;

    fn table_state(&self) -> &TableState {
        &self.table
    }
    fn columns(&self) -> &[Column] {
        &self.columns
    }
    fn width(&self) -> f32 {
        self.columns.iter().map(TableColumn::width).sum()
    }
    fn row_height(&self) -> f32 {
        table::STEP_ROW
    }
    fn list_label(&self) -> String {
        "Invented Stages and their steps".into()
    }
    fn sorting(&self, _: &Column) -> Option<((), Option<SortOrder>)> {
        None
    }
    fn sort(&mut self, _: (), _: &mut Context<Self>) {}
    fn line_count(&self) -> usize {
        self.case.stages().len()
    }
    fn line(&self, line: usize, _: &App) -> Option<Line<SharedString, Rc<Row>>> {
        let (name, cells, waits) = self.case.stages().into_iter().nth(line)?;
        Some(Line::Row(TableRow {
            key: name.into(),
            id: SharedString::from(format!("{PREFIX}-row-{name}")).into(),
            label: format!("Stage {name}").into(),
            tooltip: None,
            marked: false,
            muted: false,
            data: Rc::new((line, name, cells, waits)),
        }))
    }
    fn cell(
        &self,
        line: &TableRow<SharedString, Rc<Row>>,
        style: &RowStyle,
        column: &Column,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let (ix, name, cells, waits) = *line.data;
        let count = self.line_count();
        if column.stage {
            let into = if waits { Track::Dotted } else { Track::Solid };
            let rail = table::rail(
                format!("{PREFIX}-rail-{name}"),
                RailAt::of(ix, count),
                Tone::Good,
                into,
                Track::Solid,
                cx,
            );
            return table::cell(column)
                .px_0()
                .h_full()
                .flex()
                .items_center()
                .child(rail)
                .child(
                    v_flex()
                        .child(
                            div()
                                .h(dp(table::LINE))
                                .font_weight(FontWeight::SEMIBOLD)
                                .child(name),
                        )
                        .child(
                            div()
                                .h(dp(table::LINE))
                                .text_size(dp(11.))
                                .text_color(style.p.muted)
                                .child(if waits {
                                    "still runs brave-lynx"
                                } else {
                                    "since 14:13"
                                }),
                        ),
                )
                .into_any_element();
        }
        let at = STEPS
            .iter()
            .position(|s| column.label == *s)
            .unwrap_or_default();
        let (tone, word) = cells[at];
        let reached = tone != Tone::Outline;
        let next = cells.get(at + 1).copied();
        let id = SharedString::from(format!("{PREFIX}-{name}-{at}"));
        let picked = id.clone();
        table::stepper_cell(
            column,
            Step {
                id: id.clone(),
                tone,
                word: word.into(),
                into: track(tone, reached),
                onward: next.map_or(Track::None, |(tone, _)| track(tone, tone != Tone::Outline)),
                onward_tone: next.map_or(Tone::Outline, |(tone, _)| tone),
                tooltip: format!("{name} · {}: {word}", STEPS[at]).into(),
                selected: self.picked.as_ref() == Some(&id),
            },
            cx,
        )
        .cursor_pointer()
        .on_click(cx.listener(move |this, _, _, cx| {
            this.picked = Some(picked.clone());
            cx.notify();
        }))
        .into_any_element()
    }
    fn group(&self, _: usize, _: &mut Context<Self>) -> Option<AnyElement> {
        None
    }
    fn clickable(&self) -> bool {
        false
    }
    fn empty(&self, _: &mut Context<Self>) -> Option<AnyElement> {
        None
    }
}

impl Render for StepperStory {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        freshkube_probe::probe::hit("workbench.stepper");
        let case = self.case;
        let choices = segments(
            Case::ALL.map(|(choice, id, label)| {
                option(format!("{PREFIX}-case-{id}"), label, case == choice, cx)
                    .on_click(cx.listener(move |this, _, _, cx| this.set(choice, cx)))
            }),
            cx,
        );
        let header = PageHeader::new(PREFIX, "Stepper cell and rail")
            .control(choices)
            .meta([div().child("Invented Stages").into_any_element()])
            .render(window, cx);
        let p = palette(cx);
        let picked = self
            .picked
            .as_ref()
            .map_or("Nothing selected".to_owned(), |id| format!("Selected {id}"));
        page::page(SharedString::from(format!("{PREFIX}-page")))
            .child(page::toolbar(cx).child(header))
            .child(
                v_flex()
                    .flex_none()
                    .h(dp(table::HEADER_HEIGHT + 3. * table::STEP_ROW + 2.))
                    .child(table::data_table(self, window, cx).flex_1().min_h_0()),
            )
            .child(
                page::inset().child(
                    div()
                        .id(format!("{PREFIX}-picked"))
                        .test_support()
                        .aria_label(SharedString::from(picked.clone()))
                        .text_color(p.muted)
                        .child(picked),
                ),
            )
    }
}
