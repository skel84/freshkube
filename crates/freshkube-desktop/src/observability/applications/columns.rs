//! Application column widths are prepared when observations or Columns change.
use super::*;
use freshkube_ui::table::TableColumn;
use gpui_kit::component::Theme;

/// Font metrics belong to the page, and are used only when columns rebuild.
pub(in crate::observability) struct ApplicationMetrics {
    text_system: std::sync::Arc<WindowTextSystem>,
    family: SharedString,
    size: Pixels,
}

impl ApplicationMetrics {
    pub(in crate::observability) fn new(window: &Window, cx: &App) -> Self {
        let theme = Theme::global(cx);
        Self {
            text_system: window.text_system().clone(),
            family: theme.font_family.clone(),
            size: theme.font_size,
        }
    }

    pub(in crate::observability) fn sync(&mut self, cx: &App) -> bool {
        let theme = Theme::global(cx);
        if self.family == theme.font_family && self.size == theme.font_size {
            return false;
        }
        self.family = theme.font_family.clone();
        self.size = theme.font_size;
        true
    }

    fn measure(&self, text: SharedString, font: Font, size: f32) -> f32 {
        let scale = f32::from(self.size) / ui::BASE_TEXT;
        let run = TextRun {
            len: text.len(),
            font,
            color: Default::default(),
            background_color: None,
            underline: None,
            strikethrough: None,
        };
        f32::from(
            self.text_system
                .shape_line(text, px(size * scale), &[run], None)
                .width,
        ) / scale
    }

    fn caption(&self, label: &str) -> f32 {
        let mut face = font(self.family.clone());
        face.weight = ui::HEADING_WEIGHT;
        self.measure(label.to_uppercase().into(), face, 11.)
    }

    fn value(&self, value: SharedString, size: f32) -> f32 {
        self.measure(value, font(MONO_FONT), size)
    }
}

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
                let caption = self.application_metrics.caption(label);
                let value = self
                    .applications
                    .iter()
                    .map(|app| match kind {
                        ColumnKind::Glyph | ColumnKind::Name => 0.,
                        ColumnKind::Type => self
                            .application_metrics
                            .value(app.language.clone().into(), 12.5),
                        ColumnKind::Report(report) => {
                            let check = app.check(report);
                            self.application_metrics.value(check.value.clone(), 12.)
                                + if check.status.report_tone().is_some() {
                                    14.
                                } else {
                                    0.
                                }
                        }
                    })
                    .fold(0., f32::max);
                // Includes both cell insets, with room for fractional shaping.
                let measured = (caption.max(value) + 24.).ceil();
                let width = match kind {
                    ColumnKind::Glyph => 34.,
                    ColumnKind::Name => measured.max(120.),
                    ColumnKind::Report(_) => measured.max(48.),
                    ColumnKind::Type => measured.max(64.),
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
