// MiniSearch's SearchableMap (minisearch 7.2.0, dist/es/index.js): a radix tree over JavaScript
// strings. Scala.js strings are JS strings, so keys are compared, measured and split on UTF-16 code
// units (`length`, `charAt`, `substring`), an edge may end inside a surrogate pair, and lone
// surrogates are ordinary units.
//
// A node is what MiniSearch's node is, an insertion-ordered map (a mutable.LinkedHashMap, as it is
// a JS Map): every non-empty key is an edge label leading to a child node, and the empty key
// (`LeafKey`) holds the value of the key that ends at the node. The order of a node's keys is
// observable (iteration, prefix and fuzzy results), and LinkedHashMap keeps it as a JS Map does:
// a new key is appended, a removed key leaves the others in place, and updating an existing key
// keeps its position. Replacing an edge (a split in `createPath`, a merge after a removal) is a set
// of the new key followed by a delete of the old one, so the new edge moves to the end.

package minisearch

import scala.annotation.tailrec
import scala.collection.mutable
import scala.collection.mutable.ArrayBuffer

/** What a node maps a key to: a child node under an edge label, or a value under `LeafKey`. */
sealed trait Entry[T]

/** The value of the key that ends at a node. */
final case class Leaf[T](value: T) extends Entry[T]

/** A radix tree node: its keys in insertion order. */
final class Node[T](val entries: mutable.LinkedHashMap[String, Entry[T]] = mutable.LinkedHashMap.empty[String, Entry[T]])
    extends Entry[T]:

  /** The child under the edge label `key` (`node.get(key)` for a key that is not `LeafKey`). */
  def child(key: String): Node[T] = entries(key) match
    case node: Node[T] => node
    case Leaf(_)       => throw IllegalStateException("the leaf key has no child")

  /** The value of the key ending here (`node.get(LEAF)`). */
  def leaf: Option[T] = entries.get(SearchableMap.LeafKey) match
    case Some(Leaf(value)) => Some(value)
    case _                 => None

/** A radix tree with string keys, or a view of one at a prefix (`atPrefix`). */
final class SearchableMap[T](tree: Node[T] = Node[T](), prefix: String = ""):
  import SearchableMap.*

  /** `_size`, memoised until the next change. MiniSearch tests it with `if (this._size)`, so 0 and
    * "unknown" are the same state, as they are here.
    */
  private var knownSize = 0

  /** A view of the entries whose keys start with `prefix`. When the prefix ends inside an edge, the
    * view is a new node holding the rest of that edge's label to its child.
    */
  def atPrefix(prefix: String): SearchableMap[T] =
    if !prefix.startsWith(this.prefix) then throw IllegalArgumentException("Mismatched prefix")
    val (node, path) = trackDown(tree, prefix.substring(this.prefix.length))
    node match
      case Some(node) => SearchableMap(node, prefix)
      case None =>
        val (parentNode, key) = path.last
        parentNode.entries.keysIterator.find(k => k != LeafKey && k.startsWith(key)) match
          case Some(k) =>
            val view = Node[T]()
            view.entries(k.substring(key.length)) = parentNode.child(k)
            SearchableMap(view, prefix)
          // MiniSearch passes an undefined node, for which the constructor's default is an empty map.
          case None => SearchableMap(Node[T](), prefix)

  def delete(key: String): Unit =
    knownSize = 0
    remove(tree, key)

  /** Every entry, in `TreeIterator` order. */
  def iterator: Iterator[(String, T)] = TreeIterator(tree, prefix)

  /** Every key within `maxEditDistance` (Levenshtein, over code units) of `key`, with its value and
    * distance, in the order the walk meets them.
    */
  def fuzzyGet(key: String, maxEditDistance: Int): mutable.LinkedHashMap[String, (T, Int)] =
    fuzzySearch(tree, key, maxEditDistance)

  def get(key: String): Option[T] = lookup(tree, key).flatMap(_.leaf)

  def has(key: String): Boolean = lookup(tree, key).exists(_.entries.contains(LeafKey))

  def size: Int =
    if knownSize == 0 then knownSize = iterator.size
    knownSize

  /** The value at `key`, inserting `initial` first when there is none (`fetch`). */
  def fetch(key: String, initial: => T): T =
    knownSize = 0
    val node = createPath(tree, key)
    node.leaf match
      case Some(value) => value
      case None =>
        val value = initial
        node.entries(LeafKey) = Leaf(value)
        value

