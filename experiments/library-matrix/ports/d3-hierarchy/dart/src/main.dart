// Entry of the Dart port: one layout per input, as libraries/d3-hierarchy/reference.mjs runs it.
// The input is read in place through dart:js_interop extension types; the rows are JS arrays of
// strings, numbers and null. dart2js emits a classic script, so main() publishes `run` on
// globalThis.__matrixExports for the builder's facade.

import 'dart:js_interop';
import 'dart:js_interop_unsafe';

import 'hierarchy.dart';
import 'pack.dart';
import 'stratify.dart';
import 'tree.dart';
import 'treemap.dart';

extension type Input._(JSObject _) implements JSObject {
  external String get op;
  external String get source;
  external Datum? get data;
  external JSArray<Datum>? get rows;
  external double get width;
  external double get height;
  external double get padding;
  external bool get sort;
  external bool get round;
  external bool get nodeSize;
  external String get tile;
}

Tile _tile(String name) => switch (name) {
      'binary' => treemapBinary,
      'sliceDice' => treemapSliceDice,
      'squarify' => treemapSquarify,
      _ => throw ArgumentError('unknown tile'), // `.tile(undefined)` throws
    };

/// The reference's `n.id ?? n.data.id`.
String? _id(Node node) => node.id ?? node.data!.id;

JSArray<JSArray<JSAny?>> run(Input input) {
  final root = switch (input.source) {
    'id' => stratify(input.rows!.toDart),
    'path' => stratify(input.rows!.toDart, (d) => d.id!),
    _ => hierarchy(input.data!),
  };
  root.sum((d) => d?.value ?? 0);
  if (input.sort) root.sort((a, b) => b.value - a.value);
  final w = input.width, h = input.height, p = input.padding;
  switch (input.op) {
    case 'tree':
      tree(root, w, h, input.nodeSize);
    case 'cluster':
      cluster(root, w, h, input.nodeSize);
    case 'partition':
      partition(root, w, h, p, input.round);
    case 'pack':
      pack(root, w, h, p);
    case 'treemap':
      treemap(root, w, h, _tile(input.tile), p, input.round);
    case 'hierarchy' || 'iterator' || 'stratify':
      break;
    default:
      throw ArgumentError('unknown layout');
  }
  final output = <JSArray<JSAny?>>[];
  void visit(Node n) {
    final parent = n.parent;
    final record = <JSAny?>[
      _id(n)?.toJS,
      parent == null ? null : _id(parent)?.toJS,
      n.depth.toJS,
      n.height.toJS,
      n.value.toJS,
      ...switch (input.op) {
        'tree' || 'cluster' => [n.x.toJS, n.y.toJS],
        'pack' => [n.x.toJS, n.y.toJS, n.r.toJS],
        'partition' || 'treemap' => [n.x0.toJS, n.y0.toJS, n.x1.toJS, n.y1.toJS],
        _ => const <JSAny?>[],
      },
    ];
    output.add(record.toJS);
  }

  if (input.op == 'iterator') {
    root.breadthFirst().forEach(visit);
  } else {
    root.eachBefore(visit);
  }
  return output.toJS;
}

void main() {
  final exports = JSObject();
  exports['run'] = run.toJS;
  globalContext['__matrixExports'] = exports;
}
