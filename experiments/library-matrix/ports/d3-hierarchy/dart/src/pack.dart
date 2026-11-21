// pack/ (index.js, siblings.js, enclose.js), lcg.js and array.js's shuffle of d3-hierarchy 3.1.3.

import 'dart:math' as math;

import 'hierarchy.dart';

// lcg.js: https://en.wikipedia.org/wiki/Linear_congruential_generator#Parameters_in_common_use
const _a = 1664525.0;
const _c = 1013904223.0;
const _m = 4294967296.0; // 2^32

/// lcg.js: a seeded generator of [0, 1). Every product stays below 2^53, so doubles are exact.
double Function() _lcg() {
  var s = 1.0;
  return () => (s = (_a * s + _c) % _m) / _m;
}

/// array.js's `shuffle`: Fisher-Yates from the end, with `random() * m-- | 0`.
List<T> _shuffle<T>(List<T> array, double Function() random) {
  var m = array.length;
  while (m != 0) {
    final i = (random() * m--).toInt();
    final t = array[m];
    array[m] = array[i];
    array[i] = t;
  }
  return array;
}

/// A circle that `encloseBasis` computes.
class _Enclosure implements Circle {
  @override
  final double x, y, r;
  _Enclosure(this.x, this.y, this.r);
}

/// `packEncloseRandom`: the smallest circle enclosing [circles], by Welzl's algorithm.
Circle _packEncloseRandom(List<Circle> circles, double Function() random) {
  final shuffled = _shuffle([...circles], random);
  final n = shuffled.length;
  var i = 0;
  var basis = <Circle>[];
  Circle? e;
  while (i < n) {
    final p = shuffled[i];
    if (e != null && _enclosesWeak(e, p)) {
      ++i;
    } else {
      e = _encloseBasis(basis = _extendBasis(basis, p));
      i = 0;
    }
  }
  return e!;
}

List<Circle> _extendBasis(List<Circle> b, Circle p) {
  if (_enclosesWeakAll(p, b)) return [p];

  // If we get here then B must have at least one element.
  for (var i = 0; i < b.length; ++i) {
    if (_enclosesNot(p, b[i]) && _enclosesWeakAll(_encloseBasis2(b[i], p), b)) return [b[i], p];
  }

  // If we get here then B must have at least two elements.
  for (var i = 0; i < b.length - 1; ++i) {
    for (var j = i + 1; j < b.length; ++j) {
      if (_enclosesNot(_encloseBasis2(b[i], b[j]), p) &&
          _enclosesNot(_encloseBasis2(b[i], p), b[j]) &&
          _enclosesNot(_encloseBasis2(b[j], p), b[i]) &&
          _enclosesWeakAll(_encloseBasis3(b[i], b[j], p), b)) {
        return [b[i], b[j], p];
      }
    }
  }

  // If we get here then something is very wrong.
  throw StateError('no enclosing basis');
}

bool _enclosesNot(Circle a, Circle b) {
  final dr = a.r - b.r, dx = b.x - a.x, dy = b.y - a.y;
  return dr < 0 || dr * dr < dx * dx + dy * dy;
}

bool _enclosesWeak(Circle a, Circle b) {
  final dr = a.r - b.r + math.max(math.max(a.r, b.r), 1.0) * 1e-9, dx = b.x - a.x, dy = b.y - a.y;
  return dr > 0 && dr * dr > dx * dx + dy * dy;
}

bool _enclosesWeakAll(Circle a, List<Circle> b) => b.every((p) => _enclosesWeak(a, p));

Circle _encloseBasis(List<Circle> b) => switch (b) {
      [final a] => _Enclosure(a.x, a.y, a.r),
      [final a, final b] => _encloseBasis2(a, b),
      [final a, final b, final c] => _encloseBasis3(a, b, c),
      _ => throw StateError('no enclosing basis'),
    };

