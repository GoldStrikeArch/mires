// tree.js (Buchheim et al.'s linear-time Reingold-Tilford layout) and cluster.js of d3-hierarchy
// 3.1.3, with the default separation.

import 'dart:math' as math;

import 'hierarchy.dart';

/// `defaultSeparation`, the same function in tree.js and cluster.js.
double _separation(Node a, Node b) => identical(a.parent, b.parent) ? 1.0 : 2.0;

/// tree.js's `TreeNode`: the layout's bookkeeping for [node] (`_`), the child number [i].
class _TreeNode extends Parent<_TreeNode> {
  final Node node;
  final int i;

  /// Set for every node; d3's extra parent of the root has none.
  late _TreeNode parent;
  @override
  List<_TreeNode>? children;
  _TreeNode? defaultAncestor; // A
  late _TreeNode ancestor = this; // a
  double prelim = 0; // z
  double mod = 0; // m
  double change = 0; // c
  double shift = 0; // s
  _TreeNode? thread; // t

  _TreeNode(this.node, this.i);
}

/// The left contour's successor of [v]: its first child, or its thread.
_TreeNode? _nextLeft(_TreeNode v) {
  final children = v.children;
  return children != null ? children.first : v.thread;
}

/// The right contour's successor of [v]: its last child, or its thread.
_TreeNode? _nextRight(_TreeNode v) {
  final children = v.children;
  return children != null ? children.last : v.thread;
}

/// Shifts the subtree rooted at [wp] and prepares the shifts of the smaller subtrees in between.
void _moveSubtree(_TreeNode wm, _TreeNode wp, double shift) {
  final change = shift / (wp.i - wm.i);
  wp.change -= change;
  wp.shift += shift;
  wm.change += change;
  wp.prelim += shift;
  wp.mod += shift;
}

/// Applies the shifts that [_moveSubtree] prepared to the children of a node.
void _executeShifts(List<_TreeNode> children) {
  double shift = 0, change = 0;
  for (var i = children.length - 1; i >= 0; --i) {
    final w = children[i];
    w.prelim += shift;
    w.mod += shift;
    change += w.change;
    shift += w.shift + change;
  }
}

/// [vim]'s ancestor if it is a sibling of [v], otherwise the default [ancestor].
_TreeNode _nextAncestor(_TreeNode vim, _TreeNode v, _TreeNode ancestor) =>
    identical(vim.ancestor.parent, v.parent) ? vim.ancestor : ancestor;

/// `treeRoot`: wraps every node in a [_TreeNode], under an extra parent of the root.
_TreeNode _treeRoot(Node root) {
  final tree = _TreeNode(root, 0);
  final nodes = [tree];
  while (nodes.isNotEmpty) {
    final node = nodes.removeLast();
    final children = node.node.children;
    if (children == null) continue;
    final wrapped = [for (var i = 0; i < children.length; i++) _TreeNode(children[i], i)];
    node.children = wrapped;
    for (var i = wrapped.length - 1; i >= 0; --i) {
      final child = wrapped[i];
      nodes.add(child);
      child.parent = node;
    }
  }
  // d3's extra parent wraps null; it is never read, so it wraps the root here.
  tree.parent = _TreeNode(root, 0)..children = [tree];
  return tree;
}

/// Computes the preliminary x of [v], after its children; then combines it with its left
/// siblings.
void _firstWalk(_TreeNode v) {
  final children = v.children;
  final siblings = v.parent.children!;
  final w = v.i != 0 ? siblings[v.i - 1] : null;
  if (children != null) {
    _executeShifts(children);
    final midpoint = (children.first.prelim + children.last.prelim) / 2;
    if (w != null) {
      v.prelim = w.prelim + _separation(v.node, w.node);
      v.mod = v.prelim - midpoint;
    } else {
      v.prelim = midpoint;
    }
  } else if (w != null) {
    v.prelim = w.prelim + _separation(v.node, w.node);
  }
  v.parent.defaultAncestor = _apportion(v, w, v.parent.defaultAncestor ?? siblings[0]);
}

/// Computes the real x of every node by summing the modifiers of its ancestors.
void _secondWalk(_TreeNode v) {
  v.node.x = v.prelim + v.parent.mod;
  v.mod += v.parent.mod;
}

