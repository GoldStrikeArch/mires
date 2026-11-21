// stratify.js of d3-hierarchy 3.1.3, with the default id and parentId accessors.

import 'hierarchy.dart';

/// `stratify()(data)`, or `stratify().path(path)(data)` when [path] is given. An imputed ancestor
/// is a node whose data is null; d3 marks one with the `imputed` object and replaces that by null
/// before it returns.
Node stratify(List<Datum> data, [String Function(Datum d)? path]) {
  final rows = <Datum?>[...data];
  final List<String?> ids, parentIds;
  if (path != null) {
    final i = [for (final d in data) _normalize(path(d))];
    final p = [for (final id in i) _parentof(id)];
    final s = {...i, ''};
    for (var k = 0; k < p.length; k++) {
      final id = p[k];
      if (s.add(id)) {
        i.add(id);
        p.add(_parentof(id));
        rows.add(null);
      }
    }
    ids = i;
    parentIds = p;
  } else {
    ids = [for (final d in data) d.id];
    parentIds = [for (final d in data) d.parentId];
  }

  var n = rows.length;
  final ambiguous = Node(null);
  final nodeByKey = <String, Node>{};
  final nodes = [for (final d in rows) Node(d)];
  for (var i = 0; i < n; i++) {
    final nodeId = ids[i];
    if (nodeId != null && nodeId.isNotEmpty) {
      nodes[i].id = nodeId;
      nodeByKey[nodeId] = nodeByKey.containsKey(nodeId) ? ambiguous : nodes[i];
    }
  }

  Node? first;
  for (var i = 0; i < n; i++) {
    final node = nodes[i];
    final key = parentIds[i];
    if (key != null && key.isNotEmpty) {
      final parent = nodeByKey[key];
      if (parent == null) throw StateError('missing: $key');
      if (identical(parent, ambiguous)) throw StateError('ambiguous: $key');
      (parent.children ??= []).add(node);
      node.parent = parent;
    } else {
      if (first != null) throw StateError('multiple roots');
      first = node;
    }
  }
  if (first == null) throw StateError('no root');
  var root = first;

  // When imputing internal nodes, only introduce roots if needed.
  if (path != null) {
    while (root.data == null && root.children!.length == 1) {
      root = root.children![0];
      --n;
    }
  }

  // d3 parents the root to `preroot`, whose depth is -1, while it walks.
  root.parent = null;
  root.eachBefore((node) {
    node.depth = (node.parent?.depth ?? -1) + 1;
    --n;
  }).eachBefore(computeHeight);
  if (n > 0) throw StateError('cycle');
  return root;
}

/// Coerces nothing (ids are strings here), strips a trailing slash unless another slash precedes
/// it, and adds a leading slash if missing.
String _normalize(String path) {
  final i = path.length;
  if (_slash(path, i - 1) && !_slash(path, i - 2)) path = path.substring(0, i - 1);
  return path.startsWith('/') ? path : '/$path';
}

/// The path up to its last delimiting slash after the first character: "/a/b" to "/a", "/" to "".
String _parentof(String path) {
  var i = path.length;
  if (i < 2) return '';
  while (--i > 1) {
    if (_slash(path, i)) break;
  }
  return path.substring(0, i);
}

/// Whether `path[i]` is a delimiting slash: one preceded by an even number of backslashes.
bool _slash(String path, int i) {
  if (i >= 0 && i < path.length && path[i] == '/') {
    var k = 0;
    while (i > 0 && path[--i] == r'\') {
      ++k;
    }
    if ((k & 1) == 0) return true;
  }
  return false;
}
