// MiniSearch's SearchableMap (minisearch 7.2.0, dist/es/index.js): a radix tree with string keys.
// Dart strings on the web are JavaScript strings, so keys are compared, measured and split on
// UTF-16 code units (`length`, `codeUnitAt`, `substring`, `startsWith`), an edge can end between
// the two halves of a surrogate pair, and lone surrogates are ordinary units.
//
// A node is what MiniSearch's node is, an insertion-ordered map (Dart's default Map, a
// LinkedHashMap, as a JavaScript Map is): every non-empty key is an edge label leading to a child
// node, and the empty key ([leaf]) holds the value of the key that ends at the node. The order of a
// node's keys, the leaf's place among them included, is observable (iteration, prefix and fuzzy
// results), and LinkedHashMap keeps it as a JavaScript Map does: a new key is appended, a removed
// key leaves the others in place, and setting an existing key keeps its place. Replacing an edge (a
// split in `createPath`, a merge after a removal) sets the new key and then removes the old one, so
// the new edge moves to the end.
//
// A Dart map must not change while it is iterated (its iterator throws), where a JavaScript Map
// iterator survives. Every walk that interleaves with changes iterates a snapshot of keys, as
// MiniSearch's own TreeIterator does.

import 'dart:math' as math;
import 'dart:typed_data';

/// `LEAF`: the key under which a node holds the value of the key that ends at it.
const leaf = '';

/// What a node maps a key to: a child node under an edge label, or a [_Leaf] under [leaf].
sealed class _Entry {}

/// A radix tree node: its keys in insertion order. Nodes are the tree's structure and carry no
/// value type; values live in the leaves, which a [SearchableMap] creates and reads with its own
/// type argument.
final class _Node extends _Entry {
  final Map<String, _Entry> entries = {};

  /// The child under the edge label [key] (`node.get(key)` for a key that is not [leaf]). A type
  /// pattern rather than a cast: dart2js compiles it to `instanceof`, a cast to its general
  /// runtime type check, which cost a tenth of the indexing time on every step down the tree.
  _Node child(String key) => switch (entries[key]) {
    final _Node child => child,
    _ => throw StateError('no edge "$key"'),
  };
}

final class _Leaf<T> extends _Entry {
  _Leaf(this.value);

  final T value;
}

/// `node.get(LEAF)`: the value of the key that ends at [node], or null.
T? _valueAt<T extends Object>(_Node node) => (node.entries[leaf] as _Leaf<T>?)?.value;

/// A step of a path down the tree: a node and the edge label taken from it (or, at the end of a
/// [_trackDown] that leaves the tree, the rest of the key).
typedef _Step = (_Node node, String key);

/// A radix tree, or a view of one at a prefix ([atPrefix]). Iterating it yields every key with its
/// value, in MiniSearch's `TreeIterator` order. A missing value is null, as it is `undefined` in
/// MiniSearch, so values are not nullable.
final class SearchableMap<T extends Object> extends Iterable<(String, T)> {
  SearchableMap() : _tree = _Node(), _prefix = '';

  SearchableMap._view(this._tree, this._prefix);

  final _Node _tree;
  final String _prefix;

  /// `_size`, memoised until the next change. MiniSearch tests it with `if (this._size)`, so a
  /// memoised 0 is counted again, as here.
  int? _size;

  /// A view of the entries whose keys start with [prefix]. When the prefix ends inside an edge, the
  /// view is a new node holding the rest of that edge's label to its child.
  SearchableMap<T> atPrefix(String prefix) {
    if (!prefix.startsWith(_prefix)) throw ArgumentError('Mismatched prefix');
    final (node, path) = _trackDown(_tree, prefix.substring(_prefix.length));
    if (node == null) {
      final (parentNode, key) = path.last;
      for (final k in parentNode.entries.keys) {
        if (k != leaf && k.startsWith(key)) {
          final view = _Node();
          view.entries[k.substring(key.length)] = parentNode.child(k);
          return SearchableMap._view(view, prefix);
        }
      }
      // MiniSearch passes an undefined node, for which its constructor's default is an empty map.
      return SearchableMap._view(_Node(), prefix);
    }
    return SearchableMap._view(node, prefix);
  }

  void delete(String key) {
    _size = null;
    _remove(_tree, key);
  }

  @override
  Iterator<(String, T)> get iterator => _TreeIterator<T>(_tree, _prefix);

  /// Every key within [maxEditDistance] (Levenshtein, over code units) of [key], with its value and
  /// distance, in the order the walk meets them.
  Map<String, (T, int)> fuzzyGet(String key, int maxEditDistance) => _fuzzySearch<T>(_tree, key, maxEditDistance);

  T? get(String key) {
    final node = _lookup(_tree, key);
    return node == null ? null : _valueAt<T>(node);
  }

  bool has(String key) => _lookup(_tree, key)?.entries.containsKey(leaf) ?? false;

