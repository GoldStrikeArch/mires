// stratify.js of d3-hierarchy 3.1.3, with the default id and parentId accessors.
package hierarchy

import scala.collection.mutable
import scala.scalajs.js

/**
 * `stratify()(data)`, or `stratify().path(path)(data)` when `path` is given. An imputed ancestor
 * is a node whose data is None; d3 marks one with the `imputed` object and replaces that by null
 * before it returns.
 */
def stratify(data: js.Array[Datum], path: Option[Datum => String] = None): Node =
  val rows = mutable.ArrayBuffer.from[Option[Datum]](data.iterator.map(Some(_)))
  // d3 tests ids as `id != null && (id += "")`: undefined, null and "" all name nothing, and ""
  // stands for the three here.
  val (ids, parentIds) = path match
    case Some(path) =>
      val i = mutable.ArrayBuffer.from(data.iterator.map(d => normalize(path(d))))
      val p = i.map(parentof)
      val s = mutable.HashSet.from(i) += ""
      var k = 0
      while k < p.size do
        val id = p(k)
        k += 1
        if s.add(id) then
          i += id
          p += parentof(id)
          rows += None
      (i, p)
    case None =>
      (mutable.ArrayBuffer.from(data.iterator.map(d => present(d.id).getOrElse(""))),
        mutable.ArrayBuffer.from(data.iterator.map(d => present(d.parentId).getOrElse(""))))

  var n = rows.size
  val ambiguous = Node(None)
  val nodeByKey = mutable.HashMap.empty[String, Node]
  val nodes = rows.map(Node(_))
  for i <- 0 until n do
    val nodeId = ids(i)
    if nodeId.nonEmpty then
      nodes(i).id = Some(nodeId)
      nodeByKey(nodeId) = if nodeByKey.contains(nodeId) then ambiguous else nodes(i)

  var first: Option[Node] = None
  for i <- 0 until n do
    val node = nodes(i)
    val key = parentIds(i)
    if key.nonEmpty then
      val parent = nodeByKey.getOrElse(key, throw IllegalArgumentException("missing: " + key))
      if parent eq ambiguous then throw IllegalArgumentException("ambiguous: " + key)
      parent.children match
        case Some(children) => children += node
        case None => parent.children = Some(mutable.ArrayBuffer(node))
      node.parent = Some(parent)
    else
      if first.isDefined then throw IllegalArgumentException("multiple roots")
      first = Some(node)
  var root = first.getOrElse(throw IllegalArgumentException("no root"))

  // When imputing internal nodes, only introduce roots if needed.
  if path.isDefined then
    while root.data.isEmpty && root.children.get.size == 1 do
      root = root.children.get(0)
      n -= 1

  // d3 parents the root to `preroot`, whose depth is -1, while it walks.
  root.parent = None
  root.eachBefore { node =>
    node.depth = node.parent.fold(-1)(_.depth) + 1
    n -= 1
  }.eachBefore(computeHeight)
  if n > 0 then throw IllegalArgumentException("cycle")
  root

/**
 * Coerces nothing (ids are strings here), strips a trailing slash unless another slash precedes
 * it, and adds a leading slash if missing.
 */
private def normalize(path: String): String =
  val i = path.length
  val p = if slash(path, i - 1) && !slash(path, i - 2) then path.substring(0, i - 1) else path
  if p.startsWith("/") then p else "/" + p

/** The path up to its last delimiting slash after the first character: "/a/b" to "/a", "/" to "". */
private def parentof(path: String): String =
  var i = path.length
  if i < 2 then return ""
  i -= 1
  while i > 1 && !slash(path, i) do i -= 1
  path.substring(0, i)

/** Whether `path(i)` is a delimiting slash: one preceded by an even number of backslashes. */
private def slash(path: String, i: Int): Boolean =
  if i >= 0 && i < path.length && path.charAt(i) == '/' then
    var k = 0
    var j = i
    while j > 0 && path.charAt(j - 1) == '\\' do
      j -= 1
      k += 1
    (k & 1) == 0
  else false
