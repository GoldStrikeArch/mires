// pack/ (index.js, siblings.js, enclose.js), lcg.js and array.js's shuffle of d3-hierarchy 3.1.3.

package main

import "math"

// lcg is lcg.js's seeded generator of [0, 1). Every product stays below 2^53, so float64 is
// exact. https://en.wikipedia.org/wiki/Linear_congruential_generator#Parameters_in_common_use
type lcg struct{ s float64 }

func newLCG() *lcg { return &lcg{s: 1} }

func (g *lcg) random() float64 {
	const a, c, m = 1664525, 1013904223, 4294967296 // m is 2^32
	g.s = math.Mod(a*g.s+c, m)
	return g.s / m
}

// circle is a circle as enclose.js reads one. The front chain's nodes are copied in: they do not
// move while their enclosure is computed.
type circle struct{ x, y, r float64 }

// shuffle is array.js's Fisher-Yates from the end, with `random() * m-- | 0`.
func shuffle(array []circle, random *lcg) {
	for m := len(array); m != 0; {
		i := int(random.random() * float64(m))
		m--
		array[m], array[i] = array[i], array[m]
	}
}

// packEncloseRandom returns the smallest circle enclosing circles, by Welzl's algorithm.
func packEncloseRandom(circles []circle, random *lcg) circle {
	shuffle(circles, random)
	var basis []circle
	var e *circle
	for i := 0; i < len(circles); {
		p := circles[i]
		if e != nil && enclosesWeak(*e, p) {
			i++
		} else {
			basis = extendBasis(basis, p)
			enclosure := encloseBasis(basis)
			e = &enclosure
			i = 0
		}
	}
	return *e
}

func extendBasis(b []circle, p circle) []circle {
	if enclosesWeakAll(p, b) {
		return []circle{p}
	}

	// If we get here then B must have at least one element.
	for i := range b {
		if enclosesNot(p, b[i]) && enclosesWeakAll(encloseBasis2(b[i], p), b) {
			return []circle{b[i], p}
		}
	}

	// If we get here then B must have at least two elements.
	for i := 0; i < len(b)-1; i++ {
		for j := i + 1; j < len(b); j++ {
			if enclosesNot(encloseBasis2(b[i], b[j]), p) &&
				enclosesNot(encloseBasis2(b[i], p), b[j]) &&
				enclosesNot(encloseBasis2(b[j], p), b[i]) &&
				enclosesWeakAll(encloseBasis3(b[i], b[j], p), b) {
				return []circle{b[i], b[j], p}
			}
		}
	}

	// If we get here then something is very wrong.
	panic("no enclosing basis")
}

func enclosesNot(a, b circle) bool {
	dr, dx, dy := a.r-b.r, b.x-a.x, b.y-a.y
	return dr < 0 || dr*dr < dx*dx+dy*dy
}

func enclosesWeak(a, b circle) bool {
	dr, dx, dy := a.r-b.r+math.Max(math.Max(a.r, b.r), 1)*1e-9, b.x-a.x, b.y-a.y
	return dr > 0 && dr*dr > dx*dx+dy*dy
}

func enclosesWeakAll(a circle, b []circle) bool {
	for _, p := range b {
		if !enclosesWeak(a, p) {
			return false
		}
	}
	return true
}

func encloseBasis(b []circle) circle {
	switch len(b) {
	case 1:
		return b[0]
	case 2:
		return encloseBasis2(b[0], b[1])
	default:
		return encloseBasis3(b[0], b[1], b[2])
	}
}

func encloseBasis2(a, b circle) circle {
	x1, y1, r1 := a.x, a.y, a.r
	x2, y2, r2 := b.x, b.y, b.r
	x21, y21, r21 := x2-x1, y2-y1, r2-r1
	l := math.Sqrt(x21*x21 + y21*y21)
	return circle{(x1 + x2 + x21/l*r21) / 2, (y1 + y2 + y21/l*r21) / 2, (l + r1 + r2) / 2}
}

func encloseBasis3(a, b, c circle) circle {
	x1, y1, r1 := a.x, a.y, a.r
	x2, y2, r2 := b.x, b.y, b.r
	x3, y3, r3 := c.x, c.y, c.r
	a2 := x1 - x2
	a3 := x1 - x3
	b2 := y1 - y2
	b3 := y1 - y3
	c2 := r2 - r1
	c3 := r3 - r1
	d1 := x1*x1 + y1*y1 - r1*r1
	d2 := d1 - x2*x2 - y2*y2 + r2*r2
	d3 := d1 - x3*x3 - y3*y3 + r3*r3
	ab := a3*b2 - a2*b3
	xa := (b2*d3-b3*d2)/(ab*2) - x1
	xb := (b3*c2 - b2*c3) / ab
	ya := (a3*d2-a2*d3)/(ab*2) - y1
	yb := (a2*c3 - a3*c2) / ab
	qa := xb*xb + yb*yb - 1
	qb := 2 * (r1 + xa*xb + ya*yb)
	qc := xa*xa + ya*ya - r1*r1
	var r float64
	if math.Abs(qa) > 1e-6 {
		r = -((qb + math.Sqrt(qb*qb-4*qa*qc)) / (2 * qa))
	} else {
		r = -(qc / qb)
	}
	return circle{x1 + xa + xb*r, y1 + ya + yb*r, r}
}