  /// `size`: the number of keys, counted by iterating.
  @override
  int get length {
    final size = _size;
    if (size != null && size != 0) return size;
    var count = 0;
    final entries = iterator;
    while (entries.moveNext()) {
      count += 1;
    }
    return _size = count;
  }

  /// The value at [key], set to [initial]'s first when there is none.
  T fetch(String key, T Function() initial) {
    _size = null;
    final node = _createPath(_tree, key);
    final value = _valueAt<T>(node);
    if (value != null) return value;
    final created = initial();
    node.entries[leaf] = _Leaf<T>(created);
    return created;
  }
}

/// The node [key] ends at, or null, and the path to it: each node passed with the edge taken from
/// it, and when the key leaves the tree, the last node with the rest of the key.
(_Node?, List<_Step>) _trackDown(_Node tree, String key) {
  final path = <_Step>[];
  var node = tree;
  var rest = key;
  descend:
  while (rest.isNotEmpty) {
    for (final k in node.entries.keys) {
      if (k != leaf && rest.startsWith(k)) {
        path.add((node, k));
        node = node.child(k);
        rest = rest.substring(k.length);
        continue descend;
      }
    }
    path.add((node, rest));
    return (null, path);
  }
  return (node, path);
}

_Node? _lookup(_Node tree, String key) {
  var node = tree;
  var rest = key;
  descend:
  while (rest.isNotEmpty) {
    for (final k in node.entries.keys) {
      if (k != leaf && rest.startsWith(k)) {
        node = node.child(k);
        rest = rest.substring(k.length);
        continue descend;
      }
    }
    return null;
  }
  return node;
}

/// The node [key] ends at, created when it does not exist. An edge the key leaves part way is
/// split: an intermediate node, holding the rest of the edge, is set under the shared part and the
/// old edge removed, so it moves to the end of the node's keys.
_Node _createPath(_Node start, String key) {
  final keyLength = key.length;
  var node = start;
  var pos = 0;
  outer:
  while (pos < keyLength) {
    final unit = key.codeUnitAt(pos);
    for (final k in node.entries.keys) {
      // A candidate edge starts with the next unit of the key (no two edges of a node share one).
      if (k != leaf && k.codeUnitAt(0) == unit) {
        final len = math.min(keyLength - pos, k.length);
        // Advance offset to the point where key and k no longer match.
        var offset = 1;
        while (offset < len && key.codeUnitAt(pos + offset) == k.codeUnitAt(offset)) {
          ++offset;
        }
        final child = node.child(k);
        if (offset == k.length) {
          node = child;
        } else {
          // The map changes here, and its iteration is abandoned by the `continue` below.
          final intermediate = _Node();
          intermediate.entries[k.substring(offset)] = child;
          node.entries[key.substring(pos, pos + offset)] = intermediate;
          node.entries.remove(k);
          node = intermediate;
        }
        pos += offset;
        continue outer;
      }
    }
    // A final child node holds the rest of the key.
    final child = _Node();
    node.entries[key.substring(pos)] = child;
    return child;
  }
  return node;
}

/// `remove`: removes the key's leaf, then cleans up an emptied node or merges a node left with a
/// single edge into its parent.
void _remove(_Node tree, String key) {
  final (node, path) = _trackDown(tree, key);
  if (node == null) return;
  node.entries.remove(leaf);
  if (node.entries.isEmpty) {
    _cleanup(path, path.length);
  } else if (node.entries.length == 1) {
    final MapEntry(key: onlyKey, value: value) = node.entries.entries.first;
    _merge(path, path.length, onlyKey, value);
  }
}

/// `cleanup(path.slice(0, length))`: the last node of the path loses its edge to the emptied node;
/// it is cleaned up in turn when that empties it, or merged into its parent when it is left with
/// one edge. Neither happens at the root.
void _cleanup(List<_Step> path, int length) {
  if (length == 0) return;
  final (node, key) = path[length - 1];
  node.entries.remove(key);
  if (node.entries.isEmpty) {
    _cleanup(path, length - 1);
  } else if (node.entries.length == 1) {
    final MapEntry(key: onlyKey, value: value) = node.entries.entries.first;
    if (onlyKey != leaf) _merge(path, length - 1, onlyKey, value);
  }
}

/// `merge(path.slice(0, length), key, value)`: the node at the end of the path holds the single
/// edge [key] to [value]; its parent replaces the edge to it by the two labels joined, which moves
/// the merged edge to the end of the parent's keys.
void _merge(List<_Step> path, int length, String key, _Entry value) {
  if (length == 0) return;
  final (node, nodeKey) = path[length - 1];
  node.entries[nodeKey + key] = value;
  node.entries.remove(nodeKey);
}

