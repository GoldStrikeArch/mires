// MiniSearch's `SearchableMap` (minisearch 7.2.0, `dist/es/index.js`): a radix tree over strings,
// with the `TreeIterator` and the Levenshtein walk of `fuzzySearch`.
//
// Kotlin strings are JavaScript strings, so keys are compared, split and measured in UTF-16 code
// units as in MiniSearch (an edge label may end between the two halves of a surrogate pair), and
// lone surrogates are ordinary units.

package minisearch

import kotlin.math.max
import kotlin.math.min

/** The key under which a node holds the value of the key that ends at it. */
internal const val LEAF = ""

/** An entry of a radix-tree node: the child node under an edge label, or the value under [LEAF]. */
internal sealed interface RadixEntry<T : Any>

/**
 * A radix-tree node. As in MiniSearch, a node is an insertion-ordered map: a non-empty key is an
 * edge label leading to a child node, and [LEAF] holds the value of the key ending at this node.
 * No two edges share their first code unit. The order of the keys, the leaf's position among them
 * included, is observable; `LinkedHashMap` keeps it exactly as a JavaScript `Map` does (a new key
 * is appended, a removed key leaves its place, setting an existing key keeps its position).
 */
internal class RadixTree<T : Any> : LinkedHashMap<String, RadixEntry<T>>(), RadixEntry<T> {
    @Suppress("UNCHECKED_CAST")
    fun child(key: String): RadixTree<T> = get(key) as RadixTree<T>

    @Suppress("UNCHECKED_CAST")
    val leaf: T? get() = (get(LEAF) as Leaf<T>?)?.value
}

internal class Leaf<T : Any>(val value: T) : RadixEntry<T>

/**
 * The map, or a view of it at [prefix] (`atPrefix`). [tree] is the view's root, which for a prefix
 * ending inside an edge label is a one-entry node made for the view.
 */
internal class SearchableMap<T : Any>(
    internal val tree: RadixTree<T> = RadixTree(),
    internal val prefix: String = "",
) : Iterable<Pair<String, T>> {
    /** MiniSearch's `_size`: the counted size, reset by every change. */
    private var cachedSize: Int? = null

    fun atPrefix(prefix: String): SearchableMap<T> {
        if (!prefix.startsWith(this.prefix)) throw IllegalArgumentException("Mismatched prefix")
        val (node, path) = trackDown(tree, prefix.substring(this.prefix.length))
        if (node == null) {
            val (parentNode, key) = path.last()
            for (k in parentNode.keys) {
                if (k != LEAF && k.startsWith(key)) {
                    val view = RadixTree<T>()
                    view[k.substring(key.length)] = parentNode.getValue(k)
                    return SearchableMap(view, prefix)
                }
            }
        }
        return SearchableMap(node ?: RadixTree(), prefix)
    }

    fun delete(key: String) {
        cachedSize = null
        remove(tree, key)
    }

    /** Entries in `TreeIterator` order. */
    override fun iterator(): Iterator<Pair<String, T>> = TreeIterator(this)

    /** Every key within [maxEditDistance] of [key], with its value and distance, in walk order. */
    fun fuzzyGet(key: String, maxEditDistance: Int): LinkedHashMap<String, FuzzyMatch<T>> =
        fuzzySearch(tree, key, maxEditDistance)

    operator fun get(key: String): T? = lookup(tree, key)?.leaf

    fun has(key: String): Boolean {
        val node = lookup(tree, key)
        return node != null && node.containsKey(LEAF)
    }

    /** The value at [key], first setting it to [initial]`()` when there is none. */
    fun fetch(key: String, initial: () -> T): T {
        cachedSize = null
        val node = createPath(tree, key)
        var value = node.leaf
        if (value == null) {
            value = initial()
            node[LEAF] = Leaf(value)
        }
        return value
    }

    /** The number of keys; MiniSearch recounts while its cached size is unset or zero. */
    val size: Int
        get() {
            cachedSize?.let { if (it != 0) return it }
            var size = 0
            val iterator = iterator()
            while (iterator.hasNext()) {
                iterator.next()
                size += 1
            }
            cachedSize = size
            return size
        }
}

/**
 * MiniSearch's `TreeIterator`: depth first from the view's root, each node's keys from the most
 * recently inserted to the oldest. Each step of the path holds a snapshot of its node's keys and
 * pops from its end; a child is looked up when the walk reaches its key. So a walk that deletes
 * the term it just visited (which can merge or remove nodes) still visits exactly the terms the
 * tree held when it began, as in MiniSearch.
 */
