#![no_std]
#![no_main]
#![deny(unsafe_code)]
use mires_js::{JsValue as V, js_bindings, js_export, throw_value};
#[panic_handler]
fn panic(_: &core::panic::PanicInfo<'_>) -> ! {
    throw_value(s("chart panicked"))
}

js_bindings! {
    #[js(global,get="document")] fn document()->V;
    #[js(global,get="devicePixelRatio")] fn ratio()->f64;
    #[js(call="Reflect.get")] fn prop(v:V,k:V)->V;
    #[js(call="Reflect.set")] fn put(v:V,k:V,x:V)->bool;
    #[js(call="Math.round")] fn round(x:f64)->f64;
    #[js(index)] fn at(v:V,i:usize)->V;
    #[js(index)] fn num(v:V,i:usize)->f64;
    #[js(get="length")] fn len(v:V)->usize;
    #[js(new="Array")] fn array()->V;
    #[js(method="push")] fn push(v:V,x:V)->usize;
    #[js(method="createElement")] fn element(v:V,tag:V)->V;
    #[js(method="appendChild")] fn append(v:V,node:V)->V;
    #[js(method="remove")] fn remove(v:V);
    #[js(method="getContext")] fn context(v:V,name:V)->V;
    #[js(new="Path2D")] fn path()->V;
    #[js(method="lineTo")] fn line(v:V,x:f64,y:f64);
    #[js(method="rect")] fn rect(v:V,x:f64,y:f64,w:f64,h:f64);
    #[js(method="stroke")] fn stroke(v:V,p:V);
    #[js(method="clip")] fn clip(v:V,p:V);
    #[js(method="save")] fn save(v:V);
    #[js(method="translate")] fn translate(v:V,x:f64,y:f64);
    #[js(method="restore")] fn restore(v:V);
}
fn s(x: &str) -> V {
    V::erase(x)
}
fn field(v: V, k: &str) -> V {
    prop(v, s(k))
}
fn pair(x: V, y: V) -> V {
    let a = array();
    push(a, x);
    push(a, y);
    a
}
fn coords(x: f64, y: f64) -> V {
    pair(V::erase(x), V::erase(y))
}
struct Scale {
    min: f64,
    max: f64,
    dim: f64,
    requested: f64,
    vertical: bool,
}
impl Scale {
    fn pixel(&self, v: f64) -> f64 {
        let pct = (v - self.min) / (self.max - self.min);
        self.dim * (if self.vertical { 1.0 - pct } else { pct })
    }
    fn value(&self, p: f64) -> f64 {
        let q = if self.vertical { self.requested - p } else { p };
        self.min + (self.max - self.min) * (q / self.requested)
    }
}
fn axis(frame: V, key: &str, dim: f64, vertical: bool) -> Scale {
    let range = field(frame, key);
    Scale {
        min: num(range, 0),
        max: num(range, 1),
        dim: round(dim * 2.0) / 2.0,
        requested: dim,
        vertical,
    }
}
fn nearest(xs: V, v: f64, mut lo: isize, mut hi: isize) -> isize {
    if len(xs) == 0 {
        return -1;
    }
    if hi == 0 {
        hi = len(xs) as isize - 1;
    }
    while hi > lo + 1 {
        let mid = (lo + hi) / 2;
        if num(xs, mid as usize) < v {
            lo = mid
        } else {
            hi = mid
        }
    }
    if v - num(xs, lo as usize) <= num(xs, hi as usize) - v {
        lo
    } else {
        hi
    }
}
struct Trace {
    stroke: V,
    points: V,
}
impl Trace {
    fn point(&self, x: f64, y: f64) {
        line(self.stroke, x, y);
        push(self.points, coords(x, y));
    }
}
struct Bucket {
    first: f64,
    last: f64,
    min: f64,
    max: f64,
}
impl Bucket {
    fn add(&mut self, v: f64) {
        self.last = v;
        if v < self.min {
            self.min = v
        } else if v > self.max {
            self.max = v
        }
    }
    fn finish(self, p: &Trace, x: f64, y: &Scale) {
        let min = round(y.pixel(self.min));
        let max = round(y.pixel(self.max));
        let first = round(y.pixel(self.first));
        let last = round(y.pixel(self.last));
        if min != max {
            if first != min && last != min {
                p.point(x, min);
            }
            if first != max && last != max {
                p.point(x, max);
            }
            p.point(x, last);
        }
    }
}
fn render(ctx: V, x: &Scale, y: &Scale, xs: V, ys: V, span: bool) -> V {
    let p = Trace {
        stroke: path(),
        points: array(),
    };
    let gaps = array();
    let n = len(ys);
    let mut first = 0;
    while first < n && at(ys, first).is_null() {
        first += 1
    }
    if first < n {
        let mut last = n - 1;
        while last > first && at(ys, last).is_null() {
            last -= 1
        }
        let dense = (last - first) as f64 >= x.dim * 4.0;
        let mut col = round(x.pixel(num(xs, first)));
        let mut limit = x.value(col + 1.0);
        let mut bucket: Option<Bucket> = None;
        for i in first..=last {
            let xv = num(xs, i);
            if dense && xv >= limit {
                let next = round(x.pixel(xv));
                if next != col {
                    if let Some(b) = bucket.take() {
                        b.finish(&p, col, y);
                    }
                    col = next;
                    limit = x.value(col + 1.0);
                }
            }
            if !at(ys, i).is_null() {
                let value = num(ys, i);
                if !dense {
                    p.point(round(x.pixel(xv)), round(y.pixel(value)));
                } else if let Some(b) = bucket.as_mut() {
                    b.add(value);
                } else {
                    p.point(col, round(y.pixel(value)));
                    bucket = Some(Bucket {
                        first: value,
                        last: value,
                        min: value,
                        max: value,
                    });
                }
            }
        }
        if let Some(b) = bucket {
            b.finish(&p, col, y);
        }
        if !span {
            let mut i = first;
            while i <= last {
                if at(ys, i).is_null() {
                    let from = round(x.pixel(num(xs, i - 1)));
                    while i < last && at(ys, i).is_null() {
                        i += 1
                    }
                    let to = round(x.pixel(num(xs, i)));
                    if to >= from {
                        push(gaps, coords(from, to));
                    }
                }
                i += 1
            }
        }
    }
    let width = round(ratio() * 1000.0) / 1000.0;
    let offset = (width % 2.0) / 2.0;
    save(ctx);
    put(ctx, s("lineWidth"), V::erase(width));
    put(ctx, s("lineJoin"), s("round"));
    translate(ctx, offset, offset);
    let bounds = path();
    rect(
        bounds,
        -width / 2.0,
        -width / 2.0,
        x.dim + width,
        y.dim + width,
    );
    clip(ctx, bounds);
    if len(gaps) > 0 {
        let mask = path();
        let mut end = 0.0;
        for i in 0..len(gaps) {
            let interval = at(gaps, i);
            let a = num(interval, 0);
            let b = num(interval, 1);
            if b > a {
                if a > end {
                    rect(mask, end, 0.0, a - end, y.dim);
                }
                end = b;
            }
        }
        if x.dim > end {
            rect(mask, end, -5.0, x.dim - end, y.dim + 10.0);
        }
        clip(ctx, mask);
    }
    stroke(ctx, p.stroke);
    restore(ctx);
    pair(p.points, if span { V::null() } else { gaps })
}
fn precise(v: f64) -> V {
    let n = round(v * 1e6);
    V::erase(if n == 0.0 { 0.0 } else { n })
}
fn frame(canvas: V, ctx: V, f: V, span: bool) -> V {
    let size = field(f, "size");
    let width = num(size, 0);
    let height = num(size, 1);
    put(canvas, s("width"), V::erase(round(width)));
    put(canvas, s("height"), V::erase(round(height)));
    put(ctx, s("strokeStyle"), s("#172839"));
    let x = axis(f, "xRange", width, false);
    let y = axis(f, "yRange", height, true);
    let data = field(f, "data");
    let xs = at(data, 0);
    let paths = array();
    for i in 1..len(data) {
        push(paths, render(ctx, &x, &y, xs, at(data, i), span));
    }
    let last = len(xs) as isize - 1;
    let mut lo = nearest(xs, x.min, 0, last);
    let mut hi = nearest(xs, x.max, 0, last);
    if hi > lo + 1 {
        if num(xs, lo as usize) < x.min {
            lo += 1
        }
        if num(xs, hi as usize) > x.max {
            hi -= 1
        }
    }
    let probes = field(f, "probes");
    let answers = array();
    for i in 0..len(probes) {
        let q = at(probes, i);
        let xv = num(q, 0);
        let px = num(q, 2);
        let values = array();
        push(values, precise(x.pixel(xv)));
        push(values, precise(y.pixel(num(q, 1))));
        push(values, precise(x.value(px)));
        push(values, precise(y.value(num(q, 3))));
        push(values, V::erase(nearest(xs, xv, 0, last)));
        push(values, V::erase(nearest(xs, x.value(px), lo, hi)));
        push(answers, values);
    }
    let out = pair(field(canvas, "width"), field(canvas, "height"));
    push(out, paths);
    push(out, answers);
    out
}
#[js_export]
pub fn matrix_run(input: V) -> V {
    let doc = document();
    let canvas = element(doc, s("canvas"));
    append(field(doc, "body"), canvas);
    let ctx = context(canvas, s("2d"));
    let frames = field(input, "frames");
    let result = array();
    for i in 0..len(frames) {
        push(
            result,
            frame(
                canvas,
                ctx,
                at(frames, i),
                field(input, "spanGaps").is_truthy(),
            ),
        );
    }
    remove(canvas);
    result
}
