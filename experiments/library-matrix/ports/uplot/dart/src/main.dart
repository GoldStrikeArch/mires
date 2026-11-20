import 'dart:js_interop';
import 'dart:js_interop_unsafe';

@JS('devicePixelRatio')
external double get pixelRatio;

@JS('document')
external JSObject get document;

@JS('Path2D')
extension type Path._(JSObject _) implements JSObject {
  external factory Path();
}

@JS('Math.round')
external double rounded(double x);
typedef V = JSAny?;

class Scale {
  final double low, high, dim, requested;
  final bool vert;
  Scale(this.low, this.high, this.dim, this.requested, this.vert);
}

Scale scale(double a, double b, double d, bool v) => Scale(a, b, rounded(d * 2) / 2, d, v);
double pixel(Scale s, double v) {
  final pct = (v - s.low) / (s.high - s.low);
  return s.dim * (s.vert ? 1 - pct : pct);
}

double valueAt(Scale s, double v) {
  final q = s.vert ? s.requested - v : v;
  return s.low + (s.high - s.low) * (q / s.requested);
}

int half(int i) => i ~/ 2;
double real(int x) => x.toDouble();
V num(double x) => x.toJS;
V field(V o, String k) => (o as JSObject).getProperty(k.toJS);
V at(V a, int i) => (a as JSArray<JSAny?>)[i];
double numberAt(V a, int i) => (at(a, i) as JSNumber).toDartDouble;
int size(V a) => (a as JSArray<JSAny?>).length;
bool empty(V a) => a == null;
V array() => JSArray<JSAny?>();
V pair(V a, V b) => [a, b].toJS;
V gapResult(bool span, V a) => span ? null : a;
void appendValue(V a, V v) => (a as JSArray<JSAny?>).add(v);
V newPath() => Path();
V call(V obj, String name, [List<V> args = const []]) =>
    (obj as JSObject).callMethodVarArgs(name.toJS, args);
void line(V p, double x, double y) {
  call(p, 'lineTo', [x.toJS, y.toJS]);
}

void resize(V c, V ctx, double w, double h) {
  (c as JSObject)['width'] = rounded(w).toJS;
  c['height'] = rounded(h).toJS;
  (ctx as JSObject)['strokeStyle'] = '#172839'.toJS;
}

void draw(V ctx, V path, V gaps, double w, double h) {
  call(ctx, 'save');
  try {
    final width = rounded(pixelRatio * 1000) / 1000, offset = (width % 2) / 2;
    (ctx as JSObject)['lineWidth'] = width.toJS;
    ctx['lineJoin'] = 'round'.toJS;
    ctx['lineCap'] = 'butt'.toJS;
    call(ctx, 'translate', [offset.toJS, offset.toJS]);
    final bounds = newPath();
    call(bounds, 'rect', [(-width/2).toJS, (-width/2).toJS, (w+width).toJS, (h+width).toJS]);
    call(ctx, 'clip', [bounds]);
    if (size(gaps) > 0) {
      final mask = newPath();
      var end = 0.0;
      for (var i = 0; i < size(gaps); i++) {
        final g = at(gaps, i);
        final a = numberAt(g, 0), b = numberAt(g, 1);
        if (b > a) {
          if (a > end)
            call(mask, 'rect', [end.toJS, 0.toJS, (a - end).toJS, h.toJS]);
          end = b;
        }
      }
      if (w > end)
        call(mask, 'rect', [
          end.toJS,
          (-5).toJS,
          (w - end).toJS,
          (h + 10).toJS,
        ]);
      call(ctx, 'clip', [mask]);
    }
    call(ctx, 'stroke', [path]);
  } finally {
    call(ctx, 'restore');
  }
}

int nearest(V xs, double value, int lower, int upper) {
  if (size(xs) == 0) return -1;
  int lo = lower;
  int hi = upper;
  if (hi == 0) {
    hi = size(xs) - 1;
  }
  while (hi > lo + 1) {
    final int mid = half(lo + hi);
    if (numberAt(xs, mid) < value) {
      lo = mid;
    } else {
      hi = mid;
    }
  }
  if (value - numberAt(xs, lo) <= numberAt(xs, hi) - value) {
    return lo;
  }
  return hi;
}

void point(V stroke, V points, double x, double y) {
  line(stroke, x, y);
  appendValue(points, pair(num(x), num(y)));
}

void finish(
  V stroke,
  V points,
  double col,
  Scale y,
  double low,
  double high,
  double first,
  double last,
) {
  final double minY = rounded(pixel(y, low));
  final double maxY = rounded(pixel(y, high));
  final double firstY = rounded(pixel(y, first));
  final double lastY = rounded(pixel(y, last));
  if (minY != maxY) {
    if (firstY != minY && lastY != minY) {
      point(stroke, points, col, minY);
    }
    if (firstY != maxY && lastY != maxY) {
      point(stroke, points, col, maxY);
    }
    point(stroke, points, col, lastY);
  }
}

