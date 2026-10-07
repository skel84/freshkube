use std::{rc::Rc, time::Instant};

use gpui_kit::{
    AvailableSpace, Context, Pixels, ScrollStrategy, Size, Window, component::ActiveTheme, point,
    prelude::*, px, size,
};

use super::{
    LogSource, LogView, MeanHeight, MeasurementKey, REMEASURE_BUDGET, RESIZE_SETTLE,
    RowMeasurement,
    review::{VisibleDelta, shown_message},
};

impl<S: LogSource> LogView<S> {
    pub(super) fn measure_rows(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let key = MeasurementKey {
            width: self.width.unwrap_or_else(|| window.bounds().size.width),
            rem: window.rem_size(),
            font: cx.theme().mono_font_family.clone(),
            wrapped: self.wrapped,
            columns: self.columns,
            revision: self.review.revision,
        };
        let unchanged = self.measured.as_ref() == Some(&key);
        if unchanged && !self.row_exact.contains(&false) {
            return;
        }
        let mut estimated = false;
        if unchanged {
            // Only estimated rows are left: a scroll may have exposed some,
            // or the resize settled. Anchor on what is on screen now.
            if self.pending_reveal.is_none() {
                self.capture_anchor();
            }
        } else {
            estimated = self.rebuild_sizes(&key, window, cx);
        }
        let mut changed = !unchanged;
        changed |= self.measure_viewport(&key, window, cx);
        if estimated && self.row_exact.contains(&false) {
            // Lines arrive faster than frames show them: measure the rest
            // once they stop.
            self.defer_settling(cx);
        }
        if self.settled {
            changed |= self.remeasure_estimates(&key, window, cx);
            if self.row_exact.contains(&false) {
                window.request_animation_frame();
            }
        }
        let width = key.width;
        self.measured = Some(key);
        if !changed {
            return;
        }
        self.unwrapped_width = self
            .row_widths
            .iter()
            .fold(width, |widest, width| widest.max(*width));
        if !self.following && self.pending_reveal.is_none() {
            self.restore_anchor();
        }
        if self.following {
            self.pending_reveal = self.last_row_id();
        }
    }

    /// Rebuilds `sizes` for a new revision or geometry. Rows already
    /// measured at another wrap width keep that height as an estimate, and
    /// rows never measured get the mean height. Returns whether any row
    /// was never measured.
    fn rebuild_sizes(
        &mut self,
        key: &MeasurementKey,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let previous = self.measured.as_ref();
        let measurements_stale = previous.is_none_or(|previous| {
            previous.rem != key.rem
                || previous.font != key.font
                || previous.wrapped != key.wrapped
                || previous.columns != key.columns
        });
        let resized = previous.is_some_and(|previous| previous.width != key.width);
        if measurements_stale {
            self.row_measurements.clear();
        } else if resized && key.wrapped {
            // A live resize changes the width every frame; reshaping every
            // retained row each time is what made it sluggish.
            self.defer_settling(cx);
        }
        if resized || measurements_stale {
            self.mean_height = MeanHeight::default();
        }
        let delta = self.review.take_delta();
        // Rows that left the front and ones that joined the end are the
        // only changes between batches; the sizes in between still hold.
        let rows = self.review.visible.len();
        let kept = match delta {
            VisibleDelta::Extended { dropped, added }
                if !measurements_stale
                    && dropped <= self.sizes.len()
                    && self.sizes.len() - dropped + added == rows
                    && self.row_widths.len() == self.sizes.len()
                    && self.row_exact.len() == self.sizes.len() =>
            {
                Some((dropped, rows - added))
            }
            _ => None,
        };
        let first_new = match kept {
            Some((dropped, first_new)) => {
                let sizes = Rc::make_mut(&mut self.sizes);
                for row in sizes.drain(..dropped) {
                    self.sizes_height -= f64::from(f32::from(row.height));
                }
                self.row_widths.drain(..dropped);
                self.row_exact.drain(..dropped);
                if resized {
                    for row in sizes.iter_mut() {
                        row.width = key.width;
                    }
                    // Unwrapped rows keep their natural width at any pane width.
                    if key.wrapped {
                        self.row_exact.fill(false);
                    }
                }
                first_new
            }
            None => {
                self.sizes = Rc::new(Vec::with_capacity(rows));
                self.sizes_height = 0.;
                self.row_widths.clear();
                self.row_exact.clear();
                0
            }
        };
        let wrap_width = key.wrapped.then_some(key.width);
        // A row never measured is estimated until a frame shows it, so a
        // flood of lines costs layout only for the rows on screen.
        let estimate = match self.mean_height.get() {
            Some(height) => height,
            None if first_new < rows => self.measure_row(rows - 1, key, window, cx).height,
            None => px(0.),
        };
        let mut estimated = false;
        for ix in first_new..rows {
            let (measured, exact) = match self.row_measurements.get(&self.review.id(ix)) {
                Some(cached) => (cached.size, cached.wrap_width == wrap_width),
                None => {
                    estimated = true;
                    (size(px(0.), estimate), false)
                }
            };
            self.row_widths.push(measured.width);
            self.row_exact.push(exact);
            Rc::make_mut(&mut self.sizes).push(size(key.width, measured.height));
            self.sizes_height += f64::from(f32::from(measured.height));
        }
        estimated
    }

