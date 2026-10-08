//! The install disks as the shared table: rows derived when the session
//! changes, and their cells. A row click does nothing; the install disk is
//! chosen only with its button, since it decides which disk is erased.
use freshkube_ui::table::{
    self, DataTable, Line, RowStyle, SortOrder, TableColumn, TableRow, TableSource, TableState,
};

use super::*;

/// At most this many disks are listed.
const MAX_DISKS: usize = 256;
/// One flag's room in the Flags column: the widest tag, Read-only, with its
/// glyph. A usable disk shows only Selected, so most rows need one.
const FLAG_WIDTH: f32 = 88.;
/// The action column, as wide as its button.
const ACTION_WIDTH: f32 = 108.;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Field {
    Device,
    Id,
    Size,
    Model,
    Serial,
    Flags,
    Action,
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
        self.field == Field::Device
    }

    /// The device, its flags and its button stay in view when the table
    /// scrolls sideways, so the choice never needs a scroll to reach.
    fn pinned(&self) -> bool {
        matches!(self.field, Field::Device | Field::Flags | Field::Action)
    }
}

pub(crate) struct DiskRow {
    path: SharedString,
    id: SharedString,
    size: SharedString,
    model: SharedString,
    serial: SharedString,
    readonly: bool,
    cdrom: bool,
    label: SharedString,
}

impl DiskRow {
    fn usable(&self) -> bool {
        !self.readonly && !self.cdrom
    }

    /// Why the disk can't be chosen, as tags. Optical drives are read-only
    /// by design, as Storage says, so only a real disk that can't be
    /// written warns.
    fn flags(&self) -> impl Iterator<Item = (Tone, &'static str)> {
        let read_only = if self.cdrom {
            Tone::Outline
        } else {
            Tone::Warn
        };
        [
            self.readonly.then_some((read_only, "Read-only")),
            self.cdrom.then_some((Tone::Outline, "Optical")),
        ]
        .into_iter()
        .flatten()
    }
}

/// The table's rows and columns, and the chosen disk's path.
pub(super) struct Disks {
    rows: Vec<DiskRow>,
    columns: Vec<Column>,
    width: f32,
    selected: Option<SharedString>,
    table: TableState,
}

impl Default for Disks {
    fn default() -> Self {
        let mut disks = Self {
            rows: Vec::new(),
            columns: Vec::new(),
            width: 0.,
            selected: None,
            table: TableState::new("maint-disks"),
        };
        disks.columns = columns(&[]);
        disks.width = disks.columns.iter().map(|column| column.width).sum();
        disks
    }
}

impl Disks {
    /// Derives the rows from the session's disks and its install target.
    pub(super) fn sync(&mut self, session: Option<&BootstrapSession>) {
        let snapshot = session.and_then(|session| session.insecure_snapshot.as_ref());
        self.rows = snapshot
            .map(|snapshot| {
                snapshot
                    .disks
                    .iter()
                    .take(MAX_DISKS)
                    .map(|disk| {
                        let model = disk.model.as_deref().unwrap_or("model unknown");
                        let serial = disk.serial.as_deref().unwrap_or("serial unknown");
                        DiskRow {
                            label: format!(
                                "Disk {}, {} · {model} · {serial}",
                                disk.dev_path, disk.size_pretty
                            )
                            .into(),
                            path: disk.dev_path.clone().into(),
                            id: disk.id.clone().into(),
                            size: disk.size_pretty.clone().into(),
                            model: model.to_owned().into(),
                            serial: serial.to_owned().into(),
                            readonly: disk.readonly,
                            cdrom: disk.cdrom,
                        }
                    })
                    .collect()
            })
            .unwrap_or_default();
        self.selected = session
            .and_then(|session| session.install_target.as_ref())
            .map(|target| target.device_path().to_owned().into());
        self.columns = columns(&self.rows);
        self.width = self.columns.iter().map(|column| column.width).sum();
    }
}

/// The loading rows the disks' table shows: a node has a disk or two.
const LOADING_LINES: usize = 2;

fn columns(rows: &[DiskRow]) -> Vec<Column> {
    let column = |field, label: &str, width| Column {
        field,
        label: label.to_owned().into(),
        width,
    };
    let fit = |label: &str, text: fn(&DiskRow) -> &SharedString| {
        table::fit(label, rows.iter().map(text), 280.)
    };
    // A disk both read-only and optical shows two flags.
    let flags = rows
        .iter()
        .map(|row| usize::from(row.readonly) + usize::from(row.cdrom))
        .max()
        .unwrap_or_default()
        .max(1) as f32;
    let flags_width = flags * FLAG_WIDTH + (flags - 1.) * 4. + 2. * table::CELL_PAD;
    vec![
        column(Field::Device, "Device", fit("Device", |row| &row.path)),
        column(Field::Flags, "", flags_width),
        column(Field::Action, "", ACTION_WIDTH),
        column(Field::Size, "Size", fit("Size", |row| &row.size)),
        column(Field::Model, "Model", fit("Model", |row| &row.model)),
        column(Field::Serial, "Serial", fit("Serial", |row| &row.serial)),
        column(Field::Id, "ID", fit("ID", |row| &row.id)),
    ]
}

impl MaintenanceView {
    /// Whether the node is being read for the first time: its disks are
    /// still to come, and their table shows its loading rows.
    pub(super) fn collecting(&self) -> bool {
        self.work.is_some()
            && self.session.as_ref().is_some_and(|session| {
                session.insecure_snapshot.is_none()
                    && session.phase == BootstrapPhase::CollectingInsecureData
            })
    }

