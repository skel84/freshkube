//! Where each panel and row header sits, in dp from the grid's top: the
//! dashboard's 24 columns, or one column under the narrow breakpoint.
//! Derived when the dashboard opens or a row folds, never while drawing.
use freshkube_core::monitoring::model::{layout, schema::GridPos};

/// Content narrower than this, in dp, stacks the panels in one column.
pub(super) const NARROW: f32 = 720.;
/// One grid row with its gap, in dp.
pub(super) const UNIT: f32 = layout::ROW_HEIGHT + layout::GAP;
/// A row header's height, in dp.
pub(super) const ROW_HEADER: f32 = 36.;
/// The fewest grid rows a stacked panel takes.
const NARROW_MIN_ROWS: u32 = 4;
/// Panels this many columns wide or narrower pair up when stacked.
const NARROW_PAIR_COLUMNS: u32 = 8;
/// The fewest grid rows a paired panel takes.
const NARROW_PAIR_MIN_ROWS: u32 = 3;

/// One section of the dashboard as the layout needs it.
pub(super) struct SectionShape {
    pub(super) has_header: bool,
    pub(super) collapsed: bool,
    /// Grid rows the section spans when open.
    pub(super) height: u32,
    /// Its panels by slot, with their places in the section.
    pub(super) panels: Vec<(usize, GridPos)>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct Placement {
    pub(super) slot: usize,
    /// From the left and wide, as fractions of the grid's width.
    pub(super) left: f32,
    pub(super) width: f32,
    pub(super) top: f32,
    pub(super) height: f32,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub(super) struct Layout {
    pub(super) panels: Vec<Placement>,
    /// Each row header's top, by section.
    pub(super) rows: Vec<(usize, f32)>,
    pub(super) height: f32,
}

impl Placement {
    /// Some part of the panel lies within `top..bottom`.
    pub(super) fn within(&self, top: f32, bottom: f32) -> bool {
        self.top < bottom && self.top + self.height > top
    }
}

impl Layout {
    /// The panels any part of which lies within `top..bottom`.
    pub(super) fn within(&self, top: f32, bottom: f32) -> impl Iterator<Item = usize> + '_ {
        self.panels
            .iter()
            .filter(move |place| place.within(top, bottom))
            .map(|place| place.slot)
    }
}

pub(super) fn layout(sections: &[SectionShape], narrow: bool) -> Layout {
    crate::probe::hit("monitoring-derive");
    let columns = layout::COLUMNS as f32;
    let mut out = Layout::default();
    let mut top = 0.;
    for (index, section) in sections.iter().enumerate() {
        if section.has_header {
            out.rows.push((index, top));
            top += ROW_HEADER;
        }
        if section.collapsed {
            continue;
        }
        if narrow {
            let mut panels = section.panels.clone();
            panels.sort_by_key(|(_, pos)| (pos.y, pos.x));
            let small = |pos: &GridPos| pos.w <= NARROW_PAIR_COLUMNS;
            let mut rest = panels.as_slice();
            while let Some(((slot, pos), after)) = rest.split_first() {
                // Two small panels in a row share it, as two stats would.
                if let Some(((partner, other), after)) = after.split_first()
                    && small(pos)
                    && small(other)
                {
                    let height = pos.h.max(other.h).max(NARROW_PAIR_MIN_ROWS) as f32 * UNIT;
                    for (index, slot) in [*slot, *partner].into_iter().enumerate() {
                        out.panels.push(Placement {
                            slot,
                            left: index as f32 * 0.5,
                            width: 0.5,
                            top,
                            height,
                        });
                    }
                    top += height;
                    rest = after;
                    continue;
                }
                let least = if small(pos) {
                    NARROW_PAIR_MIN_ROWS
                } else {
                    NARROW_MIN_ROWS
                };
                let height = pos.h.max(least) as f32 * UNIT;
                out.panels.push(Placement {
                    slot: *slot,
                    left: 0.,
                    width: 1.,
                    top,
                    height,
                });
                top += height;
                rest = after;
            }
        } else {
            for &(slot, pos) in &section.panels {
                out.panels.push(Placement {
                    slot,
                    left: pos.x as f32 / columns,
                    width: pos.w as f32 / columns,
                    top: top + pos.y as f32 * UNIT,
                    height: pos.h as f32 * UNIT,
                });
            }
            top += section.height as f32 * UNIT;
        }
    }
    out.height = top;
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pos(x: u32, y: u32, w: u32, h: u32) -> GridPos {
        GridPos { x, y, w, h }
    }

    fn sections(collapsed: bool) -> Vec<SectionShape> {
        vec![
            SectionShape {
                has_header: false,
                collapsed: false,
                height: 3,
                panels: vec![(0, pos(0, 0, 12, 3)), (1, pos(12, 0, 12, 3))],
            },
            SectionShape {
                has_header: true,
                collapsed,
                height: 8,
                panels: vec![(3, pos(12, 0, 12, 8)), (2, pos(0, 0, 12, 8))],
            },
        ]
    }

    #[test]
    fn wide_follows_the_grid_and_a_folded_row_keeps_only_its_header() {
        let open = layout(&sections(false), false);
        assert_eq!(open.panels.len(), 4);
        assert_eq!(open.panels[1].left, 0.5);
        assert_eq!(open.panels[1].width, 0.5);
        assert_eq!(open.rows, vec![(1, 3. * UNIT)]);
        assert_eq!(open.panels[2].top, 3. * UNIT + ROW_HEADER);
        assert_eq!(open.height, 11. * UNIT + ROW_HEADER);
        let folded = layout(&sections(true), false);
        assert_eq!(folded.panels.len(), 2);
        assert_eq!(folded.height, 3. * UNIT + ROW_HEADER);
    }

    #[test]
    fn narrow_stacks_panels_in_reading_order_with_a_minimum_height() {
        let stacked = layout(&sections(false), true);
        let order: Vec<usize> = stacked.panels.iter().map(|place| place.slot).collect();
        assert_eq!(order, [0, 1, 2, 3]);
        assert!(stacked.panels.iter().all(|place| place.width == 1.));
        assert_eq!(stacked.panels[0].height, 4. * UNIT);
        assert_eq!(stacked.panels[1].top, 4. * UNIT);
        let near: Vec<usize> = stacked.within(0., 5. * UNIT).collect();
        assert_eq!(near, [0, 1]);
    }

    #[test]
    fn narrow_pairs_small_panels_and_a_lone_one_takes_the_row() {
        let stats = [SectionShape {
            has_header: false,
            collapsed: false,
            height: 4,
            panels: (0..3)
                .map(|slot| (slot, pos(slot as u32 * 6, 0, 6, 4)))
                .collect(),
        }];
        let stacked = layout(&stats, true);
        let places: Vec<(usize, f32, f32, f32)> = stacked
            .panels
            .iter()
            .map(|place| (place.slot, place.left, place.width, place.top))
            .collect();
        assert_eq!(
            places,
            [(0, 0., 0.5, 0.), (1, 0.5, 0.5, 0.), (2, 0., 1., 4. * UNIT)]
        );
        assert_eq!(stacked.height, 8. * UNIT);
    }
}