V render(V ctx, Scale x, Scale y, V xs, V ys, bool span) {
  final V stroke = newPath();
  final V points = array();
  final V gaps = array();
  final int n = size(ys);
  int first = 0;
  while (first < n && empty(at(ys, first))) {
    first += 1;
  }
  if (first < n) {
    int last = n - 1;
    while (last > first && empty(at(ys, last))) {
      last -= 1;
    }
    final bool dense = real(last - first) >= x.dim * 4.0;
    double col = rounded(pixel(x, numberAt(xs, first)));
    double limit = valueAt(x, col + 1.0);
    bool has = false;
    double startY = 0.0;
    double endY = 0.0;
    double low = 0.0;
    double high = 0.0;
    int i = first;
    while (i <= last) {
      final double xv = numberAt(xs, i);
      if (dense && xv >= limit) {
        final double next = rounded(pixel(x, xv));
        if (next != col) {
          if (has) {
            finish(stroke, points, col, y, low, high, startY, endY);
          }
          has = false;
          col = next;
          limit = valueAt(x, col + 1.0);
        }
      }
      if (!empty(at(ys, i))) {
        final double v = numberAt(ys, i);
        if (!dense) {
          point(stroke, points, rounded(pixel(x, xv)), rounded(pixel(y, v)));
        } else if (!has) {
          point(stroke, points, col, rounded(pixel(y, v)));
          has = true;
          startY = v;
          endY = v;
          low = v;
          high = v;
        } else {
          endY = v;
          if (v < low) {
            low = v;
          } else if (v > high) {
            high = v;
          }
        }
      }
      i += 1;
    }
    if (has) {
      finish(stroke, points, col, y, low, high, startY, endY);
    }
    if (!span) {
      i = first;
      while (i <= last) {
        if (empty(at(ys, i))) {
          final double left = rounded(pixel(x, numberAt(xs, i - 1)));
          while (i < last && empty(at(ys, i))) {
            i += 1;
          }
          final double right = rounded(pixel(x, numberAt(xs, i)));
          if (right >= left) {
            appendValue(gaps, pair(num(left), num(right)));
          }
        }
        i += 1;
      }
    }
  }
  draw(ctx, stroke, gaps, x.dim, y.dim);
  return pair(points, gapResult(span, gaps));
}

V precise(double v) {
  final double n = rounded(v * 1000000.0);
  if (n == 0.0) {
    return num(0.0);
  }
  return num(n);
}

V frame(V canvas, V ctx, V f, bool span) {
  final V dims = field(f, "size");
  final double w = numberAt(dims, 0);
  final double h = numberAt(dims, 1);
  resize(canvas, ctx, w, h);
  final V xr = field(f, "xRange");
  final V yr = field(f, "yRange");
  final Scale x = scale(numberAt(xr, 0), numberAt(xr, 1), w, false);
  final Scale y = scale(numberAt(yr, 0), numberAt(yr, 1), h, true);
  final V data = field(f, "data");
  final V xs = at(data, 0);
  final V paths = array();
  int i = 1;
  while (i < size(data)) {
    appendValue(paths, render(ctx, x, y, xs, at(data, i), span));
    i += 1;
  }
  int lo = nearest(xs, x.low, 0, size(xs) - 1);
  int hi = nearest(xs, x.high, 0, size(xs) - 1);
  if (hi > lo + 1) {
    if (numberAt(xs, lo) < x.low) {
      lo += 1;
    }
    if (numberAt(xs, hi) > x.high) {
      hi -= 1;
    }
  }
  final V probes = field(f, "probes");
  final V answers = array();
  i = 0;
  while (i < size(probes)) {
    final V q = at(probes, i);
    final double xv = numberAt(q, 0);
    final double px = numberAt(q, 2);
    final V values = pair(
      precise(pixel(x, xv)),
      precise(pixel(y, numberAt(q, 1))),
    );
    appendValue(values, precise(valueAt(x, px)));
    appendValue(values, precise(valueAt(y, numberAt(q, 3))));
    appendValue(values, num(real(nearest(xs, xv, 0, size(xs) - 1))));
    appendValue(values, num(real(nearest(xs, valueAt(x, px), lo, hi))));
    appendValue(answers, values);
    i += 1;
  }
  final V result = pair(field(canvas, "width"), field(canvas, "height"));
  appendValue(result, paths);
  appendValue(result, answers);
  return result;
}

V run(JSObject input) {
  final c = call(document, 'createElement', ['canvas'.toJS]);
  call(field(document, 'body'), 'appendChild', [c]);
  try {
    final ctx = call(c, 'getContext', ['2d'.toJS]);
    final frames = field(input, 'frames');
    final out = array();
    for (var i = 0; i < size(frames); i++) {
      appendValue(
        out,
        frame(
          c,
          ctx,
          at(frames, i),
          (field(input, 'spanGaps') as JSBoolean).toDart,
        ),
      );
    }
    return out;
  } finally {
    call(c, 'remove');
  }
}

void main() {
  final exports = JSObject();
  exports['run'] = run.toJS;
  globalContext['__matrixExports'] = exports;
}