// place places c tangent to a and b.
func place(b, a, c *Node) {
	dx, dy := b.x-a.x, b.y-a.y
	d2 := dx*dx + dy*dy
	if !truthy(d2) {
		c.x = a.x + c.r
		c.y = a.y
		return
	}
	a2 := a.r + c.r
	a2 *= a2
	b2 := b.r + c.r
	b2 *= b2
	if a2 > b2 {
		x := (d2 + b2 - a2) / (2 * d2)
		y := math.Sqrt(math.Max(0, b2/d2-x*x))
		c.x = b.x - x*dx - y*dy
		c.y = b.y - x*dy + y*dx
	} else {
		x := (d2 + a2 - b2) / (2 * d2)
		y := math.Sqrt(math.Max(0, a2/d2-x*x))
		c.x = a.x + x*dx - y*dy
		c.y = a.y + x*dy + y*dx
	}
}

func intersects(a, b *Node) bool {
	dr, dx, dy := a.r+b.r-1e-6, b.x-a.x, b.y-a.y
	return dr > 0 && dr*dr > dx*dx+dy*dy
}

// link is siblings.js's front-chain Node: a circle (_) and its neighbours on the chain.
type link struct {
	circle         *Node
	next, previous *link
}

// score is the squared distance from the origin to the weighted centre of node and its
// successor.
func score(node *link) float64 {
	a, b := node.circle, node.next.circle
	ab := a.r + b.r
	dx := (a.x*b.r + b.x*a.r) / ab
	dy := (a.y*b.r + b.y*a.r) / ab
	return dx*dx + dy*dy
}

// packSiblingsRandom packs circles around the origin and returns the enclosing radius.
func packSiblingsRandom(circles []*Node, random *lcg) float64 {
	n := len(circles)
	if n == 0 {
		return 0
	}

	// Place the first circle.
	first := circles[0]
	first.x, first.y = 0, 0
	if n <= 1 {
		return first.r
	}

	// Place the second circle.
	second := circles[1]
	first.x = -second.r
	second.x, second.y = first.r, 0
	if n <= 2 {
		return first.r + second.r
	}

	// Place the third circle.
	place(second, first, circles[2])

	// Initialize the front-chain using the first three circles a, b and c.
	a, b, c := &link{circle: first}, &link{circle: second}, &link{circle: circles[2]}
	a.next, c.previous = b, b
	b.next, a.previous = c, c
	c.next, b.previous = a, a

	// Attempt to place each remaining circle…
pack:
	for i := 3; i < n; i++ {
		place(a.circle, b.circle, circles[i])
		c = &link{circle: circles[i]}

		// Find the closest intersecting circle on the front-chain, if any. "Closeness" is
		// determined by linear distance along the front-chain, and so are "ahead" and "behind".
		j, k := b.next, a.previous
		sj, sk := b.circle.r, a.circle.r
		for {
			if sj <= sk {
				if intersects(j.circle, c.circle) {
					b = j
					a.next, b.previous = b, a
					i--
					continue pack
				}
				sj += j.circle.r
				j = j.next
			} else {
				if intersects(k.circle, c.circle) {
					a = k
					a.next, b.previous = b, a
					i--
					continue pack
				}
				sk += k.circle.r
				k = k.previous
			}
			if j == k.next {
				break
			}
		}

		// Success! Insert the new circle c between a and b.
		c.previous, c.next = a, b
		a.next, b.previous = c, c
		b = c

		// Compute the new closest circle pair to the centroid.
		aa := score(a)
		for c = c.next; c != b; c = c.next {
			if ca := score(c); ca < aa {
				a, aa = c, ca
			}
		}
		b = a.next
	}

	// Compute the enclosing circle of the front chain.
	front := []circle{{b.circle.x, b.circle.y, b.circle.r}}
	for c = b.next; c != b; c = c.next {
		front = append(front, circle{c.circle.x, c.circle.y, c.circle.r})
	}
	e := packEncloseRandom(front, random)

	// Translate the circles to put the enclosing circle around the origin.
	for _, node := range circles {
		node.x -= e.x
		node.y -= e.y
	}
	return e.r
}

// radiusLeaf is radiusLeaf(defaultRadius): a leaf's radius is the square root of its value.
func radiusLeaf(node *Node) {
	if node.children == nil {
		r := math.Sqrt(node.value)
		if !truthy(r) {
			r = 0
		}
		node.r = math.Max(0, r)
	}
}

// packChildrenRandom is packChildrenRandom(padding, k, random) with a constant padding.
func packChildrenRandom(node *Node, padding, k float64, random *lcg) {
	children := node.children
	if children == nil {
		return
	}
	r := padding * k
	if !truthy(r) {
		r = 0
	}
	if truthy(r) {
		for _, child := range children {
			child.r += r
		}
	}
	e := packSiblingsRandom(children, random)
	if truthy(r) {
		for _, child := range children {
			child.r -= r
		}
	}
	node.r = e + r
}

// translateChild is translateChild(k).
func translateChild(node *Node, k float64) {
	node.r *= k
	if parent := node.parent; parent != nil {
		node.x = parent.x + k*node.x
		node.y = parent.y + k*node.y
	}
}

// pack is pack().size([dx, dy]).padding(padding): one generator serves both packing passes.
func pack(root *Node, dx, dy, padding float64) {
	random := newLCG()
	root.x, root.y = dx/2, dy/2
	before, after := root.eachBefore(), root.eachAfter()
	for _, node := range before {
		radiusLeaf(node)
	}
	for _, node := range after {
		packChildrenRandom(node, 0, 1, random)
	}
	k := root.r / math.Min(dx, dy)
	for _, node := range after {
		packChildrenRandom(node, padding, k, random)
	}
	scale := math.Min(dx, dy) / (2 * root.r)
	for _, node := range before {
		translateChild(node, scale)
	}
}
