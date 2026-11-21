// Entry of the GopherJS port: one layout per input, as libraries/d3-hierarchy/reference.mjs runs
// it. The input is read through *js.Object; stratify's ids become Go (UTF-8) strings, while the
// ids of nested data are returned as the host strings they are. A failure is a panic, which
// GopherJS throws to the caller.

package main

import "github.com/gopherjs/gopherjs/js"

// id is the reference's n.id ?? n.data.id.
func id(node *Node) interface{} {
	if node.id != "" {
		return node.id
	}
	return node.data.Get("id")
}

func run(input *js.Object) interface{} {
	var root *Node
	switch input.Get("source").String() {
	case "id":
		root = stratify(input.Get("rows"), false)
	case "path":
		root = stratify(input.Get("rows"), true)
	default:
		root = hierarchy(input.Get("data"))
	}
	root.sum()
	if input.Get("sort").Bool() {
		root.sortByValue()
	}
	w, h, p := input.Get("width").Float(), input.Get("height").Float(), input.Get("padding").Float()
	op := input.Get("op").String()
	switch op {
	case "tree":
		tree(root, w, h, input.Get("nodeSize").Bool())
	case "cluster":
		cluster(root, w, h, input.Get("nodeSize").Bool())
	case "partition":
		partition(root, w, h, p, input.Get("round").Bool())
	case "pack":
		pack(root, w, h, p)
	case "treemap":
		var t tile
		switch input.Get("tile").String() {
		case "binary":
			t = binary
		case "sliceDice":
			t = sliceDice
		case "squarify":
			t = squarify
		default:
			panic("unknown tile") // .tile(undefined) throws
		}
		treemap(root, w, h, t, p, input.Get("round").Bool())
	case "hierarchy", "iterator", "stratify":
	default:
		panic("unknown layout")
	}

	var order []*Node
	if op == "iterator" {
		order = root.breadthFirst()
	} else {
		order = root.eachBefore()
	}
	output := make([]interface{}, len(order))
	for i, n := range order {
		var parent interface{}
		if n.parent != nil {
			parent = id(n.parent)
		}
		record := []interface{}{id(n), parent, n.depth, n.height, n.value}
		switch op {
		case "tree", "cluster":
			record = append(record, n.x, n.y)
		case "pack":
			record = append(record, n.x, n.y, n.r)
		case "partition", "treemap":
			record = append(record, n.x0, n.y0, n.x1, n.y1)
		}
		output[i] = record
	}
	return output
}

func main() { js.Module.Get("exports").Set("run", run) }
