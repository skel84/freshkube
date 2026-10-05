//! The Disks and Volumes tables as one `TableSource`: their rows and columns,
//! derived when the node's answer arrives, and each row's cells. The showing
//! table follows the view mode, and each keeps its own scroll and selection.
use super::*;
use table::{
    Line, RowStyle, SortOrder, TableColumn, TableRow, TableSource, TableState, WIDEST,
    WIDEST_FLEXIBLE, fit,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Field {
    Glyph,
    Device,
    Volume,
    Size,
    Type,
    Transport,
    Flags,
    Model,
    Phase,
    Filesystem,
    Encryption,
    Mount,
}

#[derive(Debug)]
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
        matches!(self.field, Field::Model | Field::Mount)
    }

    /// The glyph and the device or volume stay in view when the table
    /// scrolls sideways.
    fn pinned(&self) -> bool {
        matches!(self.field, Field::Glyph | Field::Device | Field::Volume)
    }
}

fn column(field: Field, label: &str, width: f32) -> Column {
    Column {
        field,
        label: label.to_owned().into(),
        width,
    }
}

/// One side's rows and the columns sized to them.
#[derive(Debug)]
pub(crate) struct Listing<R> {
    pub(super) rows: Vec<R>,
    columns: Vec<Column>,
    width: f32,
}

impl<R> Listing<R> {
    fn new(rows: Vec<R>, columns: Vec<Column>) -> Self {
        let width = columns.iter().map(|column| column.width).sum();
        Self {
            rows,
            columns,
            width,
        }
    }
}

/// A disk and what its row shows.
#[derive(Debug)]
pub(crate) struct DiskRow {
    pub(super) id: SharedString,
    pub(super) info: DiskInfo,
    /// The row's element id, `disk-<id>`.
    element: SharedString,
    label: SharedString,
    device: SharedString,
    size: SharedString,
    kind: SharedString,
    transport: SharedString,
    flags: SharedString,
    model: SharedString,
    /// A real disk that can't be written: the only disk that shows a glyph.
    /// A disk has no state to call good, so the others show none.
    warn: bool,
}

impl DiskRow {
    fn new(info: DiskInfo) -> Self {
        let kind = disk_type(&info);
        let size = format_bytes(info.size);
        let label = format!(
            "{} · {size} · {kind}{}",
            info.dev_path,
            if info.readonly { " · read-only" } else { "" }
        );
        Self {
            id: info.id.clone().into(),
            element: format!("disk-{}", info.id).into(),
            label: label.into(),
            device: info.dev_path.clone().into(),
            size: size.into(),
            kind: kind.into(),
            transport: info.transport.clone().unwrap_or_default().into(),
            flags: flags(&info).into(),
            model: info.model.clone().unwrap_or_default().into(),
            warn: unexpected_read_only(&info),
            info,
        }
    }
}

/// A volume and what its row shows.
#[derive(Debug)]
pub(crate) struct VolumeRow {
    pub(super) id: SharedString,
    pub(super) info: VolumeStatus,
    /// The row's element id, `volume-<id>`.
    element: SharedString,
    label: SharedString,
    size: SharedString,
    phase: SharedString,
    filesystem: SharedString,
    encryption: SharedString,
    mount: SharedString,
    tone: Tone,
}

impl VolumeRow {
    fn new(info: VolumeStatus) -> Self {
        let label = format!(
            "{} · {} · {} · encryption {}",
            info.id,
            info.size,
            info.phase,
            encryption(&info)
        );
        Self {
            id: info.id.clone().into(),
            element: format!("volume-{}", info.id).into(),
            label: label.into(),
            size: info.size.clone().into(),
            phase: info.phase.clone().into(),
            filesystem: info.filesystem.clone().unwrap_or_default().into(),
            encryption: encryption(&info).to_owned().into(),
            mount: info.mount_location.clone().unwrap_or_default().into(),
            tone: phase_tone(&info.phase),
            info,
        }
    }
}

