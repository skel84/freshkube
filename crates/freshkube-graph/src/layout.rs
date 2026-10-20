//! Places a graph's boxes in columns, callers left of what they call, and
//! routes each connection around them. Positions are in dp at
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

/// Space between two lines that share a gutter.
pub const LANE: f32 = 6.;

/// The sizes [`route_with`] lays a graph out by, in dp at the default text
/// size.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Metrics {
    pub node_w: f32,
    pub node_h: f32,
    /// From a box's left edge to the next column's.
    pub column: f32,
    /// From a box's top to the next box's in a column.
    pub row: f32,
    /// A column taller than this wraps into another beside it.
    pub max_rows: usize,
}

impl Metrics {
    /// The service map's: what [`route`] uses.
    pub const MAP: Self = Self {
        node_w: NODE_W,
        node_h: NODE_H,
        column: COLUMN,
        row: ROW,
        max_rows: MAX_ROWS,
    };

    /// The map's boxes with a wider gutter between columns, for lines that
    /// carry a label, and columns that never wrap.
    pub const fn labelled(gutter: f32) -> Self {
        Self {
            column: NODE_W + gutter,
            max_rows: usize::MAX,
            ..Self::MAP
        }
    }

    /// The gap between two boxes in a column.
    fn gutter(&self) -> f32 {
        self.row - self.node_h
    }
}

/// Where [`route`] put the boxes, and the line each call follows.
#[derive(Clone, Debug, PartialEq)]
pub struct Layout {
    pub width: f32,
    pub height: f32,
    /// One per link, in the links' order.
    pub routes: Vec<Route>,
}

/// A call's line in dp: cubic segments end to end, then an arrowhead from
/// the last segment's end to `tip`.
#[derive(Clone, Debug, PartialEq)]
pub struct Route {
    pub segments: Vec<[(f32, f32); 4]>,
    pub tip: (f32, f32),
    /// Halfway along the route, where a marker can sit.
    pub mid: (f32, f32),
}

impl Route {
    fn new(segments: Vec<[(f32, f32); 4]>, tip: (f32, f32)) -> Self {
        let mid = point_at(&segments[segments.len() / 2], 0.5);
        Self { segments, tip, mid }
    }

    /// The arrowhead: its tip, then the two corners of its base.
    pub fn head(&self) -> [(f32, f32); 3] {
        let end = self.segments.last().map_or(self.tip, |segment| segment[3]);
        let (dx, dy) = (self.tip.0 - end.0, self.tip.1 - end.1);
        let length = dx.hypot(dy).max(f32::EPSILON);
        let (nx, ny) = (-dy / length * ARROW * 0.6, dx / length * ARROW * 0.6);
        [self.tip, (end.0 + nx, end.1 + ny), (end.0 - nx, end.1 - ny)]
    }
}

/// Places callers left of what they call and routes every call around the
/// boxes. Returns the map's size and each call's route.
///
/// Each box's column is the length of the longest chain of callers above
/// it. Boxes with no call share the last columns. A call that closes a cycle is turned back for layering, so columns only
/// see forward calls. Tall columns wrap into ordinary columns. A call that
/// spans several columns crosses each one through the gutter between two
/// boxes, in a lane of its own; a crowded gutter grows by what its lanes
/// need. One pass down and one back up orders each column by its
/// neighbours to cut crossings. A call that closes a cycle leaves its
/// source at the bottom, runs left through the gutters and enters its
/// target from below.
///
/// The same input always gives the same layout. It is not free: run it
/// when the graph changes, never while drawing.
pub fn route(nodes: &mut [(f32, f32)], links: &[(usize, usize)]) -> Layout {
    route_with(&Metrics::MAP, nodes, links)
}

