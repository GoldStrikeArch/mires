// tree.js (Buchheim et al.'s linear-time Reingold-Tilford layout) and cluster.js of d3-hierarchy
// 3.1.3, with the default separation.

package main

import "math"

// separation is defaultSeparation, the same function in tree.js and cluster.js.
func separation(a, b *Node) float64 {
	if a.parent == b.parent {
		return 1
	}
	return 2
}

// treeNode is tree.js's TreeNode: the layout's bookkeeping for node (_), the child number i.
type treeNode struct {
	node            *Node
	i               int
	parent          *treeNode
	children        []*treeNode
	defaultAncestor *treeNode // A
	ancestor        *treeNode // a
	prelim          float64   // z
	mod             float64   // m
	change          float64   // c
	shift           float64   // s
	thread          *treeNode // t
}

func newTreeNode(node *Node, i int) *treeNode {
	t := &treeNode{node: node, i: i}
	t.ancestor = t
	return t
}

// eachBefore and eachAfter return the orders of node.eachBefore and node.eachAfter.
func (root *treeNode) eachBefore() []*treeNode {
	var order []*treeNode
	nodes := []*treeNode{root}
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

func (root *treeNode) eachAfter() []*treeNode {
	var next []*treeNode
	nodes := []*treeNode{root}
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

// nextLeft is the left contour's successor of v: its first child, or its thread.
func nextLeft(v *treeNode) *treeNode {
	if len(v.children) > 0 {
		return v.children[0]
	}
	return v.thread
}

// nextRight is the right contour's successor of v: its last child, or its thread.
func nextRight(v *treeNode) *treeNode {
	if len(v.children) > 0 {
		return v.children[len(v.children)-1]
	}
	return v.thread
}

// moveSubtree shifts the subtree rooted at wp and prepares the shifts of the smaller subtrees in
// between.
func moveSubtree(wm, wp *treeNode, shift float64) {
	change := shift / float64(wp.i-wm.i)
	wp.change -= change
	wp.shift += shift
	wm.change += change
	wp.prelim += shift
	wp.mod += shift
}

// executeShifts applies the shifts that moveSubtree prepared to the children of v.
func executeShifts(v *treeNode) {
	shift, change := 0.0, 0.0
	for i := len(v.children) - 1; i >= 0; i-- {
		w := v.children[i]
		w.prelim += shift
		w.mod += shift
		change += w.change
		shift += w.shift + change
	}
}

// nextAncestor is vim's ancestor if it is a sibling of v, otherwise the default ancestor.
func nextAncestor(vim, v, ancestor *treeNode) *treeNode {
	if vim.ancestor.parent == v.parent {
		return vim.ancestor
	}
	return ancestor
}

// treeRoot wraps every node in a treeNode, under an extra parent of the root.
func treeRoot(root *Node) *treeNode {
	tree := newTreeNode(root, 0)
	nodes := []*treeNode{tree}
	for len(nodes) > 0 {
		node := nodes[len(nodes)-1]
		nodes = nodes[:len(nodes)-1]
		children := node.node.children
		if children == nil {
			continue
		}
		node.children = make([]*treeNode, len(children))
		for i := len(children) - 1; i >= 0; i-- {
			child := newTreeNode(children[i], i)
			child.parent = node
			node.children[i] = child
			nodes = append(nodes, child)
		}
	}
	// d3's extra parent wraps null; it is never read, so it wraps nothing here either.
	tree.parent = newTreeNode(nil, 0)
	tree.parent.children = []*treeNode{tree}
	return tree
}

// firstWalk computes the preliminary x of v, after its children; then combines it with its left
// siblings.
func firstWalk(v *treeNode) {
	siblings := v.parent.children
	var w *treeNode
	if v.i != 0 {
		w = siblings[v.i-1]
	}
	if children := v.children; children != nil {
		executeShifts(v)
		midpoint := (children[0].prelim + children[len(children)-1].prelim) / 2
		if w != nil {
			v.prelim = w.prelim + separation(v.node, w.node)
			v.mod = v.prelim - midpoint
		} else {
			v.prelim = midpoint
		}
	} else if w != nil {
		v.prelim = w.prelim + separation(v.node, w.node)
	}
	ancestor := v.parent.defaultAncestor
	if ancestor == nil {
		ancestor = siblings[0]
	}
	v.parent.defaultAncestor = apportion(v, w, ancestor)
}

// secondWalk computes the real x of v by summing the modifiers of its ancestors.
func secondWalk(v *treeNode) {
	v.node.x = v.prelim + v.parent.mod
	v.mod += v.parent.mod
}

// apportion is the core of the algorithm: it combines the subtree of v with those of its left
// siblings, walking the inside and outside contours, and returns the new default ancestor.
func apportion(v, w, ancestor *treeNode) *treeNode {
	if w == nil {
		return ancestor
	}
	vip, vop, vim, vom := v, v, w, v.parent.children[0]
	sip, sop, sim, som := vip.mod, vop.mod, vim.mod, vom.mod
	for vim, vip = nextRight(vim), nextLeft(vip); vim != nil && vip != nil; vim, vip = nextRight(vim), nextLeft(vip) {
		vom = nextLeft(vom)
		vop = nextRight(vop)
		vop.ancestor = v
		shift := vim.prelim + sim - vip.prelim - sip + separation(vim.node, vip.node)
		if shift > 0 {
			moveSubtree(nextAncestor(vim, v, ancestor), v, shift)
			sip += shift
			sop += shift
		}
		sim += vim.mod
		sip += vip.mod
		som += vom.mod
		sop += vop.mod
	}
	if vim != nil && nextRight(vop) == nil {
		vop.thread = vim
		vop.mod += sim - sop
	}
	if vip != nil && nextLeft(vom) == nil {
		vom.thread = vip
		vom.mod += sip - som
		ancestor = v
	}
	return ancestor
}

// tree is tree().size([dx, dy]), or tree().nodeSize([dx, dy]) when nodeSize is set.
func tree(root *Node, dx, dy float64, nodeSize bool) {
	t := treeRoot(root)

	// Compute the layout using Buchheim et al.'s algorithm.
	for _, v := range t.eachAfter() {
		firstWalk(v)
	}
	t.parent.mod = -t.prelim
	for _, v := range t.eachBefore() {
		secondWalk(v)
	}

	if nodeSize {
		// A fixed node size scales x and y.
		for _, node := range root.eachBefore() {
			node.x *= dx
			node.y = float64(node.depth) * dy
		}
		return
	}

	// A fixed tree size scales x and y by the extent of the left-most, right-most and deepest
	// nodes.
	order := root.eachBefore()
	left, right, bottom := root, root, root
	for _, node := range order {
		if node.x < left.x {
			left = node
		}
		if node.x > right.x {
			right = node
		}
		if node.depth > bottom.depth {
			bottom = node
		}
	}
	s := 1.0
	if left != right {
		s = separation(left, right) / 2
	}
	tx := s - left.x
	kx := dx / (right.x + s + tx)
	depth := bottom.depth
	if depth == 0 {
		depth = 1
	}
	ky := dy / float64(depth)
	for _, node := range order {
		node.x = (node.x + tx) * kx
		node.y = float64(node.depth) * ky
	}
}

// cluster is cluster().size([dx, dy]), or cluster().nodeSize([dx, dy]) when nodeSize is set.
func cluster(root *Node, dx, dy float64, nodeSize bool) {
	var previousNode *Node
	x := 0.0
	order := root.eachAfter()

	// First walk, computing the initial x and y values.
	for _, node := range order {
		if children := node.children; children != nil {
			sum, y := 0.0, 0.0
			for _, c := range children {
				sum += c.x
				y = math.Max(y, c.y)
			}
			node.x = sum / float64(len(children)) // meanX
			node.y = 1 + y                        // maxY
		} else {
			if previousNode != nil {
				x += separation(node, previousNode)
				node.x = x
			} else {
				node.x = 0
			}
			node.y = 0
			previousNode = node
		}
	}

	left, right := root, root
	for left.children != nil {
		left = left.children[0]
	}
	for right.children != nil {
		right = right.children[len(right.children)-1]
	}
	x0 := left.x - separation(left, right)/2
	x1 := right.x + separation(right, left)/2

	// Second walk, normalizing x and y to the desired size. The root comes last, so the walk
	// reads the root's unnormalized position throughout.
	for _, node := range order {
		if nodeSize {
			node.x = (node.x - root.x) * dx
			node.y = (root.y - node.y) * dy
		} else {
			node.x = (node.x - x0) / (x1 - x0) * dx
			k := 1.0
			if truthy(root.y) {
				k = node.y / root.y
			}
			node.y = (1 - k) * dy
		}
	}
}