internal class TreeIterator<T : Any>(private val set: SearchableMap<T>) : Iterator<Pair<String, T>> {
    private class Step<T : Any>(val node: RadixTree<T>, val keys: MutableList<String>)

    private val path = ArrayList<Step<T>>()

    init {
        val node = set.tree
        val keys = ArrayList(node.keys)
        if (keys.isNotEmpty()) path.add(Step(node, keys))
    }

    // Every node below the root holds at least one key, so a non-empty path always reaches a leaf.
    override fun hasNext() = path.isNotEmpty()

    override fun next(): Pair<String, T> {
        if (path.isEmpty()) throw NoSuchElementException()
        val value = dive()
        backtrack()
        return value
    }

    private fun dive(): Pair<String, T> {
        while (true) {
            val step = path.last()
            val key = step.keys.last()
            if (key == LEAF) return Pair(key(), step.node.leaf!!)
            val child = step.node.child(key)
            path.add(Step(child, ArrayList(child.keys)))
        }
    }

    private fun backtrack() {
        while (path.isNotEmpty()) {
            val keys = path.last().keys
            keys.removeAt(keys.lastIndex)
            if (keys.isNotEmpty()) return
            path.removeAt(path.lastIndex)
        }
    }

    private fun key(): String = buildString {
        append(set.prefix)
        for (step in path) {
            val key = step.keys.last()
            if (key != LEAF) append(key)
        }
    }
}

/** A step of a path down the tree: a node and the key taken from it (`[node, key]`). */
private typealias PathStep<T> = Pair<RadixTree<T>, String>

/**
 * `trackDown`: the node [key] ends at (null when it ends inside an edge or leaves the tree) and
 * the path to it; when the node is null, the path's last step holds the unmatched rest of the key.
 */
private fun <T : Any> trackDown(
    tree: RadixTree<T>?,
    key: String,
    path: MutableList<PathStep<T>> = ArrayList(),
): Pair<RadixTree<T>?, MutableList<PathStep<T>>> {
    if (key.isEmpty() || tree == null) return Pair(tree, path)
    for (k in tree.keys) {
        if (k != LEAF && key.startsWith(k)) {
            path.add(Pair(tree, k))
            return trackDown(tree.child(k), key.substring(k.length), path)
        }
    }
    path.add(Pair(tree, key))
    return trackDown(null, "", path)
}

private fun <T : Any> lookup(tree: RadixTree<T>?, key: String): RadixTree<T>? {
    if (key.isEmpty() || tree == null) return tree
    for (k in tree.keys) {
        if (k != LEAF && key.startsWith(k)) return lookup(tree.child(k), key.substring(k.length))
    }
    return null
}

/**
 * `createPath`: the node [key] ends at, created when missing. Where the key leaves an edge, an
 * intermediate node is set under the shared part and the old edge deleted, so the intermediate
 * node moves to the end of its parent's keys.
 */
private fun <T : Any> createPath(root: RadixTree<T>, key: String): RadixTree<T> {
    var node = root
    val keyLength = key.length
    var pos = 0
    outer@ while (pos < keyLength) {
        for (k in node.keys) {
            // A candidate edge: its first unit matches.
            if (k != LEAF && key[pos] == k[0]) {
                val len = min(keyLength - pos, k.length)
                var offset = 1
                while (offset < len && key[pos + offset] == k[offset]) ++offset
                val child = node.child(k)
                if (offset == k.length) {
                    node = child
                } else {
                    // Partial match: an intermediate node holds both the old subtree and the new key.
                    val intermediate = RadixTree<T>()
                    intermediate[k.substring(offset)] = child
                    node[key.substring(pos, pos + offset)] = intermediate
                    node.remove(k)
                    node = intermediate
                }
                pos += offset
                continue@outer
            }
        }
        // A final child for the rest of the key.
        val child = RadixTree<T>()
        node[key.substring(pos)] = child
        return child
    }
    return node
}

private fun <T : Any> remove(tree: RadixTree<T>, key: String) {
    val (node, path) = trackDown(tree, key)
    if (node == null) return
    node.remove(LEAF)
    if (node.isEmpty()) {
        cleanup(path)
    } else if (node.size == 1) {
        val (onlyKey, value) = node.entries.first()
        merge(path, onlyKey, value)
    }
}