    /// Holds off measuring rows off screen until nothing has changed the
    /// rows' geometry or added rows for `RESIZE_SETTLE`.
    fn defer_settling(&mut self, cx: &mut Context<Self>) {
        self.settled = false;
        self.settle = Some(cx.spawn(async move |weak, cx| {
            cx.background_executor().timer(RESIZE_SETTLE).await;
            let _ = weak.update(cx, |this, cx| {
                this.settled = true;
                cx.notify();
            });
        }));
    }

    /// Brings row `ix` into view by its middle. A search's match taller than
    /// the list instead shows its matched line in the list's middle, so a
    /// long stack trace doesn't hide the line the search found, whichever
    /// of its lines that is.
    pub(super) fn reveal_row(&mut self, ix: usize, window: &mut Window, cx: &mut Context<Self>) {
        // A matched line is found within the row's measured height, never
        // an estimate. `measure_viewport` settles the revealed row first,
        // so this lays nothing out unless that changes.
        if self.reveal_matched_line.is_some()
            && self.row_exact.get(ix) == Some(&false)
            && let Some(key) = self.measured.clone()
        {
            self.settle_row(ix, &key, window, cx);
        }
        let list = self.scroll.bounds().size.height;
        let tall = self.sizes.get(ix).is_some_and(|row| row.height > list);
        if let Some(line) = self.reveal_matched_line
            && tall
            && list > px(0.)
            && let Some(line_end) = self.matched_line_end(ix, line, window, cx)
        {
            let before: Pixels = self.sizes[..ix].iter().map(|row| row.height).sum();
            let top = (before + line_end - list / 2.).max(px(0.));
            self.scroll.set_offset(point(self.scroll.offset().x, -top));
            #[cfg(test)]
            {
                self.revealed_line = Some((self.review.id(ix), line, line_end));
            }
        } else {
            self.scroll.scroll_to_item(ix, ScrollStrategy::Center);
        }
    }

