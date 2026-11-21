// treemap/ (index.js, dice.js, slice.js, sliceDice.js, squarify.js, binary.js, round.js) and
// partition.js of d3-hierarchy 3.1.3.

import 'dart:js_interop';
import 'dart:math' as math;

import 'hierarchy.dart';

/// A tiling method: lays the children of a node out in the rectangle [x0, y0, x1, y1].
typedef Tile = void Function(Node parent, double x0, double y0, double x1, double y1);

/// Math.round, which rounds halves up, keeps -0 and returns -0 for [-0.5, 0).
@JS('Math.round')
external double _round(double x);

/// round.js.
void _roundNode(Node node) {
  node.x0 = _round(node.x0);
  node.y0 = _round(node.y0);
  node.x1 = _round(node.x1);
  node.y1 = _round(node.y1);
}

/// dice.js on the children [nodes] of a parent, or of a squarified row, whose value is [value]:
/// columns from left to right.
void treemapDice(List<Node> nodes, double value, double x0, double y0, double x1, double y1) {
  final k = truthy(value) ? (x1 - x0) / value : value;
  for (final node in nodes) {
    node.y0 = y0;
    node.y1 = y1;
    node.x0 = x0;
    node.x1 = x0 += node.value * k;
  }
}

/// slice.js, as [treemapDice]: rows from top to bottom.
void treemapSlice(List<Node> nodes, double value, double x0, double y0, double x1, double y1) {
  final k = truthy(value) ? (y1 - y0) / value : value;
  for (final node in nodes) {
    node.x0 = x0;
    node.x1 = x1;
    node.y0 = y0;
    node.y1 = y0 += node.value * k;
  }
}

/// sliceDice.js: slice at odd depths, dice at even ones.
void treemapSliceDice(Node parent, double x0, double y0, double x1, double y1) =>
    ((parent.depth & 1) != 0 ? treemapSlice : treemapDice)(parent.children!, parent.value, x0, y0, x1, y1);

final _phi = (1 + math.sqrt(5)) / 2;

/// squarify.js with the default ratio, the golden ratio.
void treemapSquarify(Node parent, double x0, double y0, double x1, double y1) =>
    _squarifyRatio(_phi, parent, x0, y0, x1, y1);

/// `squarifyRatio`. d3 also collects the rows, for treemapResquarify; this port only lays them
/// out.
void _squarifyRatio(double ratio, Node parent, double x0, double y0, double x1, double y1) {
  final nodes = parent.children!, n = nodes.length;
  var i0 = 0, i1 = 0;
  var value = parent.value;

  while (i0 < n) {
    final dx = x1 - x0, dy = y1 - y0;

    // Find the next non-empty node.
    double sumValue;
    do {
      sumValue = nodes[i1++].value;
    } while (!truthy(sumValue) && i1 < n);
    var minValue = sumValue, maxValue = sumValue;
    final alpha = math.max(dy / dx, dx / dy) / (value * ratio);
    var beta = sumValue * sumValue * alpha;
    var minRatio = math.max(maxValue / beta, beta / minValue);

    // Keep adding nodes while the aspect ratio maintains or improves.
    for (; i1 < n; ++i1) {
      final nodeValue = nodes[i1].value;
      sumValue += nodeValue;
      if (nodeValue < minValue) minValue = nodeValue;
      if (nodeValue > maxValue) maxValue = nodeValue;
      beta = sumValue * sumValue * alpha;
      final newRatio = math.max(maxValue / beta, beta / minValue);
      if (newRatio > minRatio) {
        sumValue -= nodeValue;
        break;
      }
      minRatio = newRatio;
    }

    // Position the row in its orientation.
    final row = nodes.sublist(i0, i1);
    if (dx < dy) {
      treemapDice(row, sumValue, x0, y0, x1, truthy(value) ? y0 += dy * sumValue / value : y1);
    } else {
      treemapSlice(row, sumValue, x0, y0, truthy(value) ? x0 += dx * sumValue / value : x1, y1);
    }
    value -= sumValue;
    i0 = i1;
  }
}

