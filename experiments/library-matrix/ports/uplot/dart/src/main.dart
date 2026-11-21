// The boundary: what libraries/uplot/reference.mjs does, on the port's uPlot slice. Extension
// types view the JS input; each frame's arrays become Dart lists, and the result is converted back
// with toJS. main() publishes `run` for the comparator facade.
import 'dart:js_interop';
import 'dart:js_interop_unsafe';

import 'dom.dart';
import 'uplot.dart';

extension type Input._(JSObject _) implements JSObject {
  external bool get spanGaps;
  external JSArray<Frame> get frames;
}

extension type Frame._(JSObject _) implements JSObject {
  external JSArray<JSNumber> get size;
  external JSArray<JSNumber> get xRange;
  external JSArray<JSNumber> get yRange;

  /// The aligned data: x values, then one array per series with null for a missing value.
  external JSArray<JSArray<JSNumber?>> get data;
  external JSArray<JSArray<JSNumber>> get probes;
}

List<double> _doubles(JSArray<JSNumber> values) => [for (final v in values.toDart) v.toDartDouble];

JSArray<JSNumber> _pair((double, double) p) => [p.$1.toJS, p.$2.toJS].toJS;

/// The adapter's precise(): Math.round(x * 1e6) || 0, which maps -0 and NaN to 0.
double _precise(double value) {
  final n = jsRound(value * 1e6);
  return n == 0 || n.isNaN ? 0 : n;
}

/// One frame: setSize, setData and setScale in a batch, then the recorded paths and the probes.
JSArray _frame(Canvas canvas, Context2D ctx, bool spanGaps, Frame f, double pxRatio, ScaleState state) {
  final size = _doubles(f.size), xRange = _doubles(f.xRange), yRange = _doubles(f.yRange);
  // The adapter passes CSS sizes; uPlot multiplies them back by pxRatio.
  final widthCss = size[0] / pxRatio, heightCss = size[1] / pxRatio;
  final plot = Plot(pxRatio, widthCss, heightCss);
  canvas
    ..width = jsRound(widthCss * pxRatio)
    ..height = jsRound(heightCss * pxRatio);

  final columns = f.data.toDart;
  // x values are never null.
  final xs = [for (final v in columns.first.toDart) v!.toDartDouble];
  final chart = state.applyScales(plot, Scale(xRange[0], xRange[1]), Scale(yRange[0], yRange[1]), xs);
  final series = [
    for (final column in columns.skip(1)) [for (final v in column.toDart) v?.toDartDouble]
  ];

  // drawSeries runs only with data; the adapter then reports an empty trace for each series.
  final paths = xs.isEmpty ? null : [for (final ys in series) linear(chart, ys, spanGaps)];
  for (final p in paths ?? const <SeriesPaths>[]) {
    drawPath(ctx, plot, p);
  }
  JSArray trace(int i) {
    final p = paths?[i];
    if (p == null) return [JSArray(), spanGaps ? null : JSArray()].toJS;
    return [p.stroke.vertices.map(_pair).toList().toJS, p.gaps?.map(_pair).toList().toJS].toJS;
  }

  final traces = [for (var i = 0; i < series.length; i++) trace(i)];

  final probes = <JSArray>[];
  for (final probe in f.probes.toDart) {
    final [x, y, px, py] = _doubles(probe);
    probes.add([
      _precise(chart.valToPosX(x)).toJS,
      _precise(chart.valToPosY(y)).toJS,
      _precise(chart.posToValX(px)).toJS,
      _precise(chart.posToValY(py)).toJS,
      chart.valToIdx(x).toJS,
      chart.posToIdx(px).toJS,
    ].toJS);
  }
  return [canvas.width.toJS, canvas.height.toJS, traces.toJS, probes.toJS].toJS;
}

JSArray<JSArray> run(Input input) {
  final pxRatio = devicePixelRatio;
  // The canvas stands in for uPlot's elements in the adapter's host div; both leave the page.
  final canvas = document.createElement('canvas');
  document.body.appendChild(canvas);
  try {
    final ctx = canvas.getContext('2d');
    final state = ScaleState();
    return [for (final f in input.frames.toDart) _frame(canvas, ctx, input.spanGaps, f, pxRatio, state)].toJS;
  } finally {
    canvas.remove();
  }
}

void main() {
  final exports = JSObject();
  exports['run'] = run.toJS;
  globalContext['__matrixExports'] = exports;
}
