//! Core earcut 3.2.4 polygon triangulation algorithm.
//!
//! Faithfully mirrors `node_modules/earcut/src/earcut.js`:
//! - Append-only arena of circular doubly-linked `Node`s (removed nodes retain their `prev`/`next`
//!   links so `node.prev.next != node` dead-node detection and `liveBlockHead`/`liveBlockStop`
//!   behave identically to JS object references).
//! - Single-vertex Steiner hole preservation (`node.steiner`).
//! - `eliminateHoles` with stable `compareXYSlope` sorting and the `K = 16` append-only
//!   block-bbox index (`indexSegment`, `growBlock`, `liveBlockHead`, `liveBlockStop`,
//!   `findHoleBridge`).
//! - 30-bit Morton `zOrder` hashing, `indexCurve`, and `sortNodes` (stable insertion sort for
//!   `n <= 32`, 4-pass 8-bit LSD radix sort for `n > 32`).
//! - 3-stage `earcutLinked` (`filterPoints` fixpoint -> single `cureLocalIntersections` pass ->
//!   `splitEarcut` fallback).
//!
//! Hot loops bind `&[Node]` / `&mut [Node]` / `&[f64]` slices outside the loop to avoid repeated
//! `Vec` derefs (`$rt_unscale` + `WeakMap` lookups under `mires`, see `PERF-004`).

use alloc::vec;
use alloc::vec::Vec;
use core::cmp::Ordering;

const NONE: u32 = u32::MAX;
const BLOCK_K: usize = 16;

struct Node {
    /// Vertex index in the flat coordinates array.
    i: u32,
    /// Vertex X and Y coordinates.
    x: f64,
    y: f64,
    /// Previous and next vertex nodes in the circular polygon ring.
    prev: u32,
    next: u32,
    /// Z-order curve value; doubles as owning block index during `eliminate_holes`.
    z: i32,
    /// Previous and next nodes in z-order (`NONE` at ends).
    prev_z: u32,
    next_z: u32,
    /// Single-vertex hole preserved across `filter_points` (Steiner point).
    steiner: bool,
}

// PERF-004: `min3`, `max3`, `point_in_triangle`, `area_sl` and `z_order_inline` are macros so
// they expand inside the hot loops and read the slice those loops already borrow. See NOTES.md,
// "Performance accommodations (PERF-004)".
macro_rules! min3 {
    ($a:expr, $b:expr, $c:expr) => {{
        let m = if $a < $b { $a } else { $b };
        if m < $c { m } else { $c }
    }};
}

macro_rules! max3 {
    ($a:expr, $b:expr, $c:expr) => {{
        let m = if $a > $b { $a } else { $b };
        if m > $c { m } else { $c }
    }};
}

macro_rules! point_in_triangle {
    ($ax:expr, $ay:expr, $bx:expr, $by:expr, $cx:expr, $cy:expr, $px:expr, $py:expr) => {
        ($cx - $px) * ($ay - $py) >= ($ax - $px) * ($cy - $py)
            && ($ax - $px) * ($by - $py) >= ($bx - $px) * ($ay - $py)
            && ($bx - $px) * ($cy - $py) >= ($cx - $px) * ($by - $py)
    };
}

macro_rules! area_sl {
    ($nodes:expr, $p:expr, $q:expr, $r:expr) => {{
        let np = &$nodes[$p as usize];
        let nq = &$nodes[$q as usize];
        let nr = &$nodes[$r as usize];
        (nq.y - np.y) * (nr.x - nq.x) - (nq.x - np.x) * (nr.y - nq.y)
    }};
}

macro_rules! z_order_inline {
    ($x:expr, $y:expr, $min_x:expr, $min_y:expr, $inv_size:expr) => {{
        let fx = ($x - $min_x) * $inv_size;
        let fy = ($y - $min_y) * $inv_size;
        let mut ix = if fx >= -2147483648.0 && fx <= 2147483647.0 {
            (fx as i32) as u32
        } else {
            js_to_i32_slow(fx) as u32
        };
        let mut iy = if fy >= -2147483648.0 && fy <= 2147483647.0 {
            (fy as i32) as u32
        } else {
            js_to_i32_slow(fy) as u32
        };

        ix = (ix | (ix << 8)) & 0x00FF_00FF;
        ix = (ix | (ix << 4)) & 0x0F0F_0F0F;
        ix = (ix | (ix << 2)) & 0x3333_3333;
        ix = (ix | (ix << 1)) & 0x5555_5555;

        iy = (iy | (iy << 8)) & 0x00FF_00FF;
        iy = (iy | (iy << 4)) & 0x0F0F_0F0F;
        iy = (iy | (iy << 2)) & 0x3333_3333;
        iy = (iy | (iy << 1)) & 0x5555_5555;

        (ix | (iy << 1)) as i32
    }};
}

#[cold]
fn js_to_i32_slow(v: f64) -> i32 {
    // The remainder has magnitude below 2^32. Truncate its magnitude before
    // restoring the sign so negative fractions wrap exactly as JS ToInt32.
    let magnitude = (v.abs() % 4294967296.0) as u32;
    if v < 0.0 {
        magnitude.wrapping_neg() as i32
    } else {
        magnitude as i32
    }
}

#[inline]
fn equals_sl(nodes: &[Node], p1: u32, p2: u32) -> bool {
    let a = &nodes[p1 as usize];
    let b = &nodes[p2 as usize];
    a.x == b.x && a.y == b.y
}