pub(super) fn disk_listing(disks: Vec<DiskInfo>) -> Listing<DiskRow> {
    let rows: Vec<DiskRow> = disks.into_iter().map(DiskRow::new).collect();
    let fits = |field, label: &str, text: fn(&DiskRow) -> &SharedString| {
        column(field, label, fit(label, rows.iter().map(text), WIDEST))
    };
    let columns = vec![
        column(Field::Glyph, "", table::GLYPH_WIDTH),
        fits(Field::Device, "Device", |row| &row.device),
        fits(Field::Size, "Size", |row| &row.size),
        fits(Field::Type, "Type", |row| &row.kind),
        fits(Field::Transport, "Transport", |row| &row.transport),
        fits(Field::Flags, "Flags", |row| &row.flags),
        column(
            Field::Model,
            "Model",
            fit("Model", rows.iter().map(|row| &row.model), WIDEST_FLEXIBLE),
        ),
    ];
    Listing::new(rows, columns)
}

pub(super) fn volume_listing(volumes: Vec<VolumeStatus>) -> Listing<VolumeRow> {
    let rows: Vec<VolumeRow> = volumes.into_iter().map(VolumeRow::new).collect();
    let fits = |field, label: &str, text: fn(&VolumeRow) -> &SharedString| {
        column(field, label, fit(label, rows.iter().map(text), WIDEST))
    };
    let columns = vec![
        column(Field::Glyph, "", table::GLYPH_WIDTH),
        fits(Field::Volume, "Volume", |row| &row.id),
        fits(Field::Size, "Size", |row| &row.size),
        fits(Field::Phase, "Phase", |row| &row.phase),
        fits(Field::Filesystem, "Filesystem", |row| &row.filesystem),
        fits(Field::Encryption, "Encryption", |row| &row.encryption),
        column(
            Field::Mount,
            "Mount",
            fit("Mount", rows.iter().map(|row| &row.mount), WIDEST_FLEXIBLE),
        ),
    ];
    Listing::new(rows, columns)
}

/// What a side's table says when it has no rows: why it is unknown, or that
/// the node reported none.
pub(super) fn no_rows<T>(side: &Result<Vec<T>, String>, what: &str) -> Option<SharedString> {
    match side {
        Err(error) => Some(format!("Unknown: {error}").into()),
        Ok(rows) if rows.is_empty() => Some(format!("This node didn't report any {what}.").into()),
        Ok(_) => None,
    }
}

