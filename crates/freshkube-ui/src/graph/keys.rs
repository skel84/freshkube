//! Walking a graph with the keyboard: where each arrow goes from a box or a
//! call. It is a pure function of the placed boxes and the calls between
//! them, so the walk is tested without a window.
//!
//! Up and down visit the boxes in column order, left to right and top to
//! bottom. Left walks toward the origin: from a box to the call into it,
//! and from a call to the box it comes from. Right walks the other way: from
//! a box to the first call out of it, and from a call to the box it ends at.

use std::collections::BTreeMap;

/// Where the keyboard is: on a box, or on the call between two boxes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Cursor<K> {
    Node(K),
    Link(K, K),
}

/// One arrow key.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Step {
    /// Left: toward the origin.
    Back,
    /// Right: away from it.
    Forward,
    /// Up: the previous box in column order.
    Previous,
    /// Down: the next box in column order.
    Next,
}

/// The boxes as drawn, `(x, y)` by key, and the calls as `(from, to)` in the
/// source's order.
pub(super) struct Walk<'a, K> {
    pub nodes: &'a [(K, f32, f32)],
    pub calls: &'a [(K, K)],
}

impl<K: Clone + Ord> Walk<'_, K> {
    /// The cursor after `step` from `from`, or `None` where the key goes
    /// nowhere. With no cursor yet, any step lands on the first box.
    pub(super) fn step(&self, from: Option<&Cursor<K>>, step: Step) -> Option<Cursor<K>> {
        let order = self.order();
        let Some(from) = from else {
            return order.first().cloned().map(Cursor::Node);
        };
        match (from, step) {
            (Cursor::Node(key), Step::Previous | Step::Next) => {
                let at = order.iter().position(|k| k == key)?;
                let to = if step == Step::Next {
                    at.checked_add(1)?
                } else {
                    at.checked_sub(1)?
                };
                order.get(to).cloned().map(Cursor::Node)
            }
            // Up and down on a call move to the box it ends at, then walk on.
            (Cursor::Link(_, to), Step::Previous | Step::Next) => {
                self.step(Some(&Cursor::Node(to.clone())), step)
            }
            (Cursor::Node(key), Step::Back) => self
                .calls
                .iter()
                .find(|(_, to)| to == key)
                .map(|(from, to)| Cursor::Link(from.clone(), to.clone())),
            (Cursor::Node(key), Step::Forward) => self
                .calls
                .iter()
                .find(|(from, _)| from == key)
                .map(|(from, to)| Cursor::Link(from.clone(), to.clone())),
            (Cursor::Link(from, _), Step::Back) => Some(Cursor::Node(from.clone())),
            (Cursor::Link(_, to), Step::Forward) => Some(Cursor::Node(to.clone())),
        }
    }

    /// The boxes left to right, then top to bottom; boxes in one column
    /// share an `x`.
    fn order(&self) -> Vec<K> {
        let mut by_place: BTreeMap<(i64, i64, &K), ()> = BTreeMap::new();
        for (key, x, y) in self.nodes {
            by_place.insert((place(*x), place(*y), key), ());
        }
        by_place
            .into_keys()
            .map(|(_, _, key)| key.clone())
            .collect()
    }
}

/// The left edge a sideways scroll should have so the span `left..right`
/// is in view, given the current left edge `at` and the view's `width`,
/// all in dp, with `margin` beside the span. It never scrolls before the
/// start, and leaves the scroll alone when the span is already in view.
pub(super) fn scroll_to_show(at: f32, width: f32, left: f32, right: f32, margin: f32) -> f32 {
    let to = if left - margin < at {
        left - margin
    } else if right + margin > at + width {
        right + margin - width
    } else {
        at
    };
    to.max(0.)
}

/// A coordinate as a whole number of hundredths, so equal columns compare
/// equal and the order is total.
fn place(value: f32) -> i64 {
    (value * 100.).round() as i64
}

#[cfg(test)]
mod tests {
    use super::*;

    use Cursor::{Link, Node};
    use Step::{Back, Forward, Next, Previous};