// PERF-004: npm's removeNode is split in two. This one takes the borrowed node slice and skips
// growBlock; it serves every call made while npm's `indexActive` is false.
// `EarcutState::remove_node_indexed` serves the merge loop. See NOTES.md,
// "Performance accommodations (PERF-004)".
#[inline]
fn remove_node_sl(nodes: &mut [Node], p: u32) {
    let (prev, next, prev_z, next_z) = {
        let np = &nodes[p as usize];
        (np.prev, np.next, np.prev_z, np.next_z)
    };
    nodes[next as usize].prev = prev;
    nodes[prev as usize].next = next;

    if prev_z != NONE {
        nodes[prev_z as usize].next_z = next_z;
    }
    if next_z != NONE {
        nodes[next_z as usize].prev_z = prev_z;
    }
}

#[inline]
fn locally_inside_sl(nodes: &[Node], a: u32, b: u32) -> bool {
    let na = &nodes[a as usize];
    let a_prev = na.prev;
    let a_next = na.next;
    if area_sl!(nodes, a_prev, a, a_next) < 0.0 {
        area_sl!(nodes, a, b, a_next) >= 0.0 && area_sl!(nodes, a, a_prev, b) >= 0.0
    } else {
        area_sl!(nodes, a, b, a_prev) < 0.0 || area_sl!(nodes, a, a_next, b) < 0.0
    }
}

#[inline]
fn on_segment_sl(nodes: &[Node], p: u32, q: u32, r: u32) -> bool {
    let np = &nodes[p as usize];
    let nq = &nodes[q as usize];
    let nr = &nodes[r as usize];
    let max_x = if np.x > nr.x { np.x } else { nr.x };
    let min_x = if np.x < nr.x { np.x } else { nr.x };
    let max_y = if np.y > nr.y { np.y } else { nr.y };
    let min_y = if np.y < nr.y { np.y } else { nr.y };
    nq.x <= max_x && nq.x >= min_x && nq.y <= max_y && nq.y >= min_y
}

fn intersects_sl(nodes: &[Node], p1: u32, q1: u32, p2: u32, q2: u32, include_boundary: bool) -> bool {
    let o1 = area_sl!(nodes, p1, q1, p2);
    let o2 = area_sl!(nodes, p1, q1, q2);
    let o3 = area_sl!(nodes, p2, q2, p1);
    let o4 = area_sl!(nodes, p2, q2, q1);

    if ((o1 > 0.0 && o2 < 0.0) || (o1 < 0.0 && o2 > 0.0))
        && ((o3 > 0.0 && o4 < 0.0) || (o3 < 0.0 && o4 > 0.0))
    {
        return true;
    }

    if !include_boundary {
        return false;
    }

    if o1 == 0.0 && on_segment_sl(nodes, p1, p2, q1) {
        return true;
    }
    if o2 == 0.0 && on_segment_sl(nodes, p1, q2, q1) {
        return true;
    }
    if o3 == 0.0 && on_segment_sl(nodes, p2, p1, q2) {
        return true;
    }
    if o4 == 0.0 && on_segment_sl(nodes, p2, q1, q2) {
        return true;
    }
    false
}

// PERF-004: npm's filterPoints, on the borrowed node slice, for calls made while `indexActive` is
// false. `EarcutState::filter_points_indexed` is the merge-loop copy. See NOTES.md,
// "Performance accommodations (PERF-004)".
fn filter_points_sl(nodes: &mut [Node], start: u32, mut end: u32, filtered_out: &mut bool) -> u32 {
    let full = end == start;
    let mut p = start;
    loop {
        let mut again = false;
        let (p_prev, p_next, is_steiner) = {
            let np = &nodes[p as usize];
            (np.prev, np.next, np.steiner)
        };
        if p != p_next
            && !is_steiner
            && (equals_sl(nodes, p, p_next) || area_sl!(nodes, p_prev, p, p_next) == 0.0)
        {
            if full || p == end {
                end = p_prev;
            }
            *filtered_out = true;
            remove_node_sl(nodes, p);
            p = p_prev;
            again = true;
        } else if full || p != end {
            p = p_next;
            again = !full;
        }

        if !again && p == end {
            break;
        }
    }
    end
}

fn is_ear_sl(nodes: &[Node], ear: u32) -> bool {
    let nb = &nodes[ear as usize];
    let a = nb.prev;
    let c = nb.next;
    let na = &nodes[a as usize];
    let nc = &nodes[c as usize];

    let ax = na.x;
    let ay = na.y;
    let bx = nb.x;
    let by = nb.y;
    let cx = nc.x;
    let cy = nc.y;

    let x0 = min3!(ax, bx, cx);
    let y0 = min3!(ay, by, cy);
    let x1 = max3!(ax, bx, cx);
    let y1 = max3!(ay, by, cy);

    let mut p = nc.next;
    while p != a {
        let np = &nodes[p as usize];
        let px = np.x;
        let py = np.y;
        if px >= x0
            && px <= x1
            && py >= y0
            && py <= y1
            && !(ax == px && ay == py)
            && point_in_triangle!(ax, ay, bx, by, cx, cy, px, py)
        {
            let pp = &nodes[np.prev as usize];
            let pn = &nodes[np.next as usize];
            if (py - pp.y) * (pn.x - px) - (px - pp.x) * (pn.y - py) >= 0.0 {
                return false;
            }
        }
        p = np.next;
    }
    true
}