/// A row of either table, borrowed for the frame.
pub(crate) enum RowRef<'a> {
    Disk(&'a DiskRow),
    Volume(&'a VolumeRow),
}

impl StorageScreen {
    fn disk_side(&self) -> Option<&Listing<DiskRow>> {
        self.loader.data()?.disks.as_ref().ok()
    }

    fn volume_side(&self) -> Option<&Listing<VolumeRow>> {
        self.loader.data()?.volumes.as_ref().ok()
    }

    fn disk_cell(
        &self,
        disk: &DiskRow,
        style: &RowStyle,
        column: &Column,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let cell = table::cell(column);
        let text = match column.field {
            Field::Glyph => {
                return table::glyph_cell(column)
                    .when(disk.warn, |this| {
                        this.child(ui::status_mark(
                            SharedString::from(format!("{}-state", disk.element)),
                            Tone::Warn,
                            "Read-only, though not a loop device or an optical drive",
                            cx,
                        ))
                    })
                    .into_any_element();
            }
            Field::Device => &disk.device,
            Field::Size => &disk.size,
            Field::Type => &disk.kind,
            Field::Transport => &disk.transport,
            Field::Flags => {
                return cell
                    .when(disk.warn && !style.selected, |this| {
                        this.text_color(style.p.warn_ink)
                    })
                    .child(disk.flags.clone())
                    .into_any_element();
            }
            Field::Model => &disk.model,
            _ => return cell.into_any_element(),
        };
        cell.child(text.clone()).into_any_element()
    }

    fn volume_cell(
        &self,
        volume: &VolumeRow,
        style: &RowStyle,
        column: &Column,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let cell = table::cell(column);
        let text = match column.field {
            Field::Glyph => {
                return table::glyph_cell(column)
                    .child(ui::status_mark(
                        SharedString::from(format!("{}-state", volume.element)),
                        volume.tone,
                        format!("Phase: {}", volume.phase),
                        cx,
                    ))
                    .into_any_element();
            }
            Field::Volume => &volume.id,
            Field::Size => &volume.size,
            Field::Phase => {
                let ink = match volume.tone {
                    Tone::Good => style.p.good_ink,
                    Tone::Crit => style.p.crit_ink,
                    _ => style.p.warn_ink,
                };
                return cell
                    .when(!style.selected, |this| this.text_color(ink))
                    .child(volume.phase.clone())
                    .into_any_element();
            }
            Field::Filesystem => &volume.filesystem,
            Field::Encryption => &volume.encryption,
            Field::Mount => &volume.mount,
            _ => return cell.into_any_element(),
        };
        cell.child(text.clone()).into_any_element()
    }
}

impl TableSource for StorageScreen {
    type Key = SharedString;
    type Sort = ();
    type Column = Column;
    type Row<'a> = RowRef<'a>;

    fn table_state(&self) -> &TableState {
        match self.mode {
            ViewMode::Disks => &self.disk_table,
            ViewMode::Volumes => &self.volume_table,
        }
    }

    fn columns(&self) -> &[Column] {
        let columns = match self.mode {
            ViewMode::Disks => self.disk_side().map(|side| &side.columns),
            ViewMode::Volumes => self.volume_side().map(|side| &side.columns),
        };
        match columns {
            Some(columns) => columns.as_slice(),
            None => &[],
        }
    }

    fn width(&self) -> f32 {
        match self.mode {
            ViewMode::Disks => self.disk_side().map_or(0., |side| side.width),
            ViewMode::Volumes => self.volume_side().map_or(0., |side| side.width),
        }
    }

    fn list_label(&self) -> String {
        match self.mode {
            ViewMode::Disks => "Disks on the target node; arrows select, Tab switches to volumes",
            ViewMode::Volumes => "Volumes on the target node; arrows select, Tab switches to disks",
        }
        .into()
    }

    fn sorting(&self, _: &Column) -> Option<((), Option<SortOrder>)> {
        None
    }

    fn sort(&mut self, _: (), _: &mut Context<Self>) {}

    fn line_count(&self) -> usize {
        match self.mode {
            ViewMode::Disks => self.disks().len(),
            ViewMode::Volumes => self.volumes().len(),
        }
    }

    fn line(&self, line: usize, _: &App) -> Option<Line<SharedString, RowRef<'_>>> {
        let (key, element, label, data) = match self.mode {
            ViewMode::Disks => {
                let row = self.disks().get(line)?;
                (&row.id, &row.element, &row.label, RowRef::Disk(row))
            }
            ViewMode::Volumes => {
                let row = self.volumes().get(line)?;
                (&row.id, &row.element, &row.label, RowRef::Volume(row))
            }
        };
        Some(Line::Row(TableRow {
            key: key.clone(),
            id: element.clone().into(),
            label: label.clone(),
            tooltip: None,
            marked: false,
            muted: false,
            data,
        }))
    }

    fn cell(
        &self,
        row: &TableRow<SharedString, RowRef<'_>>,
        style: &RowStyle,
        column: &Column,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        match row.data {
            RowRef::Disk(disk) => self.disk_cell(disk, style, column, cx),
            RowRef::Volume(volume) => self.volume_cell(volume, style, column, cx),
        }
    }

    fn group(&self, _: usize, _: &mut Context<Self>) -> Option<AnyElement> {
        None
    }

    fn selected_key(&self) -> Option<&SharedString> {
        match self.mode {
            ViewMode::Disks => self.disk_key(),
            ViewMode::Volumes => self.volume_key(),
        }
    }

    fn line_of(&self, key: &SharedString) -> Option<usize> {
        match self.mode {
            ViewMode::Disks => self.disks().iter().position(|disk| &disk.id == key),
            ViewMode::Volumes => self.volumes().iter().position(|volume| &volume.id == key),
        }
    }

    fn click(
        &mut self,
        key: &SharedString,
        _: &ClickEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.choose(key.clone());
        window.focus(&self.focus, cx);
        cx.notify();
    }

    /// Says whether the showing side failed or the node reported none.
    fn empty(&self, _: &mut Context<Self>) -> Option<AnyElement> {
        let data = self.loader.data()?;
        let message = match self.mode {
            ViewMode::Disks => &data.no_disks,
            ViewMode::Volumes => &data.no_volumes,
        };
        message.clone().map(IntoElement::into_any_element)
    }
}
