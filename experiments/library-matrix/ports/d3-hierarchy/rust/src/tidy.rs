// Buchheim's linear-time version of Reingold–Tilford, from d3-hierarchy 3.1.3.
use super::{separation, walk, Node, NONE};
use alloc::vec::Vec;
#[derive(Clone, Copy)]
struct Tidy {
    ancestor: usize,
    default_ancestor: usize,
    thread: usize,
    number: usize,
    z: f64,
    m: f64,
    c: f64,
    s: f64,
}
impl Tidy {
    fn new(i: usize) -> Self {
        Self {
            ancestor: i,
            default_ancestor: NONE,
            thread: NONE,
            number: 0,
            z: 0.,
            m: 0.,
            c: 0.,
            s: 0.,
        }
    }
}
fn contour(nodes: &[Node], meta: &[Tidy], v: usize, left: bool) -> usize {
    let c = &nodes[v].children;
    if c.is_empty() {
        meta[v].thread
    } else if left {
        c[0]
    } else {
        c[c.len() - 1]
    }
}
fn apportion(nodes: &[Node], t: &mut [Tidy], v: usize, w: usize, mut ancestor: usize) -> usize {
    if w == NONE {
        return ancestor;
    }
    let p = nodes[v].parent;
    let (mut vip, mut vop, mut vim, mut vom) = (v, v, w, nodes[p].children[0]);
    let (mut sip, mut sop, mut sim, mut som) = (t[vip].m, t[vop].m, t[vim].m, t[vom].m);
    loop {
        vim = contour(nodes, t, vim, false);
        vip = contour(nodes, t, vip, true);
        if vim == NONE || vip == NONE {
            break;
        }
        vom = contour(nodes, t, vom, true);
        vop = contour(nodes, t, vop, false);
        t[vop].ancestor = v;
        let shift = t[vim].z + sim - t[vip].z - sip + separation(nodes, vim, vip);
        if shift > 0. {
            let anc = t[vim].ancestor;
            let wm = if nodes[anc].parent == nodes[v].parent {
                anc
            } else {
                ancestor
            };
            let change = shift / (t[v].number - t[wm].number) as f64;
            t[v].c -= change;
            t[v].s += shift;
            t[wm].c += change;
            t[v].z += shift;
            t[v].m += shift;
            sip += shift;
            sop += shift;
        }
        sim += t[vim].m;
        sip += t[vip].m;
        som += t[vom].m;
        sop += t[vop].m;
    }
    if vim != NONE && contour(nodes, t, vop, false) == NONE {
        t[vop].thread = vim;
        t[vop].m += sim - sop;
    }
    if vip != NONE && contour(nodes, t, vom, true) == NONE {
        t[vom].thread = vip;
        t[vom].m += sip - som;
        ancestor = v;
    }
    ancestor
}
pub(super) fn layout(nodes: &mut [Node], root: usize, w: f64, h: f64, fixed: bool) {
    let mut t = Vec::with_capacity(nodes.len());
    for i in 0..nodes.len() {
        t.push(Tidy::new(i));
    }
    let order = walk(nodes, root, false);
    for &v in &order {
        for (i, &c) in nodes[v].children.iter().enumerate() {
            t[c].number = i;
        }
    }
    for v in walk(nodes, root, true) {
        let parent = nodes[v].parent;
        let prev = if t[v].number > 0 {
            nodes[parent].children[t[v].number - 1]
        } else {
            NONE
        };
        let children = &nodes[v].children;
        if !children.is_empty() {
            let (mut shift, mut change) = (0., 0.);
            for &c in children.iter().rev() {
                t[c].z += shift;
                t[c].m += shift;
                change += t[c].c;
                shift += t[c].s + change;
            }
            let mid = (t[children[0]].z + t[children[children.len() - 1]].z) / 2.;
            if prev == NONE {
                t[v].z = mid;
            } else {
                t[v].z = t[prev].z + separation(nodes, v, prev);
                t[v].m = t[v].z - mid;
            }
        } else if prev != NONE {
            t[v].z = t[prev].z + separation(nodes, v, prev);
        }
        if parent != NONE {
            let ancestor = if t[parent].default_ancestor == NONE {
                nodes[parent].children[0]
            } else {
                t[parent].default_ancestor
            };
            t[parent].default_ancestor = apportion(nodes, &mut t, v, prev, ancestor);
        }
    }
    let offset = -t[root].z;
    let (mut left, mut right, mut bottom) = (root, root, root);
    for &v in &order {
        let parent = nodes[v].parent;
        let m = if v == root { offset } else { t[parent].m };
        nodes[v].x = t[v].z + m;
        t[v].m += m;
        if nodes[v].x < nodes[left].x {
            left = v;
        }
        if nodes[v].x > nodes[right].x {
            right = v;
        }
        if nodes[v].depth > nodes[bottom].depth {
            bottom = v;
        }
    }
    if fixed {
        for v in order {
            nodes[v].x *= w;
            nodes[v].y = nodes[v].depth as f64 * h;
        }
    } else {
        let s = if left == right {
            1.
        } else {
            separation(nodes, left, right) / 2.
        };
        let tx = s - nodes[left].x;
        let kx = w / (nodes[right].x + s + tx);
        let ky = h / nodes[bottom].depth.max(1) as f64;
        for v in order {
            nodes[v].x = (nodes[v].x + tx) * kx;
            nodes[v].y = nodes[v].depth as f64 * ky;
        }
    }
}
