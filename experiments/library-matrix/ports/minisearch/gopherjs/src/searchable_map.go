// MiniSearch's SearchableMap: a radix tree over UTF-16 keys.
//
// A node is what MiniSearch's node is, an insertion-ordered map: edge labels to child nodes, and
// the empty label (LEAF) to the value of the key that ends at the node. The order of a node's
// entries is observable (iteration, prefix and fuzzy results, toJSON), so every operation keeps
// exactly the order a JavaScript Map would: set of a new key appends, delete removes in place,
// and replacing an edge (a split in createPath, a merge after a removal) is set of the new edge
// followed by delete of the old one, which moves it to the end. A Go map has no order (its
// iteration order is deliberately random), so a node is a slice of entries.
//
// Labels are compared and split on code units, so an edge may end inside a surrogate pair.

package main

// node is a radix tree node: its entries in insertion order.
type node struct {
	entries []*entry
}

// entry is one key of a node and its value. Entries never change: a split or a merge makes new
// ones.
type entry struct {
	label []uint16  // the key: empty for the leaf, the value of the key ending here
	child *node     // an edge's subtree
	data  *termData // the leaf's value
}

func (e *entry) isLeaf() bool {
	return len(e.label) == 0
}

// leaf is node.get(LEAF): the value of the key ending at this node, or nil.
func (n *node) leaf() *termData {
	for _, e := range n.entries {
		if e.isLeaf() {
			return e.data
		}
	}
	return nil
}

// edgeStartingWith is the edge whose label starts with unit (no two edges of a node share their
// first unit), or nil.
func (n *node) edgeStartingWith(unit uint16) *entry {
	for _, e := range n.entries {
		if !e.isLeaf() && e.label[0] == unit {
			return e
		}
	}
	return nil
}

// edgePrefixOf is the edge whose label is a prefix of key (key.startsWith(k)), or nil.
func (n *node) edgePrefixOf(key []uint16) *entry {
	if e := n.edgeStartingWith(key[0]); e != nil && hasPrefix(key, e.label) {
		return e
	}
	return nil
}

// add is Map.set of a key the node does not hold: it is appended. (Every set MiniSearch makes on a
// node is of a new key: a leaf where there is none, or an edge whose first unit no other edge of
// the node has, since the edge it replaces is deleted only after it.)
func (n *node) add(e *entry) {
	n.entries = append(n.entries, e)
}

// delete is Map.delete: the entries after the key keep their order.
func (n *node) delete(label []uint16) {
	for i, e := range n.entries {
		if equal(e.label, label) {
			copy(n.entries[i:], n.entries[i+1:])
			n.entries[len(n.entries)-1] = nil
			n.entries = n.entries[:len(n.entries)-1]
			return
		}
	}
}

// SearchableMap is the radix tree MiniSearch's index is.
type SearchableMap struct {
	root *node
}

func newSearchableMap() *SearchableMap {
	return &SearchableMap{root: &node{}}
}

// lookup is the node key ends at, or nil.
func lookup(n *node, key []uint16) *node {
	for len(key) > 0 {
		e := n.edgePrefixOf(key)
		if e == nil {
			return nil
		}
		n, key = e.child, key[len(e.label):]
	}
	return n
}

// get is the value of key, or nil.
func (m *SearchableMap) get(key []uint16) *termData {
	if n := lookup(m.root, key); n != nil {
		return n.leaf()
	}
	return nil
}

// has reports whether key is in the map.
func (m *SearchableMap) has(key []uint16) bool {
	return m.get(key) != nil
}

// fetch is the value of key, created (an empty termData) when there is none.
func (m *SearchableMap) fetch(key []uint16) *termData {
	n := createPath(m.root, key)
	data := n.leaf()
	if data == nil {
		data = &termData{}
		n.add(&entry{data: data})
	}
	return data
}