/// [`route`] with other sizes: `route(..)` is `route_with(&Metrics::MAP, ..)`.
pub fn route_with(m: &Metrics, nodes: &mut [(f32, f32)], links: &[(usize, usize)]) -> Layout {
    let n = nodes.len();
    if n == 0 {
        return Layout {
            width: 0.,
            height: 0.,
            routes: vec![],
        };
    }
    let back = back_links(n, links);
    // Each call left to right; a self-call has no direction.
    let forward: Vec<Option<(usize, usize)>> = links
        .iter()
        .zip(&back)
        .map(|(&(from, to), &back)| match (from == to, back) {
            (true, _) => None,
            (false, true) => Some((to, from)),
            (false, false) => Some((from, to)),
        })
        .collect();
    let layer = longest_path(n, &forward);
    let mut grid = Grid::columns(*m, n, links, &forward, &layer);
    grid.order();
    grid.add_ports(n, links, &back);
    let height = grid.place(nodes);
    let columns = grid.columns.len();
    let routes = links
        .iter()
        .enumerate()
        .map(|(ix, &(from, to))| {
            if back[ix] {
                grid.back_route(nodes, ix, from, to)
            } else {
                grid.forward_route(nodes, ix, from, to)
            }
        })
        .collect();
    // Boxes with no calls share the last columns.
    let mut rows = 0;
    for (c, chunk) in grid.loose.chunks(m.max_rows).enumerate() {
        for (r, &ix) in chunk.iter().enumerate() {
            nodes[ix] = (
                PAD + (columns + c) as f32 * m.column,
                PAD + r as f32 * m.row,
            );
        }
        rows = rows.max(chunk.len());
    }
    let loose = grid.loose.len().div_ceil(m.max_rows);
    let bottom = if rows > 0 {
        PAD + rows as f32 * m.row - m.gutter()
    } else {
        0.
    };
    Layout {
        width: 2. * PAD + (columns + loose) as f32 * m.column - (m.column - m.node_w),
        height: height.max(bottom) + PAD,
        routes,
    }
}

/// The calls a depth-first walk finds closing a cycle, self-calls among
/// them. Turning them round leaves the graph without a cycle.
fn back_links(n: usize, links: &[(usize, usize)]) -> Vec<bool> {
    #[derive(Clone, Copy, PartialEq)]
    enum Mark {
        New,
        Open,
        Done,
    }
    let mut out = vec![vec![]; n];
    for (ix, &(from, _)) in links.iter().enumerate() {
        out[from].push(ix);
    }
    let mut mark = vec![Mark::New; n];
    let mut back = vec![false; links.len()];
    for start in 0..n {
        if mark[start] != Mark::New {
            continue;
        }
        mark[start] = Mark::Open;
        let mut stack = vec![(start, 0)];
        while let Some(top) = stack.last_mut() {
            let node = top.0;
            match out[node].get(top.1).copied() {
                Some(link) => {
                    top.1 += 1;
                    let to = links[link].1;
                    match mark[to] {
                        Mark::Open => back[link] = true,
                        Mark::New => {
                            mark[to] = Mark::Open;
                            stack.push((to, 0));
                        }
                        Mark::Done => {}
                    }
                }
                None => {
                    mark[node] = Mark::Done;
                    stack.pop();
                }
            }
        }
    }
    back
}

/// Each node's column: the length of the longest chain of callers above it.
fn longest_path(n: usize, forward: &[Option<(usize, usize)>]) -> Vec<usize> {
    let mut waiting = vec![0usize; n];
    let mut out = vec![vec![]; n];
    for &(from, to) in forward.iter().flatten() {
        waiting[to] += 1;
        out[from].push(to);
    }
    let mut layer = vec![0; n];
    let mut ready: Vec<usize> = (0..n).filter(|&ix| waiting[ix] == 0).collect();
    while let Some(from) = ready.pop() {
        for &to in &out[from] {
            layer[to] = layer[to].max(layer[from] + 1);
            waiting[to] -= 1;
            if waiting[to] == 0 {
                ready.push(to);
            }
        }
    }
    layer
}

/// The columns while [`route`] works on them. An item is a box (below the
/// node count), a waypoint where a long call crosses a column, or a port
/// where a call that closes a cycle leaves or enters a box.
struct Grid {
    m: Metrics,
    /// Items top to bottom, column by column.
    columns: Vec<Vec<usize>>,
    /// Each item's neighbours in the column to its left and to its right.
    left: Vec<Vec<usize>>,
    right: Vec<Vec<usize>>,
    /// Each call's waypoints, left to right.
    chains: Vec<Vec<usize>>,
    /// The ports below each box, and each call's ports: where it leaves
    /// and where it enters.
    below: Vec<Vec<usize>>,
    ports: Vec<(usize, usize)>,
    /// Boxes with no calls.
    loose: Vec<usize>,
    /// Each item's column and its top (a box) or line (anything else).
    x: Vec<f32>,
    y: Vec<f32>,
}

