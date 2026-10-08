//! The parts table as a `TableSource`: its columns, sized from the rows
//! when they change, the cells, the group rows and the Link legend.
use super::*;
use crate::ui;
use freshkube_ui::palette::palette;
use freshkube_ui::table::{Line, RowStyle, SortOrder, TableColumn, TableRow, TableSource};
use gpui_kit::component::h_flex;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Field {
    Glyph,
    Object,
    Cluster,
    Namespace,
    Link,
    FoundBy,
    ReadFrom,
}

pub(crate) struct Column {
    field: Field,
    label: SharedString,
    width: f32,
}

impl TableColumn for Column {
    fn label(&self) -> &SharedString {
        &self.label
    }

    fn width(&self) -> f32 {
        self.width
    }

    fn flexible(&self) -> bool {
        self.field == Field::ReadFrom
    }

    /// The glyph and the object stay in view when the table scrolls
    /// sideways.
    fn pinned(&self) -> bool {
        matches!(self.field, Field::Glyph | Field::Object)
    }
}

/// DESIGN.md's widths: 7.5 a character plus 24, between 64 and 280.
fn fit<'a>(label: &str, texts: impl Iterator<Item = &'a SharedString>) -> f32 {
    let chars = texts
        .map(|text| text.chars().count())
        .chain([label.len()])
        .max()
        .unwrap_or_default();
    (chars as f32 * 7.5 + 24.).clamp(64., 280.)
}

/// The columns for these rows, and their total width.
pub(super) fn columns(rows: &[PartRow]) -> (Vec<Column>, f32) {
    let column = |field, label: &str, width| Column {
        field,
        label: label.to_owned().into(),
        width,
    };
    let columns = vec![
        column(Field::Glyph, "", kit::GLYPH_WIDTH),
        column(
            Field::Object,
            "Object",
            fit("Object", rows.iter().map(|row| &row.name)).min(220.),
        ),
        column(
            Field::Cluster,
            "Cluster",
            fit("Cluster", rows.iter().map(|row| &row.cluster)),
        ),
        column(
            Field::Namespace,
            "Namespace",
            fit("Namespace", rows.iter().map(|row| &row.namespace)),
        ),
        // The widest word and its glyph.
        column(
            Field::Link,
            "Link",
            fit("Link", ["Confirmed".into()].iter()) + 12.,
        ),
        column(
            Field::FoundBy,
            "Found by",
            fit("Found by", rows.iter().map(|row| &row.found_by)),
        ),
        column(Field::ReadFrom, "Read from", 96.),
    ];
    let width = columns.iter().map(|column| column.width).sum();
    (columns, width)
}

/// The row's glyph: good for Confirmed, as its Link draws none.
pub(super) fn glyph_tone(confidence: Confidence) -> ui::Tone {
    link_tone(confidence).unwrap_or(ui::Tone::Good)
}

/// The Link cell and the legend's mark: the glyph, if any, and the word,
/// muted when Confirmed.
pub(super) fn link_mark(confidence: Confidence, cx: &App) -> Div {
    let p = palette(cx);
    h_flex()
        .items_center()
        .gap(ui::dp(5.))
        .children(link_tone(confidence).and_then(|tone| ui::status_glyph(tone, cx)))
        .child(
            div()
                .flex_none()
                .text_color(match confidence {
                    Confidence::Confirmed => p.muted,
                    _ => p.ink,
                })
                .child(link_word(confidence)),
        )
}

impl TableSource for ApplicationPage {
    type Key = SharedString;
    type Sort = ();
    type Column = Column;
    type Row<'a> = &'a PartRow;

    fn table_state(&self) -> &TableState {
        &self.table
    }

    fn columns(&self) -> &[Column] {
        &self.columns
    }

    fn width(&self) -> f32 {
        self.width
    }

    fn list_label(&self) -> String {
        format!(
            "The parts of {}, grouped by kind, with how sure each one's link to it is",
            self.name
        )
    }

    fn sorting(&self, _: &Column) -> Option<((), Option<SortOrder>)> {
        None
    }

    fn sort(&mut self, _: (), _: &mut Context<Self>) {}

    fn line_count(&self) -> usize {
        self.lines.len()
    }

