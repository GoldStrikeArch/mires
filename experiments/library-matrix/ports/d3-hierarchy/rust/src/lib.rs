#![no_std]
#![no_main]
#![deny(unsafe_code)]
extern crate alloc;

use alloc::{
    collections::BTreeMap,
    string::{String, ToString},
    vec,
    vec::Vec,
};
use mires_js::{js_bindings, js_export, throw_value, JsArray, JsValue};

mod circles;
mod rectangles;
mod tidy;

const NONE: usize = usize::MAX;

#[panic_handler]
fn panic(_: &core::panic::PanicInfo<'_>) -> ! {
    fail()
}
fn fail() -> ! {
    throw_value(JsValue::erase("invalid hierarchy"))
}

js_bindings! {
    #[js(call="Reflect.get")] fn prop(obj:JsValue,key:JsValue)->JsValue;
    #[js(get="length")] fn len(obj:JsValue)->usize;
    #[js(index)] fn at(obj:JsValue,i:usize)->JsValue;
    #[js(call="Math.sqrt")] fn sqrt(x:f64)->f64;
    #[js(call="Math.round")] fn round(x:f64)->f64;
}
fn get(v: JsValue, k: &str) -> JsValue {
    prop(v, JsValue::erase(k))
}
// d3 reads every field of a datum with `!= null` or `??`: undefined and null both mean absent.
fn text(v: JsValue, k: &str) -> String {
    let p = get(v, k);
    if p.is_null() || p.is_undefined() {
        String::new()
    } else {
        p.read::<&str>().to_string()
    }
}
fn num(v: JsValue, k: &str) -> f64 {
    let p = get(v, k);
    if p.is_null() || p.is_undefined() {
        0.
    } else {
        p.read::<f64>()
    }
}
fn flag(v: JsValue, k: &str) -> bool {
    get(v, k).is_truthy()
}

/// JavaScript's ToBoolean of a number: false for 0, -0 and NaN. d3 tests numbers this way
/// (`value ? … : …`, `if (d2)`, `+x || 0`).
pub(crate) fn truthy(x: f64) -> bool {
    !(x == 0. || x.is_nan())
}

/// `Math.max(a, b)`: NaN if either is NaN, and +0 above -0. `f64::max` returns the other operand
/// for a NaN and either zero for a pair of zeros.
pub(crate) fn js_max(a: f64, b: f64) -> f64 {
    if a.is_nan() || b.is_nan() {
        f64::NAN
    } else if a == b {
        if a.is_sign_negative() {
            b
        } else {
            a
        }
    } else if a > b {
        a
    } else {
        b
    }
}

/// `Math.min(a, b)`: NaN if either is NaN, and -0 below +0.
pub(crate) fn js_min(a: f64, b: f64) -> f64 {
    if a.is_nan() || b.is_nan() {
        f64::NAN
    } else if a == b {
        if a.is_sign_negative() {
            a
        } else {
            b
        }
    } else if a < b {
        a
    } else {
        b
    }
}

