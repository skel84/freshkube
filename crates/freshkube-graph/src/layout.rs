//! Places a graph's boxes in columns, callers left of what they call, and
//! draws each connection as a curve between them. Positions are in dp at
//! the default text size; nothing here knows GPUI or where the graph came
//! from.

// The layout's grid, in dp at the default text size.
pub const NODE_W: f32 = 148.;
pub const NODE_H: f32 = 54.;
pub const COLUMN: f32 = NODE_W + 40.;
pub const ROW: f32 = NODE_H + 22.;
pub const PAD: f32 = 16.;
pub const ARROW: f32 = 7.;
/// A column taller than this wraps into another beside it.
pub const MAX_ROWS: usize = 8;

/// A box the layout places, by its top-left corner in dp.
pub trait Node {
    fn position(&self) -> (f32, f32);
    fn set_position(&mut self, x: f32, y: f32);
}

/// Places callers left of what they call: each node's column is the length
/// of the longest chain of callers above it, tall columns wrap, and each
/// column is ordered by where its callers sit to keep crossings down. Nodes
/// with no connection share a last column. Returns the map's size in dp.
pub fn layered<N: Node>(nodes: &mut [N], links: &[(usize, usize)]) -> (f32, f32) {
    let n = nodes.len();
    if n == 0 {
        return (0., 0.);
    }
    let mut layer = vec![0usize; n];
    // Longest path by relaxation; a cycle stops changing after n passes.
    for _ in 0..n {
        let mut changed = false;
        for &(from, to) in links {
            if layer[to] < layer[from] + 1 && layer[from] + 1 < n {
                layer[to] = layer[from] + 1;
                changed = true;
            }
        }
        if !changed {
            break;
        }
    }
    let linked: Vec<bool> = (0..n)
        .map(|ix| links.iter().any(|&(a, b)| a == ix || b == ix))
        .collect();
    let last = layer
        .iter()
        .zip(&linked)
        .filter(|(_, l)| **l)
        .map(|(l, _)| *l)
        .max();
    let mut columns: Vec<Vec<usize>> = vec![vec![]; last.map_or(0, |l| l + 1)];
    let mut loose = vec![];
    for ix in 0..n {
        if linked[ix] {
            columns[layer[ix]].push(ix);
        } else {
            loose.push(ix);
        }
    }
    let mut row = vec![0f32; n];
    for column in &mut columns {
        let key = |ix: usize| {
            let callers: Vec<f32> = links
                .iter()
                .filter(|&&(from, to)| to == ix && layer[from] < layer[ix])
                .map(|&(from, _)| row[from])
                .collect();
            if callers.is_empty() {
                f32::MAX
            } else {
                callers.iter().sum::<f32>() / callers.len() as f32
            }
        };
        column.sort_by(|&a, &b| key(a).total_cmp(&key(b)).then(a.cmp(&b)));
        for (ix, &node) in column.iter().enumerate() {
            row[node] = ix as f32;
        }
    }
    if !loose.is_empty() {
        columns.push(loose);
    }
    let (mut x, mut rows) = (0, 0);
    for column in columns {
        for chunk in column.chunks(MAX_ROWS) {
            for (r, &ix) in chunk.iter().enumerate() {
                nodes[ix].set_position(PAD + x as f32 * COLUMN, PAD + r as f32 * ROW);
            }
            rows = rows.max(chunk.len());
            x += 1;
        }
    }
    (
        2. * PAD + x as f32 * COLUMN - (COLUMN - NODE_W),
        2. * PAD + rows as f32 * ROW - (ROW - NODE_H),
    )
}

