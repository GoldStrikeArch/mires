// treemap/ (index.js, dice.js, slice.js, sliceDice.js, squarify.js, binary.js, round.js) and
// partition.js of d3-hierarchy 3.1.3.

package main

import (
	"math"

	"github.com/gopherjs/gopherjs/js"
)

// tile names one of d3's tiling methods. d3 passes the function itself.
type tile int

const (
	squarify tile = iota
	binary
	sliceDice
)

// layOut lays the children of parent out in the rectangle [x0, y0, x1, y1].
func (t tile) layOut(parent *Node, x0, y0, x1, y1 float64) {
	switch t {
	case binary:
		treemapBinary(parent, x0, y0, x1, y1)
	case sliceDice:
		treemapSliceDice(parent, x0, y0, x1, y1)
	default:
		treemapSquarify(parent, x0, y0, x1, y1)
	}
}

// roundNode is round.js. Math.round, from the host, rounds halves up, keeps -0 and returns -0
// for [-0.5, 0).
func roundNode(node *Node) {
	m := js.Global.Get("Math")
	node.x0 = m.Call("round", node.x0).Float()
	node.y0 = m.Call("round", node.y0).Float()
	node.x1 = m.Call("round", node.x1).Float()
	node.y1 = m.Call("round", node.y1).Float()
}

// treemapDice is dice.js on nodes, the children of a parent or of a squarified row, whose value
// is value: columns from left to right.
func treemapDice(nodes []*Node, value, x0, y0, x1, y1 float64) {
	k := value
	if truthy(value) {
		k = (x1 - x0) / value
	}
	for _, node := range nodes {
		node.y0, node.y1 = y0, y1
		node.x0 = x0
		x0 += node.value * k
		node.x1 = x0
	}
}

// treemapSlice is slice.js, as treemapDice: rows from top to bottom.
func treemapSlice(nodes []*Node, value, x0, y0, x1, y1 float64) {
	k := value
	if truthy(value) {
		k = (y1 - y0) / value
	}
	for _, node := range nodes {
		node.x0, node.x1 = x0, x1
		node.y0 = y0
		y0 += node.value * k
		node.y1 = y0
	}
}

// treemapSliceDice is sliceDice.js: slice at odd depths, dice at even ones.
func treemapSliceDice(parent *Node, x0, y0, x1, y1 float64) {
	if parent.depth&1 != 0 {
		treemapSlice(parent.children, parent.value, x0, y0, x1, y1)
	} else {
		treemapDice(parent.children, parent.value, x0, y0, x1, y1)
	}
}

// phi is the golden ratio, squarify.js's default ratio.
var phi = (1 + math.Sqrt(5)) / 2

// treemapSquarify is squarify.js with the default ratio.
func treemapSquarify(parent *Node, x0, y0, x1, y1 float64) {
	squarifyRatio(phi, parent, x0, y0, x1, y1)
}

// squarifyRatio lays out the rows of squarify.js. d3 also collects them, for
// treemapResquarify; this port only lays them out.
func squarifyRatio(ratio float64, parent *Node, x0, y0, x1, y1 float64) {
	nodes := parent.children
	n := len(nodes)
	i0, i1 := 0, 0
	value := parent.value

	for i0 < n {
		dx, dy := x1-x0, y1-y0

		// Find the next non-empty node.
		sumValue := nodes[i1].value
		i1++
		for !truthy(sumValue) && i1 < n {
			sumValue = nodes[i1].value
			i1++
		}
		minValue, maxValue := sumValue, sumValue
		alpha := math.Max(dy/dx, dx/dy) / (value * ratio)
		beta := sumValue * sumValue * alpha
		minRatio := math.Max(maxValue/beta, beta/minValue)

		// Keep adding nodes while the aspect ratio maintains or improves.
		for ; i1 < n; i1++ {
			nodeValue := nodes[i1].value
			sumValue += nodeValue
			if nodeValue < minValue {
				minValue = nodeValue
			}
			if nodeValue > maxValue {
				maxValue = nodeValue
			}
			beta = sumValue * sumValue * alpha
			newRatio := math.Max(maxValue/beta, beta/minValue)
			if newRatio > minRatio {
				sumValue -= nodeValue
				break
			}
			minRatio = newRatio
		}

		// Position the row in its orientation: the row ends where the remaining space begins.
		row := nodes[i0:i1]
		if dx < dy {
			bottom := y1
			if truthy(value) {
				bottom = y0 + dy*sumValue/value
			}
			treemapDice(row, sumValue, x0, y0, x1, bottom)
			if truthy(value) {
				y0 = bottom
			}
		} else {
			right := x1
			if truthy(value) {
				right = x0 + dx*sumValue/value
			}
			treemapSlice(row, sumValue, x0, y0, right, y1)
			if truthy(value) {
				x0 = right
			}
		}
		value -= sumValue
		i0 = i1
	}
}