struct Node {
    /// The id stratify gave the node, empty where d3 leaves `node.id` undefined.
    id: String,
    /// The datum (`node.data`): a nested record, a stratify row, or null for an imputed ancestor.
    data: JsValue,
    parent: usize,
    children: Vec<usize>,
    depth: usize,
    height: usize,
    value: f64,
    x: f64,
    y: f64,
    r: f64,
    x0: f64,
    y0: f64,
    x1: f64,
    y1: f64,
}
impl Node {
    fn new(id: String, data: JsValue, value: f64, parent: usize) -> Self {
        Self {
            id,
            data,
            parent,
            children: Vec::new(),
            depth: 0,
            height: 0,
            value,
            x: 0.,
            y: 0.,
            r: 0.,
            x0: 0.,
            y0: 0.,
            x1: 0.,
            y1: 0.,
        }
    }
}
fn nested(data: JsValue) -> (Vec<Node>, usize) {
    let mut nodes: Vec<Node> = Vec::new();
    let mut todo = vec![(data, NONE)];
    while let Some((d, parent)) = todo.pop() {
        let pos = nodes.len();
        nodes.push(Node::new(String::new(), d, num(d, "value"), parent));
        if parent != NONE {
            nodes[parent].children.push(pos);
        }
        let c = get(d, "children");
        if !c.is_undefined() && !c.is_null() {
            for j in (0..len(c)).rev() {
                todo.push((at(c, j), pos));
            }
        }
    }
    (nodes, 0)
}
fn slash(s: &[u8], i: usize) -> bool {
    if i >= s.len() || s[i] != b'/' {
        return false;
    }
    let mut k = i;
    while k > 0 && s[k - 1] == b'\\' {
        k -= 1;
    }
    (i - k) % 2 == 0
}
fn normalize(mut s: String) -> String {
    let n = s.len();
    if n > 0 && slash(s.as_bytes(), n - 1) && (n == 1 || !slash(s.as_bytes(), n - 2)) {
        s.pop();
    }
    if !s.starts_with('/') {
        s.insert(0, '/');
    }
    s
}
fn parent_path(s: &str) -> String {
    let mut i = s.len();
    if i < 2 {
        return String::new();
    }
    loop {
        i -= 1;
        if i <= 1 || slash(s.as_bytes(), i) {
            break;
        }
    }
    s[..i].to_string()
}
/// An entry of stratify's id map: the one node with that id, or d3's `ambiguous`.
enum Key {
    Unique(usize),
    Ambiguous,
}
fn stratify(rows: JsValue, paths: bool) -> (Vec<Node>, usize) {
    let count = len(rows);
    let mut nodes = Vec::new();
    let mut parents = Vec::new();
    let mut map = BTreeMap::new();
    for i in 0..count {
        let row = at(rows, i);
        let mut id = text(row, "id");
        let parent = if paths {
            id = normalize(id);
            parent_path(&id)
        } else {
            text(row, "parentId")
        };
        if !id.is_empty() {
            let key = if map.contains_key(&id) {
                Key::Ambiguous
            } else {
                Key::Unique(i)
            };
            map.insert(id.clone(), key);
        }
        parents.push(parent);
        nodes.push(Node::new(id, row, num(row, "value"), NONE));
    }
    if paths {
        let mut i = 0;
        while i < parents.len() {
            let p = &parents[i];
            if !p.is_empty() && !map.contains_key(p) {
                let id = p.clone();
                let parent = parent_path(&id);
                map.insert(id.clone(), Key::Unique(nodes.len()));
                nodes.push(Node::new(id, JsValue::null(), 0., NONE));
                parents.push(parent);
            }
            i += 1;
        }
    }
    let mut root = NONE;
    for i in 0..nodes.len() {
        if parents[i].is_empty() {
            if root != NONE {
                fail();
            }
            root = i;
        } else {
            let p = match map.get(&parents[i]) {
                Some(&Key::Unique(p)) => p,
                _ => fail(),
            };
            nodes[i].parent = p;
            nodes[p].children.push(i);
        }
    }
    if root == NONE {
        fail();
    }
    let mut expected = nodes.len();
    if paths {
        while root >= count && nodes[root].children.len() == 1 {
            root = nodes[root].children[0];
            expected -= 1;
        }
    }
    nodes[root].parent = NONE;
    if walk(&nodes, root, false).len() != expected {
        fail();
    }
    (nodes, root)
}
fn walk(nodes: &[Node], root: usize, post: bool) -> Vec<usize> {
    let mut out = Vec::with_capacity(nodes.len());
    let mut stack = vec![root];
    while let Some(v) = stack.pop() {
        out.push(v);
        if post {
            for &c in &nodes[v].children {
                stack.push(c);
            }
        } else {
            for &c in nodes[v].children.iter().rev() {
                stack.push(c);
            }
        }
    }
    if post {
        out.reverse();
    }
    out
}
fn prepare(nodes: &mut [Node], root: usize, sort: bool) {
    for v in walk(nodes, root, true) {
        let mut height = 0;
        // `+value(node.data) || 0`
        let mut val = if truthy(nodes[v].value) {
            nodes[v].value
        } else {
            0.0
        };
        for &c in nodes[v].children.iter().rev() {
            val += nodes[c].value;
            height = height.max(nodes[c].height + 1);
        }
        nodes[v].height = height;
        nodes[v].value = val;
        if sort {
            let mut children = core::mem::take(&mut nodes[v].children);
            // d3's comparator `b.value - a.value`; Array.prototype.sort counts a NaN result as 0.
            let order = |a: &usize, b: &usize| {
                nodes[*b]
                    .value
                    .partial_cmp(&nodes[*a].value)
                    .unwrap_or(core::cmp::Ordering::Equal)
            };
            if children.iter().any(|&c| nodes[c].value.is_nan()) {
                // A NaN aggregate makes that comparator inconsistent, which `sort_by` may answer
                // with a panic. A stable insertion sort moves a child only past ones it orders
                // before (outside the domain beyond two siblings: CONTRACT.md).
                for i in 1..children.len() {
                    let mut j = i;
                    while j > 0
                        && order(&children[j - 1], &children[j]) == core::cmp::Ordering::Greater
                    {
                        children.swap(j - 1, j);
                        j -= 1;
                    }
                }
            } else {
                children.sort_by(order);
            }
            nodes[v].children = children;
        }
    }
    for v in walk(nodes, root, false) {
        let p = nodes[v].parent;
        if p != NONE {
            nodes[v].depth = nodes[p].depth + 1;
        }
    }
}
fn separation(nodes: &[Node], a: usize, b: usize) -> f64 {
    if nodes[a].parent == nodes[b].parent {
        1.
    } else {
        2.
    }
}
fn cluster(nodes: &mut [Node], root: usize, w: f64, h: f64, fixed: bool) {
    let post = walk(nodes, root, true);
    let mut previous = NONE;
    let mut x = 0.;
    for &v in &post {
        if nodes[v].children.is_empty() {
            if previous != NONE {
                x += separation(nodes, v, previous);
            }
            nodes[v].x = x;
            nodes[v].y = 0.;
            previous = v;
        } else {
            let mut sum = 0.;
            let mut y = 0.0f64;
            for &c in &nodes[v].children {
                sum += nodes[c].x;
                y = js_max(y, nodes[c].y);
            }
            nodes[v].x = sum / nodes[v].children.len() as f64;
            nodes[v].y = 1. + y;
        }
    }
    let left = post[0];
    let right = previous;
    let x0 = nodes[left].x - separation(nodes, left, right) / 2.;
    let x1 = nodes[right].x + separation(nodes, right, left) / 2.;
    let rx = nodes[root].x;
    let ry = nodes[root].y;
    for v in post {
        if fixed {
            nodes[v].x = (nodes[v].x - rx) * w;
            nodes[v].y = (ry - nodes[v].y) * h;
        } else {
            nodes[v].x = (nodes[v].x - x0) / (x1 - x0) * w;
            nodes[v].y = (1. - if truthy(ry) { nodes[v].y / ry } else { 1. }) * h;
        }
    }
}