impl Grid {
    /// Boxes in layers, tall layers wrapped into columns, and a waypoint
    /// in every column a call crosses.
    fn columns(
        m: Metrics,
        n: usize,
        links: &[(usize, usize)],
        forward: &[Option<(usize, usize)>],
        layer: &[usize],
    ) -> Self {
        let mut linked = vec![false; n];
        let mut callers = vec![vec![]; n];
        for &(from, to) in links {
            linked[from] = true;
            linked[to] = true;
        }
        for &(from, to) in forward.iter().flatten() {
            callers[to].push(from);
        }
        let depth = (0..n).filter(|&ix| linked[ix]).map(|ix| layer[ix]).max();
        let mut layers = vec![vec![]; depth.map_or(0, |d| d + 1)];
        let mut loose = vec![];
        for ix in 0..n {
            if linked[ix] {
                layers[layer[ix]].push(ix);
            } else {
                loose.push(ix);
            }
        }
        let mut row = vec![0f32; n];
        let mut columns = vec![];
        let mut column_of = vec![0; n];
        for nodes in &mut layers {
            let mut keyed: Vec<(f32, usize)> = nodes
                .iter()
                .map(|&ix| {
                    let rows = &callers[ix];
                    let key = if rows.is_empty() {
                        f32::MAX
                    } else {
                        rows.iter().map(|&c| row[c]).sum::<f32>() / rows.len() as f32
                    };
                    (key, ix)
                })
                .collect();
            keyed.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)));
            for (r, &(_, ix)) in keyed.iter().enumerate() {
                row[ix] = r as f32;
            }
            // A tall layer wraps into ordinary columns.
            for chunk in keyed.chunks(m.max_rows) {
                for &(_, ix) in chunk {
                    column_of[ix] = columns.len();
                }
                columns.push(chunk.iter().map(|&(_, ix)| ix).collect::<Vec<_>>());
            }
        }
        let mut grid = Self {
            m,
            columns,
            left: vec![vec![]; n],
            right: vec![vec![]; n],
            chains: vec![vec![]; links.len()],
            below: vec![vec![]; n],
            ports: vec![(0, 0); links.len()],
            loose,
            x: vec![],
            y: vec![],
        };
        for (ix, call) in forward.iter().enumerate() {
            let Some((from, to)) = *call else { continue };
            let mut previous = from;
            for column in column_of[from] + 1..column_of[to] {
                let waypoint = grid.item();
                grid.columns[column].push(waypoint);
                grid.chains[ix].push(waypoint);
                grid.right[previous].push(waypoint);
                grid.left[waypoint].push(previous);
                previous = waypoint;
            }
            grid.right[previous].push(to);
            grid.left[to].push(previous);
        }
        grid
    }

    fn item(&mut self) -> usize {
        self.left.push(vec![]);
        self.right.push(vec![]);
        self.left.len() - 1
    }

    /// One pass down and one back up, each column sorted by the mean place
    /// of its neighbours on the side just settled. An item with none there
    /// keeps its place; ties keep their order.
    fn order(&mut self) {
        let mut place = vec![0f32; self.left.len()];
        for column in &self.columns {
            for (p, &item) in column.iter().enumerate() {
                place[item] = p as f32;
            }
        }
        let count = self.columns.len();
        for c in 1..count {
            sort_column(&mut self.columns[c], &self.left, &mut place);
        }
        for c in (0..count.saturating_sub(1)).rev() {
            sort_column(&mut self.columns[c], &self.right, &mut place);
        }
    }

    /// A port below each end of a call that closes a cycle; a self-call has
    /// one for both.
    fn add_ports(&mut self, n: usize, links: &[(usize, usize)], back: &[bool]) {
        for (ix, &(from, to)) in links.iter().enumerate() {
            if !back[ix] {
                continue;
            }
            let leave = self.item();
            self.below[from].push(leave);
            let enter = if from == to {
                leave
            } else {
                let port = self.item();
                self.below[to].push(port);
                port
            };
            self.ports[ix] = (leave, enter);
        }
        // Ports sit first in the gutter below their box.
        for column in &mut self.columns {
            let mut items = Vec::with_capacity(column.len());
            for &item in column.iter() {
                items.push(item);
                if item < n {
                    items.extend(&self.below[item]);
                }
            }
            *column = items;
        }
    }

    /// Gives every box its place and every lane its line, and returns the
    /// bottom of the tallest column.
    fn place(&mut self, nodes: &mut [(f32, f32)]) -> f32 {
        let n = nodes.len();
        self.x = vec![0.; self.left.len()];
        self.y = vec![0.; self.left.len()];
        let mut bottom = 0f32;
        for (c, column) in self.columns.iter().enumerate() {
            let x = PAD + c as f32 * self.m.column;
            let (mut y, mut lanes, mut first) = (PAD, vec![], true);
            for &item in column {
                self.x[item] = x;
                if item < n {
                    y += lanes_in(&lanes, y, first, self.m.gutter(), &mut self.y);
                    lanes.clear();
                    nodes[item] = (x, y);
                    self.y[item] = y;
                    y += self.m.node_h;
                    first = false;
                } else {
                    lanes.push(item);
                }
            }
            y += lanes_in(&lanes, y, true, self.m.gutter(), &mut self.y);
            bottom = bottom.max(y);
        }
        bottom
    }

    /// From the caller's right edge, through a lane in each column it
    /// crosses, to the arrowhead at the callee's left edge.
    fn forward_route(&self, nodes: &[(f32, f32)], ix: usize, from: usize, to: usize) -> Route {
        let (from_x, from_y) = nodes[from];
        let mut at = (from_x + self.m.node_w, from_y + self.m.node_h / 2.);
        let mut segments = vec![];
        for &waypoint in &self.chains[ix] {
            // A lane starts where an arrowhead would, so the curves into a
            // column all span the same width and keep out of each other's way.
            let (x, y) = (self.x[waypoint], self.y[waypoint]);
            segments.push(bend(at, (x - ARROW, y)));
            segments.push(straight((x - ARROW, y), (x + self.m.node_w, y)));
            at = (x + self.m.node_w, y);
        }
        let (to_x, to_y) = nodes[to];
        let end = (to_x - ARROW, to_y + self.m.node_h / 2.);
        segments.push(bend(at, end));
        Route::new(segments, (to_x, end.1))
    }

    /// Down from the caller's bottom into its lane, left through the
    /// gutters, and up into the callee's bottom.
    fn back_route(&self, nodes: &[(f32, f32)], ix: usize, from: usize, to: usize) -> Route {
        let (leave, enter) = self.ports[ix];
        let ((from_x, from_y), (to_x, to_y)) = (nodes[from], nodes[to]);
        // Calls sharing a box leave and enter it side by side.
        let rank = |port: usize, of: usize| {
            self.below[of].iter().position(|&p| p == port).unwrap_or(0) as f32
        };
        let out_x =
            from_x + (self.m.node_w * 0.6 + rank(leave, from) * LANE).min(self.m.node_w - 8.);
        let in_x = to_x + (self.m.node_w * 0.4 - rank(enter, to) * LANE).max(8.);
        let mut at = (out_x, self.y[leave]);
        let mut segments = vec![straight((out_x, from_y + self.m.node_h), at)];
        if from != to {
            segments.push(straight(at, (from_x, at.1)));
            at = (from_x, at.1);
            for &waypoint in self.chains[ix].iter().rev() {
                let (x, y) = (self.x[waypoint], self.y[waypoint]);
                segments.push(bend(at, (x + self.m.node_w, y)));
                segments.push(straight((x + self.m.node_w, y), (x, y)));
                at = (x, y);
            }
            let lane = (to_x + self.m.node_w, self.y[enter]);
            segments.push(bend(at, lane));
            at = lane;
        }
        let under = (in_x, self.y[enter]);
        segments.push(straight(at, under));
        segments.push(straight(under, (in_x, to_y + self.m.node_h + ARROW)));
        Route::new(segments, (in_x, to_y + self.m.node_h))
    }
}