/// The core of the algorithm: combines the subtree of [v] with those of its left siblings,
/// walking the inside and outside contours, and returns the new default ancestor.
_TreeNode _apportion(_TreeNode v, _TreeNode? w, _TreeNode ancestor) {
  if (w == null) return ancestor;
  var vop = v, vom = v.parent.children![0];
  var sip = v.mod, sop = v.mod, sim = w.mod, som = vom.mod;
  var vim = _nextRight(w), vip = _nextLeft(v);
  while (vim != null && vip != null) {
    vom = _nextLeft(vom)!;
    vop = _nextRight(vop)!;
    vop.ancestor = v;
    final shift = vim.prelim + sim - vip.prelim - sip + _separation(vim.node, vip.node);
    if (shift > 0) {
      _moveSubtree(_nextAncestor(vim, v, ancestor), v, shift);
      sip += shift;
      sop += shift;
    }
    sim += vim.mod;
    sip += vip.mod;
    som += vom.mod;
    sop += vop.mod;
    vim = _nextRight(vim);
    vip = _nextLeft(vip);
  }
  if (vim != null && _nextRight(vop) == null) {
    vop.thread = vim;
    vop.mod += sim - sop;
  }
  if (vip != null && _nextLeft(vom) == null) {
    vom.thread = vip;
    vom.mod += sip - som;
    ancestor = v;
  }
  return ancestor;
}

/// `tree().size([dx, dy])`, or `tree().nodeSize([dx, dy])` when [nodeSize] is set.
Node tree(Node root, double dx, double dy, bool nodeSize) {
  final t = _treeRoot(root);

  // Compute the layout using Buchheim et al.'s algorithm.
  t.eachAfter(_firstWalk);
  t.parent.mod = -t.prelim;
  t.eachBefore(_secondWalk);

  if (nodeSize) {
    // A fixed node size scales x and y.
    root.eachBefore((node) {
      node.x *= dx;
      node.y = node.depth * dy;
    });
  } else {
    // A fixed tree size scales x and y by the extent of the left-most, right-most and deepest
    // nodes.
    var left = root, right = root, bottom = root;
    root.eachBefore((node) {
      if (node.x < left.x) left = node;
      if (node.x > right.x) right = node;
      if (node.depth > bottom.depth) bottom = node;
    });
    final s = identical(left, right) ? 1.0 : _separation(left, right) / 2;
    final tx = s - left.x;
    final kx = dx / (right.x + s + tx);
    final ky = dy / (bottom.depth != 0 ? bottom.depth : 1);
    root.eachBefore((node) {
      node.x = (node.x + tx) * kx;
      node.y = node.depth * ky;
    });
  }
  return root;
}

/// `cluster().size([dx, dy])`, or `cluster().nodeSize([dx, dy])` when [nodeSize] is set.
Node cluster(Node root, double dx, double dy, bool nodeSize) {
  Node? previousNode;
  double x = 0;

  // First walk, computing the initial x and y values.
  root.eachAfter((node) {
    final children = node.children;
    if (children != null) {
      node.x = children.fold(0.0, (sum, c) => sum + c.x) / children.length; // meanX
      node.y = 1 + children.fold(0.0, (y, c) => math.max(y, c.y)); // maxY
    } else {
      final previous = previousNode;
      node.x = previous != null ? x += _separation(node, previous) : 0.0;
      node.y = 0;
      previousNode = node;
    }
  });

  final left = _leafLeft(root), right = _leafRight(root);
  final x0 = left.x - _separation(left, right) / 2;
  final x1 = right.x + _separation(right, left) / 2;

  // Second walk, normalizing x and y to the desired size. The root comes last, so the walk reads
  // the root's unnormalized position throughout.
  return root.eachAfter(nodeSize
      ? (node) {
          node.x = (node.x - root.x) * dx;
          node.y = (root.y - node.y) * dy;
        }
      : (node) {
          node.x = (node.x - x0) / (x1 - x0) * dx;
          node.y = (1 - (truthy(root.y) ? node.y / root.y : 1.0)) * dy;
        });
}

Node _leafLeft(Node node) {
  for (var children = node.children; children != null; children = node.children) {
    node = children.first;
  }
  return node;
}

Node _leafRight(Node node) {
  for (var children = node.children; children != null; children = node.children) {
    node = children.last;
  }
  return node;
}