// createPath is the node key ends at, created when it does not exist. Where key leaves an edge,
// the edge is split: an intermediate node holding the rest of the edge is set under the shared
// part and the old edge deleted, so the intermediate node moves to the end of its parent.
func createPath(n *node, key []uint16) *node {
	pos := 0
outer:
	for pos < len(key) {
		unit := key[pos]
		for _, e := range n.entries {
			if e.isLeaf() || e.label[0] != unit {
				continue
			}
			length := min(len(key)-pos, len(e.label))
			offset := 1
			for offset < length && key[pos+offset] == e.label[offset] {
				offset++
			}
			if offset == len(e.label) {
				n = e.child
			} else {
				intermediate := &node{entries: []*entry{{label: e.label[offset:], child: e.child}}}
				n.add(&entry{label: clip(key[pos : pos+offset]), child: intermediate})
				n.delete(e.label)
				n = intermediate
			}
			pos += offset
			continue outer
		}
		child := &node{}
		n.add(&entry{label: clip(key[pos:]), child: child})
		return child
	}
	return n
}

// step is one step of trackDown's path: a node and the label taken from it.
type step struct {
	node  *node
	label []uint16
}

// trackDown follows key down from n. It returns the node key ends at, or nil when key ends inside
// an edge or leaves the tree, with the path taken; in the second case the last step holds the
// node where the walk stopped and the rest of the key.
func trackDown(n *node, key []uint16) (*node, []step) {
	var path []step
	for len(key) > 0 {
		e := n.edgePrefixOf(key)
		if e == nil {
			return nil, append(path, step{n, key})
		}
		path = append(path, step{n, e.label})
		n, key = e.child, key[len(e.label):]
	}
	return n, path
}

// delete removes key (remove): delete its leaf, then clean up an emptied node or merge a node left
// with a single edge into its parent.
func (m *SearchableMap) delete(key []uint16) {
	n, path := trackDown(m.root, key)
	if n == nil {
		return
	}
	n.delete(nil)
	switch len(n.entries) {
	case 0:
		cleanup(path)
	case 1:
		merge(path, n.entries[0])
	}
}

// cleanup deletes the edge to the empty node at the end of path; if its parent is now empty, that
// is cleaned up in turn, and if the parent now holds a single edge, it is merged into its own
// parent. Nothing happens above the root.
func cleanup(path []step) {
	if len(path) == 0 {
		return
	}
	last := path[len(path)-1]
	last.node.delete(last.label)
	switch len(last.node.entries) {
	case 0:
		cleanup(path[:len(path)-1])
	case 1:
		if e := last.node.entries[0]; !e.isLeaf() {
			merge(path[:len(path)-1], e)
		}
	}
}

// merge replaces the edge to the node at the end of path, whose only entry is the edge e, by one
// edge with the two labels joined: set, then delete, so the merged edge moves to the end of the
// parent. Nothing happens at the root.
func merge(path []step, e *entry) {
	if len(path) == 0 {
		return
	}
	last := path[len(path)-1]
	last.node.add(&entry{label: concat(last.label, e.label), child: e.child})
	last.node.delete(last.label)
}

// termEntry is a term and its value, as an iteration yields them.
type termEntry struct {
	term []uint16
	data *termData
}

// walk appends the entries below n in TreeIterator order: depth first, each node's keys from the
// most recently inserted to the oldest (the iterator pops its snapshot of the keys from the end).
// prefix spells the path to n; its spare capacity is reused below, so every term is copied.
func walk(n *node, prefix []uint16, out []termEntry) []termEntry {
	for i := len(n.entries) - 1; i >= 0; i-- {
		e := n.entries[i]
		if e.isLeaf() {
			out = append(out, termEntry{append([]uint16(nil), prefix...), e.data})
		} else {
			out = walk(e.child, append(prefix, e.label...), out)
		}
	}
	return out
}

// entries is every term with its value, in iteration order.
//
// MiniSearch deletes terms while its iterator walks the tree (vacuuming, and the cleanup of
// discarded postings during a prefix search). The iterator works on snapshots of each node's
// keys, and only the term just visited is ever deleted, so it visits exactly the terms of the tree
// as it was when the walk began (CONTRACT.md, "The index"). Collecting them first is therefore
// the same walk.
func (m *SearchableMap) entries() []termEntry {
	return walk(m.root, nil, nil)
}

