//! Application column widths are prepared when observations or Columns change.
use super::*;
use freshkube_ui::table::TableColumn;

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(in crate::observability) enum ColumnKind {
    Glyph,
    Name,
    Type,
    Report(Report),
}

#[derive(Clone)]
pub(crate) struct ApplicationColumn {
    pub(in crate::observability) kind: ColumnKind,
    label: SharedString,
    width: f32,
}

impl TableColumn for ApplicationColumn {
    fn label(&self) -> &SharedString {
        &self.label
    }
    fn width(&self) -> f32 {
        self.width
    }
    fn flexible(&self) -> bool {
        self.kind == ColumnKind::Name
    }
}

impl ObservabilityPage {
    pub(in crate::observability) fn prepare_application_columns(&mut self) {
        self.application_columns = [ColumnKind::Glyph, ColumnKind::Name, ColumnKind::Type]
            .into_iter()
            .chain(Report::ALL.map(ColumnKind::Report))
            .filter(|kind| !self.hidden_application_columns.contains(kind))
            .map(|kind| {
                let label = match kind {
                    ColumnKind::Glyph => "",
                    ColumnKind::Name => "Application",
                    ColumnKind::Type => "Type",
                    ColumnKind::Report(report) => report.label(),
                };
                let longest_value = self
                    .applications
                    .iter()
                    .map(|app| match kind {
                        ColumnKind::Glyph => 0,
                        ColumnKind::Name => app.name.chars().count(),
                        ColumnKind::Type => app.language.chars().count(),
                        ColumnKind::Report(report) => app.check(report).value.chars().count(),
                    })
                    .max()
                    .unwrap_or(0);
                let longest = longest_value.max(label.chars().count());
                let width = match kind {
                    ColumnKind::Glyph => 34.,
                    ColumnKind::Name => 120.,
                    ColumnKind::Report(_) => (longest as f32 * 7.5
                        + if longest_value > label.chars().count() {
                            38.
                        } else {
                            0.
                        })
                    .clamp(48., 280.),
                    ColumnKind::Type => 64.,
                };
                ApplicationColumn {
                    kind,
                    label: label.into(),
                    width,
                }
            })
            .collect();
        self.application_width = self
            .application_columns
            .iter()
            .map(TableColumn::width)
            .sum();
    }
}
