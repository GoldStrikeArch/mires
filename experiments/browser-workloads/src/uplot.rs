//! Canvas time-series workload inspired by uPlot.

use mires_js::JsValue;

use crate::host::{call, get, index, length, object, parse_float, set};

/// Scales numeric arrays and paints a line series on a canvas.
#[must_use]
pub fn render(options: JsValue) -> JsValue {
    render_series(get(options, "canvas"), get(options, "xs"), get(options, "ys"))
}

#[must_use]
pub fn render_series(canvas: JsValue, xs: JsValue, ys: JsValue) -> JsValue {
    let count = length(xs).min(length(ys));
    let summary = object();
    if count == 0 {
        set(summary, "points", JsValue::erase(0.0));
        return summary;
    }
    let mut x_min = parse_float(index(xs, 0));
    let mut x_max = x_min;
    let mut y_min = parse_float(index(ys, 0));
    let mut y_max = y_min;
    for point in 1..count {
        let x = parse_float(index(xs, point));
        let y = parse_float(index(ys, point));
        x_min = x_min.min(x);
        x_max = x_max.max(x);
        y_min = y_min.min(y);
        y_max = y_max.max(y);
    }

    let width = number_or(get(canvas, "clientWidth"), 640.0).max(1.0);
    let height = number_or(get(canvas, "clientHeight"), 320.0).max(1.0);
    set(canvas, "width", JsValue::erase(width));
    set(canvas, "height", JsValue::erase(height));
    let context = call(canvas, "getContext", &[JsValue::erase("2d")]);
    let _ = call(context, "clearRect", &[JsValue::erase(0.0), JsValue::erase(0.0), JsValue::erase(width), JsValue::erase(height)]);
    let _ = call(context, "beginPath", &[]);
    set(context, "strokeStyle", JsValue::erase("#2563eb"));
    set(context, "lineWidth", JsValue::erase(1.5));
    let x_span = (x_max - x_min).max(f64::EPSILON);
    let y_span = (y_max - y_min).max(f64::EPSILON);
    let mut checksum = 0.0;
    for point in 0..count {
        let x = (parse_float(index(xs, point)) - x_min) / x_span * width;
        let y = height - (parse_float(index(ys, point)) - y_min) / y_span * height;
        let method = if point == 0 { "moveTo" } else { "lineTo" };
        let _ = call(context, method, &[JsValue::erase(x), JsValue::erase(y)]);
        checksum += x * 0.25 + y * 0.75;
    }
    let _ = call(context, "stroke", &[]);
    set(summary, "points", JsValue::erase(count as f64));
    set(summary, "xMin", JsValue::erase(x_min));
    set(summary, "xMax", JsValue::erase(x_max));
    set(summary, "yMin", JsValue::erase(y_min));
    set(summary, "yMax", JsValue::erase(y_max));
    set(summary, "checksum", JsValue::erase(checksum));
    summary
}

fn number_or(value: JsValue, fallback: f64) -> f64 {
    if value.is_number() { value.read() } else { fallback }
}
