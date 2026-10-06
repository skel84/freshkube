//! How Storage draws: the header with its Disks / Volumes segment, the rows
//! and the selection's details.
use std::rc::Rc;

use super::*;
use freshkube_ui::page::{self, PageHeader};
use table::DataTable;

impl StorageScreen {
    /// The toolbar: the title, the Disks / Volumes segment, which folds into
    /// checked items, and Refresh; the counts go in the meta line.
    fn render_header(&self, window: &mut Window, cx: &mut Context<Self>) -> Div {
        let header = PageHeader::new(PREFIX, "Storage");
        let data = self.loader.data();
        let header = match data {
            Some(data) => {
                header.foldable(self.render_segment(data, cx), self.segment_items(data, cx))
            }
            None => header,
        };
        let refresh = refresh_control(
            header.id("refresh"),
            "Refresh storage",
            self.source.as_ref(),
            &self.loader,
            cx,
        );
        let parts = data.map(|data| data.summary.clone()).unwrap_or_default();
        header
            .control(refresh)
            .meta(meta(
                self.source.as_ref(),
                Scope::Node,
                &self.loader,
                self.embedded,
                parts,
            ))
            .render(window, cx)
    }

    fn render_segment(&self, data: &StorageData, cx: &mut Context<Self>) -> ButtonGroup {
        let mode = self.mode;
        ButtonGroup::new("storage-view")
            .outline()
            .small()
            .child(
                Button::new("storage-view-disks")
                    .h(dp(ui::CONTROL_HEIGHT))
                    .icon(IconName::HardDrive)
                    .label(data.disks_label.clone())
                    .selected(mode == ViewMode::Disks),
            )
            .child(
                Button::new("storage-view-volumes")
                    .h(dp(ui::CONTROL_HEIGHT))
                    .icon(IconName::Database)
                    .label(data.volumes_label.clone())
                    .selected(mode == ViewMode::Volumes),
            )
            .on_click(cx.listener(|view, selected: &Vec<usize>, _, cx| {
                let mode = match selected.first() {
                    Some(1) => ViewMode::Volumes,
                    _ => ViewMode::Disks,
                };
                view.switch(mode, cx);
            }))
    }

    /// The segment folded: a checked item for each side.
    fn segment_items(&self, data: &StorageData, cx: &mut Context<Self>) -> page::MenuItems {
        let disks = page::checked_item(
            data.disks_label.clone(),
            self.mode == ViewMode::Disks,
            page::handler(cx, |view: &mut Self, _, cx| {
                view.switch(ViewMode::Disks, cx)
            }),
        );
        let volumes = page::checked_item(
            data.volumes_label.clone(),
            self.mode == ViewMode::Volumes,
            page::handler(cx, |view: &mut Self, _, cx| {
                view.switch(ViewMode::Volumes, cx)
            }),
        );
        Rc::new(move |menu, window, cx| {
            let menu = disks(menu, window, cx);
            volumes(menu, window, cx)
        })
    }

    /// The showing side's table, with the selection's details beside it on
    /// a wide page and below it on a narrow one.
    fn render_split(&self, window: &Window, cx: &mut Context<Self>) -> AnyElement {
        let beside = crate::screens::beside(window, self.embedded);
        let table = div()
            .id("storage-table")
            .w_full()
            .child(
                DataTable::new()
                    .fit(table::TableSource::line_count(self).max(1))
                    .render(self, window, cx)
                    .w_full()
                    .flex_none(),
            )
            .into_any_element();
        let details = match self.mode {
            ViewMode::Disks => self.disk_details(cx),
            ViewMode::Volumes => self.volume_details(cx),
        };
        let details = div()
            .id("storage-details")
            .when_else(
                beside,
                |this| this.pr(dp(page::PANE_PADDING)).py(dp(page::PANE_PADDING_Y)),
                |this| this.px(dp(page::PANE_PADDING)).pb(dp(page::PANE_PADDING_Y)),
            )
            .child(details)
            .into_any_element();
        crate::screens::split_narrow("storage-split", beside, table, Some(details))
    }

