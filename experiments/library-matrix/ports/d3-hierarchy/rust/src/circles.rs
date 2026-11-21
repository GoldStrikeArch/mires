// Front-chain circle packing and randomized enclosing basis from d3-hierarchy.
use super::{fail, js_max, js_min, sqrt, truthy, walk, Node};
use alloc::{vec, vec::Vec};
#[derive(Copy, Clone)]
struct Circle {
    x: f64,
    y: f64,
    r: f64,
}
fn weak(a: Circle, b: Circle) -> bool {
    let dr = a.r - b.r + js_max(js_max(a.r, b.r), 1.) * 1e-9;
    let dx = b.x - a.x;
    let dy = b.y - a.y;
    dr > 0. && dr * dr > dx * dx + dy * dy
}
fn not_enclosed(a: Circle, b: Circle) -> bool {
    let dr = a.r - b.r;
    let dx = b.x - a.x;
    let dy = b.y - a.y;
    dr < 0. || dr * dr < dx * dx + dy * dy
}
fn pair(a: Circle, b: Circle) -> Circle {
    let dx = b.x - a.x;
    let dy = b.y - a.y;
    let dr = b.r - a.r;
    let l = sqrt(dx * dx + dy * dy);
    Circle {
        x: (a.x + b.x + dx / l * dr) / 2.,
        y: (a.y + b.y + dy / l * dr) / 2.,
        r: (l + a.r + b.r) / 2.,
    }
}
fn triple(a: Circle, b: Circle, c: Circle) -> Circle {
    let a2 = a.x - b.x;
    let a3 = a.x - c.x;
    let b2 = a.y - b.y;
    let b3 = a.y - c.y;
    let c2 = b.r - a.r;
    let c3 = c.r - a.r;
    let d1 = a.x * a.x + a.y * a.y - a.r * a.r;
    let d2 = d1 - b.x * b.x - b.y * b.y + b.r * b.r;
    let d3 = d1 - c.x * c.x - c.y * c.y + c.r * c.r;
    let ab = a3 * b2 - a2 * b3;
    let xa = (b2 * d3 - b3 * d2) / (ab * 2.) - a.x;
    let xb = (b3 * c2 - b2 * c3) / ab;
    let ya = (a3 * d2 - a2 * d3) / (ab * 2.) - a.y;
    let yb = (a2 * c3 - a3 * c2) / ab;
    let qa = xb * xb + yb * yb - 1.;
    let qb = 2. * (a.r + xa * xb + ya * yb);
    let qc = xa * xa + ya * ya - a.r * a.r;
    let r = -(if qa.abs() > 1e-6 {
        (qb + sqrt(qb * qb - 4. * qa * qc)) / (2. * qa)
    } else {
        qc / qb
    });
    Circle {
        x: a.x + xa + xb * r,
        y: a.y + ya + yb * r,
        r,
    }
}
fn covers(a: Circle, b: &[Circle]) -> bool {
    for &p in b {
        if !weak(a, p) {
            return false;
        }
    }
    true
}
fn extend(b: &[Circle], p: Circle) -> Vec<Circle> {
    if covers(p, b) {
        return vec![p];
    }
    for &x in b {
        if not_enclosed(p, x) && covers(pair(x, p), b) {
            return vec![x, p];
        }
    }
    for (i, &x) in b.iter().enumerate() {
        for &y in &b[i + 1..] {
            if not_enclosed(pair(x, y), p)
                && not_enclosed(pair(x, p), y)
                && not_enclosed(pair(y, p), x)
                && covers(triple(x, y, p), b)
            {
                return vec![x, y, p];
            }
        }
    }
    fail()
}
fn enclose(mut circles: Vec<Circle>, seed: &mut u32) -> Circle {
    for n in (1..=circles.len()).rev() {
        *seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
        let i = (*seed as f64 / 4294967296. * n as f64) as usize;
        circles.swap(i, n - 1);
    }
    let mut basis = Vec::new();
    let mut i = 0;
    let mut e = Circle {
        x: 0.,
        y: 0.,
        r: 0.,
    };
    while i < circles.len() {
        let p = circles[i];
        if !basis.is_empty() && weak(e, p) {
            i += 1;
        } else {
            basis = extend(&basis, p);
            e = match basis.len() {
                1 => basis[0],
                2 => pair(basis[0], basis[1]),
                3 => triple(basis[0], basis[1], basis[2]),
                _ => fail(),
            };
            i = 0;
        }
    }
    e
}
fn place(b: Circle, a: Circle, mut c: Circle) -> Circle {
    let dx = b.x - a.x;
    let dy = b.y - a.y;
    let d2 = dx * dx + dy * dy;
    if truthy(d2) {
        let a2 = (a.r + c.r) * (a.r + c.r);
        let b2 = (b.r + c.r) * (b.r + c.r);
        if a2 > b2 {
            let x = (d2 + b2 - a2) / (2. * d2);
            let y = sqrt(js_max(0., b2 / d2 - x * x));
            c.x = b.x - x * dx - y * dy;
            c.y = b.y - x * dy + y * dx;
        } else {
            let x = (d2 + a2 - b2) / (2. * d2);
            let y = sqrt(js_max(0., a2 / d2 - x * x));
            c.x = a.x + x * dx - y * dy;
            c.y = a.y + x * dy + y * dx;
        }
    } else {
        c.x = a.x + c.r;
        c.y = a.y;
    }
    c
}
fn intersects(a: Circle, b: Circle) -> bool {
    let dr = a.r + b.r - 1e-6;
    let dx = b.x - a.x;
    let dy = b.y - a.y;
    dr > 0. && dr * dr > dx * dx + dy * dy
}
fn score(a: Circle, b: Circle) -> f64 {
    let ab = a.r + b.r;
    let dx = (a.x * b.r + b.x * a.r) / ab;
    let dy = (a.y * b.r + b.y * a.r) / ab;
    dx * dx + dy * dy
}
fn siblings(c: &mut [Circle], seed: &mut u32) -> f64 {
    let n = c.len();
    c[0].x = 0.;
    c[0].y = 0.;
    if n == 1 {
        return c[0].r;
    }
    c[0].x = -c[1].r;
    c[1].x = c[0].r;
    c[1].y = 0.;
    if n == 2 {
        return c[0].r + c[1].r;
    }
    c[2] = place(c[1], c[0], c[2]);
    let mut next = vec![0; n];
    let mut prev = vec![0; n];
    next[0] = 1;
    prev[2] = 1;
    next[1] = 2;
    prev[0] = 2;
    next[2] = 0;
    prev[1] = 0;
    let mut a = 0;
    let mut b = 1;
    let mut i = 3;
    'pack: while i < n {
        c[i] = place(c[a], c[b], c[i]);
        let (mut j, mut k, mut sj, mut sk) = (next[b], prev[a], c[b].r, c[a].r);
        loop {
            if sj <= sk {
                if intersects(c[j], c[i]) {
                    b = j;
                    next[a] = b;
                    prev[b] = a;
                    continue 'pack;
                }
                sj += c[j].r;
                j = next[j];
            } else {
                if intersects(c[k], c[i]) {
                    a = k;
                    next[a] = b;
                    prev[b] = a;
                    continue 'pack;
                }
                sk += c[k].r;
                k = prev[k];
            }
            if j == next[k] {
                break;
            }
        }
        prev[i] = a;
        next[i] = b;
        next[a] = i;
        prev[b] = i;
        b = i;
        let mut best = score(c[a], c[next[a]]);
        let mut j = next[i];
        while j != b {
            let val = score(c[j], c[next[j]]);
            if val < best {
                best = val;
                a = j;
            }
            j = next[j];
        }
        b = next[a];
        i += 1;
    }
    let mut front = vec![c[b]];
    let mut j = next[b];
    while j != b {
        front.push(c[j]);
        j = next[j];
    }
    let e = enclose(front, seed);
    for p in c {
        p.x -= e.x;
        p.y -= e.y;
    }
    e.r
}
fn pack_children(nodes: &mut [Node], order: &[usize], p: f64, k: f64, seed: &mut u32) {
    // `padding(node) * k || 0`
    let pad = if truthy(p * k) { p * k } else { 0. };
    for &v in order {
        if nodes[v].children.is_empty() {
            continue;
        }
        let kids = core::mem::take(&mut nodes[v].children);
        let mut circles = Vec::with_capacity(kids.len());
        for &id in &kids {
            let mut r = nodes[id].r;
            if truthy(pad) {
                r += pad;
            }
            circles.push(Circle {
                x: nodes[id].x,
                y: nodes[id].y,
                r,
            });
        }
        let e = siblings(&mut circles, seed);
        for (j, &id) in kids.iter().enumerate() {
            let c = circles[j];
            nodes[id].x = c.x;
            nodes[id].y = c.y;
            if truthy(pad) {
                nodes[id].r = c.r - pad;
            }
        }
        nodes[v].r = e + pad;
        nodes[v].children = kids;
    }
}
pub(super) fn layout(nodes: &mut [Node], root: usize, w: f64, h: f64, p: f64) {
    let pre = walk(nodes, root, false);
    let post = walk(nodes, root, true);
    let mut seed = 1;
    nodes[root].x = w / 2.;
    nodes[root].y = h / 2.;
    for &v in &pre {
        if nodes[v].children.is_empty() {
            // `Math.max(0, +radius(node) || 0)`
            let r = sqrt(nodes[v].value);
            nodes[v].r = js_max(0., if truthy(r) { r } else { 0. });
        }
    }
    pack_children(nodes, &post, 0., 1., &mut seed);
    let k = nodes[root].r / js_min(w, h);
    pack_children(nodes, &post, p, k, &mut seed);
    let k = js_min(w, h) / (2. * nodes[root].r);
    for v in pre {
        nodes[v].r *= k;
        if v != root {
            let parent = nodes[v].parent;
            nodes[v].x = nodes[parent].x + k * nodes[v].x;
            nodes[v].y = nodes[parent].y + k * nodes[v].y;
        }
    }
}