    fn line(&self, line: usize, _: &App) -> Option<Line<SharedString, &PartRow>> {
        let ix = match *self.lines.get(line)? {
            Entry::Group(group) => return Some(Line::Group(group)),
            Entry::Row(ix) => ix,
        };
        let row = self.rows.get(ix)?;
        Some(Line::Row(TableRow {
            key: row.key.clone(),
            id: SharedString::from(format!("application-part-{}", row.key)).into(),
            label: row.label.clone(),
            tooltip: Some(row.tooltip.clone()),
            marked: false,
            muted: false,
            data: row,
        }))
    }

    fn cell(
        &self,
        line: &TableRow<SharedString, &PartRow>,
        style: &RowStyle,
        column: &Column,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let row = line.data;
        let cell = kit::cell(column);
        match column.field {
            Field::Glyph => kit::glyph_cell(column)
                .child(ui::status_mark(
                    SharedString::from(format!("application-part-{}-mark", row.key)),
                    glyph_tone(row.confidence),
                    link_word(row.confidence),
                    cx,
                ))
                .into_any_element(),
            Field::Object => cell.child(row.name.clone()).into_any_element(),
            Field::Cluster => cell.child(row.cluster.clone()).into_any_element(),
            Field::Namespace => cell.child(row.namespace.clone()).into_any_element(),
            Field::Link => cell
                .child(
                    link_mark(row.confidence, cx)
                        .id(SharedString::from(format!(
                            "application-part-{}-link",
                            row.key
                        )))
                        .test_support()
                        .aria_label(link_word(row.confidence)),
                )
                .into_any_element(),
            Field::FoundBy => cell.child(row.found_by.clone()).into_any_element(),
            Field::ReadFrom => cell
                .text_color(style.p.muted)
                .child(row.read_from.clone())
                .into_any_element(),
        }
    }

    fn group(&self, group: usize, cx: &mut Context<Self>) -> Option<AnyElement> {
        let line = self.groups.get(group)?;
        Some(
            kit::GroupRow::new(
                self.table.id(&format!("group-{}", line.slug)),
                line.tone,
                line.label.clone(),
                kit::ROW_HEIGHT,
            )
            .detail(line.detail.clone())
            .render(cx)
            .into_any_element(),
        )
    }

    fn selected_key(&self) -> Option<&SharedString> {
        self.selected.as_ref()
    }

    fn line_of(&self, key: &SharedString) -> Option<usize> {
        self.lines
            .iter()
            .position(|entry| matches!(entry, Entry::Row(ix) if &self.rows[*ix].key == key))
    }

    /// A click selects the part, which opens the Inspector, and puts the
    /// keyboard on the table.
    fn click(
        &mut self,
        key: &SharedString,
        _: &ClickEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.select(key.clone(), cx);
        self.focus(window, cx);
    }

    fn empty(&self, _: &mut Context<Self>) -> Option<AnyElement> {
        self.lines
            .is_empty()
            .then(|| "Nothing of it was read.".into_any_element())
    }

    /// What the Link column's three words mean: the whole legend when the
    /// table is wide, one line with it in a tooltip when narrow.
    fn legend(&self, window: &Window, cx: &mut Context<Self>) -> Option<AnyElement> {
        const WHOLE: &str = "Link: how sure a part's link to the application is. Confirmed: both \
                             sides were read and agree. Claimed: only a label, annotation, name \
                             or your override says so. Unknown: a side couldn't be read.";
        if self.table_width(window) < 900. {
            return Some(
                kit::legend_line(
                    self.table.id("legend"),
                    "Confirmed · Claimed · Unknown ⓘ",
                    WHOLE,
                    cx,
                )
                .into_any_element(),
            );
        }
        let item = |confidence: Confidence, text: &str| {
            kit::legend_item(link_mark(confidence, cx), text.to_owned()).into_any_element()
        };
        Some(
            kit::legend(
                [
                    item(Confidence::Confirmed, "both sides were read and agree"),
                    item(
                        Confidence::Claimed,
                        "only a label, annotation or name says so",
                    ),
                    item(Confidence::Unknown, "a side couldn't be read"),
                ],
                cx,
            )
            .id(self.table.id("legend"))
            .test_support()
            .aria_label(WHOLE)
            .into_any_element(),
        )
    }
}

impl ApplicationPage {
    /// The table's width: the page's, less the Inspector's beside it.
    fn table_width(&self, window: &Window) -> f32 {
        let page = crate::screens::page_width(window);
        if self.selected.is_some() && page >= freshkube_ui::inspector::SPLIT_WIDTH {
            page - self.split.width()
        } else {
            page
        }
    }
}