fn is_ear_hashed_sl(nodes: &[Node], ear: u32, min_x: f64, min_y: f64, inv_size: f64) -> bool {
    let nb = &nodes[ear as usize];
    let a = nb.prev;
    let c = nb.next;
    let na = &nodes[a as usize];
    let nc = &nodes[c as usize];

    let ax = na.x;
    let ay = na.y;
    let bx = nb.x;
    let by = nb.y;
    let cx = nc.x;
    let cy = nc.y;

    let x0 = min3!(ax, bx, cx);
    let y0 = min3!(ay, by, cy);
    let x1 = max3!(ax, bx, cx);
    let y1 = max3!(ay, by, cy);

    let min_z = z_order_inline!(x0, y0, min_x, min_y, inv_size);
    let max_z = z_order_inline!(x1, y1, min_x, min_y, inv_size);

    let mut p = nb.prev_z;
    while p != NONE {
        let np = &nodes[p as usize];
        if np.z < min_z {
            break;
        }
        let px = np.x;
        let py = np.y;
        if px >= x0
            && px <= x1
            && py >= y0
            && py <= y1
            && p != c
            && !(ax == px && ay == py)
            && point_in_triangle!(ax, ay, bx, by, cx, cy, px, py)
        {
            let pp = &nodes[np.prev as usize];
            let pn = &nodes[np.next as usize];
            if (py - pp.y) * (pn.x - px) - (px - pp.x) * (pn.y - py) >= 0.0 {
                return false;
            }
        }
        p = np.prev_z;
    }

    let mut n = nb.next_z;
    while n != NONE {
        let nn = &nodes[n as usize];
        if nn.z > max_z {
            break;
        }
        let nx = nn.x;
        let ny = nn.y;
        if nx >= x0
            && nx <= x1
            && ny >= y0
            && ny <= y1
            && n != c
            && !(ax == nx && ay == ny)
            && point_in_triangle!(ax, ay, bx, by, cx, cy, nx, ny)
        {
            let np = &nodes[nn.prev as usize];
            let nnext = &nodes[nn.next as usize];
            if (ny - np.y) * (nnext.x - nx) - (nx - np.x) * (nnext.y - ny) >= 0.0 {
                return false;
            }
        }
        n = nn.next_z;
    }

    true
}

fn cure_local_intersections_sl(
    nodes: &mut [Node],
    mut start: u32,
    triangles: &mut Vec<u32>,
    filtered_out: &mut bool,
) -> u32 {
    let mut p = start;
    let mut cured = false;
    loop {
        let (a, p_next) = {
            let np = &nodes[p as usize];
            (np.prev, np.next)
        };
        let b = nodes[p_next as usize].next;

        if intersects_sl(nodes, a, p, p_next, b, false)
            && locally_inside_sl(nodes, a, b)
            && locally_inside_sl(nodes, b, a)
        {
            let ai = nodes[a as usize].i;
            let pi = nodes[p as usize].i;
            let bi = nodes[b as usize].i;
            triangles.push(ai);
            triangles.push(pi);
            triangles.push(bi);

            remove_node_sl(nodes, p);
            remove_node_sl(nodes, p_next);

            p = b;
            start = b;
            cured = true;
        }
        p = nodes[p as usize].next;
        if p == start {
            break;
        }
    }

    if cured {
        filter_points_sl(nodes, p, p, filtered_out)
    } else {
        p
    }
}

struct EarcutState {
    nodes: Vec<Node>,
    filtered_out: bool,
    index_active: bool,
    block_bbox: Vec<f64>,
    block_head: Vec<u32>,
    block_stop: Vec<u32>,
    num_blocks: usize,
    sort_arr: Vec<u32>,
    sort_buf: Vec<u32>,
    z_arr: Vec<u32>,
    z_buf: Vec<u32>,
}

impl EarcutState {
    fn new(node_cap: usize) -> Self {
        Self {
            nodes: Vec::with_capacity(node_cap),
            filtered_out: false,
            index_active: false,
            block_bbox: Vec::new(),
            block_head: Vec::new(),
            block_stop: Vec::new(),
            num_blocks: 0,
            sort_arr: Vec::new(),
            sort_buf: Vec::new(),
            z_arr: Vec::new(),
            z_buf: Vec::new(),
        }
    }

    #[inline]
    fn create_node(&mut self, i: u32, x: f64, y: f64) -> u32 {
        let idx = self.nodes.len() as u32;
        self.nodes.push(Node {
            i,
            x,
            y,
            prev: idx,
            next: idx,
            z: 0,
            prev_z: NONE,
            next_z: NONE,
            steiner: false,
        });
        idx
    }

    #[inline]
    fn insert_node(&mut self, i: u32, x: f64, y: f64, last: u32) -> u32 {
        let p = self.create_node(i, x, y);
        if last != NONE {
            let nodes = self.nodes.as_mut_slice();
            let last_next = nodes[last as usize].next;
            nodes[p as usize].next = last_next;
            nodes[p as usize].prev = last;
            nodes[last_next as usize].prev = p;
            nodes[last as usize].next = p;
        }
        p
    }