// atPrefix is the entries of atPrefix(prefix), in iteration order: the subtree where prefix ends,
// or, when it ends inside an edge whose label continues it, the subtree below that edge.
func (m *SearchableMap) atPrefix(prefix []uint16) []termEntry {
	n, path := trackDown(m.root, prefix)
	if n != nil {
		return walk(n, clip(prefix), nil)
	}
	last := path[len(path)-1]
	for _, e := range last.node.entries {
		if !e.isLeaf() && hasPrefix(e.label, last.label) {
			return walk(e.child, concat(prefix, e.label[len(last.label):]), nil)
		}
	}
	return nil
}

// size is the number of terms.
func (m *SearchableMap) size() int {
	return m.root.size()
}

func (n *node) size() int {
	total := 0
	for _, e := range n.entries {
		if e.isLeaf() {
			total++
		} else {
			total += e.child.size()
		}
	}
	return total
}

// fuzzyMatch is a term within the edit distance of a fuzzy query, with its value and distance.
type fuzzyMatch struct {
	term     []uint16
	data     *termData
	distance int
}

// fuzzyGet is fuzzySearch: every term within maxDistance (Levenshtein, over code units) of query,
// in the order the walk meets them: depth first, each node's keys oldest first.
func (m *SearchableMap) fuzzyGet(query []uint16, maxDistance int) []fuzzyMatch {
	// The Levenshtein matrix: one column per query prefix, one row per key prefix up to the longest
	// key that can still match. MiniSearch keeps it in a Uint8Array, as here: every store is reduced
	// modulo 256 (inside the parity domain no value reaches 256).
	columns := len(query) + 1
	rows := columns + maxDistance
	matrix := make([]uint8, rows*columns)
	for i := range matrix {
		matrix[i] = uint8(maxDistance + 1)
	}
	for j := 0; j < columns; j++ {
		matrix[j] = uint8(j)
	}
	for i := 1; i < rows; i++ {
		matrix[i*columns] = uint8(i)
	}
	f := fuzzySearch{query: query, maxDistance: maxDistance, columns: columns, rows: rows, matrix: matrix}
	f.recurse(m.root, 1, nil)
	return f.results
}

// fuzzySearch is the state of the walk.
type fuzzySearch struct {
	query                      []uint16
	maxDistance, columns, rows int
	matrix                     []uint8
	results                    []fuzzyMatch
}

// recurse visits the keys of n; row is the next matrix row to fill, one past the length of prefix.
func (f *fuzzySearch) recurse(n *node, row int, prefix []uint16) {
	columns := f.columns
entries:
	for _, e := range n.entries {
		if e.isLeaf() {
			// The last cell of the current row is the distance of the term spelled by prefix.
			if distance := int(f.matrix[row*columns-1]); distance <= f.maxDistance {
				f.results = append(f.results, fuzzyMatch{append([]uint16(nil), prefix...), e.data, distance})
			}
			continue
		}
		i := row
		for _, char := range e.label {
			if i >= f.rows {
				// Past the last row no key can match. MiniSearch walks on (reading undefined, which
				// never prunes and never matches); stopping here finds the same keys.
				continue entries
			}
			thisRow := columns * i
			previousRow := thisRow - columns
			minDistance := int(f.matrix[thisRow])
			jmin := max(0, i-f.maxDistance-1)
			jmax := min(columns-1, i+f.maxDistance)
			for j := jmin; j < jmax; j++ {
				different := 0
				if char != f.query[j] {
					different = 1
				}
				replace := int(f.matrix[previousRow+j]) + different
				del := int(f.matrix[previousRow+j+1]) + 1
				insert := int(f.matrix[thisRow+j]) + 1
				distance := min(replace, del, insert)
				f.matrix[thisRow+j+1] = uint8(distance)
				if distance < minDistance {
					minDistance = distance
				}
			}
			// Distances never decrease along a row: no key below can match.
			if minDistance > f.maxDistance {
				continue entries
			}
			i++
		}
		f.recurse(e.child, i, append(prefix, e.label...))
	}
}