/// binary.js: splits the children in two of about equal value, recursively.
void treemapBinary(Node parent, double x0, double y0, double x1, double y1) {
  final nodes = parent.children!, n = nodes.length;
  final sums = List<double>.filled(n + 1, 0);
  double sum = 0;
  for (var i = 0; i < n; ++i) {
    sums[i + 1] = sum += nodes[i].value;
  }

  void partition(int i, int j, double value, double x0, double y0, double x1, double y1) {
    if (i >= j - 1) {
      final node = nodes[i];
      node.x0 = x0;
      node.y0 = y0;
      node.x1 = x1;
      node.y1 = y1;
      return;
    }

    final valueOffset = sums[i], valueTarget = value / 2 + valueOffset;
    var k = i + 1, hi = j - 1;
    while (k < hi) {
      final mid = (k + hi) >>> 1;
      if (sums[mid] < valueTarget) {
        k = mid + 1;
      } else {
        hi = mid;
      }
    }
    if (valueTarget - sums[k - 1] < sums[k] - valueTarget && i + 1 < k) --k;

    final valueLeft = sums[k] - valueOffset, valueRight = value - valueLeft;
    if (x1 - x0 > y1 - y0) {
      final xk = truthy(value) ? (x0 * valueRight + x1 * valueLeft) / value : x1;
      partition(i, k, valueLeft, x0, y0, xk, y1);
      partition(k, j, valueRight, xk, y0, x1, y1);
    } else {
      final yk = truthy(value) ? (y0 * valueRight + y1 * valueLeft) / value : y1;
      partition(i, k, valueLeft, x0, y0, x1, yk);
      partition(k, j, valueRight, x0, yk, x1, y1);
    }
  }

  partition(0, n, parent.value, x0, y0, x1, y1);
}

/// `treemap().size([dx, dy]).tile(tile).padding(padding).round(round)`: the inner and every outer
/// padding are [padding].
Node treemap(Node root, double dx, double dy, Tile tile, double padding, bool round) {
  // d3 keeps the stack between calls and resets it to [0]; depth d reads index d.
  final paddingStack = List<double>.filled(root.height + 1, 0);
  root
    ..x0 = 0
    ..y0 = 0
    ..x1 = dx
    ..y1 = dy;
  root.eachBefore((node) {
    var p = paddingStack[node.depth];
    var x0 = node.x0 + p, y0 = node.y0 + p, x1 = node.x1 - p, y1 = node.y1 - p;
    if (x1 < x0) x0 = x1 = (x0 + x1) / 2;
    if (y1 < y0) y0 = y1 = (y0 + y1) / 2;
    node
      ..x0 = x0
      ..y0 = y0
      ..x1 = x1
      ..y1 = y1;
    if (node.children != null) {
      p = paddingStack[node.depth + 1] = padding / 2;
      x0 += padding - p;
      y0 += padding - p;
      x1 -= padding - p;
      y1 -= padding - p;
      if (x1 < x0) x0 = x1 = (x0 + x1) / 2;
      if (y1 < y0) y0 = y1 = (y0 + y1) / 2;
      tile(node, x0, y0, x1, y1);
    }
  });
  if (round) root.eachBefore(_roundNode);
  return root;
}

/// `partition().size([dx, dy]).padding(padding).round(round)`.
Node partition(Node root, double dx, double dy, double padding, bool round) {
  final n = root.height + 1;
  root
    ..x0 = padding
    ..y0 = padding
    ..x1 = dx
    ..y1 = dy / n;
  root.eachBefore((node) {
    final children = node.children;
    if (children != null) {
      treemapDice(children, node.value, node.x0, dy * (node.depth + 1) / n, node.x1,
          dy * (node.depth + 2) / n);
    }
    var x0 = node.x0, y0 = node.y0, x1 = node.x1 - padding, y1 = node.y1 - padding;
    if (x1 < x0) x0 = x1 = (x0 + x1) / 2;
    if (y1 < y0) y0 = y1 = (y0 + y1) / 2;
    node
      ..x0 = x0
      ..y0 = y0
      ..x1 = x1
      ..y1 = y1;
  });
  if (round) root.eachBefore(_roundNode);
  return root;
}