fn sort_column(column: &mut Vec<usize>, neighbours: &[Vec<usize>], place: &mut [f32]) {
    let mut keyed: Vec<(f32, usize)> = column
        .iter()
        .enumerate()
        .map(|(p, &item)| {
            let near = &neighbours[item];
            let key = if near.is_empty() {
                p as f32
            } else {
                near.iter().map(|&i| place[i]).sum::<f32>() / near.len() as f32
            };
            (key, item)
        })
        .collect();
    keyed.sort_by(|a, b| a.0.total_cmp(&b.0));
    column.clear();
    column.extend(keyed.iter().map(|&(_, item)| item));
    for (p, &item) in column.iter().enumerate() {
        place[item] = p as f32;
    }
}

/// Spreads a gutter's lanes evenly from `top` and returns its height: a
/// box's usual gutter, or more when its lanes need it. Above a column's
/// first box and below its last, an empty gutter takes no room.
fn lanes_in(lanes: &[usize], top: f32, edge: bool, gutter: f32, y: &mut [f32]) -> f32 {
    if lanes.is_empty() {
        return if edge { 0. } else { gutter };
    }
    let slots = (lanes.len() + 1) as f32;
    let height = gutter.max(slots * LANE);
    for (i, &lane) in lanes.iter().enumerate() {
        y[lane] = top + (i + 1) as f32 * height / slots;
    }
    height
}