object SearchableMap:
  /** `LEAF`: the key under which a node holds the value of the key ending at it. */
  val LeafKey = ""

  /** The node `key` ends at, or None, and the path to it: each node passed with the edge taken
    * from it, and when the key leaves the tree, the last node with the rest of the key.
    */
  private def trackDown[T](tree: Node[T], key: String): (Option[Node[T]], ArrayBuffer[(Node[T], String)]) =
    val path = ArrayBuffer.empty[(Node[T], String)]
    @tailrec def walk(node: Node[T], key: String): Option[Node[T]] =
      if key.isEmpty then Some(node)
      else
        node.entries.keysIterator.find(k => k != LeafKey && key.startsWith(k)) match
          case Some(k) =>
            path += ((node, k))
            walk(node.child(k), key.substring(k.length))
          case None =>
            path += ((node, key))
            None
    (walk(tree, key), path)

  @tailrec private def lookup[T](tree: Node[T], key: String): Option[Node[T]] =
    if key.isEmpty then Some(tree)
    else
      tree.entries.keysIterator.find(k => k != LeafKey && key.startsWith(k)) match
        case Some(k) => lookup(tree.child(k), key.substring(k.length))
        case None    => None

  /** The node `key` ends at, created when it does not exist: an edge the key leaves part way is
    * split, the shared part leading to a new intermediate node that holds the rest of the edge.
    */
  private def createPath[T](start: Node[T], key: String): Node[T] =
    val keyLength = key.length
    var node = start
    var pos = 0
    while pos < keyLength do
      val unit = key.charAt(pos)
      // A candidate edge starts with the next unit of the key (no two edges of a node share one).
      node.entries.keysIterator.find(k => k != LeafKey && k.charAt(0) == unit) match
        case Some(k) =>
          val len = math.min(keyLength - pos, k.length)
          // Advance offset to the point where key and k no longer match.
          var offset = 1
          while offset < len && key.charAt(pos + offset) == k.charAt(offset) do offset += 1
          val child = node.child(k)
          if offset == k.length then node = child
          else
            // Partial match: the intermediate node is set under the shared part and the old edge
            // deleted, so it moves to the end of the node's keys.
            val intermediate = Node[T]()
            intermediate.entries(k.substring(offset)) = child
            node.entries(key.substring(pos, pos + offset)) = intermediate
            node.entries -= k
            node = intermediate
          pos += offset
        case None =>
          // A final child node holds the rest of the key.
          val child = Node[T]()
          node.entries(key.substring(pos)) = child
          return child
    node

  /** `remove`: delete the key's leaf, then clean up an emptied node or merge a node left with a
    * single edge into its parent.
    */
  private def remove[T](tree: Node[T], key: String): Unit =
    val (found, path) = trackDown(tree, key)
    for node <- found do
      node.entries -= LeafKey
      if node.entries.isEmpty then cleanup(path)
      else if node.entries.size == 1 then
        val (onlyKey, value) = node.entries.head
        merge(path, onlyKey, value)

  @tailrec private def cleanup[T](path: collection.Seq[(Node[T], String)]): Unit =
    if path.nonEmpty then
      val (node, key) = path.last
      node.entries -= key
      if node.entries.isEmpty then cleanup(path.init)
      else if node.entries.size == 1 then
        val (onlyKey, value) = node.entries.head
        if onlyKey != LeafKey then merge(path.init, onlyKey, value)

  /** The node at the end of `path` holds the single edge `key` to `value`: its parent replaces the
    * edge to it by the two labels joined, which moves the merged edge to the end of its keys.
    */
  private def merge[T](path: collection.Seq[(Node[T], String)], key: String, value: Entry[T]): Unit =
    if path.nonEmpty then
      val (node, nodeKey) = path.last
      node.entries(nodeKey + key) = value
      node.entries -= nodeKey

  /** `fuzzySearch`: a depth-first walk over each node's keys, oldest first, keeping a Levenshtein
    * matrix of `query.length + 1` columns and `query.length + 1 + maxDistance` rows. MiniSearch keeps
    * it in a Uint8Array, so every store is reduced modulo 256 (here a byte, read back unsigned).
    */
  private def fuzzySearch[T](node: Node[T], query: String, maxDistance: Int): mutable.LinkedHashMap[String, (T, Int)] =
    val results = mutable.LinkedHashMap.empty[String, (T, Int)]
    // Number of columns in the Levenshtein matrix.
    val n = query.length + 1
    // Matching terms can never be longer than N + maxDistance.
    val m = n + maxDistance
    // Fill first matrix row and column with numbers: 0 1 2 3 ...
    val matrix = Array.fill[Byte](m * n)((maxDistance + 1).toByte)
    for j <- 0 until n do matrix(j) = j.toByte
    for i <- 1 until m do matrix(i * n) = i.toByte
    recurse(node, query, maxDistance, results, matrix, 1, n, "")
    results

  /** One node of the fuzzy walk; `m` is the next matrix row to fill, one past the length of `prefix`. */
  private def recurse[T](
      node: Node[T],
      query: String,
      maxDistance: Int,
      results: mutable.LinkedHashMap[String, (T, Int)],
      matrix: Array[Byte],
      m: Int,
      n: Int,
      prefix: String,
  ): Unit =
    val offset = m * n
    val rows = matrix.length / n
    node.entries.foreachEntry { (key, entry) =>
      entry match
        case Leaf(value) =>
          // A leaf: the last cell of the current row is the edit distance of the key.
          val distance = matrix(offset - 1) & 0xff
          if distance <= maxDistance then results(prefix) = (value, distance)
        case child: Node[T] =>
          // Every unit of the label fills the next row; once the row's minimum exceeds the maximum
          // distance no key below can match, and the subtree is skipped.
          var i = m
          var pos = 0
          var reachable = true
          while reachable && pos < key.length do
            if i >= rows then
              // Past the last row a key is longer than query.length + maxDistance and cannot match.
              // MiniSearch reads past its Uint8Array there (undefined never prunes and never
              // matches); stopping finds the same keys.
              reachable = false
            else
              val char = key.charAt(pos)
              val thisRowOffset = n * i
              val prevRowOffset = thisRowOffset - n
              // Set the first column based on the previous row, and initialize the minimum distance
              // in the current row.
              var minDistance = matrix(thisRowOffset) & 0xff
              val jmin = math.max(0, i - maxDistance - 1)
              val jmax = math.min(n - 1, i + maxDistance)
              var j = jmin
              while j < jmax do
                val different = if char != query.charAt(j) then 1 else 0
                val rpl = (matrix(prevRowOffset + j) & 0xff) + different
                val del = (matrix(prevRowOffset + j + 1) & 0xff) + 1
                val ins = (matrix(thisRowOffset + j) & 0xff) + 1
                val dist = math.min(rpl, math.min(del, ins))
                matrix(thisRowOffset + j + 1) = dist.toByte
                if dist < minDistance then minDistance = dist
                j += 1
              if minDistance > maxDistance then reachable = false
              else
                pos += 1
                i += 1
          if reachable then recurse(child, query, maxDistance, results, matrix, i, n, prefix + key)
    }