    /// How far down row `ix` its message's line `line` ends, counting lines
    /// split after each newline, measured by laying the row out again with
    /// its message cut after that line, so it wraps as the row does. `None`
    /// when the row has no such line.
    fn matched_line_end(
        &mut self,
        ix: usize,
        line: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<Pixels> {
        let key = self.measured.clone()?;
        let message = shown_message(self.review.entry(ix));
        let mut lines = message.split_inclusive('\n');
        let before: usize = lines.by_ref().take(line).map(str::len).sum();
        let end = before + lines.next()?.len();
        freshkube_probe::probe::hit("logs.reveal_line");
        let shown = message[..end].trim_end_matches('\n').to_owned();
        let mut row = (self.render_row_showing(ix, true, Some(&shown), cx)).into_any_element();
        Some(row.layout_as_root(available(&key), window, cx).height)
    }

    /// Lays out row `ix` as the list draws it and caches its size.
    /// VirtualList trusts supplied heights, so this measures the actual
    /// styled row, not character counts.
    fn measure_row(
        &mut self,
        ix: usize,
        key: &MeasurementKey,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Size<Pixels> {
        freshkube_probe::probe::hit("logs.measure");
        let _span = freshkube_probe::perf::span("logs.measure_row");
        let mut row = self.render_row(ix, true, cx).into_any_element();
        let measured = row.layout_as_root(available(key), window, cx);
        self.mean_height.add(measured.height);
        self.row_measurements.insert(
            self.review.id(ix),
            RowMeasurement {
                wrap_width: key.wrapped.then_some(key.width),
                size: measured,
            },
        );
        measured
    }

    /// Replaces row `ix`'s estimated height with its measured one and
    /// returns whether the height changed.
    fn settle_row(
        &mut self,
        ix: usize,
        key: &MeasurementKey,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        if self.row_exact[ix] {
            return false;
        }
        let wrap_width = key.wrapped.then_some(key.width);
        let measured = match self.row_measurements.get(&self.review.id(ix)) {
            Some(cached) if cached.wrap_width == wrap_width => cached.size,
            _ => self.measure_row(ix, key, window, cx),
        };
        self.row_exact[ix] = true;
        self.row_widths[ix] = measured.width;
        let row = &mut Rc::make_mut(&mut self.sizes)[ix];
        let changed = row.height != measured.height;
        self.sizes_height +=
            f64::from(f32::from(measured.height)) - f64::from(f32::from(row.height));
        row.height = measured.height;
        changed
    }

    /// Settles every estimated row the next frame can show, so neither a
    /// live resize nor a scroll paints an estimated height.
    fn measure_viewport(
        &mut self,
        key: &MeasurementKey,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let rows = self.sizes.len();
        if rows == 0 || !self.row_exact.contains(&false) {
            return false;
        }
        // Where the frame will scroll to: the tail, a revealed row, the
        // review anchor (row 0 once evicted), or the current offset.
        let (first, within) = if self.following {
            (rows - 1, px(0.))
        } else if let Some(ix) = self
            .pending_reveal
            .and_then(|id| self.review.row_for_id(id))
        {
            (ix, px(0.))
        } else if let Some(anchor) = self.review_anchor {
            self.review
                .row_for_id(anchor.id)
                .map_or((0, px(0.)), |ix| (ix, anchor.within_row))
        } else {
            let mut offset = -self.scroll.offset().y;
            let mut ix = 0;
            while ix + 1 < rows && offset >= self.sizes[ix].height {
                offset -= self.sizes[ix].height;
                ix += 1;
            }
            (ix, offset.max(px(0.)))
        };
        // The whole panel bounds the viewport, whatever chrome is open;
        // measure that much on both sides of where the frame lands.
        let span = self
            .panel_height
            .unwrap_or_else(|| window.bounds().size.height);
        let mut changed = false;
        let mut covered = -within;
        let mut ix = first;
        while ix < rows && covered < span {
            changed |= self.settle_row(ix, key, window, cx);
            covered += self.sizes[ix].height;
            ix += 1;
        }
        let mut covered = px(0.);
        let mut ix = first;
        while ix > 0 && covered < span {
            ix -= 1;
            changed |= self.settle_row(ix, key, window, cx);
            covered += self.sizes[ix].height;
        }
        changed
    }

    /// Settles estimated rows off screen within a frame budget, once the
    /// width has held.
    fn remeasure_estimates(
        &mut self,
        key: &MeasurementKey,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let started = Instant::now();
        let mut changed = false;
        for ix in 0..self.sizes.len() {
            if self.row_exact[ix] {
                continue;
            }
            changed |= self.settle_row(ix, key, window, cx);
            if started.elapsed() >= REMEASURE_BUDGET {
                break;
            }
        }
        changed
    }
}

/// The room a row is laid out in: the list's width when wrapped, its own
/// width otherwise.
fn available(key: &MeasurementKey) -> Size<AvailableSpace> {
    size(
        if key.wrapped {
            AvailableSpace::Definite(key.width)
        } else {
            AvailableSpace::MaxContent
        },
        AvailableSpace::MinContent,
    )
}
