// The node of d3-hierarchy 3.1.3 and the operations the contract uses: hierarchy/index.js
// (hierarchy, computeHeight), eachBefore.js, eachAfter.js, iterator.js, sum.js and sort.js.

package main

import (
	"math"
	"sort"

	"github.com/gopherjs/gopherjs/js"
)

// Node is hierarchy/index.js's Node. data is nil for an ancestor that stratify imputed from a
// path, and children is nil for a leaf, as in d3. The layouts write the coordinates of their kind.
type Node struct {
	data *js.Object
	// id is the id stratify gave the node. d3 leaves node.id undefined unless it is a non-empty
	// string, so "" stands for undefined.
	id             string
	parent         *Node
	children       []*Node
	depth, height  int
	value          float64
	x, y, r        float64
	x0, y0, x1, y1 float64
}

// truthy is JavaScript's ToBoolean of a number: false for 0, -0 and NaN. d3 tests numbers this
// way.
func truthy(x float64) bool { return !(x == 0 || math.IsNaN(x)) }

// eachBefore returns the nodes in the order node.eachBefore visits them: pre-order, from an
// explicit stack.
func (root *Node) eachBefore() []*Node {
	var order []*Node
	nodes := []*Node{root}
	for len(nodes) > 0 {
		node := nodes[len(nodes)-1]
		nodes = nodes[:len(nodes)-1]
		order = append(order, node)
		for i := len(node.children) - 1; i >= 0; i-- {
			nodes = append(nodes, node.children[i])
		}
	}
	return order
}

// eachAfter returns the nodes in the order node.eachAfter visits them: post-order, the reverse
// of a stack walk's visiting order.
func (root *Node) eachAfter() []*Node {
	var next []*Node
	nodes := []*Node{root}
	for len(nodes) > 0 {
		node := nodes[len(nodes)-1]
		nodes = nodes[:len(nodes)-1]
		next = append(next, node)
		nodes = append(nodes, node.children...)
	}
	for i, j := 0, len(next)-1; i < j; i, j = i+1, j-1 {
		next[i], next[j] = next[j], next[i]
	}
	return next
}

// breadthFirst returns the order of node[Symbol.iterator]: each level from left to right.
func (root *Node) breadthFirst() []*Node {
	var order []*Node
	next := []*Node{root}
	for len(next) > 0 {
		current := next
		next = nil
		for _, node := range current {
			order = append(order, node)
			next = append(next, node.children...)
		}
	}
	return order
}

// hierarchy is hierarchy(data) with the default children accessor, d.children.
func hierarchy(data *js.Object) *Node {
	root := &Node{data: data}
	nodes := []*Node{root}
	for len(nodes) > 0 {
		node := nodes[len(nodes)-1]
		nodes = nodes[:len(nodes)-1]
		childs := node.data.Get("children")
		if childs == js.Undefined || childs == nil || childs.Length() == 0 {
			continue
		}
		node.children = make([]*Node, childs.Length())
		for i := len(node.children) - 1; i >= 0; i-- {
			child := &Node{data: childs.Index(i), parent: node, depth: node.depth + 1}
			node.children[i] = child
			nodes = append(nodes, child)
		}
	}
	for _, node := range root.eachBefore() {
		computeHeight(node)
	}
	return root
}

// computeHeight raises the heights of the ancestors that node makes taller.
func computeHeight(node *Node) {
	height := 0
	for {
		node.height = height
		node = node.parent
		height++
		if node == nil || node.height >= height {
			return
		}
	}
}

// weight is the reference's accessor d => d?.value ?? 0.
func weight(d *js.Object) float64 {
	if d == nil {
		return 0
	}
	v := d.Get("value")
	if v == js.Undefined || v == nil {
		return 0
	}
	return v.Float()
}

// sum is node.sum(weight): each aggregate is +weight(data) || 0 plus the children's values, the
// last child first.
func (root *Node) sum() {
	for _, node := range root.eachAfter() {
		sum := weight(node.data)
		if !truthy(sum) {
			sum = 0
		}
		for i := len(node.children) - 1; i >= 0; i-- {
			sum += node.children[i].value
		}
		node.value = sum
	}
}

// sortByValue is node.sort((a, b) => b.value - a.value). Array.prototype.sort is stable and
// counts a NaN difference as 0; sort.SliceStable keeps the same order.
func (root *Node) sortByValue() {
	for _, node := range root.eachBefore() {
		children := node.children
		sort.SliceStable(children, func(i, j int) bool { return children[j].value-children[i].value < 0 })
	}
}
