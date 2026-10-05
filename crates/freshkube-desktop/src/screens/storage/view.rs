//! How Storage draws: the summary, the Disks / Volumes toolbar, the rows
//! and the selection's details.
use super::*;
use table::DataTable;

impl StorageScreen {
    /// One line above the tabs: counts and anything that needs a look.
    fn summary(&self, data: &StorageData, cx: &App) -> impl IntoElement {
        let disks = match &data.disks {
            Ok(disks) => format!(
                "{} · {}",
                disks.rows.len(),
                format_bytes(disks.rows.iter().map(|disk| disk.info.size).sum())
            ),
            Err(_) => "unknown".into(),
        };
        let volumes = match &data.volumes {
            Ok(volumes) => volumes.rows.len().to_string(),
            Err(_) => "unknown".into(),
        };
        let not_ready = match &data.volumes {
            Ok(volumes) => volumes
                .rows
                .iter()
                .filter(|volume| volume.info.phase != "ready")
                .count()
                .to_string(),
            Err(_) => "unknown".into(),
        };
        h_flex()
            .id("storage-summary")
            .gap_3()
            .flex_wrap()
            .child(stat("Disks", disks, cx))
            .child(stat("Volumes", volumes, cx))
            .child(stat("Not ready", not_ready, cx))
    }

    fn toolbar(&self, data: &StorageData, cx: &mut Context<Self>) -> Div {
        let mode = self.mode;
        let count = |len: Option<usize>| len.map_or("?".to_owned(), |len| len.to_string());
        let disks = count(data.disks.as_ref().ok().map(|disks| disks.rows.len()));
        let volumes = count(data.volumes.as_ref().ok().map(|volumes| volumes.rows.len()));
        h_flex().gap_2p5().flex_wrap().child(
            ButtonGroup::new("storage-view")
                .outline()
                .small()
                .child(
                    Button::new("storage-view-disks")
                        .icon(IconName::HardDrive)
                        .label(format!("Disks {disks}"))
                        .selected(mode == ViewMode::Disks),
                )
                .child(
                    Button::new("storage-view-volumes")
                        .icon(IconName::Database)
                        .label(format!("Volumes {volumes}"))
                        .selected(mode == ViewMode::Volumes),
                )
                .on_click(cx.listener(|view, selected: &Vec<usize>, _, cx| {
                    let mode = match selected.first() {
                        Some(1) => ViewMode::Volumes,
                        _ => ViewMode::Disks,
                    };
                    view.switch(mode, cx);
                })),
        )
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
        if let Some(page) = gated_page_mode(
            "storage-page",
            "Storage",
            Scope::Node,
            self.source.as_ref(),
            &self.loader,
            "disks and volumes",
            self.embedded,
            cx,
        ) {
            return page;
        }
        let (Some(source), Some(data)) = (self.source.clone(), self.loader.data()) else {
            return div().into_any_element();
        };
        let mode = self.mode;
        let mut missing = Vec::new();
        if let Err(error) = &data.disks {
            missing.push(format!("Disks: {error}"));
        }
        if let Err(error) = &data.volumes {
            missing.push(format!("Volumes: {error}"));
        }
        // The keys live on a wrapper drawn in every state, so the page keeps
        // Tab and the arrows while a side shows no rows.
        let list = div()
            .id("storage-table")
            .key_context(CONTEXT)
            .track_focus(&self.focus)
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
            .flex()
            .flex_col()
            .flex_1()
            .min_h(dp(LIST_MIN_HEIGHT))
            .child(
                DataTable::new()
                    .carded()
                    .render(self, window, cx)
                    .flex_1()
                    .min_h_0(),
            );
        let details = match mode {
            ViewMode::Disks => self.disk_details(cx),
            ViewMode::Volumes => self.volume_details(cx),
        };
        let wide = content_width(window) >= SIDE_DETAILS;
        // Short windows scroll the page rather than squeezing the list.
        let split = if wide {
            h_flex()
                .flex_1()
                .min_h(dp(LIST_MIN_HEIGHT))
                .items_stretch()
                .gap(dp(14.))
                .child(v_flex().flex_1().min_w_0().min_h_0().child(list))
                .child(
                    div()
                        .id("storage-details")
                        .w(dp(340.))
                        .flex_none()
                        .overflow_y_scroll()
                        .child(details),
                )
        } else {
            h_flex()
                .flex_1()
                .min_h(dp(LIST_MIN_HEIGHT + 14. + DETAILS_HEIGHT))
                .child(
                    v_flex().size_full().gap(dp(14.)).child(list).child(
                        div()
                            .id("storage-details")
                            .h(dp(DETAILS_HEIGHT))
                            .flex_none()
                            .overflow_y_scroll()
                            .child(details),
                    ),
                )
        };
        v_flex()
            .id("storage-page")
            .size_full()
            .min_h_0()
            .overflow_y_scroll()
            .px(dp(crate::desktop::PAGE_PADDING))
            .pt(dp(22.))
            .pb(dp(18.))
            .gap(dp(14.))
            .child(header_mode(
                "Storage",
                &source,
                Scope::Node,
                &self.loader,
                self.embedded,
                cx,
            ))
            .children(failure_banner(&self.loader, cx))
            .children(partial_notice(missing, cx))
            .child(self.summary(data, cx))
            .child(self.toolbar(data, cx))
            .child(split)
            .into_any_element()
    }
}