/** MiniSearch's `TreeIterator`: depth first from the view's root. Entering a node snapshots its keys,
  * which are then visited from the most recently inserted to the oldest (the snapshot is popped from
  * its end); a leaf yields the key spelled by the labels on the path. MiniSearch deletes the key just
  * visited while it iterates (vacuuming, and search's cleanup of discarded postings); the snapshots,
  * and the detached nodes a merge leaves on the path, keep the walk on exactly the keys the tree had
  * when it began.
  */
private final class TreeIterator[T](root: Node[T], prefix: String) extends Iterator[(String, T)]:
  import SearchableMap.LeafKey

  /** The nodes from the root down to the current one, each with the keys still to visit (last first). */
  private val path = ArrayBuffer.empty[(Node[T], ArrayBuffer[String])]
  if root.entries.nonEmpty then path += ((root, ArrayBuffer.from(root.entries.keys)))

  def hasNext: Boolean = path.nonEmpty

  def next(): (String, T) =
    if path.isEmpty then throw java.util.NoSuchElementException("TreeIterator.next")
    val value = dive()
    backtrack()
    value

  @tailrec private def dive(): (String, T) =
    val (node, keys) = path.last
    if keys.last == LeafKey then (key, value)
    else
      val child = node.child(keys.last)
      path += ((child, ArrayBuffer.from(child.entries.keys)))
      dive()

  @tailrec private def backtrack(): Unit =
    if path.nonEmpty then
      val keys = path.last._2
      keys.dropRightInPlace(1)
      if keys.isEmpty then
        path.dropRightInPlace(1)
        backtrack()

  private def key: String = prefix + path.iterator.map(_._2.last).filter(_ != LeafKey).mkString

  private def value: T = path.last._1.leaf.getOrElse(throw IllegalStateException("TreeIterator: no leaf"))