    // PERF-004: the merge-loop copy of `remove_node_sl`, which also grows the block bbox while
    // `index_active` (npm's growBlock). See NOTES.md, "Performance accommodations (PERF-004)".
    #[inline]
    fn remove_node_indexed(&mut self, p: u32) {
        let nodes = self.nodes.as_mut_slice();
        let (prev, next, prev_z, next_z) = {
            let np = &nodes[p as usize];
            (np.prev, np.next, np.prev_z, np.next_z)
        };
        nodes[next as usize].prev = prev;
        nodes[prev as usize].next = next;

        if prev_z != NONE {
            nodes[prev_z as usize].next_z = next_z;
        }
        if next_z != NONE {
            nodes[next_z as usize].prev_z = prev_z;
        }

        if self.index_active {
            let g = (nodes[prev as usize].z as usize) * 4;
            let (tx, ty) = {
                let nt = &nodes[next as usize];
                (nt.x, nt.y)
            };
            let bb = self.block_bbox.as_mut_slice();
            if tx < bb[g] {
                bb[g] = tx;
            }
            if ty < bb[g + 1] {
                bb[g + 1] = ty;
            }
            if tx > bb[g + 2] {
                bb[g + 2] = tx;
            }
            if ty > bb[g + 3] {
                bb[g + 3] = ty;
            }
        }
    }

    fn linked_list(
        &mut self,
        data: &[f64],
        start: usize,
        end: usize,
        dim: usize,
        clockwise: bool,
    ) -> u32 {
        let mut last = NONE;
        if clockwise == (signed_area(data, start, end, dim) > 0.0) {
            let mut i = start;
            while i < end {
                last = self.insert_node((i / dim) as u32, data[i], data[i + 1], last);
                i += dim;
            }
        } else {
            let mut i = end;
            while i > start {
                i -= dim;
                last = self.insert_node((i / dim) as u32, data[i], data[i + 1], last);
            }
        }

        if last != NONE {
            let nodes = self.nodes.as_mut_slice();
            let next = nodes[last as usize].next;
            if equals_sl(nodes, last, next) {
                remove_node_sl(nodes, last);
                last = next;
            }
        }
        last
    }

    // PERF-004: the merge-loop copy of `filter_points_sl`; it removes through
    // `remove_node_indexed`. `eliminate_hole` is its only caller. See NOTES.md,
    // "Performance accommodations (PERF-004)".
    fn filter_points_indexed(&mut self, start: u32, mut end: u32) -> u32 {
        let full = end == start;
        let mut p = start;
        loop {
            let mut again = false;
            let (p_prev, p_next, removable) = {
                let nodes = self.nodes.as_slice();
                let np = &nodes[p as usize];
                let r = p != np.next
                    && !np.steiner
                    && (equals_sl(nodes, p, np.next) || area_sl!(nodes, np.prev, p, np.next) == 0.0);
                (np.prev, np.next, r)
            };
            if removable {
                if full || p == end {
                    end = p_prev;
                }
                self.filtered_out = true;
                self.remove_node_indexed(p);
                p = p_prev;
                again = true;
            } else if full || p != end {
                p = p_next;
                again = !full;
            }

            if !again && p == end {
                break;
            }
        }
        end
    }

    fn earcut_linked(
        &mut self,
        mut ear: u32,
        triangles: &mut Vec<u32>,
        min_x: f64,
        min_y: f64,
        inv_size: f64,
    ) {
        if inv_size != 0.0 {
            self.index_curve(ear, min_x, min_y, inv_size);
        }

        let need_split = {
            let nodes = self.nodes.as_mut_slice();
            let mut stop = ear;
            let mut cured = false;
            let mut split_at = NONE;

            loop {
                let (prev, next, ear_i) = {
                    let ne = &nodes[ear as usize];
                    (ne.prev, ne.next, ne.i)
                };
                if prev == next {
                    break;
                }

                if area_sl!(nodes, prev, ear, next) < 0.0
                    && if inv_size != 0.0 {
                        is_ear_hashed_sl(nodes, ear, min_x, min_y, inv_size)
                    } else {
                        is_ear_sl(nodes, ear)
                    }
                {
                    let prev_i = nodes[prev as usize].i;
                    let next_i = nodes[next as usize].i;
                    triangles.push(prev_i);
                    triangles.push(ear_i);
                    triangles.push(next_i);

                    remove_node_sl(nodes, ear);
                    ear = next;
                    stop = next;
                    continue;
                }

                ear = next;

                if ear == stop {
                    self.filtered_out = false;
                    ear = filter_points_sl(nodes, ear, ear, &mut self.filtered_out);
                    if self.filtered_out {
                        stop = ear;
                        continue;
                    }

                    if !cured {
                        ear = cure_local_intersections_sl(
                            nodes,
                            ear,
                            triangles,
                            &mut self.filtered_out,
                        );
                        stop = ear;
                        cured = true;
                        continue;
                    }

                    split_at = ear;
                    break;
                }
            }
            split_at
        };

        if need_split != NONE {
            self.split_earcut(need_split, triangles, min_x, min_y, inv_size);
        }
    }

    fn split_earcut(
        &mut self,
        start: u32,
        triangles: &mut Vec<u32>,
        min_x: f64,
        min_y: f64,
        inv_size: f64,
    ) {
        let pair = {
            let nodes = self.nodes.as_slice();
            let mut found = (NONE, NONE);
            let mut a = start;
            'outer: loop {
                let na = &nodes[a as usize];
                let a_prev = na.prev;
                let a_next = na.next;
                let a_i = na.i;
                let mut b = nodes[a_next as usize].next;
                while b != a_prev {
                    if a_i != nodes[b as usize].i && is_valid_diagonal_sl(nodes, a, b) {
                        found = (a, b);
                        break 'outer;
                    }
                    b = nodes[b as usize].next;
                }
                a = a_next;
                if a == start {
                    break;
                }
            }
            found
        };

