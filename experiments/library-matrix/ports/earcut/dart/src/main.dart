import 'dart:js_interop';
import 'dart:js_interop_unsafe';

import 'earcut.dart';

extension type Input._(JSObject _) implements JSObject {
  external String get op;
  external JSArray<JSNumber> get data;
  external JSArray<JSNumber> get coords;
  external JSArray<JSNumber> get triangles;
  external JSArray<JSNumber>? get holeIndices;
  external int? get dim;
  external bool? get refine;
  external JSArray<JSArray<JSArray<JSNumber>>> get rings;
  external JSArray<Input> get items;
}

List<double> coordinates(JSArray<JSNumber> data) => [for (final x in data.toDart) x.toDartDouble];

List<int> indices(JSArray<JSNumber>? data) => [
  if (data != null)
    for (final x in data.toDart) x.toDartInt,
];

JSArray<JSNumber> output(List<int> values) {
  final result = JSArray<JSNumber>.withLength(values.length);
  for (var i = 0; i < values.length; i++) {
    result[i] = values[i].toJS;
  }
  return result;
}

JSArray<JSNumber> triangulate(Input input, bool refine) {
  final core = Earcut();
  final data = coordinates(input.data);
  final dim = input.dim ?? 2;
  final triangles = core.earcut(data, indices(input.holeIndices), dim);
  if (refine) core.refine(triangles, data, dim);
  return output(triangles);
}

JSObject flatten(JSArray<JSArray<JSArray<JSNumber>>> rings) {
  final dimensions = rings.toDart.first.toDart.first.length;
  final vertices = JSArray<JSAny?>();
  final holes = <int>[];
  var previousLength = 0;
  var offset = 0;
  for (final ring in rings.toDart) {
    for (final point in ring.toDart) {
      for (var d = 0; d < dimensions; d++) {
        // A short point contributes undefined for each absent dimension.
        vertices.add(point.getProperty<JSAny?>(d.toJS));
      }
    }
    if (previousLength > 0) {
      offset += previousLength;
      holes.add(offset);
    }
    previousLength = ring.length;
  }
  final result = JSObject();
  result['vertices'] = vertices;
  result['holes'] = output(holes);
  result['dimensions'] = dimensions.toJS;
  return result;
}

JSAny run(Input input) {
  switch (input.op) {
    case 'earcut':
      return triangulate(input, false);
    case 'earcutRefine':
      return triangulate(input, true);
    case 'refine':
      final t = indices(input.triangles);
      Earcut().refine(t, coordinates(input.coords), input.dim ?? 2);
      return output(t);
    case 'deviation':
      return Earcut()
          .deviation(
            coordinates(input.data),
            indices(input.holeIndices),
            input.dim ?? 2,
            indices(input.triangles),
          )
          .toJS;
    case 'flatten':
      return flatten(input.rings);
    case 'batch':
      final result = JSArray<JSArray<JSNumber>>();
      for (final item in input.items.toDart) {
        result.add(triangulate(item, item.refine ?? false));
      }
      return result;
    default:
      throw ArgumentError('unknown operation ${input.op}');
  }
}

void main() {
  final exports = JSObject();
  exports['run'] = run.toJS;
  globalContext['__matrixExports'] = exports;
}
