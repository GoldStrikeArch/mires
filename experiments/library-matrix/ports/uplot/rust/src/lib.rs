#![no_std]
#![no_main]
#![deny(unsafe_code)]
use mires_js::{JsValue as V, js_bindings, js_export, throw_value, try_host};
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
    #[js(call="Math.floor")] fn floor(x:f64)->f64;
    #[js(call="Math.ceil")] fn ceil(x:f64)->f64;
    #[js(call="Math.pow")] fn pow(x:f64,y:f64)->f64;
    #[js(call="Math.log10")] fn log10(x:f64)->f64;
    #[js(call="Number.isInteger")] fn is_int(x:f64)->bool;
    #[js(call="String")] fn string(x:f64)->V;
    #[js(call="Number")] fn parse_num(x:V)->f64;
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
// uPlot's roundDec: rounds half away from zero at `dec` decimals.
fn round_dec(v: f64, dec: i32) -> f64 {
    if is_int(v) {
        return v;
    }
    let p = pow(10.0, dec as f64);
    round(v * p * (1.0 + f64::EPSILON)) / p
}
// uPlot's regex6, /\.\d*?(?=9{6,}|0{6,})/: the number of digits between the point and the first
// run of six or more 9s or 0s, if there is one.
fn decimals_before_run(text: &str) -> Option<usize> {
    let digits = &text[text.find('.')? + 1..];
    let digits = &digits[..digits.find(|c: char| !c.is_ascii_digit()).unwrap_or(digits.len())];
    (0..digits.len()).find(|&k| digits[k..].starts_with("999999") || digits[k..].starts_with("000000"))
}
// uPlot's fixFloat: snaps a value whose JS string has a run of six or more 9s or 0s after the
// point to the decimals before the run, handling "e-" exponent strings by rounding the mantissa.
// uPlot first returns members of its fixedDec map unchanged; the string path returns every member
// unchanged too.
fn fix_float(v: f64) -> f64 {
    if is_int(v) {
        return v;
    }
    let text_v = string(v);
    let text = text_v.read::<&str>();
    let Some(dec) = decimals_before_run(text) else {
        return v;
    };
    if let Some(e_pos) = text.find("e-") {
        let mut exp_buf = [0u8; 16];
        let exp_bytes = &text.as_bytes()[e_pos..];
        let exp_len = exp_bytes.len();
        if exp_len <= exp_buf.len() {
            exp_buf[..exp_len].copy_from_slice(exp_bytes);
            let mant = parse_num(s(&text[..e_pos]));
            let rounded_v = string(round_dec(mant, dec as i32));
            let rounded = rounded_v.read::<&str>();
            let mut buf = [0u8; 64];
            let n = rounded.len();
            if n + exp_len <= buf.len() {
                buf[..n].copy_from_slice(rounded.as_bytes());
                buf[n..n + exp_len].copy_from_slice(&exp_buf[..exp_len]);
                if let Ok(joined) = core::str::from_utf8(&buf[..n + exp_len]) {
                    return parse_num(s(joined));
                }
            }
        }
    }
    round_dec(v, dec as i32)
}
fn incr_round(v: f64, incr: f64) -> f64 {
    fix_float(round_dec(fix_float(v / incr), 0) * incr)
}
fn incr_round_up(v: f64, incr: f64) -> f64 {
    fix_float(ceil(fix_float(v / incr)) * incr)
}
fn incr_round_dn(v: f64, incr: f64) -> f64 {
    fix_float(floor(fix_float(v / incr)) * incr)
}
fn js_min(a: f64, b: f64) -> f64 {
    if a.is_nan() || b.is_nan() {
        f64::NAN
    } else if a < b {
        a
    } else {
        b
    }
}
fn js_max(a: f64, b: f64) -> f64 {
    if a.is_nan() || b.is_nan() {
        f64::NAN
    } else if a > b {
        a
    } else {
        b
    }
}
// uPlot's rangeNum(min, max, 0.1, true): used by autoScaleX when all x values are equal.
fn range_num(min: f64, max: f64) -> (f64, f64) {
    let mut delta = max - min;
    let scalar_max = js_max(min.abs(), max.abs());
    if delta < 1e-24 || (log10(scalar_max) - log10(delta)).abs() > 10.0 {
        delta = 0.0;
        if min == 0.0 || max == 0.0 {
            delta = 1e-24;
        }
    }
    let non_zero_delta = if delta != 0.0 {
        delta
    } else if scalar_max != 0.0 {
        scalar_max
    } else {
        1e3
    };
    let base = pow(10.0, floor(log10(non_zero_delta)));
    let pad_min = non_zero_delta * if delta == 0.0 { if min == 0.0 { 0.1 } else { 1.0 } } else { 0.1 };
    let new_min = round_dec(incr_round_dn(min - pad_min, base / 10.0), 24);
    let soft_min = if min >= 0.0 && new_min <= 0.0 { 0.0 } else { f64::INFINITY };
    let min_lim = if new_min < soft_min && min >= soft_min {
        soft_min
    } else {
        js_min(soft_min, new_min)
    };
    let pad_max = non_zero_delta * if delta == 0.0 { if max == 0.0 { 0.1 } else { 1.0 } } else { 0.1 };
    let new_max = round_dec(incr_round_up(max + pad_max, base / 10.0), 24);
    let soft_max = if max <= 0.0 && new_max >= 0.0 { 0.0 } else { f64::NEG_INFINITY };
    let mut max_lim = if new_max > soft_max && max <= soft_max {
        soft_max
    } else {
        js_max(soft_max, new_max)
    };
    if min_lim == max_lim && min_lim == 0.0 {
        max_lim = 100.0;
    }
    (min_lim, max_lim)
}
// One axis of the plot: the scale range, calcSize's offset and size in canvas pixels, and the
// CSS size uPlot was given (the adapter divides the physical size by pxRatio).
struct Scale {
    min: f64,
    max: f64,
    off: f64,
    dim: f64,
    css: f64,
    ratio: f64,
    vertical: bool,
}
impl Scale {
    // getHPos / getVPos (dir 1) in canvas pixels, keeping uPlot's `off +`, which turns -0 into 0.
    fn pixel(&self, v: f64) -> f64 {
        let pct = (v - self.min) / (self.max - self.min);
        self.off + self.dim * (if self.vertical { 1.0 - pct } else { pct })
    }
    // posToVal(p, key, true): the CSS position (the plot's CSS offset is 0), from the bottom for y.
    fn value(&self, p: f64) -> f64 {
        let pos = p / self.ratio;
        let q = if self.vertical { self.css - pos } else { pos };
        self.min + (self.max - self.min) * (q / self.css)
    }
}
fn axis(range: (f64, f64), css: f64, ratio: f64, vertical: bool) -> Scale {
    Scale {
        min: range.0,
        max: range.1,
        off: incr_round(0.0 * ratio, 0.5),
        dim: incr_round(css * ratio, 0.5),
        css,
        ratio,
        vertical,
    }
}
// Persistent scale bounds across frames for uPlot's setScale (< 1e-16) guard.
struct ScaleState {
    x: Option<(f64, f64)>,
    y: Option<(f64, f64)>,
}
impl ScaleState {
    fn new() -> Self {
        Self { x: None, y: None }
    }
    // Mirrors setData -> autoScaleX, explicit setScale('x'/'y'), and commit -> setScales.
    // Returns the active (x, y) bounds (None behaves as (0.0, 0.0), matching JS null arithmetic)
    // and the visible index window (lo, hi).
    fn apply(&mut self, xs: V, x_req: (f64, f64), y_req: (f64, f64)) -> ((f64, f64), (f64, f64), isize, isize) {
        let n = len(xs);
        let (mut lo, mut hi) = if n > 0 { (0, n as isize - 1) } else { (0, 0) };
        let mut pend_x = if n <= 1 || !(x_req.1 - x_req.0 < 1e-16) {
            true
        } else {
            let mut a_min = num(xs, 0);
            let mut a_max = num(xs, n - 1);
            if a_min == a_max {
                let (r_min, r_max) = range_num(a_min, a_max);
                a_min = r_min;
                a_max = r_max;
            }
            if a_min > a_max {
                core::mem::swap(&mut a_min, &mut a_max);
            }
            !(a_max - a_min < 1e-16)
        };
        let mut pend_y = !(n > 1 && y_req.1 - y_req.0 < 1e-16);
        if pend_x || pend_y {
            if !pend_x && self.x.is_none() {
                pend_x = true;
            }
            if !pend_y && (self.y.is_none() || pend_x) {
                pend_y = true;
            }
            if pend_x {
                self.x = Some(x_req);
                if n > 0 {
                    let last = n as isize - 1;
                    lo = nearest(xs, x_req.0, 0, last);
                    hi = nearest(xs, x_req.1, 0, last);
                    if hi > lo + 1 {
                        if num(xs, lo as usize) < x_req.0 {
                            lo += 1;
                        }
                        if num(xs, hi as usize) > x_req.1 {
                            hi -= 1;
                        }
                    }
                }
            }
            if pend_y {
                self.y = Some(y_req);
            }
        }
        (self.x.unwrap_or((0.0, 0.0)), self.y.unwrap_or((0.0, 0.0)), lo, hi)
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
            if dense && !(xv < limit) {
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
    // drawPath: the series width 1 in canvas pixels, the half-pixel translate, the bounds clip
    // and clipGaps' mask.
    let width = round_dec(x.ratio, 3);
    let offset = (width % 2.0) / 2.0;
    if offset > 0.0 {
        translate(ctx, offset, offset);
    }
    let bounds = path();
    rect(
        bounds,
        x.off - width / 2.0,
        y.off - width / 2.0,
        x.dim + width,
        y.dim + width,
    );
    put(ctx, s("strokeStyle"), s("#172839"));
    put(ctx, s("lineWidth"), V::erase(width));
    put(ctx, s("lineJoin"), s("round"));
    put(ctx, s("lineCap"), s("butt"));
    save(ctx);
    clip(ctx, bounds);
    if len(gaps) > 0 {
        let mask = path();
        let mut end = x.off;
        for i in 0..len(gaps) {
            let interval = at(gaps, i);
            let a = num(interval, 0);
            let b = num(interval, 1);
            if b > a {
                if a - end > 0.0 {
                    rect(mask, end, y.off, a - end, y.off + y.dim);
                }
                end = b;
            }
        }
        let rest = x.off + x.dim - end;
        if rest > 0.0 {
            rect(mask, end, y.off - 5.0, rest, y.off + y.dim + 10.0);
        }
        clip(ctx, mask);
    }
    if width > 0.0 {
        stroke(ctx, p.stroke);
    }
    restore(ctx);
    if offset > 0.0 {
        translate(ctx, -offset, -offset);
    }
    pair(p.points, if span { V::null() } else { gaps })
}
// The adapter's precise(): Math.round(x * 1e6) || 0, which maps -0 and NaN to 0.
fn precise(v: f64) -> V {
    let n = round(v * 1e6);
    V::erase(if n == 0.0 || n.is_nan() { 0.0 } else { n })
}
fn frame(canvas: V, ctx: V, f: V, span: bool, ratio: f64, state: &mut ScaleState) -> V {
    // The adapter passes CSS sizes; uPlot multiplies them back by pxRatio.
    let size = field(f, "size");
    let width = num(size, 0) / ratio;
    let height = num(size, 1) / ratio;
    put(canvas, s("width"), V::erase(round(width * ratio)));
    put(canvas, s("height"), V::erase(round(height * ratio)));
    let data = field(f, "data");
    let xs = at(data, 0);
    let xr = field(f, "xRange");
    let yr = field(f, "yRange");
    let (x_bounds, y_bounds, lo, hi) = state.apply(xs, (num(xr, 0), num(xr, 1)), (num(yr, 0), num(yr, 1)));
    let x = axis(x_bounds, width, ratio, false);
    let y = axis(y_bounds, height, ratio, true);
    let paths = array();
    for i in 1..len(data) {
        // drawSeries runs only with data; the adapter then reports an empty trace.
        push(
            paths,
            if len(xs) > 0 {
                render(ctx, &x, &y, xs, at(data, i), span)
            } else {
                pair(array(), if span { V::null() } else { array() })
            },
        );
    }
    let last = len(xs) as isize - 1;
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
fn frames(input: V, canvas: V) -> V {
    let ctx = context(canvas, s("2d"));
    let ratio = ratio();
    let span = field(input, "spanGaps").is_truthy();
    let frames = field(input, "frames");
    let result = array();
    let mut state = ScaleState::new();
    for i in 0..len(frames) {
        push(result, frame(canvas, ctx, at(frames, i), span, ratio, &mut state));
    }
    result
}
#[js_export]
pub fn matrix_run(input: V) -> V {
    let doc = document();
    let canvas = element(doc, s("canvas"));
    append(field(doc, "body"), canvas);
    // The reference removes its host in `finally`. try_host catches a throw, including a panic
    // (the panic handler throws), so the canvas leaves the page before the exception does.
    let result = try_host(|| frames(input, canvas));
    remove(canvas);
    result.unwrap_or_else(|error| throw_value(error))
}
