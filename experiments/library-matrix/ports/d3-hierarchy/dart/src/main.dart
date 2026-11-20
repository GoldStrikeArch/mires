import 'dart:js_interop';
import 'dart:js_interop_unsafe';

import 'geometry.dart';

extension type Datum._(JSObject _) implements JSObject {
  external String? get id;
  external String? get parentId;
  external double? get value;
  external JSArray<Datum>? get children;
}

extension type Input._(JSObject _) implements JSObject {
  external String get source;
  external String get op;
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

bool slash(String s, int i) {
  if (i < 0 || i >= s.length || s[i] != '/') return false;
  var k = i;
  while (k > 0 && s[k - 1] == '\\') {
    k--;
  }
  return (i - k) % 2 == 0;
}

String normalize(String s) {
  if (slash(s, s.length - 1) && !slash(s, s.length - 2))
    s = s.substring(0, s.length - 1);
  return s.startsWith('/') ? s : '/' + s;
}

String parentPath(String s) {
  if (s.length < 2) return '';
  var i = s.length - 1;
  while (i > 1 && !slash(s, i)) {
    i--;
  }
  return s.substring(0, i);
}

List<int> walk(List<Node> ns, int root, bool post) {
  final todo = [root], out = <int>[];
  while (todo.isNotEmpty) {
    final i = todo.removeLast();
    out.add(i);
    todo.addAll(post ? ns[i].kids : ns[i].kids.reversed);
  }
  return post ? out.reversed.toList() : out;
}

(List<Node>, int) read(Input input) {
  final nodes = <Node>[];
  if (input.source != 'id' && input.source != 'path') {
    int nested(Datum d, int parent) {
      final i = nodes.length,
          n = Node()
            ..id = d.id ?? ''
            ..value = d.value ?? 0
            ..parent = parent;
      nodes.add(n);
      n.kids = [for (final c in d.children?.toDart ?? []) nested(c, i)];
      return i;
    }

    nested(input.data!, -1);
    return (nodes, 0);
  }
  final rows = input.rows!.toDart,
      paths = input.source == 'path',
      byId = <String, int>{},
      parents = <String>[];
  for (final row in rows) {
    final n = Node()
      ..id = paths ? normalize(row.id ?? '') : row.id ?? ''
      ..value = row.value ?? 0;
    if (n.id.isNotEmpty)
      byId[n.id] = byId.containsKey(n.id) ? -1 : nodes.length;
    nodes.add(n);
    parents.add(paths ? parentPath(n.id) : row.parentId ?? '');
  }
  for (var i = 0; paths && i < parents.length; i++) {
    final p = parents[i];
    if (p.isNotEmpty && !byId.containsKey(p)) {
      byId[p] = nodes.length;
      nodes.add(Node()..id = p);
      parents.add(parentPath(p));
    }
  }
  var root = -1;
  for (var i = 0; i < parents.length; i++) {
    final key = parents[i];
    if (key.isEmpty) {
      if (root >= 0) throw StateError('multiple roots');
      root = i;
    } else {
      final p = byId[key] ?? -1;
      if (p < 0) throw StateError('missing or ambiguous parent');
      nodes[i].parent = p;
      nodes[p].kids.add(i);
    }
  }
  if (root < 0) throw StateError('no root');
  var expected = nodes.length;
  while (paths && root >= rows.length && nodes[root].kids.length == 1) {
    root = nodes[root].kids.first;
    expected--;
  }
  nodes[root].parent = -1;
  if (walk(nodes, root, false).length != expected) throw StateError('cycle');
  return (nodes, root);
}

JSArray<JSArray<JSAny?>> run(Input input) {
  final (nodes, root) = read(input);
  for (final i in walk(nodes, root, true)) {
    final n = nodes[i];
    if (n.value == 0) n.value = 0;
    for (final k in n.kids.reversed) {
      final c = nodes[k];
      n.value += c.value;
      if (n.height < c.height + 1) n.height = c.height + 1;
    }
    if (input.sort) {
      final order = {for (final (j, k) in n.kids.indexed) k: j};
      n.kids.sort((a, b) {
        final diff = nodes[b].value.compareTo(nodes[a].value);
        return diff != 0 ? diff : order[a]!.compareTo(order[b]!);
      });
    }
  }
  var pre = walk(nodes, root, false);
  final post = walk(nodes, root, true);
  for (final i in pre) {
    final n = nodes[i];
    if (n.parent >= 0) n.depth = nodes[n.parent].depth + 1;
  }
  final w = input.width, h = input.height, p = input.padding, op = input.op;
  switch (op) {
    case 'tree':
      tidy(nodes, pre, post, w, h, input.nodeSize);
    case 'cluster':
      cluster(nodes, pre, post, w, h, input.nodeSize);
    case 'pack':
      pack(nodes, pre, post, w, h, p);
    case 'treemap':
    case 'partition':
      rectangles(
        nodes,
        pre,
        w,
        h,
        p,
        op == 'partition',
        input.tile,
        input.round,
      );
    case 'iterator':
      pre = [root];
      for (var i = 0; i < pre.length; i++) {
        pre.addAll(nodes[pre[i]].kids);
      }
    case 'stratify':
    case 'hierarchy':
      break;
    default:
      throw ArgumentError('unknown operation');
  }
  final out = <JSArray<JSAny?>>[];
  for (final i in pre) {
    final n = nodes[i];
    final row = <JSAny?>[
      n.id.toJS,
      n.parent < 0 ? null : nodes[n.parent].id.toJS,
      n.depth.toJS,
      n.height.toJS,
      n.value.toJS,
    ];
    final coords = switch (op) {
      'tree' || 'cluster' => [n.x, n.y],
      'pack' => [n.x, n.y, n.r],
      'treemap' || 'partition' => [n.x0, n.y0, n.x1, n.y1],
      _ => <double>[],
    };
    row.addAll(coords.map((x) => x.toJS));
    out.add(row.toJS);
  }
  return out.toJS;
}

void main() {
  final exports = JSObject();
  exports['run'] = run.toJS;
  globalContext['__matrixExports'] = exports;
}