/// A curve leaving and arriving level, for the space between two columns.
fn bend(from: (f32, f32), to: (f32, f32)) -> [(f32, f32); 4] {
    let pull = (to.0 - from.0) / 2.;
    [from, (from.0 + pull, from.1), (to.0 - pull, to.1), to]
}

fn straight(from: (f32, f32), to: (f32, f32)) -> [(f32, f32); 4] {
    let at = |t: f32| (from.0 + (to.0 - from.0) * t, from.1 + (to.1 - from.1) * t);
    [from, at(1. / 3.), at(2. / 3.), to]
}

/// The point at `t` along a cubic.
pub fn point_at(segment: &[(f32, f32); 4], t: f32) -> (f32, f32) {
    let u = 1. - t;
    let w = [u * u * u, 3. * u * u * t, 3. * u * t * t, t * t * t];
    let mut point = (0., 0.);
    for (weight, p) in w.iter().zip(segment) {
        point.0 += weight * p.0;
        point.1 += weight * p.1;
    }
    point
}

/// How many times the routes cross one another, each sampled into short
/// straight pieces. Lines that only touch, such as calls leaving the same
/// box, don't count. It compares every pair, so keep it to small graphs.
pub fn crossings(routes: &[Route]) -> usize {
    let lines: Vec<Vec<(f32, f32)>> = routes.iter().map(polyline).collect();
    let mut count = 0;
    for (i, a) in lines.iter().enumerate() {
        for b in &lines[i + 1..] {
            for pa in a.windows(2) {
                for pb in b.windows(2) {
                    if crosses(pa[0], pa[1], pb[0], pb[1]) {
                        count += 1;
                    }
                }
            }
        }
    }
    count
}

fn polyline(route: &Route) -> Vec<(f32, f32)> {
    const STEPS: usize = 12;
    let mut points = vec![route.segments[0][0]];
    for segment in &route.segments {
        points.extend((1..=STEPS).map(|s| point_at(segment, s as f32 / STEPS as f32)));
    }
    points
}

