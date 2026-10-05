//! The services table as a `TableSource`: its columns, sized from the rows
//! when they change, and the cells of each row.
use super::*;
use table::{Line, RowStyle, SortOrder, TableColumn, TableRow, TableSource, TableState};

#[derive(Clone, Copy, PartialEq, Eq)]
enum Field {
    Glyph,
    Node,
    Service,
    State,
    Message,
    Actions,
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
        self.field == Field::Message
    }
}

/// DESIGN.md's widths: 7.5 a character plus 24, between 64 and 280; the
/// glyph's is 34.
fn fit<'a>(label: &str, texts: impl Iterator<Item = &'a SharedString>) -> f32 {
    let chars = texts
        .map(|text| text.chars().count())
        .chain([label.len()])
        .max()
        .unwrap_or_default();
    (chars as f32 * 7.5 + 24.).clamp(64., 280.)
}

/// The columns for these rows, and their total width.
pub(super) fn columns(rows: &[ServiceRow]) -> (Vec<Column>, f32) {
    let column = |field, label: &str, width| Column {
        field,
        label: label.to_owned().into(),
        width,
    };
    let columns = vec![
        column(Field::Glyph, "", 34.),
        column(
            Field::Node,
            "Node",
            fit("Node", rows.iter().map(|row| &row.node)),
        ),
        column(
            Field::Service,
            "Service",
            fit("Service", rows.iter().map(|row| &row.service)),
        ),
        column(
            Field::State,
            "State",
            fit("State", rows.iter().map(|row| &row.state)),
        ),
        column(Field::Message, "Health check", 240.),
        column(Field::Actions, "", 168.),
    ];
    let width = columns.iter().map(|column| column.width).sum();
    (columns, width)
}

impl TableSource for SystemServices {
    type Key = SharedString;
    type Sort = ();
    type Column = Column;
    type Row<'a> = &'a ServiceRow;

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
        "System services on every node, problems first; each row opens its logs or its node".into()
    }

    fn sorting(&self, _: &Column) -> Option<((), Option<SortOrder>)> {
        None
    }

    fn sort(&mut self, _: (), _: &mut Context<Self>) {}

    fn line_count(&self) -> usize {
        self.lines.len()
    }

    fn line(&self, line: usize, _: &App) -> Option<Line<SharedString, &ServiceRow>> {
        let ix = match *self.lines.get(line)? {
            Entry::Group(status) => return Some(Line::Group(status)),
            Entry::Row(ix) => ix,
        };
        let row = self.rows.get(ix)?;
        Some(Line::Row(TableRow {
            key: row.id.clone(),
            id: row.id.clone().into(),
            label: format!(
                "{} · {} · {} · {}",
                row.node,
                row.service,
                presentation::health_text(&row.health),
                row.message
            )
            .into(),
            tooltip: (!row.message.is_empty()).then(|| row.message.clone()),
            marked: false,
            muted: false,
            data: row,
        }))
    }

    fn cell(
        &self,
        line: &TableRow<SharedString, &ServiceRow>,
        style: &RowStyle,
        column: &Column,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let row = line.data;
        let cell = table::cell(column);
        match column.field {
            Field::Glyph => cell
                .flex()
                .items_center()
                .child(ui::health_mark(
                    SharedString::from(format!("{}-health", row.id)),
                    row.health,
                    cx,
                ))
                .into_any_element(),
            Field::Node => cell.child(row.node.clone()).into_any_element(),
            Field::Service => cell.child(row.service.clone()).into_any_element(),
            Field::State => cell.child(row.state.clone()).into_any_element(),
            Field::Message => cell
                .text_color(style.p.muted)
                .child(row.message.clone())
                .into_any_element(),
            Field::Actions => {
                let (logs_node, logs_service) = (row.node.to_string(), row.service.to_string());
                let (open_node, open_service) = (logs_node.clone(), logs_service.clone());
                cell.flex()
                    .items_center()
                    .justify_end()
                    .gap_1()
                    .child(Button::new("logs").ghost().xsmall().label("Logs").on_click(
                        cx.listener(move |_, _, _, cx| {
                            cx.emit(ServiceEvent::Logs(logs_node.clone(), logs_service.clone()))
                        }),
                    ))
                    .child(
                        Button::new("open")
                            .outline()
                            .xsmall()
                            .label("Open node")
                            .on_click(cx.listener(move |_, _, _, cx| {
                                cx.emit(ServiceEvent::Open(open_node.clone(), open_service.clone()))
                            })),
                    )
                    .into_any_element()
            }
        }
    }

    fn group(&self, status: usize, cx: &mut Context<Self>) -> Option<AnyElement> {
        let Status { health, label, .. } = *STATUSES.get(status)?;
        let count = self.counts[status];
        Some(
            table::GroupRow::new(
                self.table
                    .id(&format!("group-{}", label.to_lowercase().replace(' ', "-"))),
                ui::health_tone(health),
                label,
                self.table.row_height(),
            )
            .detail(vec![format!(
                "{count} {}",
                if count == 1 { "service" } else { "services" }
            )])
            .render(cx)
            .into_any_element(),
        )
    }

    fn clickable(&self) -> bool {
        false
    }

    fn empty(&self, _: &mut Context<Self>) -> Option<AnyElement> {
        if !self.lines.is_empty() {
            return None;
        }
        Some(
            if self.rows.is_empty() {
                "No node has reported its services yet."
            } else {
                "No services match this filter."
            }
            .into_any_element(),
        )
    }
}
