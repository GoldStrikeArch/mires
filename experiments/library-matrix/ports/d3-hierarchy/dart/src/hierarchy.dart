// The node of d3-hierarchy 3.1.3 and the operations the contract uses: hierarchy/index.js
// (`hierarchy`, `computeHeight`), eachBefore.js, eachAfter.js, iterator.js, sum.js and sort.js.

import 'dart:js_interop';

/// A record of the input: a nested datum or a stratify row, read in place. Dart reads undefined
/// and null alike as `null`, which is how d3 reads every field (`!= null`, `??`).
extension type Datum._(JSObject _) implements JSObject {
  external String? get id;

  /// The id as the input has it, undefined included: Dart would turn undefined into null.
  @JS('id')
  external JSAny? get idValue;
  external String? get parentId;
  external double? get value;
  external JSArray<Datum>? get children;
}

/// A circle, as pack/enclose.js reads one: a node or a computed enclosure.
abstract interface class Circle {
  double get x;
  double get y;
  double get r;
}

/// A node with children: [Node], and tree.js's `TreeNode`, whose prototype is Node's.
abstract class Parent<N extends Parent<N>> {
  List<N>? get children;

  /// `node.eachBefore(callback)`: pre-order, on an explicit stack.
  N eachBefore(void Function(N node) callback) {
    final nodes = <N>[this as N];
    while (nodes.isNotEmpty) {
      final node = nodes.removeLast();
      callback(node);
      final children = node.children;
      if (children != null) {
        for (var i = children.length - 1; i >= 0; --i) {
          nodes.add(children[i]);
        }
      }
    }
    return this as N;
  }

  /// `node.eachAfter(callback)`: post-order, from the reversed visiting order of a stack walk.
  N eachAfter(void Function(N node) callback) {
    final nodes = <N>[this as N], next = <N>[];
    while (nodes.isNotEmpty) {
      final node = nodes.removeLast();
      next.add(node);
      final children = node.children;
      if (children != null) nodes.addAll(children);
    }
    while (next.isNotEmpty) {
      callback(next.removeLast());
    }
    return this as N;
  }
}

/// `Node` of hierarchy/index.js. [data] is null for an ancestor that stratify imputed from a path,
/// and [children] is null for a leaf, as in d3. The layouts write the coordinates of their kind.
class Node extends Parent<Node> implements Circle {
  Datum? data;
  String? id;
  Node? parent;
  @override
  List<Node>? children;
  int depth = 0, height = 0;
  double value = 0;
  @override
  double x = 0, y = 0, r = 0;
  double x0 = 0, y0 = 0, x1 = 0, y1 = 0;

  Node(this.data);

  /// The order of `node[Symbol.iterator]`: breadth-first, each level from left to right.
  List<Node> breadthFirst() {
    final order = <Node>[];
    var next = <Node>[this];
    do {
      final current = next;
      next = [];
      for (final node in current) {
        order.add(node);
        final children = node.children;
        if (children != null) next.addAll(children);
      }
    } while (next.isNotEmpty);
    return order;
  }

  /// `node.sum(value)`: each aggregate is `+value(data) || 0` plus the children's, last child
  /// first.
  Node sum(double Function(Datum? data) value) => eachAfter((node) {
        final own = value(node.data);
        var sum = truthy(own) ? own : 0.0;
        final children = node.children;
        if (children != null) {
          for (var i = children.length - 1; i >= 0; --i) {
            sum += children[i].value;
          }
        }
        node.value = sum;
      });

  /// `node.sort(compare)` with a JavaScript comparator. Array.prototype.sort is stable and counts
  /// a NaN result as 0. List.sort is not stable, so a child's original position breaks ties.
  Node sort(double Function(Node a, Node b) compare) => eachBefore((node) {
        final children = node.children;
        if (children == null) return;
        final position = {for (final (i, child) in children.indexed) child: i};
        children.sort((a, b) {
          final order = compare(a, b);
          return order < 0 ? -1 : (order > 0 ? 1 : position[a]! - position[b]!);
        });
      });
}

/// JavaScript's ToBoolean of a number: false for 0, -0 and NaN. d3 tests numbers this way.
bool truthy(double x) => !(x == 0 || x.isNaN);

/// `hierarchy(data)` with the default children accessor, `d.children`.
Node hierarchy(Datum data) {
  final root = Node(data);
  final nodes = [root];
  while (nodes.isNotEmpty) {
    final node = nodes.removeLast();
    final childs = node.data?.children?.toDart;
    if (childs != null && childs.isNotEmpty) {
      final children = [for (final child in childs) Node(child)];
      node.children = children;
      for (var i = children.length - 1; i >= 0; --i) {
        final child = children[i];
        nodes.add(child);
        child.parent = node;
        child.depth = node.depth + 1;
      }
    }
  }
  return root.eachBefore(computeHeight);
}

/// `computeHeight`: raises the heights of the ancestors the node makes taller.
void computeHeight(Node start) {
  Node? node = start;
  var height = 0;
  do {
    node!.height = height;
    node = node.parent;
  } while (node != null && node.height < ++height);
}