/// Whether two straight pieces cross at a point inside both.
fn crosses(p1: (f32, f32), p2: (f32, f32), q1: (f32, f32), q2: (f32, f32)) -> bool {
    let side = |a: (f32, f32), b: (f32, f32), c: (f32, f32)| {
        (b.0 - a.0) * (c.1 - a.1) - (b.1 - a.1) * (c.0 - a.0)
    };
    let apart = |s: f32, t: f32| (s > 1e-3 && t < -1e-3) || (s < -1e-3 && t > 1e-3);
    apart(side(q1, q2, p1), side(q1, q2, p2)) && apart(side(p1, p2, q1), side(p1, p2, q2))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A box's top-left corner, as `route` takes it.
    type Box = (f32, f32);

    fn place(count: usize, links: &[(usize, usize)]) -> (Vec<Box>, (f32, f32)) {
        let (nodes, layout) = routed(count, links);
        (nodes, (layout.width, layout.height))
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
        assert_eq!(nodes[0], (PAD, PAD));
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

    /// A named graph: its box count and its links between box indices.
    type Graph = (&'static str, usize, Vec<(usize, usize)>);

    /// The workbench story's shapes, and a self-call.
    fn shapes() -> Vec<Graph> {
        vec![
            (
                "shop",
                8,
                vec![
                    (0, 1),
                    (1, 2),
                    (1, 3),
                    (1, 4),
                    (2, 5),
                    (2, 6),
                    (5, 6),
                    (2, 7),
                ],
            ),
            ("fan-out", 13, (1..=12).map(|to| (0, to)).collect()),
            ("cycle", 3, vec![(0, 1), (1, 2), (2, 0)]),
            ("loose", 5, vec![(0, 1), (1, 2)]),
            ("self-call", 2, vec![(0, 1), (1, 1)]),
        ]
    }

    /// A made-up graph: mostly short calls forward, with calls that wrap
    /// round into cycles and a few self-calls.
    fn generated(boxes: usize, calls: usize) -> Vec<(usize, usize)> {
        let mut seed = 0x2545_f491_4f6c_dd1du64;
        let mut next = move |bound: usize| {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            (seed % bound as u64) as usize
        };
        (0..calls)
            .map(|_| {
                let from = next(boxes);
                let to = if next(50) == 0 {
                    from
                } else {
                    (from + 1 + next(40)) % boxes
                };
                (from, to)
            })
            .collect()
    }

    fn routed(count: usize, links: &[(usize, usize)]) -> (Vec<Box>, Layout) {
        let mut nodes = vec![Box::default(); count];
        let layout = route(&mut nodes, links);
        (nodes, layout)
    }

    /// The first point of a route found inside a box, if any.
    fn through_a_box(nodes: &[Box], route: &Route) -> Option<(f32, f32)> {
        route.segments.iter().find_map(|segment| {
            (0..=16)
                .map(|s| point_at(segment, s as f32 / 16.))
                .find(|&(x, y)| {
                    nodes.iter().any(|b| {
                        x > b.0 + 0.5
                            && x < b.0 + NODE_W - 0.5
                            && y > b.1 + 0.5
                            && y < b.1 + NODE_H - 0.5
                    })
                })
        })
    }

    #[test]
    fn no_route_runs_through_a_box() {
        let mut graphs = shapes();
        graphs.push(("generated", 120, generated(120, 300)));
        for (name, count, links) in graphs {
            let (nodes, layout) = routed(count, &links);
            assert_eq!(layout.routes.len(), links.len());
            for (route, link) in layout.routes.iter().zip(&links) {
                assert_eq!(through_a_box(&nodes, route), None, "{name}: {link:?}");
                assert!(route.segments.iter().flatten().all(|&(x, y)| {
                    x >= 0. && x <= layout.width && y >= 0. && y <= layout.height
                }));
            }
            // No two boxes share a place.
            for (i, a) in nodes.iter().enumerate() {
                assert!(nodes[i + 1..].iter().all(|b| a != b), "{name}");
            }
        }
    }

    #[test]
    fn routes_join_their_own_boxes() {
        for (name, count, links) in shapes() {
            let (nodes, layout) = routed(count, &links);
            for (route, &(from, to)) in layout.routes.iter().zip(&links) {
                let (start, (a, b)) = (route.segments[0][0], (nodes[from], nodes[to]));
                if route.tip.0 == b.0 {
                    // Right edge to left edge, the arrowhead pointing right.
                    assert_eq!(start, (a.0 + NODE_W, a.1 + NODE_H / 2.), "{name}");
                    assert_eq!(route.tip, (b.0, b.1 + NODE_H / 2.), "{name}");
                } else {
                    // Bottom edge to bottom edge, the arrowhead pointing up.
                    assert_eq!(start.1, a.1 + NODE_H, "{name}");
                    assert_eq!(route.tip.1, b.1 + NODE_H, "{name}");
                    assert!(route.head()[1].1 > route.tip.1, "{name}");
                }
                // The segments join end to end.
                for pair in route.segments.windows(2) {
                    assert_eq!(pair[0][3], pair[1][0], "{name}");
                }
            }
        }
    }

    #[test]
    fn a_call_that_closes_a_cycle_comes_back_below() {
        let (nodes, layout) = routed(3, &[(0, 1), (1, 2), (2, 0)]);
        assert_eq!(
            nodes.iter().map(|n| column(*n)).collect::<Vec<_>>(),
            [0, 1, 2]
        );
        let back = &layout.routes[2];
        // It leaves 2's bottom, crosses 1's column below its box and
        // enters 0 from below.
        assert_eq!(back.segments[0][0].1, nodes[2].1 + NODE_H);
        let lowest = back
            .segments
            .iter()
            .flatten()
            .map(|p| p.1)
            .fold(0., f32::max);
        assert!(lowest > nodes[1].1 + NODE_H);
        assert!(back.tip.0 > nodes[0].0 && back.tip.0 < nodes[0].0 + NODE_W);
    }

    #[test]
    fn a_self_call_loops_under_its_box() {
        let (nodes, layout) = routed(1, &[(0, 0)]);
        let route = &layout.routes[0];
        let (start, tip) = (route.segments[0][0], route.tip);
        assert_eq!((start.1, tip.1), (nodes[0].1 + NODE_H, nodes[0].1 + NODE_H));
        assert!(start.0 > tip.0);
        assert!(layout.height > nodes[0].1 + NODE_H + PAD);
    }

    #[test]
    fn a_long_call_crosses_each_column_in_a_gutter() {
        // 0 → 1 → 2 → 3, and 0 → 3 across columns 1 and 2.
        let (nodes, layout) = routed(4, &[(0, 1), (1, 2), (2, 3), (0, 3)]);
        let long = &layout.routes[3];
        assert_eq!(long.segments.len(), 5);
        for (column, segment) in [(1, &long.segments[1]), (2, &long.segments[3])] {
            let y = segment[0].1;
            assert_eq!(segment[3].1, y);
            assert_eq!(segment[0].0, nodes[column].0 - ARROW);
            assert!(y < nodes[column].1 || y > nodes[column].1 + NODE_H);
        }
        assert_eq!(long.mid.1, long.segments[2][0].1);
    }

    #[test]
    fn a_crowded_gutter_grows_to_fit_its_lanes() {
        // 1 calls 2 to 8; 0 calls 1 and, across 1's column, 3 to 8.
        let mut links = vec![(0, 1)];
        links.extend((2..=8).map(|to| (1, to)));
        links.extend((3..=8).map(|to| (0, to)));
        let (nodes, layout) = routed(9, &links);
        let mut lanes: Vec<f32> = layout.routes[8..]
            .iter()
            .map(|route| route.segments[1][0].1)
            .collect();
        lanes.sort_by(f32::total_cmp);
        assert!(
            lanes.windows(2).all(|w| w[1] - w[0] >= LANE - 0.01),
            "{lanes:?}"
        );
        let below = lanes.iter().filter(|&&y| y > nodes[1].1).count();
        assert!(below > 3, "{lanes:?}");
        for route in &layout.routes {
            assert_eq!(through_a_box(&nodes, route), None);
        }
    }

    #[test]
    fn the_same_graph_routes_the_same() {
        let links = generated(500, 1_500);
        let (first_nodes, first) = routed(500, &links);
        let (second_nodes, second) = routed(500, &links);
        assert_eq!(first_nodes, second_nodes);
        assert_eq!(first, second);
    }

    #[test]
    fn five_hundred_boxes_route_in_time() {
        let links = generated(500, 1_500);
        let mut nodes = vec![Box::default(); 500];
        let start = std::time::Instant::now();
        let layout = route(&mut nodes, &links);
        let took = start.elapsed();
        let waypoints: usize = layout.routes.iter().map(|r| r.segments.len()).sum();
        eprintln!(
            "route: 500 boxes, 1,500 calls, {waypoints} segments, {} x {} dp in {took:?}",
            layout.width, layout.height
        );
        // Generous, for a loaded machine and an unoptimised build.
        assert!(took < std::time::Duration::from_secs(2), "{took:?}");
        // One page of the service map: 24 boxes and the selected ends.
        let links = generated(26, 78);
        let mut nodes = vec![Box::default(); 26];
        let start = std::time::Instant::now();
        route(&mut nodes, &links);
        eprintln!("route: 26 boxes, 78 calls in {:?}", start.elapsed());
    }

    #[test]
    fn a_fan_out_routes_without_crossings() {
        // The story's fan-out: 43 crossings, 4 through a box, as curves.
        let links: Vec<_> = (1..=12).map(|to| (0, to)).collect();
        let (nodes, layout) = routed(13, &links);
        assert_eq!(crossings(&layout.routes), 0);
        assert!(
            layout
                .routes
                .iter()
                .all(|route| through_a_box(&nodes, route).is_none())
        );
    }

    /// A fingerprint of a layout: its boxes, size and every route's numbers.
    fn fingerprint(count: usize, links: &[(usize, usize)]) -> u64 {
        let (nodes, layout) = routed(count, links);
        format!("{nodes:?}{layout:?}")
            .bytes()
            .fold(0xcbf2_9ce4_8422_2325, |hash, byte| {
                (hash ^ u64::from(byte)).wrapping_mul(0x0100_0000_01b3)
            })
    }

    /// Pins the service map's layout: these were taken from `route` before
    /// `Metrics` existed, so a change that moves a box or a line fails here.
    #[test]
    fn the_maps_layout_is_pinned() {
        let mut graphs = shapes();
        graphs.push(("generated", 120, generated(120, 300)));
        graphs.push(("wrapped", 11, (1..=10).map(|to| (0, to)).collect()));
        let got: Vec<_> = graphs
            .iter()
            .map(|(name, count, links)| (*name, fingerprint(*count, links)))
            .collect();
        assert_eq!(got, PINNED, "{got:#x?}");
    }

    #[test]
    fn route_is_route_with_the_map_metrics() {
        let mut graphs = shapes();
        graphs.push(("generated", 120, generated(120, 300)));
        for (name, count, links) in graphs {
            let mut a = vec![Box::default(); count];
            let mut b = a.clone();
            let one = route(&mut a, &links);
            let two = route_with(&Metrics::MAP, &mut b, &links);
            assert_eq!((a, one), (b, two), "{name}");
        }
    }

    #[test]
    fn a_wider_gutter_moves_columns_apart_and_routes_still_avoid_boxes() {
        let wide = Metrics::labelled(84.);
        for (name, count, links) in shapes() {
            let mut nodes = vec![Box::default(); count];
            let layout = route_with(&wide, &mut nodes, &links);
            for node in &nodes {
                let columns = (node.0 - PAD) / wide.column;
                assert!((columns - columns.round()).abs() < 1e-3, "{name}");
            }
            let widest = nodes.iter().map(|n| n.0).fold(0., f32::max);
            assert_eq!(layout.width, widest + wide.node_w + PAD, "{name}");
            for (route, link) in layout.routes.iter().zip(&links) {
                let inside = route.segments.iter().find_map(|segment| {
                    (0..=16)
                        .map(|s| point_at(segment, s as f32 / 16.))
                        .find(|&(x, y)| {
                            nodes.iter().any(|b| {
                                x > b.0 + 0.5
                                    && x < b.0 + wide.node_w - 0.5
                                    && y > b.1 + 0.5
                                    && y < b.1 + wide.node_h - 0.5
                            })
                        })
                });
                assert_eq!(inside, None, "{name}: {link:?}");
            }
        }
    }

    #[test]
    fn a_labelled_chain_never_wraps() {
        // One caller of forty: the map wraps them at eight; a chain doesn't.
        let links: Vec<_> = (1..=40).map(|to| (0, to)).collect();
        let mut nodes = vec![Box::default(); 41];
        let layout = route_with(&Metrics::labelled(84.), &mut nodes, &links);
        assert!(nodes[1..].iter().all(|n| n.0 == nodes[1].0));
        let mut rows: Vec<_> = nodes[1..].iter().map(|n| n.1).collect();
        rows.dedup();
        assert_eq!(rows.len(), 40);
        assert!(layout.height > 40. * NODE_H);
        // Loose boxes stay in one column too.
        let mut loose = vec![Box::default(); 20];
        route_with(&Metrics::labelled(84.), &mut loose, &[]);
        assert!(loose.iter().all(|n| n.0 == PAD));
    }

    const PINNED: &[(&str, u64)] = &[
        ("shop", 3937795867122466415),
        ("fan-out", 17303332123121938718),
        ("cycle", 18168086660578549213),
        ("loose", 14455150901419364279),
        ("self-call", 2664253225597394985),
        ("generated", 10038829460731074263),
        ("wrapped", 18324783326817147324),
    ];
}