Circle _encloseBasis2(Circle a, Circle b) {
  final x1 = a.x, y1 = a.y, r1 = a.r;
  final x2 = b.x, y2 = b.y, r2 = b.r;
  final x21 = x2 - x1, y21 = y2 - y1, r21 = r2 - r1;
  final l = math.sqrt(x21 * x21 + y21 * y21);
  return _Enclosure((x1 + x2 + x21 / l * r21) / 2, (y1 + y2 + y21 / l * r21) / 2, (l + r1 + r2) / 2);
}

Circle _encloseBasis3(Circle a, Circle b, Circle c) {
  final x1 = a.x, y1 = a.y, r1 = a.r;
  final x2 = b.x, y2 = b.y, r2 = b.r;
  final x3 = c.x, y3 = c.y, r3 = c.r;
  final a2 = x1 - x2,
      a3 = x1 - x3,
      b2 = y1 - y2,
      b3 = y1 - y3,
      c2 = r2 - r1,
      c3 = r3 - r1,
      d1 = x1 * x1 + y1 * y1 - r1 * r1,
      d2 = d1 - x2 * x2 - y2 * y2 + r2 * r2,
      d3 = d1 - x3 * x3 - y3 * y3 + r3 * r3,
      ab = a3 * b2 - a2 * b3,
      xa = (b2 * d3 - b3 * d2) / (ab * 2) - x1,
      xb = (b3 * c2 - b2 * c3) / ab,
      ya = (a3 * d2 - a2 * d3) / (ab * 2) - y1,
      yb = (a2 * c3 - a3 * c2) / ab,
      qa = xb * xb + yb * yb - 1,
      qb = 2 * (r1 + xa * xb + ya * yb),
      qc = xa * xa + ya * ya - r1 * r1,
      r = -(qa.abs() > 1e-6 ? (qb + math.sqrt(qb * qb - 4 * qa * qc)) / (2 * qa) : qc / qb);
  return _Enclosure(x1 + xa + xb * r, y1 + ya + yb * r, r);
}

/// Places [c] tangent to [a] and [b].
void _place(Circle b, Circle a, Node c) {
  final dx = b.x - a.x, dy = b.y - a.y, d2 = dx * dx + dy * dy;
  if (truthy(d2)) {
    var a2 = a.r + c.r, b2 = b.r + c.r;
    a2 *= a2;
    b2 *= b2;
    if (a2 > b2) {
      final x = (d2 + b2 - a2) / (2 * d2), y = math.sqrt(math.max(0.0, b2 / d2 - x * x));
      c.x = b.x - x * dx - y * dy;
      c.y = b.y - x * dy + y * dx;
    } else {
      final x = (d2 + a2 - b2) / (2 * d2), y = math.sqrt(math.max(0.0, a2 / d2 - x * x));
      c.x = a.x + x * dx - y * dy;
      c.y = a.y + x * dy + y * dx;
    }
  } else {
    c.x = a.x + c.r;
    c.y = a.y;
  }
}

bool _intersects(Circle a, Circle b) {
  final dr = a.r + b.r - 1e-6, dx = b.x - a.x, dy = b.y - a.y;
  return dr > 0 && dr * dr > dx * dx + dy * dy;
}

/// siblings.js's front-chain `Node`: a circle ([circle] is `_`) and its neighbours on the chain.
class _Link {
  final Node circle;
  late _Link next, previous;
  _Link(this.circle);
}

/// The squared distance from the origin to the weighted centre of [node] and its successor.
double _score(_Link node) {
  final a = node.circle, b = node.next.circle, ab = a.r + b.r;
  final dx = (a.x * b.r + b.x * a.r) / ab, dy = (a.y * b.r + b.y * a.r) / ab;
  return dx * dx + dy * dy;
}

