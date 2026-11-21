// stratify.js of d3-hierarchy 3.1.3, with the default id and parentId accessors.

package main

import (
	"strings"

	"github.com/gopherjs/gopherjs/js"
)

// text reads a string field. d3 tests ids as `id != null && (id += "")`: undefined, null and ""
// all name nothing, and "" stands for the three here.
func text(d *js.Object, key string) string {
	v := d.Get(key)
	if v == js.Undefined || v == nil {
		return ""
	}
	return v.String()
}

// stratify is stratify()(data), or stratify().path(d => d.id)(data) when path is set. An imputed
// ancestor is a node whose data is nil; d3 marks one with its imputed object and replaces that by
// null before it returns.
func stratify(data *js.Object, path bool) *Node {
	rows := make([]*js.Object, data.Length())
	ids := make([]string, len(rows))
	parentIDs := make([]string, len(rows))
	for i := range rows {
		rows[i] = data.Index(i)
		if path {
			ids[i] = normalize(text(rows[i], "id"))
			parentIDs[i] = parentof(ids[i])
		} else {
			ids[i] = text(rows[i], "id")
			parentIDs[i] = text(rows[i], "parentId")
		}
	}
	if path {
		seen := map[string]bool{"": true}
		for _, id := range ids {
			seen[id] = true
		}
		for k := 0; k < len(parentIDs); k++ {
			if id := parentIDs[k]; !seen[id] {
				seen[id] = true
				ids = append(ids, id)
				parentIDs = append(parentIDs, parentof(id))
				rows = append(rows, nil)
			}
		}
	}

	n := len(rows)
	ambiguous := &Node{}
	nodeByKey := map[string]*Node{}
	nodes := make([]*Node, n)
	for i, d := range rows {
		node := &Node{data: d}
		nodes[i] = node
		if id := ids[i]; id != "" {
			node.id = id
			if _, ok := nodeByKey[id]; ok {
				nodeByKey[id] = ambiguous
			} else {
				nodeByKey[id] = node
			}
		}
	}

	var root *Node
	for i, node := range nodes {
		if key := parentIDs[i]; key != "" {
			parent, ok := nodeByKey[key]
			if !ok {
				panic("missing: " + key)
			}
			if parent == ambiguous {
				panic("ambiguous: " + key)
			}
			parent.children = append(parent.children, node)
			node.parent = parent
		} else {
			if root != nil {
				panic("multiple roots")
			}
			root = node
		}
	}
	if root == nil {
		panic("no root")
	}

	// When imputing internal nodes, only introduce roots if needed.
	if path {
		for root.data == nil && len(root.children) == 1 {
			root = root.children[0]
			n--
		}
	}

	// d3 parents the root to preroot, whose depth is -1, while it walks.
	root.parent = nil
	order := root.eachBefore()
	for _, node := range order {
		if node.parent == nil {
			node.depth = 0
		} else {
			node.depth = node.parent.depth + 1
		}
		n--
	}
	for _, node := range order {
		computeHeight(node)
	}
	if n > 0 {
		panic("cycle")
	}
	return root
}

// normalize coerces nothing (ids are strings here), strips a trailing slash unless another
// slash precedes it, and adds a leading slash if missing.
func normalize(path string) string {
	i := len(path)
	if slash(path, i-1) && !slash(path, i-2) {
		path = path[:i-1]
	}
	if strings.HasPrefix(path, "/") {
		return path
	}
	return "/" + path
}

// parentof returns the path up to its last delimiting slash after the first character: "/a/b"
// to "/a", "/" to "".
func parentof(path string) string {
	i := len(path)
	if i < 2 {
		return ""
	}
	for i--; i > 1; i-- {
		if slash(path, i) {
			break
		}
	}
	return path[:i]
}

// slash reports whether path[i] is a delimiting slash: one preceded by an even number of
// backslashes. Go strings are UTF-8, and '/' and '\\' are never part of a multi-byte sequence.
func slash(path string, i int) bool {
	if i >= 0 && i < len(path) && path[i] == '/' {
		k := 0
		for i > 0 && path[i-1] == '\\' {
			i--
			k++
		}
		return k&1 == 0
	}
	return false
}
