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