    fn disk_details(&self, cx: &App) -> AnyElement {
        let p = palette(cx);
        let Some(disk) = self.selected_disk_row().map(|row| &row.info) else {
            return panel(cx)
                .p_4()
                .text_color(p.muted)
                .text_size(dp(12.5))
                .child("No disk selected.")
                .into_any_element();
        };
        let unknown = || div().text_color(p.muted).child("not reported");
        let optional = |value: &Option<String>| match value {
            Some(value) if !value.is_empty() => mono(value.clone()).into_any_element(),
            _ => unknown().into_any_element(),
        };
        panel(cx)
            .id("disk-details")
            .test_support()
            .aria_label(format!("Details of {}", disk.dev_path))
            .p_4()
            .gap_2p5()
            .child(
                h_flex()
                    .gap_2()
                    .flex_wrap()
                    .child(
                        div()
                            .font_family(MONO_FONT)
                            .text_size(dp(14.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .truncate()
                            .child(disk.dev_path.clone()),
                    )
                    .child(ui::tag(Tone::Outline, None, disk_type(disk), cx))
                    .when(disk.readonly, |this| {
                        let tone = if unexpected_read_only(disk) {
                            Tone::Warn
                        } else {
                            Tone::Outline
                        };
                        this.child(ui::tag(tone, Some(IconName::Lock), "Read-only", cx))
                    }),
            )
            .child(field("Disk ID", mono(disk.id.clone()), cx))
            .child(field(
                "Size",
                // One unit everywhere: talosctl's own pretty size is
                // decimal and would disagree with the totals.
                mono(format!("{} ({} bytes)", format_bytes(disk.size), disk.size)),
                cx,
            ))
            .child(field("Model", optional(&disk.model), cx))
            .child(field("Serial", optional(&disk.serial), cx))
            .child(field("Transport", optional(&disk.transport), cx))
            .child(field("WWID", optional(&disk.wwid), cx))
            .child(field("Bus path", optional(&disk.bus_path), cx))
            .into_any_element()
    }

    fn volume_details(&self, cx: &App) -> AnyElement {
        let p = palette(cx);
        let Some(volume) = self.selected_volume_row().map(|row| &row.info) else {
            return panel(cx)
                .p_4()
                .text_color(p.muted)
                .text_size(dp(12.5))
                .child("No volume selected.")
                .into_any_element();
        };
        let unknown = || div().text_color(p.muted).child("not reported");
        let optional = |value: &Option<String>| match value {
            Some(value) if !value.is_empty() => mono(value.clone()).into_any_element(),
            _ => unknown().into_any_element(),
        };
        let encrypted = volume.encryption_provider.is_some();
        panel(cx)
            .id("volume-details")
            .test_support()
            .aria_label(format!("Details of volume {}", volume.id))
            .p_4()
            .gap_2p5()
            .child(
                h_flex()
                    .gap_2()
                    .flex_wrap()
                    .child(
                        div()
                            .font_family(MONO_FONT)
                            .text_size(dp(14.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .truncate()
                            .child(volume.id.clone()),
                    )
                    .child(ui::tag(
                        phase_tone(&volume.phase),
                        None,
                        volume.phase.clone(),
                        cx,
                    )),
            )
            .child(field("Size", mono(volume.size.clone()), cx))
            .child(field("Filesystem", optional(&volume.filesystem), cx))
            .child(field("Mount", optional(&volume.mount_location), cx))
            .child(field(
                "Encryption",
                h_flex()
                    .gap_2()
                    .child(mono(encryption(volume).to_owned()))
                    .when(encrypted, |this| {
                        this.child(ui::tag(
                            Tone::Outline,
                            Some(IconName::Lock),
                            "Encrypted",
                            cx,
                        ))
                    }),
                cx,
            ))
            .into_any_element()
    }
}

impl Render for StorageScreen {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let header = self.render_header(window, cx);
        let state = gate(
            self.source.as_ref(),
            &self.loader,
            Scope::Node,
            "disks and volumes",
            cx,
        );
        // The table runs edge to edge under the toolbar; the banners and a
        // state in the table's place sit in an inset between them.
        let page = page::page("storage-page")
            .h_auto()
            .flex_none()
            .child(page::toolbar(cx).child(header));
        let page = match (state, self.loader.data()) {
            (Some(state), _) => page.child(
                page::inset()
                    .id("storage-state")
                    .test_support()
                    .child(state),
            ),
            (None, Some(data)) => {
                let mut missing = Vec::new();
                if let Err(error) = &data.disks {
                    missing.push(format!("Disks: {error}"));
                }
                if let Err(error) = &data.volumes {
                    missing.push(format!("Volumes: {error}"));
                }
                let banners: Vec<AnyElement> = failure_banner(&self.loader, cx)
                    .map(IntoElement::into_any_element)
                    .into_iter()
                    .chain(partial_notice(missing, cx))
                    .collect();
                page.when(!banners.is_empty(), |page| {
                    page.child(
                        page::inset()
                            .flex()
                            .flex_col()
                            .gap(dp(page::PANE_PADDING_Y))
                            .children(banners),
                    )
                })
                .child(self.render_split(window, cx))
            }
            (None, None) => page,
        };
        // The keys live on a wrapper drawn in every state, so the page keeps
        // Tab and the arrows while a side shows no rows or a state shows.
        div()
            .id("storage-scroll")
            .key_context(CONTEXT)
            .track_focus(&self.focus)
            .size_full()
            .min_h_0()
            .overflow_y_scroll()
            .restrict_scroll_to_axis()
            .on_action(cx.listener(|view, _: &NextRow, _, cx| view.step(1, cx)))
            .on_action(cx.listener(|view, _: &PreviousRow, _, cx| view.step(-1, cx)))
            .on_action(cx.listener(|view, _: &FirstRow, _, cx| view.step(isize::MIN, cx)))
            .on_action(cx.listener(|view, _: &LastRow, _, cx| view.step(isize::MAX, cx)))
            .on_action(cx.listener(|view, _: &NextPage, _, cx| view.step(PAGE_ROWS, cx)))
            .on_action(cx.listener(|view, _: &PreviousPage, _, cx| view.step(-PAGE_ROWS, cx)))
            .on_action(cx.listener(|view, _: &SwitchView, _, cx| {
                let next = match view.mode {
                    ViewMode::Disks => ViewMode::Volumes,
                    ViewMode::Volumes => ViewMode::Disks,
                };
                view.switch(next, cx);
            }))
            .child(page)
    }
}