/// A connection's curve in dp: from the caller's right edge to just short of
/// the callee's left edge, where the arrowhead takes over.
pub fn curve(from: &impl Node, to: &impl Node) -> [(f32, f32); 4] {
    let ((from_x, from_y), (to_x, to_y)) = (from.position(), to.position());
    let a = (from_x + NODE_W, from_y + NODE_H / 2.);
    let d = (to_x - ARROW, to_y + NODE_H / 2.);
    let pull = ((d.0 - a.0).abs() / 2.).max(48.);
    [a, (a.0 + pull, a.1), (d.0 - pull, d.1), d]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Clone, Copy, Debug, Default, PartialEq)]
    struct Box(f32, f32);

    impl Node for Box {
        fn position(&self) -> (f32, f32) {
            (self.0, self.1)
        }

        fn set_position(&mut self, x: f32, y: f32) {
            (self.0, self.1) = (x, y);
        }
    }

    fn place(count: usize, links: &[(usize, usize)]) -> (Vec<Box>, (f32, f32)) {
        let mut nodes = vec![Box::default(); count];
        let size = layered(&mut nodes, links);
        (nodes, size)
    }

    /// The column a box sits in.
    fn column(node: Box) -> usize {
        ((node.0 - PAD) / COLUMN).round() as usize
    }

    #[test]
    fn nothing_takes_no_room() {
        assert_eq!(place(0, &[]).1, (0., 0.));
    }

    #[test]
    fn one_box_is_padded_on_every_side() {
        let (nodes, size) = place(1, &[]);
        assert_eq!(nodes[0], Box(PAD, PAD));
        assert_eq!(size, (NODE_W + 2. * PAD, NODE_H + 2. * PAD));
    }

    #[test]
    fn callers_sit_left_of_what_they_call() {
        // 0 → 1 → 2, and 0 → 2 directly: 2 goes by its longest chain.
        let (nodes, (width, _)) = place(3, &[(0, 1), (1, 2), (0, 2)]);
        assert_eq!(
            nodes.iter().map(|n| column(*n)).collect::<Vec<_>>(),
            [0, 1, 2]
        );
        assert_eq!(width, 2. * PAD + 3. * COLUMN - (COLUMN - NODE_W));
    }

    #[test]
    fn unconnected_boxes_share_a_last_column() {
        let (nodes, _) = place(4, &[(0, 1)]);
        assert_eq!(column(nodes[2]), 2);
        assert_eq!(column(nodes[3]), 2);
        assert_ne!(nodes[2].1, nodes[3].1);
    }

    #[test]
    fn a_tall_column_wraps_into_the_next() {
        // One caller of ten: eight callees, then two beside them.
        let links: Vec<_> = (1..=10).map(|to| (0, to)).collect();
        let (nodes, (_, height)) = place(11, &links);
        let columns: Vec<_> = nodes[1..].iter().map(|n| column(*n)).collect();
        assert_eq!(columns.iter().filter(|&&c| c == 1).count(), MAX_ROWS);
        assert_eq!(columns.iter().filter(|&&c| c == 2).count(), 2);
        assert_eq!(height, 2. * PAD + MAX_ROWS as f32 * ROW - (ROW - NODE_H));
    }

    #[test]
    fn callees_follow_their_callers_rows() {
        // Callers 0 and 1; 0 calls 3 and 1 calls 2, so 3 sits above 2.
        let (nodes, _) = place(4, &[(0, 3), (1, 2)]);
        assert!(nodes[0].1 < nodes[1].1);
        assert!(nodes[3].1 < nodes[2].1);
    }

    #[test]
    fn a_cycle_still_places_every_box() {
        let (nodes, (width, height)) = place(3, &[(0, 1), (1, 2), (2, 0)]);
        assert!(width.is_finite() && height.is_finite());
        for node in &nodes {
            assert!(node.0 >= PAD && node.0 + NODE_W <= width - PAD + 0.01);
            assert!(node.1 >= PAD && node.1 + NODE_H <= height - PAD + 0.01);
        }
        // No two boxes share a place.
        for (i, a) in nodes.iter().enumerate() {
            assert!(nodes[i + 1..].iter().all(|b| a != b));
        }
    }

    #[test]
    fn a_curve_runs_from_the_right_edge_to_the_arrowhead() {
        let (from, to) = (Box(PAD, PAD), Box(PAD + COLUMN, PAD + ROW));
        let [a, b, c, d] = curve(&from, &to);
        assert_eq!(a, (PAD + NODE_W, PAD + NODE_H / 2.));
        assert_eq!(d, (PAD + COLUMN - ARROW, PAD + ROW + NODE_H / 2.));
        // The handles leave and arrive level, at least 48 dp out.
        assert_eq!((b.1, c.1), (a.1, d.1));
        assert!(b.0 - a.0 >= 48. && d.0 - c.0 >= 48.);
    }
}