    /// Boxes by key and corner, and calls by their ends.
    type Fixture = (
        Vec<(&'static str, f32, f32)>,
        Vec<(&'static str, &'static str)>,
    );

    /// A chain a → b → c → d, with a side branch `gap` hung from `b` in
    /// the column of `c`, below it.
    fn chain() -> Fixture {
        (
            vec![
                ("a", 0., 0.),
                ("b", 100., 0.),
                ("c", 200., 0.),
                ("gap", 200., 80.),
                ("d", 300., 0.),
            ],
            vec![("a", "b"), ("b", "c"), ("b", "gap"), ("c", "d")],
        )
    }

    /// Two Stages that run one commit: `freight` feeds both, which sit in
    /// one column.
    fn two_stages() -> Fixture {
        (
            vec![
                ("commit", 0., 0.),
                ("freight", 100., 0.),
                ("dev", 200., 0.),
                ("prod", 200., 80.),
            ],
            vec![
                ("commit", "freight"),
                ("freight", "dev"),
                ("freight", "prod"),
            ],
        )
    }

    fn walk<'a>(
        nodes: &'a [(&'static str, f32, f32)],
        calls: &'a [(&'static str, &'static str)],
    ) -> Walk<'a, &'static str> {
        Walk { nodes, calls }
    }

    #[test]
    fn up_and_down_visit_the_boxes_in_column_order_side_branch_included() {
        let (nodes, calls) = chain();
        let w = walk(&nodes, &calls);
        let mut at = None;
        let mut seen = vec![];
        while let Some(next) = w.step(at.as_ref(), Next) {
            if let Node(key) = &next {
                seen.push(*key);
            }
            at = Some(next);
        }
        assert_eq!(seen, ["a", "b", "c", "gap", "d"]);
        // And back up again.
        let mut back = vec![];
        while let Some(prev) = w.step(at.as_ref(), Previous) {
            if let Node(key) = &prev {
                back.push(*key);
            }
            at = Some(prev);
        }
        assert_eq!(back, ["gap", "c", "b", "a"]);
    }

    #[test]
    fn the_ends_of_the_column_order_go_nowhere() {
        let (nodes, calls) = chain();
        let w = walk(&nodes, &calls);
        assert_eq!(w.step(Some(&Node("a")), Previous), None);
        assert_eq!(w.step(Some(&Node("d")), Next), None);
    }

    #[test]
    fn with_no_cursor_a_key_lands_on_the_first_box() {
        let (nodes, calls) = chain();
        for step in [Back, Forward, Previous, Next] {
            assert_eq!(walk(&nodes, &calls).step(None, step), Some(Node("a")));
        }
        let none: [(&str, f32, f32); 0] = [];
        assert_eq!(walk(&none, &[]).step(None, Next), None);
    }

    #[test]
    fn left_walks_toward_the_origin_and_right_away_from_it() {
        let (nodes, calls) = chain();
        let w = walk(&nodes, &calls);
        // From c: the call into it, then the box it comes from.
        let into_c = w.step(Some(&Node("c")), Back).unwrap();
        assert_eq!(into_c, Link("b", "c"));
        assert_eq!(w.step(Some(&into_c), Back), Some(Node("b")));
        // Right goes the other way: b's first call out, then the box it ends at.
        let out_of_b = w.step(Some(&Node("b")), Forward).unwrap();
        assert_eq!(out_of_b, Link("b", "c"));
        assert_eq!(w.step(Some(&out_of_b), Forward), Some(Node("c")));
        // The origin has no call into it; the last box none out.
        assert_eq!(w.step(Some(&Node("a")), Back), None);
        assert_eq!(w.step(Some(&Node("d")), Forward), None);
    }

    #[test]
    fn a_side_branch_is_reached_by_its_link_and_its_box_by_down() {
        let (nodes, calls) = chain();
        let w = walk(&nodes, &calls);
        // Into the branch: the call from b, which is not b's first call out.
        assert_eq!(w.step(Some(&Node("gap")), Back), Some(Link("b", "gap")));
        // Down from c lands on the branch; right from it goes nowhere.
        assert_eq!(w.step(Some(&Node("c")), Next), Some(Node("gap")));
        assert_eq!(w.step(Some(&Node("gap")), Forward), None);
    }

    #[test]
    fn two_stages_at_one_commit_each_have_their_own_link_in() {
        let (nodes, calls) = two_stages();
        let w = walk(&nodes, &calls);
        assert_eq!(
            w.step(Some(&Node("dev")), Back),
            Some(Link("freight", "dev"))
        );
        assert_eq!(
            w.step(Some(&Node("prod")), Back),
            Some(Link("freight", "prod"))
        );
        // Right from the freight takes the first call; the other stage is
        // one step down from the first.
        assert_eq!(
            w.step(Some(&Node("freight")), Forward),
            Some(Link("freight", "dev"))
        );
        assert_eq!(w.step(Some(&Node("dev")), Next), Some(Node("prod")));
        // The commit is drawn once: one call out of it.
        assert_eq!(
            w.step(Some(&Node("commit")), Forward),
            Some(Link("commit", "freight"))
        );
    }

    #[test]
    fn scrolling_to_a_span_moves_only_as_far_as_needed() {
        // Already in view: no move.
        assert_eq!(scroll_to_show(100., 400., 150., 300., 24.), 100.);
        // Past the right edge: its right edge, plus the margin, at the view's.
        assert_eq!(scroll_to_show(0., 400., 500., 648., 24.), 272.);
        // Before the left edge: the span's left, less the margin.
        assert_eq!(scroll_to_show(300., 400., 200., 348., 24.), 176.);
        // Never before the start.
        assert_eq!(scroll_to_show(50., 400., 10., 158., 24.), 0.);
    }

    #[test]
    fn up_and_down_from_a_call_walk_on_from_the_box_it_ends_at() {
        let (nodes, calls) = chain();
        let w = walk(&nodes, &calls);
        assert_eq!(w.step(Some(&Link("b", "c")), Next), Some(Node("gap")));
        assert_eq!(w.step(Some(&Link("b", "c")), Previous), Some(Node("b")));
    }
}