        if pair.0 != NONE {
            let mut a = pair.0;
            let b = pair.1;
            let mut c = self.split_polygon(a, b);

            {
                let nodes = self.nodes.as_mut_slice();
                let a_n = nodes[a as usize].next;
                a = filter_points_sl(nodes, a, a_n, &mut self.filtered_out);
                let c_n = nodes[c as usize].next;
                c = filter_points_sl(nodes, c, c_n, &mut self.filtered_out);
            }

            self.earcut_linked(a, triangles, min_x, min_y, inv_size);
            self.earcut_linked(c, triangles, min_x, min_y, inv_size);
        }
    }

    fn eliminate_holes(
        &mut self,
        data: &[f64],
        hole_indices: &[u32],
        mut outer_node: u32,
        dim: usize,
    ) -> u32 {
        let len = hole_indices.len();
        let mut queue: Vec<u32> = Vec::with_capacity(len);

        let mut i = 0;
        while i < len {
            let start = (hole_indices[i] as usize) * dim;
            let end = if i + 1 < len {
                (hole_indices[i + 1] as usize) * dim
            } else {
                data.len()
            };
            let list = self.linked_list(data, start, end, dim, false);
            if list != NONE {
                {
                    let nodes = self.nodes.as_mut_slice();
                    if list == nodes[list as usize].next {
                        nodes[list as usize].steiner = true;
                    }
                }
                queue.push(get_leftmost_sl(self.nodes.as_slice(), list));
            }
            i += 1;
        }

        stable_sort_holes(self.nodes.as_slice(), &mut queue);

        self.build_block_index(data.len() / dim, len);
        self.index_segment(outer_node, outer_node);

        self.index_active = true;
        let qsl = queue.as_slice();
        let qlen = qsl.len();
        let mut q = 0;
        while q < qlen {
            outer_node = self.eliminate_hole(qsl[q], outer_node);
            q += 1;
        }
        self.index_active = false;

        let nodes = self.nodes.as_mut_slice();
        filter_points_sl(nodes, outer_node, outer_node, &mut self.filtered_out)
    }

    fn eliminate_hole(&mut self, hole: u32, outer_node: u32) -> u32 {
        let bridge = self.find_hole_bridge(hole, outer_node);
        if bridge == NONE {
            return outer_node;
        }

        let bridge_reverse = self.split_polygon(bridge, hole);
        let (bridge2, bridge2_next) = {
            let nodes = self.nodes.as_slice();
            let b2 = nodes[bridge_reverse as usize].next;
            (b2, nodes[b2 as usize].next)
        };
        self.index_segment(bridge, bridge2_next);

        self.filter_points_indexed(bridge_reverse, bridge2);
        let bridge_next = self.nodes[bridge as usize].next;
        self.filter_points_indexed(bridge, bridge_next)
    }

    fn build_block_index(&mut self, max_nodes: usize, num_holes: usize) {
        let max_blocks = (max_nodes + 2 * num_holes + BLOCK_K - 1) / BLOCK_K + num_holes + 2;
        self.block_bbox.clear();
        self.block_bbox.reserve(max_blocks * 4);
        self.block_head.clear();
        self.block_head.reserve(max_blocks);
        self.block_stop.clear();
        self.block_stop.reserve(max_blocks);
        self.num_blocks = 0;
    }

    fn index_segment(&mut self, head: u32, stop: u32) {
        let nodes = self.nodes.as_mut_slice();
        let mut p = head;
        loop {
            let b = self.num_blocks;
            self.num_blocks += 1;
            self.block_head.push(p);

            let (mut min_x, mut min_y, mut max_x, mut max_y) = {
                let np = &nodes[p as usize];
                (np.x, np.y, np.x, np.y)
            };

            let mut k = 0;
            loop {
                let c = {
                    let np = &mut nodes[p as usize];
                    np.z = b as i32;
                    np.next
                };
                let nc = &nodes[c as usize];
                let cx = nc.x;
                let cy = nc.y;
                if cx < min_x {
                    min_x = cx;
                }
                if cx > max_x {
                    max_x = cx;
                }
                if cy < min_y {
                    min_y = cy;
                }
                if cy > max_y {
                    max_y = cy;
                }
                p = c;
                k += 1;
                if k >= BLOCK_K || p == stop {
                    break;
                }
            }

            self.block_stop.push(p);
            self.block_bbox.push(min_x);
            self.block_bbox.push(min_y);
            self.block_bbox.push(max_x);
            self.block_bbox.push(max_y);

            if p == stop {
                break;
            }
        }
    }

    fn find_hole_bridge(&mut self, hole: u32, outer_node: u32) -> u32 {
        let nodes = self.nodes.as_slice();
        let bb = self.block_bbox.as_slice();
        let bhead = self.block_head.as_mut_slice();
        let bstop = self.block_stop.as_mut_slice();

        let (hx, hy) = {
            let nh = &nodes[hole as usize];
            (nh.x, nh.y)
        };

        if equals_sl(nodes, hole, outer_node) {
            return outer_node;
        }

        let mut qx = f64::NEG_INFINITY;
        let mut m = NONE;
        let num_blocks = self.num_blocks;

        let mut b = 0;
        let mut g = 0;
        while b < num_blocks {
            if hy < bb[g + 1] || hy > bb[g + 3] || bb[g] > hx || bb[g + 2] <= qx {
                b += 1;
                g += 4;
                continue;
            }

            let mut stop = bstop[b];
            while nodes[nodes[stop as usize].prev as usize].next != stop {
                stop = nodes[stop as usize].next;
            }
            bstop[b] = stop;

            let mut head = bhead[b];
            while nodes[nodes[head as usize].prev as usize].next != head {
                head = nodes[head as usize].next;
            }
            bhead[b] = head;

            let mut p = head;
            loop {
                let np = &nodes[p as usize];
                let p_next = np.next;
                if nodes[np.prev as usize].next == p {
                    let nn = &nodes[p_next as usize];
                    if hx == nn.x && hy == nn.y {
                        return p_next;
                    } else if hy <= np.y && hy >= nn.y && nn.y != np.y {
                        let x = np.x + (hy - np.y) * (nn.x - np.x) / (nn.y - np.y);
                        if x <= hx && x > qx {
                            qx = x;
                            m = if np.x < nn.x { p } else { p_next };
                            if x == hx {
                                return m;
                            }
                        }
                    }
                }
                p = p_next;
                if p == stop {
                    break;
                }
            }

            b += 1;
            g += 4;
        }

        if m == NONE {
            return NONE;
        }

        let (mx, my) = {
            let nm = &nodes[m as usize];
            (nm.x, nm.y)
        };
        let tmin_y = if hy < my { hy } else { my };
        let tmax_y = if hy > my { hy } else { my };
        let mut tan_min = f64::INFINITY;

        let ax = if hy < my { hx } else { qx };
        let cx = if hy < my { qx } else { hx };

        b = 0;
        g = 0;
        while b < num_blocks {
            if bb[g + 2] < mx || bb[g] > hx || bb[g + 3] < tmin_y || bb[g + 1] > tmax_y {
                b += 1;
                g += 4;
                continue;
            }

            let mut stop = bstop[b];
            while nodes[nodes[stop as usize].prev as usize].next != stop {
                stop = nodes[stop as usize].next;
            }
            bstop[b] = stop;

            let mut head = bhead[b];
            while nodes[nodes[head as usize].prev as usize].next != head {
                head = nodes[head as usize].next;
            }
            bhead[b] = head;

            let mut p = head;
            loop {
                let np = &nodes[p as usize];
                let px = np.x;
                let py = np.y;
                if nodes[np.prev as usize].next == p
                    && hx >= px
                    && px >= mx
                    && hx != px
                    && point_in_triangle!(ax, hy, mx, my, cx, hy, px, py)
                {
                    let tan = (hy - py).abs() / (hx - px);
                    let pn = &nodes[np.next as usize];
                    let valid = locally_inside_sl(nodes, p, hole)
                        || (py == hy && pn.y == hy && pn.x > hx);
                    if valid {
                        let m_cur_x = nodes[m as usize].x;
                        if tan < tan_min
                            || (tan == tan_min
                                && (px > m_cur_x
                                    || (px == m_cur_x && sector_contains_sector_sl(nodes, m, p))))
                        {
                            m = p;
                            tan_min = tan;
                        }
                    }
                }
                p = np.next;
                if p == stop {
                    break;
                }
            }

            b += 1;
            g += 4;
        }

        m
    }

    fn index_curve(&mut self, start: u32, min_x: f64, min_y: f64, inv_size: f64) {
        self.sort_arr.clear();
        {
            let nodes = self.nodes.as_mut_slice();
            let mut p = start;
            loop {
                let np = &mut nodes[p as usize];
                np.z = z_order_inline!(np.x, np.y, min_x, min_y, inv_size);
                let next = np.next;
                self.sort_arr.push(p);
                p = next;
                if p == start {
                    break;
                }
            }
        }

        let n = self.sort_arr.len();
        self.sort_nodes(n);

        let nodes = self.nodes.as_mut_slice();
        let arr = self.sort_arr.as_slice();
        let mut prev = NONE;
        let mut i = 0;
        while i < n {
            let node = arr[i];
            nodes[node as usize].prev_z = prev;
            if prev != NONE {
                nodes[prev as usize].next_z = node;
            }
            prev = node;
            i += 1;
        }
        if prev != NONE {
            nodes[prev as usize].next_z = NONE;
        }
    }

    fn sort_nodes(&mut self, n: usize) {
        if n <= 32 {
            let nodes = self.nodes.as_slice();
            let arr = self.sort_arr.as_mut_slice();
            let mut i = 1;
            while i < n {
                let node = arr[i];
                let z = nodes[node as usize].z;
                let mut j = i;
                while j > 0 && nodes[arr[j - 1] as usize].z > z {
                    arr[j] = arr[j - 1];
                    j -= 1;
                }
                arr[j] = node;
                i += 1;
            }
            return;
        }

        if self.z_arr.len() < n {
            self.z_arr.resize(n, 0);
            self.z_buf.resize(n, 0);
            self.sort_buf.resize(n, 0);
        }
        {
            let nodes = self.nodes.as_slice();
            let arr = self.sort_arr.as_slice();
            let za = self.z_arr.as_mut_slice();
            let mut i = 0;
            while i < n {
                za[i] = nodes[arr[i] as usize].z as u32;
                i += 1;
            }
        }

        let mut counts = [0u32; 256];
        radix_pass(
            n,
            &self.sort_arr,
            &self.z_arr,
            &mut self.sort_buf,
            &mut self.z_buf,
            0,
            &mut counts,
        );
        radix_pass(
            n,
            &self.sort_buf,
            &self.z_buf,
            &mut self.sort_arr,
            &mut self.z_arr,
            8,
            &mut counts,
        );
        radix_pass(
            n,
            &self.sort_arr,
            &self.z_arr,
            &mut self.sort_buf,
            &mut self.z_buf,
            16,
            &mut counts,
        );
        radix_pass(
            n,
            &self.sort_buf,
            &self.z_buf,
            &mut self.sort_arr,
            &mut self.z_arr,
            24,
            &mut counts,
        );
    }

    fn split_polygon(&mut self, a: u32, b: u32) -> u32 {
        let (ai, ax, ay, an, bi, bx, by, bp) = {
            let nodes = self.nodes.as_slice();
            let na = &nodes[a as usize];
            let nb = &nodes[b as usize];
            (na.i, na.x, na.y, na.next, nb.i, nb.x, nb.y, nb.prev)
        };
        let a2 = self.create_node(ai, ax, ay);
        let b2 = self.create_node(bi, bx, by);

        let nodes = self.nodes.as_mut_slice();
        nodes[a as usize].next = b;
        nodes[b as usize].prev = a;

        nodes[a2 as usize].next = an;
        nodes[an as usize].prev = a2;

        nodes[b2 as usize].next = a2;
        nodes[a2 as usize].prev = b2;

        nodes[bp as usize].next = b2;
        nodes[b2 as usize].prev = bp;

        b2
    }
}