// treemapBinary is binary.js: it splits the children in two of about equal value, recursively.
func treemapBinary(parent *Node, x0, y0, x1, y1 float64) {
	nodes := parent.children
	sums := make([]float64, len(nodes)+1)
	sum := 0.0
	for i, node := range nodes {
		sum += node.value
		sums[i+1] = sum
	}
	binaryPartition(nodes, sums, 0, len(nodes), parent.value, x0, y0, x1, y1)
}

// binaryPartition is binary.js's inner partition(i, j, value, x0, y0, x1, y1).
func binaryPartition(nodes []*Node, sums []float64, i, j int, value, x0, y0, x1, y1 float64) {
	if i >= j-1 {
		node := nodes[i]
		node.x0, node.y0 = x0, y0
		node.x1, node.y1 = x1, y1
		return
	}

	valueOffset := sums[i]
	valueTarget := value/2 + valueOffset
	k, hi := i+1, j-1
	for k < hi {
		mid := int(uint(k+hi) >> 1)
		if sums[mid] < valueTarget {
			k = mid + 1
		} else {
			hi = mid
		}
	}
	if valueTarget-sums[k-1] < sums[k]-valueTarget && i+1 < k {
		k--
	}

	valueLeft := sums[k] - valueOffset
	valueRight := value - valueLeft
	if x1-x0 > y1-y0 {
		xk := x1
		if truthy(value) {
			xk = (x0*valueRight + x1*valueLeft) / value
		}
		binaryPartition(nodes, sums, i, k, valueLeft, x0, y0, xk, y1)
		binaryPartition(nodes, sums, k, j, valueRight, xk, y0, x1, y1)
	} else {
		yk := y1
		if truthy(value) {
			yk = (y0*valueRight + y1*valueLeft) / value
		}
		binaryPartition(nodes, sums, i, k, valueLeft, x0, y0, x1, yk)
		binaryPartition(nodes, sums, k, j, valueRight, x0, yk, x1, y1)
	}
}

// collapse is the treemap's and partition's `if (x1 < x0) x0 = x1 = (x0 + x1) / 2`, and the same
// for y.
func collapse(x0, y0, x1, y1 float64) (float64, float64, float64, float64) {
	if x1 < x0 {
		x0 = (x0 + x1) / 2
		x1 = x0
	}
	if y1 < y0 {
		y0 = (y0 + y1) / 2
		y1 = y0
	}
	return x0, y0, x1, y1
}

// treemap is treemap().size([dx, dy]).tile(t).padding(padding).round(round): the inner and
// every outer padding are padding.
func treemap(root *Node, dx, dy float64, t tile, padding float64, round bool) {
	// d3 keeps the stack between calls and resets it to [0]; depth d reads index d.
	paddingStack := make([]float64, root.height+1)
	root.x0, root.y0 = 0, 0
	root.x1, root.y1 = dx, dy
	order := root.eachBefore()
	for _, node := range order {
		p := paddingStack[node.depth]
		x0, y0, x1, y1 := collapse(node.x0+p, node.y0+p, node.x1-p, node.y1-p)
		node.x0, node.y0, node.x1, node.y1 = x0, y0, x1, y1
		if node.children != nil {
			p = padding / 2
			paddingStack[node.depth+1] = p
			x0, y0, x1, y1 = collapse(x0+(padding-p), y0+(padding-p), x1-(padding-p), y1-(padding-p))
			t.layOut(node, x0, y0, x1, y1)
		}
	}
	if round {
		for _, node := range order {
			roundNode(node)
		}
	}
}

// partition is partition().size([dx, dy]).padding(padding).round(round).
func partition(root *Node, dx, dy, padding float64, round bool) {
	n := float64(root.height + 1)
	root.x0, root.y0 = padding, padding
	root.x1, root.y1 = dx, dy/n
	order := root.eachBefore()
	for _, node := range order {
		if node.children != nil {
			treemapDice(node.children, node.value, node.x0, dy*float64(node.depth+1)/n, node.x1, dy*float64(node.depth+2)/n)
		}
		node.x0, node.y0, node.x1, node.y1 = collapse(node.x0, node.y0, node.x1-padding, node.y1-padding)
	}
	if round {
		for _, node := range order {
			roundNode(node)
		}
	}
}
