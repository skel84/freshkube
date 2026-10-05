//! Application column widths are prepared when observations or Columns change.
use super::*;
use freshkube_ui::table::TableColumn;

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(super) enum ColumnKind {
    Glyph,
    Name,
    Type,
    Report(Report),
}

#[derive(Clone)]
pub(crate) struct ApplicationColumn {
    pub(super) kind: ColumnKind,
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
    pub(super) fn prepare_application_columns(&mut self) {
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
                let longest = self
                    .applications
                    .iter()
                    .map(|app| match kind {
                        ColumnKind::Glyph => 0,
                        ColumnKind::Name => app.name.chars().count(),
                        ColumnKind::Type => app.language.chars().count(),
                        ColumnKind::Report(report) => app.check(report).value.chars().count(),
                    })
                    .max()
                    .unwrap_or(0)
                    .max(label.chars().count());
                let width = match kind {
                    ColumnKind::Glyph => 34.,
                    ColumnKind::Name => (longest as f32 * 7.5 + 24.).clamp(180., 440.),
                    ColumnKind::Report(_) => (longest as f32 * 7.5 + 38.).clamp(64., 280.),
                    ColumnKind::Type => (longest as f32 * 7.5 + 24.).clamp(64., 280.),
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