/// `fuzzySearch`: a depth-first walk over each node's keys, oldest first, keeping a Levenshtein
/// matrix of `query.length + 1` columns and `query.length + 1 + maxDistance` rows. MiniSearch
/// keeps it in a Uint8Array, as here, so every store is reduced modulo 256.
Map<String, (T, int)> _fuzzySearch<T>(_Node node, String query, int maxDistance) {
  final results = <String, (T, int)>{};
  // Number of columns in the Levenshtein matrix.
  final n = query.length + 1;
  // Matching terms can never be longer than N + maxDistance.
  final m = n + maxDistance;
  // Fill first matrix row and column with numbers: 0 1 2 3 ...
  final matrix = Uint8List(m * n)..fillRange(0, m * n, maxDistance + 1);
  for (var j = 0; j < n; ++j) {
    matrix[j] = j;
  }
  for (var i = 1; i < m; ++i) {
    matrix[i * n] = i;
  }
  _recurse<T>(node, query, maxDistance, results, matrix, 1, n, '');
  return results;
}

/// One node of the fuzzy walk; [m] is the next matrix row to fill, one past the length of
/// [prefix].
void _recurse<T>(
  _Node node,
  String query,
  int maxDistance,
  Map<String, (T, int)> results,
  Uint8List matrix,
  int m,
  int n,
  String prefix,
) {
  final offset = m * n;
  final rows = matrix.length ~/ n;
  key:
  for (final MapEntry(:key, value: entry) in node.entries.entries) {
    switch (entry) {
      // The leaf key: we've reached a leaf node. Check if the edit distance acceptable and store
      // the result if it is.
      case _Leaf():
        final distance = matrix[offset - 1];
        if (distance <= maxDistance) results[prefix] = ((entry as _Leaf<T>).value, distance);
      // An edge label: iterate over all characters in the key. Update the Levenshtein matrix and
      // check if the minimum distance in the last row is still within the maximum edit distance.
      // If it is, we can recurse over all child nodes.
      case final _Node child:
        var i = m;
        for (var pos = 0; pos < key.length; ++pos, ++i) {
          // Past the last row a key is longer than query.length + maxDistance and cannot match.
          // MiniSearch reads past its Uint8Array there (undefined never prunes and never
          // matches); skipping the subtree finds the same keys.
          if (i >= rows) continue key;
          final char = key.codeUnitAt(pos);
          final thisRowOffset = n * i;
          final prevRowOffset = thisRowOffset - n;
          // Set the first column based on the previous row, and initialize the minimum distance
          // in the current row.
          var minDistance = matrix[thisRowOffset];
          final jmin = math.max(0, i - maxDistance - 1);
          final jmax = math.min(n - 1, i + maxDistance);
          // Iterate over remaining columns (characters in the query).
          for (var j = jmin; j < jmax; ++j) {
            final different = char != query.codeUnitAt(j) ? 1 : 0;
            final rpl = matrix[prevRowOffset + j] + different;
            final del = matrix[prevRowOffset + j + 1] + 1;
            final ins = matrix[thisRowOffset + j] + 1;
            final dist = math.min(rpl, math.min(del, ins));
            matrix[thisRowOffset + j + 1] = dist;
            if (dist < minDistance) minDistance = dist;
          }
          // Because distance will never decrease, we can stop. There will be no matching child
          // nodes.
          if (minDistance > maxDistance) continue key;
        }
        _recurse<T>(child, query, maxDistance, results, matrix, i, n, prefix + key);
    }
  }
}

/// MiniSearch's `TreeIterator`: depth first from the view's root. Entering a node snapshots its
/// keys, which are then visited from the most recently inserted to the oldest (the snapshot is
/// popped from its end); a leaf yields the key spelled by the labels on the path.
///
/// MiniSearch deletes the key just visited while it iterates (vacuuming, and search's cleanup of
/// discarded postings). The snapshots, and the nodes a merge detaches but the path still holds,
/// keep the walk on exactly the keys the tree had when it began.
final class _TreeIterator<T> implements Iterator<(String, T)> {
  _TreeIterator(_Node node, this._prefix) {
    final keys = node.entries.keys.toList();
    if (keys.isNotEmpty) _path.add((node, keys));
  }

  final String _prefix;

  /// The nodes from the root down to the current one, each with the keys still to visit (last
  /// first).
  final List<(_Node, List<String>)> _path = [];

  (String, T)? _current;

  @override
  (String, T) get current => _current!;

  @override
  bool moveNext() {
    if (_path.isEmpty) {
      _current = null;
      return false;
    }
    _current = _dive();
    _backtrack();
    return true;
  }

  (String, T) _dive() {
    while (true) {
      final (node, keys) = _path.last;
      if (keys.last == leaf) return (_key(), (node.entries[leaf]! as _Leaf<T>).value);
      final child = node.child(keys.last);
      _path.add((child, child.entries.keys.toList()));
    }
  }

  void _backtrack() {
    while (_path.isNotEmpty) {
      final (_, keys) = _path.last;
      keys.removeLast();
      if (keys.isNotEmpty) return;
      _path.removeLast();
    }
  }

  String _key() {
    final key = StringBuffer(_prefix);
    for (final (_, keys) in _path) {
      key.write(keys.last);
    }
    return key.toString();
  }
}