#[inline]
fn sector_contains_sector_sl(nodes: &[Node], m: u32, p: u32) -> bool {
    let nm = &nodes[m as usize];
    let np = &nodes[p as usize];
    area_sl!(nodes, nm.prev, m, np.prev) < 0.0 && area_sl!(nodes, np.next, m, nm.next) < 0.0
}

fn get_leftmost_sl(nodes: &[Node], start: u32) -> u32 {
    let mut p = start;
    let mut leftmost = start;
    let mut lx = nodes[leftmost as usize].x;
    let mut ly = nodes[leftmost as usize].y;
    loop {
        let np = &nodes[p as usize];
        if np.x < lx || (np.x == lx && np.y < ly) {
            leftmost = p;
            lx = np.x;
            ly = np.y;
        }
        p = np.next;
        if p == start {
            break;
        }
    }
    leftmost
}

fn compare_xy_slope_sl(nodes: &[Node], a: u32, b: u32) -> Ordering {
    let na = &nodes[a as usize];
    let nb = &nodes[b as usize];
    let mut d = na.x - nb.x;
    if d == 0.0 {
        d = na.y - nb.y;
        if d == 0.0 {
            let an = &nodes[na.next as usize];
            let bn = &nodes[nb.next as usize];
            d = (an.y - na.y) / (an.x - na.x) - (bn.y - nb.y) / (bn.x - nb.x);
        }
    }
    if d < 0.0 {
        Ordering::Less
    } else if d > 0.0 {
        Ordering::Greater
    } else {
        Ordering::Equal
    }
}