#[js_export]
pub fn matrix_run(input: JsValue) -> JsValue {
    let source = text(input, "source");
    let op = text(input, "op");
    let (mut nodes, root) = if source == "id" || source == "path" {
        stratify(get(input, "rows"), source == "path")
    } else {
        nested(get(input, "data"))
    };
    prepare(&mut nodes, root, flag(input, "sort"));
    let w = num(input, "width");
    let h = num(input, "height");
    let p = num(input, "padding");
    match op.as_str() {
        "cluster" => cluster(&mut nodes, root, w, h, flag(input, "nodeSize")),
        "tree" => tidy::layout(&mut nodes, root, w, h, flag(input, "nodeSize")),
        "pack" => circles::layout(&mut nodes, root, w, h, p),
        "treemap" | "partition" => rectangles::layout(
            &mut nodes,
            root,
            &op,
            &text(input, "tile"),
            w,
            h,
            p,
            flag(input, "round"),
        ),
        "hierarchy" | "iterator" | "stratify" => (),
        _ => fail(),
    }
    let order = if op == "iterator" {
        let mut q = vec![root];
        let mut i = 0;
        while i < q.len() {
            let v = q[i];
            for &c in &nodes[v].children {
                q.push(c);
            }
            i += 1;
        }
        q
    } else {
        walk(&nodes, root, false)
    };
    // The reference's `n.id ?? n.data.id`: the datum's id as the input has it, null or
    // undefined included.
    let label = |n: &Node| {
        if n.id.is_empty() {
            get(n.data, "id")
        } else {
            JsValue::erase(n.id.as_str())
        }
    };
    let mut output = JsArray::new();
    for i in order {
        let n = &nodes[i];
        let mut row = JsArray::new();
        row.push(label(n));
        row.push(if n.parent == NONE {
            JsValue::null()
        } else {
            label(&nodes[n.parent])
        });
        row.push(JsValue::erase(n.depth as f64));
        row.push(JsValue::erase(n.height as f64));
        row.push(JsValue::erase(n.value));
        match op.as_str() {
            "cluster" | "tree" | "pack" => {
                row.push(JsValue::erase(n.x));
                row.push(JsValue::erase(n.y));
                if op == "pack" {
                    row.push(JsValue::erase(n.r));
                }
            }
            "treemap" | "partition" => {
                row.push(JsValue::erase(n.x0));
                row.push(JsValue::erase(n.y0));
                row.push(JsValue::erase(n.x1));
                row.push(JsValue::erase(n.y1));
            }
            _ => (),
        }
        output.push(row.as_value());
    }
    output.as_value()
}
