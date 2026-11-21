// Partition, alternating slice-and-dice, binary and squarified treemap tilers.
use super::{js_max, round, sqrt, truthy, walk, Node};
use alloc::vec::Vec;
#[derive(Clone, Copy)]
struct Rect {
    x: f64,
    y: f64,
    x1: f64,
    y1: f64,
}
impl Rect {
    fn inset(self, p: f64) -> Self {
        let mut a = Self {
            x: self.x + p,
            y: self.y + p,
            x1: self.x1 - p,
            y1: self.y1 - p,
        };
        a.collapse();
        a
    }
    fn collapse(&mut self) {
        if self.x1 < self.x {
            self.x = (self.x + self.x1) / 2.;
            self.x1 = self.x;
        }
        if self.y1 < self.y {
            self.y = (self.y + self.y1) / 2.;
            self.y1 = self.y;
        }
    }
    fn put(self, n: &mut Node) {
        n.x0 = self.x;
        n.y0 = self.y;
        n.x1 = self.x1;
        n.y1 = self.y1;
    }
}
fn slice(nodes: &mut [Node], ids: &[usize], mut rect: Rect, total: f64, dice: bool) {
    // `parent.value && (x1 - x0) / parent.value`
    let k = if !truthy(total) {
        total
    } else if dice {
        (rect.x1 - rect.x) / total
    } else {
        (rect.y1 - rect.y) / total
    };
    for &v in ids {
        let mut r = rect;
        if dice {
            rect.x += nodes[v].value * k;
            r.x1 = rect.x;
        } else {
            rect.y += nodes[v].value * k;
            r.y1 = rect.y;
        }
        r.put(&mut nodes[v]);
    }
}
fn binary(
    nodes: &mut [Node],
    ids: &[usize],
    sums: &[f64],
    i: usize,
    j: usize,
    total: f64,
    rect: Rect,
) {
    if i >= j - 1 {
        rect.put(&mut nodes[ids[i]]);
        return;
    }
    let offset = sums[i];
    let target = total / 2. + offset;
    let mut k = i + 1;
    let mut hi = j - 1;
    while k < hi {
        let mid = (k + hi) / 2;
        if sums[mid] < target {
            k = mid + 1;
        } else {
            hi = mid;
        }
    }
    if target - sums[k - 1] < sums[k] - target && i + 1 < k {
        k -= 1;
    }
    let left = sums[k] - offset;
    let right = total - left;
    let mut a = rect;
    let mut b = rect;
    if rect.x1 - rect.x > rect.y1 - rect.y {
        let x = if truthy(total) {
            (rect.x * right + rect.x1 * left) / total
        } else {
            rect.x1
        };
        a.x1 = x;
        b.x = x;
    } else {
        let y = if truthy(total) {
            (rect.y * right + rect.y1 * left) / total
        } else {
            rect.y1
        };
        a.y1 = y;
        b.y = y;
    }
    binary(nodes, ids, sums, i, k, left, a);
    binary(nodes, ids, sums, k, j, right, b);
}
fn squarify(nodes: &mut [Node], ids: &[usize], mut total: f64, mut rect: Rect) {
    let ratio = (1. + sqrt(5.)) / 2.;
    let mut i = 0;
    let mut j = 0;
    while i < ids.len() {
        let dx = rect.x1 - rect.x;
        let dy = rect.y1 - rect.y;
        let mut sum;
        loop {
            sum = nodes[ids[j]].value;
            j += 1;
            // `while (!sumValue && i1 < n)`
            if truthy(sum) || j == ids.len() {
                break;
            }
        }
        let mut min = sum;
        let mut max = sum;
        let alpha = js_max(dy / dx, dx / dy) / (total * ratio);
        let beta = sum * sum * alpha;
        let mut best = js_max(max / beta, beta / min);
        while j < ids.len() {
            let v = nodes[ids[j]].value;
            sum += v;
            if v < min {
                min = v;
            }
            if v > max {
                max = v;
            }
            let beta = sum * sum * alpha;
            let next = js_max(max / beta, beta / min);
            if next > best {
                sum -= v;
                break;
            }
            best = next;
            j += 1;
        }
        let mut row = rect;
        let dice = dx < dy;
        if dice {
            if truthy(total) {
                rect.y += dy * sum / total;
                row.y1 = rect.y;
            }
        } else if truthy(total) {
            rect.x += dx * sum / total;
            row.x1 = rect.x;
        }
        slice(nodes, &ids[i..j], row, sum, dice);
        total -= sum;
        i = j;
    }
}
pub(super) fn layout(
    nodes: &mut [Node],
    root: usize,
    op: &str,
    tile: &str,
    w: f64,
    h: f64,
    p: f64,
    rounded: bool,
) {
    let partition = op == "partition";
    let levels = (nodes[root].height + 1) as f64;
    if partition {
        nodes[root].x0 = p;
        nodes[root].y0 = p;
        nodes[root].y1 = h / levels;
    } else {
        nodes[root].y1 = h;
    }
    nodes[root].x1 = w;
    let order = walk(nodes, root, false);
    for &v in &order {
        let kids = core::mem::take(&mut nodes[v].children);
        let total = nodes[v].value;
        let d = nodes[v].depth;
        let mut rect = Rect {
            x: nodes[v].x0,
            y: nodes[v].y0,
            x1: nodes[v].x1,
            y1: nodes[v].y1,
        };
        if partition {
            if !kids.is_empty() {
                let child = Rect {
                    x: rect.x,
                    x1: rect.x1,
                    y: h * (d + 1) as f64 / levels,
                    y1: h * (d + 2) as f64 / levels,
                };
                slice(nodes, &kids, child, total, true);
            }
            rect.x1 -= p;
            rect.y1 -= p;
            rect.collapse();
            rect.put(&mut nodes[v]);
        } else {
            let inside = rect.inset(if v == root { 0. } else { p / 2. });
            inside.put(&mut nodes[v]);
            if !kids.is_empty() {
                let frame = inside.inset(p - p / 2.);
                match tile {
                    "binary" => {
                        let mut sums = Vec::with_capacity(kids.len() + 1);
                        sums.push(0.);
                        let mut acc = 0.;
                        for &c in &kids {
                            acc += nodes[c].value;
                            sums.push(acc);
                        }
                        binary(nodes, &kids, &sums, 0, kids.len(), total, frame);
                    }
                    "sliceDice" => slice(nodes, &kids, frame, total, d % 2 == 0),
                    "squarify" => squarify(nodes, &kids, total, frame),
                    _ => super::fail(),
                }
            }
        }
        nodes[v].children = kids;
    }
    if rounded {
        for v in order {
            let n = &mut nodes[v];
            n.x0 = round(n.x0);
            n.y0 = round(n.y0);
            n.x1 = round(n.x1);
            n.y1 = round(n.y1);
        }
    }
}