    /// The disks, carded under the hardware panel and as tall as its rows.
    pub(super) fn disks_panel(&self, window: &Window, cx: &mut Context<Self>) -> Div {
        let lines = if self.collecting() {
            LOADING_LINES
        } else {
            self.disks.rows.len().max(1)
        };
        DataTable::new()
            .carded()
            .fit(lines)
            .render(self, window, cx)
            .w_full()
    }

    /// Whether a disk's button may choose it now.
    fn may_select(&self, row: &DiskRow, cx: &App) -> bool {
        let busy = self.work.is_some() || Self::slot_busy(cx).is_some();
        let selecting = self
            .session
            .as_ref()
            .is_some_and(|session| session.phase == BootstrapPhase::SelectingInstallTarget);
        !busy && selecting && row.usable()
    }
}

impl TableSource for MaintenanceView {
    type Key = SharedString;
    type Sort = ();
    type Column = Column;
    type Row<'a> = (usize, &'a DiskRow);

    fn table_state(&self) -> &TableState {
        &self.disks.table
    }

    fn columns(&self) -> &[Column] {
        &self.disks.columns
    }

    fn width(&self) -> f32 {
        self.disks.width
    }

    fn list_label(&self) -> String {
        "Install disks; choose one with its Select install disk button".into()
    }

    fn sorting(&self, _: &Column) -> Option<((), Option<SortOrder>)> {
        None
    }

    fn sort(&mut self, _: (), _: &mut Context<Self>) {}

    fn line_count(&self) -> usize {
        self.disks.rows.len()
    }

    fn line(&self, line: usize, _: &App) -> Option<Line<SharedString, (usize, &DiskRow)>> {
        let row = self.disks.rows.get(line)?;
        Some(Line::Row(TableRow {
            key: row.path.clone(),
            id: ("maint-disk", line).into(),
            label: row.label.clone(),
            tooltip: None,
            marked: false,
            muted: false,
            data: (line, row),
        }))
    }

    fn cell(
        &self,
        line: &TableRow<SharedString, (usize, &DiskRow)>,
        _: &RowStyle,
        column: &Column,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let (ix, row) = line.data;
        let cell = table::cell(column);
        match column.field {
            Field::Device => cell
                .font_family(MONO_FONT)
                .child(row.path.clone())
                .into_any_element(),
            Field::Id => cell
                .font_family(MONO_FONT)
                .child(row.id.clone())
                .into_any_element(),
            Field::Size => cell.child(row.size.clone()).into_any_element(),
            Field::Model => cell.child(row.model.clone()).into_any_element(),
            Field::Serial => cell.child(row.serial.clone()).into_any_element(),
            // The rows are monospace; tags and buttons keep the UI's font.
            Field::Flags => cell
                .flex()
                .items_center()
                .gap_1()
                .font_family(cx.theme().font_family.clone())
                .children(
                    row.flags()
                        .map(|(tone, label)| ui::tag(tone, None, label, cx)),
                )
                .when(self.disks.selected.as_ref() == Some(&row.path), |this| {
                    this.child(ui::tag(
                        Tone::Accent,
                        Some(IconName::CircleCheck),
                        "Selected",
                        cx,
                    ))
                })
                .into_any_element(),
            Field::Action => {
                let path = row.path.to_string();
                cell.flex()
                    .items_center()
                    .font_family(cx.theme().font_family.clone())
                    .child(
                        Button::new(("maint-disk-select", ix))
                            .outline()
                            .xsmall()
                            .label("Select install disk")
                            .disabled(!self.may_select(row, cx))
                            .on_click(cx.listener(move |view, _, _, cx| {
                                view.select_disk(path.clone(), cx)
                            })),
                    )
                    .into_any_element()
            }
        }
    }

    fn group(&self, _: usize, _: &mut Context<Self>) -> Option<AnyElement> {
        None
    }

    fn selected_key(&self) -> Option<&SharedString> {
        self.disks.selected.as_ref()
    }

    fn line_of(&self, key: &SharedString) -> Option<usize> {
        self.disks.rows.iter().position(|row| &row.path == key)
    }

    /// Choosing the install disk takes its button, never a row click.
    fn clickable(&self) -> bool {
        false
    }

    fn loading(&self) -> Option<&table::LoadingRows> {
        self.loading.rows()
    }

    fn empty(&self, _: &mut Context<Self>) -> Option<AnyElement> {
        self.disks
            .rows
            .is_empty()
            .then(|| "Talos reported no disks.".into_any_element())
    }
}

#[cfg(test)]
mod tests {
    // Not `super::*`: maintenance's glob of gpui_kit brings its `test`.
    use super::{DiskRow, Tone};

    fn row(readonly: bool, cdrom: bool) -> DiskRow {
        DiskRow {
            path: "/dev/sr0".into(),
            id: "sr0".into(),
            size: "1.0 GB".into(),
            model: "Example".into(),
            serial: "EXAMPLE".into(),
            readonly,
            cdrom,
            label: "Disk".into(),
        }
    }

    #[test]
    fn only_a_real_disk_that_cant_be_written_warns() {
        let flags = |readonly, cdrom| row(readonly, cdrom).flags().collect::<Vec<_>>();
        assert_eq!(flags(false, false), []);
        assert_eq!(flags(true, false), [(Tone::Warn, "Read-only")]);
        assert_eq!(flags(false, true), [(Tone::Outline, "Optical")]);
        assert_eq!(
            flags(true, true),
            [(Tone::Outline, "Read-only"), (Tone::Outline, "Optical")]
        );
    }
}