/// `packSiblingsRandom`: packs [circles] around the origin and returns the enclosing radius.
double _packSiblingsRandom(List<Node> circles, double Function() random) {
  final n = circles.length;
  if (n == 0) return 0;

  // Place the first circle.
  final first = circles[0]
    ..x = 0
    ..y = 0;
  if (n <= 1) return first.r;

  // Place the second circle.
  final second = circles[1];
  first.x = -second.r;
  second
    ..x = first.r
    ..y = 0;
  if (n <= 2) return first.r + second.r;

  // Place the third circle.
  _place(second, first, circles[2]);

  // Initialize the front-chain using the first three circles a, b and c.
  var a = _Link(first), b = _Link(second), c = _Link(circles[2]);
  a.next = c.previous = b;
  b.next = a.previous = c;
  c.next = b.previous = a;

  // Attempt to place each remaining circle…
  pack:
  for (var i = 3; i < n; ++i) {
    _place(a.circle, b.circle, circles[i]);
    c = _Link(circles[i]);

    // Find the closest intersecting circle on the front-chain, if any. "Closeness" is determined
    // by linear distance along the front-chain, and so are "ahead" and "behind".
    var j = b.next, k = a.previous;
    var sj = b.circle.r, sk = a.circle.r;
    do {
      if (sj <= sk) {
        if (_intersects(j.circle, c.circle)) {
          b = j;
          a.next = b;
          b.previous = a;
          --i;
          continue pack;
        }
        sj += j.circle.r;
        j = j.next;
      } else {
        if (_intersects(k.circle, c.circle)) {
          a = k;
          a.next = b;
          b.previous = a;
          --i;
          continue pack;
        }
        sk += k.circle.r;
        k = k.previous;
      }
    } while (!identical(j, k.next));

    // Success! Insert the new circle c between a and b.
    c.previous = a;
    c.next = b;
    a.next = b.previous = b = c;

    // Compute the new closest circle pair to the centroid.
    var aa = _score(a);
    while (!identical(c = c.next, b)) {
      final ca = _score(c);
      if (ca < aa) {
        a = c;
        aa = ca;
      }
    }
    b = a.next;
  }

  // Compute the enclosing circle of the front chain.
  final front = <Circle>[b.circle];
  c = b;
  while (!identical(c = c.next, b)) {
    front.add(c.circle);
  }
  final e = _packEncloseRandom(front, random);

  // Translate the circles to put the enclosing circle around the origin.
  for (final circle in circles) {
    circle.x -= e.x;
    circle.y -= e.y;
  }
  return e.r;
}

/// `radiusLeaf(defaultRadius)`: a leaf's radius is the square root of its value.
void _radiusLeaf(Node node) {
  if (node.children == null) {
    final r = math.sqrt(node.value);
    node.r = math.max(0.0, truthy(r) ? r : 0.0);
  }
}

/// `packChildrenRandom(padding, k, random)` with a constant padding.
void _packChildrenRandom(Node node, double padding, double k, double Function() random) {
  final children = node.children;
  if (children == null) return;
  final scaled = padding * k, r = truthy(scaled) ? scaled : 0.0;
  if (truthy(r)) {
    for (final child in children) {
      child.r += r;
    }
  }
  final e = _packSiblingsRandom(children, random);
  if (truthy(r)) {
    for (final child in children) {
      child.r -= r;
    }
  }
  node.r = e + r;
}

/// `translateChild(k)`.
void _translateChild(Node node, double k) {
  final parent = node.parent;
  node.r *= k;
  if (parent != null) {
    node.x = parent.x + k * node.x;
    node.y = parent.y + k * node.y;
  }
}

/// `pack().size([dx, dy]).padding(padding)`: one generator serves both packing passes.
Node pack(Node root, double dx, double dy, double padding) {
  final random = _lcg();
  root
    ..x = dx / 2
    ..y = dy / 2;
  root.eachBefore(_radiusLeaf).eachAfter((node) => _packChildrenRandom(node, 0, 1, random));
  final k = root.r / math.min(dx, dy);
  root.eachAfter((node) => _packChildrenRandom(node, padding, k, random));
  final scale = math.min(dx, dy) / (2 * root.r);
  return root.eachBefore((node) => _translateChild(node, scale));
}