/// Stable sort matching JS `Array.prototype.sort(compareXYSlope)` without instantiating
/// `core::slice::sort` (~18 KB code size savings under mires).
fn stable_sort_holes(nodes: &[Node], queue: &mut [u32]) {
    let n = queue.len();
    if n <= 1 {
        return;
    }
    if n <= 16 {
        let mut i = 1;
        while i < n {
            let key = queue[i];
            let mut j = i;
            while j > 0 && compare_xy_slope_sl(nodes, queue[j - 1], key) == Ordering::Greater {
                queue[j] = queue[j - 1];
                j -= 1;
            }
            queue[j] = key;
            i += 1;
        }
        return;
    }

    let mut buf = vec![0u32; n];
    let bsl = buf.as_mut_slice();
    let mut width = 1;
    while width < n {
        let mut left = 0;
        while left < n {
            let mid = if left + width < n { left + width } else { n };
            let right = if left + 2 * width < n {
                left + 2 * width
            } else {
                n
            };
            let mut l = left;
            let mut r = mid;
            let mut out = left;
            while l < mid && r < right {
                if compare_xy_slope_sl(nodes, queue[l], queue[r]) != Ordering::Greater {
                    bsl[out] = queue[l];
                    l += 1;
                } else {
                    bsl[out] = queue[r];
                    r += 1;
                }
                out += 1;
            }
            while l < mid {
                bsl[out] = queue[l];
                l += 1;
                out += 1;
            }
            while r < right {
                bsl[out] = queue[r];
                r += 1;
                out += 1;
            }
            left += 2 * width;
        }
        let mut k = 0;
        while k < n {
            queue[k] = bsl[k];
            k += 1;
        }
        width <<= 1;
    }
}