/** The node at the end of [path] is empty: delete the edge to it, and clean up or merge upwards. */
private fun <T : Any> cleanup(path: List<PathStep<T>>) {
    if (path.isEmpty()) return
    val (node, key) = path.last()
    node.remove(key)
    if (node.isEmpty()) {
        cleanup(path.subList(0, path.size - 1))
    } else if (node.size == 1) {
        val (onlyKey, value) = node.entries.first()
        if (onlyKey != LEAF) merge(path.subList(0, path.size - 1), onlyKey, value)
    }
}

/** The node at the end of [path] holds one edge, [key] to [value]: join it to the edge above. */
private fun <T : Any> merge(path: List<PathStep<T>>, key: String, value: RadixEntry<T>) {
    if (path.isEmpty()) return
    val (node, nodeKey) = path.last()
    node[nodeKey + key] = value
    node.remove(nodeKey)
}

/** A fuzzy match: the key's value and its edit distance. */
internal class FuzzyMatch<T : Any>(val value: T, val distance: Int)

/**
 * `fuzzySearch`: a depth-first walk over each node's keys, oldest first, keeping a Levenshtein
 * matrix of `query.length + 1` columns and `query.length + 1 + maxDistance` rows. MiniSearch keeps
 * the matrix in a `Uint8Array`: here a `ByteArray` with every read masked to 0..255, which stores
 * and reads exactly as a `Uint8Array` does.
 */
internal fun <T : Any> fuzzySearch(node: RadixTree<T>, query: String, maxDistance: Int): LinkedHashMap<String, FuzzyMatch<T>> {
    val results = LinkedHashMap<String, FuzzyMatch<T>>()
    // Number of columns in the Levenshtein matrix.
    val n = query.length + 1
    // Matching terms can never be longer than n + maxDistance.
    val m = n + maxDistance
    // The first row and column are 0, 1, 2, ...; every other cell starts at maxDistance + 1.
    val matrix = ByteArray(m * n)
    matrix.fill((maxDistance + 1).toByte())
    for (j in 0 until n) matrix[j] = j.toByte()
    for (i in 1 until m) matrix[i * n] = i.toByte()
    FuzzyWalk(query, maxDistance, results, matrix, m, n).recurse(node, 1, "")
    return results
}

private class FuzzyWalk<T : Any>(
    val query: String,
    val maxDistance: Int,
    val results: LinkedHashMap<String, FuzzyMatch<T>>,
    val matrix: ByteArray,
    val rows: Int,
    val n: Int,
) {
    private fun cell(index: Int) = matrix[index].toInt() and 0xff

    /** `recurse`: [m] is the next matrix row to fill, one past the length of [prefix]. */
    fun recurse(node: RadixTree<T>, m: Int, prefix: String) {
        val offset = m * n
        key@ for (key in node.keys) {
            if (key == LEAF) {
                // A leaf: the edit distance of the key spelled so far is the last cell of the row.
                val distance = cell(offset - 1)
                if (distance <= maxDistance) {
                    @Suppress("UNCHECKED_CAST")
                    results[prefix] = FuzzyMatch((node.getValue(key) as Leaf<T>).value, distance)
                }
            } else {
                // Each unit of the label fills the next row over the band that can still match; when
                // the row's minimum exceeds maxDistance, no key below can match.
                var i = m
                for (pos in key.indices) {
                    // Past the last row no key can match. MiniSearch reads such rows as undefined,
                    // which never prunes and never matches; stopping here finds the same keys.
                    if (i >= rows) continue@key
                    val char = key[pos]
                    val thisRowOffset = n * i
                    val prevRowOffset = thisRowOffset - n
                    var minDistance = cell(thisRowOffset)
                    val jmin = max(0, i - maxDistance - 1)
                    val jmax = min(n - 1, i + maxDistance)
                    for (j in jmin until jmax) {
                        val different = if (char != query[j]) 1 else 0
                        val rpl = cell(prevRowOffset + j) + different
                        val del = cell(prevRowOffset + j + 1) + 1
                        val ins = cell(thisRowOffset + j) + 1
                        // `const dist = matrix[...] = Math.min(...)`: the value assigned, not the byte stored.
                        val dist = min(rpl, min(del, ins))
                        matrix[thisRowOffset + j + 1] = dist.toByte()
                        if (dist < minDistance) minDistance = dist
                    }
                    if (minDistance > maxDistance) continue@key
                    i += 1
                }
                recurse(node.child(key), i, prefix + key)
            }
        }
    }
}