fn intersects_polygon_sl(nodes: &[Node], a: u32, b: u32) -> bool {
    let na = &nodes[a as usize];
    let nb = &nodes[b as usize];
    let min_x = if na.x < nb.x { na.x } else { nb.x };
    let max_x = if na.x > nb.x { na.x } else { nb.x };
    let min_y = if na.y < nb.y { na.y } else { nb.y };
    let max_y = if na.y > nb.y { na.y } else { nb.y };
    let ai = na.i;
    let bi = nb.i;

    let mut p = a;
    loop {
        let np = &nodes[p as usize];
        let n = np.next;
        let nn = &nodes[n as usize];
        if !((np.x > max_x && nn.x > max_x)
            || (np.x < min_x && nn.x < min_x)
            || (np.y > max_y && nn.y > max_y)
            || (np.y < min_y && nn.y < min_y))
            && np.i != ai
            && nn.i != ai
            && np.i != bi
            && nn.i != bi
            && intersects_sl(nodes, p, n, a, b, true)
        {
            return true;
        }
        p = n;
        if p == a {
            break;
        }
    }
    false
}

fn middle_inside_sl(nodes: &[Node], a: u32, b: u32) -> bool {
    let na = &nodes[a as usize];
    let nb = &nodes[b as usize];
    let px = (na.x + nb.x) / 2.0;
    let py = (na.y + nb.y) / 2.0;

    let mut p = a;
    let mut inside = false;
    loop {
        let np = &nodes[p as usize];
        let n = np.next;
        let nn = &nodes[n as usize];
        if ((np.y > py) != (nn.y > py))
            && (px < (nn.x - np.x) * (py - np.y) / (nn.y - np.y) + np.x)
        {
            inside = !inside;
        }
        p = n;
        if p == a {
            break;
        }
    }
    inside
}

fn is_valid_diagonal_sl(nodes: &[Node], a: u32, b: u32) -> bool {
    let na = &nodes[a as usize];
    let nb = &nodes[b as usize];
    let zero_length = equals_sl(nodes, a, b)
        && area_sl!(nodes, na.prev, a, na.next) > 0.0
        && area_sl!(nodes, nb.prev, b, nb.next) > 0.0;
    nodes[na.next as usize].i != nb.i
        && (zero_length
            || (locally_inside_sl(nodes, a, b)
                && locally_inside_sl(nodes, b, a)
                && (area_sl!(nodes, na.prev, a, nb.prev) != 0.0
                    || area_sl!(nodes, a, nb.prev, b) != 0.0)))
        && !intersects_polygon_sl(nodes, a, b)
        && (zero_length || middle_inside_sl(nodes, a, b))
}

fn radix_pass(
    n: usize,
    src: &[u32],
    src_z: &[u32],
    dst: &mut [u32],
    dst_z: &mut [u32],
    shift: u32,
    counts: &mut [u32; 256],
) {
    let mut b = 0;
    while b < 256 {
        counts[b] = 0;
        b += 1;
    }
    let mut i = 0;
    while i < n {
        counts[((src_z[i] >> shift) & 0xff) as usize] += 1;
        i += 1;
    }
    let mut sum = 0u32;
    b = 0;
    while b < 256 {
        let c = counts[b];
        counts[b] = sum;
        sum += c;
        b += 1;
    }
    i = 0;
    while i < n {
        let z = src_z[i];
        let bucket = ((z >> shift) & 0xff) as usize;
        let pos = counts[bucket] as usize;
        counts[bucket] = (pos + 1) as u32;
        dst[pos] = src[i];
        dst_z[pos] = z;
        i += 1;
    }
}

pub fn signed_area(data: &[f64], start: usize, end: usize, dim: usize) -> f64 {
    if start >= end {
        return 0.0;
    }
    let mut sum = 0.0;
    let mut j = end - dim;
    let mut i = start;
    while i < end {
        sum += (data[j] - data[i]) * (data[i + 1] + data[j + 1]);
        j = i;
        i += dim;
    }
    sum
}

pub fn earcut(data: &[f64], hole_indices: Option<&[u32]>, dim: usize) -> Vec<u32> {
    let holes = match hole_indices {
        Some(h) if !h.is_empty() => Some(h),
        _ => None,
    };
    let outer_len = match holes {
        Some(h) => (h[0] as usize) * dim,
        None => data.len(),
    };
    let num_verts = data.len() / dim;
    let num_holes = match holes {
        Some(h) => h.len(),
        None => 0,
    };

    let mut state = EarcutState::new(num_verts + 2 * num_holes + 4);
    let mut outer_node = state.linked_list(data, 0, outer_len, dim, true);
    let mut triangles = Vec::new();

    if outer_node == NONE {
        return triangles;
    }
    {
        let on = &state.nodes[outer_node as usize];
        if on.next == on.prev {
            return triangles;
        }
    }

    triangles.reserve(num_verts.saturating_mul(3));

    let mut min_x = 0.0;
    let mut min_y = 0.0;
    let mut inv_size = 0.0;

    if let Some(h) = holes {
        outer_node = state.eliminate_holes(data, h, outer_node, dim);
    }

    if data.len() > 80 * dim {
        min_x = data[0];
        min_y = data[1];
        let mut max_x = min_x;
        let mut max_y = min_y;

        let mut i = dim;
        while i < outer_len {
            let x = data[i];
            let y = data[i + 1];
            if x < min_x {
                min_x = x;
            }
            if y < min_y {
                min_y = y;
            }
            if x > max_x {
                max_x = x;
            }
            if y > max_y {
                max_y = y;
            }
            i += dim;
        }

        let dx = max_x - min_x;
        let dy = max_y - min_y;
        inv_size = if dx > dy { dx } else { dy };
        inv_size = if inv_size != 0.0 {
            32767.0 / inv_size
        } else {
            0.0
        };
    }

    state.earcut_linked(outer_node, &mut triangles, min_x, min_y, inv_size);
    triangles
}
