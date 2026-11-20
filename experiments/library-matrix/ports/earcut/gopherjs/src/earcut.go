// Earcut 3.2.4, Copyright (c) 2026 Mapbox. See ../../LICENSE.
package main

// Removed nodes retain their links so a stale block endpoint can walk to a live node.
type node struct {
	i            int
	x, y         float64
	prev, next   *node
	prevZ, nextZ *node
	z            int32
	steiner      bool
}

type clipper struct {
	triangles             []int
	blocks                []*block
	indexActive, filtered bool
	minX, minY, invSize   float64
}

func triangulate(data []float64, holes []int, dim int) []int {
	c := clipper{}
	outerEnd := len(data)
	if len(holes) != 0 {
		outerEnd = holes[0] * dim
	}
	outer := c.linkedList(data, 0, outerEnd, dim, true)
	if outer == nil || outer.next == outer.prev {
		return c.triangles
	}
	if len(holes) != 0 {
		outer = c.eliminateHoles(data, holes, dim, outer)
	}
	if len(data) > 80*dim {
		c.minX, c.minY = data[0], data[1]
		maxX, maxY := c.minX, c.minY
		for i := dim; i < outerEnd; i += dim {
			c.minX, maxX = min(c.minX, data[i]), max(maxX, data[i])
			c.minY, maxY = min(c.minY, data[i+1]), max(maxY, data[i+1])
		}
		if span := max(maxX-c.minX, maxY-c.minY); span != 0 {
			c.invSize = 32767 / span
		}
	}
	c.clip(outer)
	return c.triangles
}

func (c *clipper) linkedList(data []float64, start, end, dim int, clockwise bool) *node {
	var last *node
	forward := clockwise == (signedArea(data, start, end, dim) > 0)
	for k := start; k < end; k += dim {
		i := k
		if !forward {
			i = end - dim - (k - start)
		}
		p := &node{i: i / dim, x: data[i], y: data[i+1]}
		if last == nil {
			p.prev, p.next = p, p
		} else {
			p.next, p.prev = last.next, last
			last.next.prev, last.next = p, p
		}
		last = p
	}
	if last != nil && equal(last, last.next) {
		c.remove(last)
		last = last.next
	}
	return last
}

func (c *clipper) remove(p *node) {
	p.next.prev, p.prev.next = p.prev, p.next
	if p.prevZ != nil {
		p.prevZ.nextZ = p.nextZ
	}
	if p.nextZ != nil {
		p.nextZ.prevZ = p.prevZ
	}
	if c.indexActive {
		c.blocks[p.prev.z].include(p.next)
	}
}

func splitPolygon(a, b *node) *node {
	a2, b2 := &node{i: a.i, x: a.x, y: a.y}, &node{i: b.i, x: b.x, y: b.y}
	an, bp := a.next, b.prev
	a.next, b.prev = b, a
	a2.next, an.prev = an, a2
	b2.next, a2.prev = a2, b2
	bp.next, b2.prev = b2, bp
	return b2
}

func (c *clipper) filter(start, end *node) *node {
	p := start
	full := start == end
	for {
		again := false
		if p != p.next && !p.steiner && (equal(p, p.next) || area(p.prev, p, p.next) == 0) {
			if full || p == end {
				end = p.prev
			}
			c.filtered = true
			c.remove(p)
			p = p.prev
			again = true
		} else if full || p != end {
			p = p.next
			again = !full
		}
		if !again && p == end {
			return end
		}
	}
}

func (c *clipper) clip(ear *node) {
	if c.invSize != 0 {
		c.indexCurve(ear)
	}
	stop, cured := ear, false
	for ear.prev != ear.next {
		prev, next := ear.prev, ear.next
		if area(prev, ear, next) < 0 && c.isEar(ear) {
			c.triangles = append(c.triangles, prev.i, ear.i, next.i)
			c.remove(ear)
			ear, stop = next, next
			continue
		}
		ear = next
		if ear != stop {
			continue
		}
		c.filtered = false
		ear = c.filter(ear, ear)
		if c.filtered {
			stop = ear
			continue
		}
		if !cured {
			ear = c.cureLocal(ear)
			stop, cured = ear, true
			continue
		}
		c.splitEarcut(ear)
		return
	}
}

type earTriangle struct {
	a, b, c        *node
	x0, y0, x1, y1 float64
}

// A pointer receiver shares the bounding box across the entire neighbor scan.
// GopherJS copies all seven fields for every candidate with a value receiver.
func (t *earTriangle) blocked(p *node) bool {
	return p.x >= t.x0 && p.x <= t.x1 && p.y >= t.y0 && p.y <= t.y1 && !equal(t.a, p) &&
		pointInTriangle(t.a.x, t.a.y, t.b.x, t.b.y, t.c.x, t.c.y, p.x, p.y) && area(p.prev, p, p.next) >= 0
}
func (c *clipper) isEar(b *node) bool {
	a, d := b.prev, b.next
	t := earTriangle{a, b, d, min(a.x, b.x, d.x), min(a.y, b.y, d.y), max(a.x, b.x, d.x), max(a.y, b.y, d.y)}
	if c.invSize == 0 {
		for p := d.next; p != a; p = p.next {
			if t.blocked(p) {
				return false
			}
		}
		return true
	}
	lo, hi := c.zOrder(t.x0, t.y0), c.zOrder(t.x1, t.y1)
	for p := b.prevZ; p != nil && p.z >= lo; p = p.prevZ {
		if p != d && t.blocked(p) {
			return false
		}
	}
	for p := b.nextZ; p != nil && p.z <= hi; p = p.nextZ {
		if p != d && t.blocked(p) {
			return false
		}
	}
	return true
}

func (c *clipper) cureLocal(start *node) *node {
	p, cured := start, false
	for {
		a, b := p.prev, p.next.next
		if intersects(a, p, p.next, b, false) && locallyInside(a, b) && locallyInside(b, a) {
			c.triangles = append(c.triangles, a.i, p.i, b.i)
			c.remove(p)
			c.remove(p.next)
			p, start, cured = b, b, true
		}
		p = p.next
		if p == start {
			break
		}
	}
	if cured {
		return c.filter(p, p)
	}
	return p
}

func (c *clipper) splitEarcut(start *node) {
	for a := start; ; {
		for b := a.next.next; b != a.prev; b = b.next {
			if a.i != b.i && validDiagonal(a, b) {
				second := splitPolygon(a, b)
				first := c.filter(a, a.next)
				second = c.filter(second, second.next)
				c.clip(first)
				c.clip(second)
				return
			}
		}
		a = a.next
		if a == start {
			return
		}
	}
}

func signedArea(data []float64, start, end, dim int) float64 {
	sum := 0.0
	j := end - dim
	for i := start; i < end; i += dim {
		sum += (data[j] - data[i]) * (data[i+1] + data[j+1])
		j = i
	}
	return sum
}
func equal(a, b *node) bool      { return a.x == b.x && a.y == b.y }
func area(p, q, r *node) float64 { return (q.y-p.y)*(r.x-q.x) - (q.x-p.x)*(r.y-q.y) }
func pointInTriangle(ax, ay, bx, by, cx, cy, px, py float64) bool {
	return (cx-px)*(ay-py) >= (ax-px)*(cy-py) && (ax-px)*(by-py) >= (bx-px)*(ay-py) && (bx-px)*(cy-py) >= (cx-px)*(by-py)
}
func onSegment(p, q, r *node) bool {
	return q.x <= max(p.x, r.x) && q.x >= min(p.x, r.x) && q.y <= max(p.y, r.y) && q.y >= min(p.y, r.y)
}
func crosses(a, b float64) bool { return (a > 0 && b < 0) || (a < 0 && b > 0) }
func intersects(p, q, r, s *node, boundary bool) bool {
	o1, o2, o3, o4 := area(p, q, r), area(p, q, s), area(r, s, p), area(r, s, q)
	if crosses(o1, o2) && crosses(o3, o4) {
		return true
	}
	return boundary && ((o1 == 0 && onSegment(p, r, q)) || (o2 == 0 && onSegment(p, s, q)) ||
		(o3 == 0 && onSegment(r, p, s)) || (o4 == 0 && onSegment(r, q, s)))
}
func locallyInside(a, b *node) bool {
	if area(a.prev, a, a.next) < 0 {
		return area(a, b, a.next) >= 0 && area(a, a.prev, b) >= 0
	}
	return area(a, b, a.prev) < 0 || area(a, a.next, b) < 0
}
func validDiagonal(a, b *node) bool {
	zero := equal(a, b) && area(a.prev, a, a.next) > 0 && area(b.prev, b, b.next) > 0
	if a.next.i == b.i || !(zero || (locallyInside(a, b) && locallyInside(b, a) && (area(a.prev, a, b.prev) != 0 || area(a, b.prev, b) != 0))) {
		return false
	}
	x0, x1 := min(a.x, b.x), max(a.x, b.x)
	y0, y1 := min(a.y, b.y), max(a.y, b.y)
	px, py := (a.x+b.x)/2, (a.y+b.y)/2
	inside := false
	for p := a; ; {
		n := p.next
		if !(p.x > x1 && n.x > x1 || p.x < x0 && n.x < x0 || p.y > y1 && n.y > y1 || p.y < y0 && n.y < y0) &&
			p.i != a.i && n.i != a.i && p.i != b.i && n.i != b.i && intersects(p, n, a, b, true) {
			return false
		}
		if (p.y > py) != (n.y > py) && px < (n.x-p.x)*(py-p.y)/(n.y-p.y)+p.x {
			inside = !inside
		}
		p = n
		if p == a {
			return zero || inside
		}
	}
}

// GopherJS lowers int32(v) to v >> 0, including out-of-range Morton coordinates.
func spread(v float64) uint32 {
	x := uint32(int32(v))
	x = (x | x<<8) & 0x00ff00ff
	x = (x | x<<4) & 0x0f0f0f0f
	x = (x | x<<2) & 0x33333333
	return (x | x<<1) & 0x55555555
}
func (c *clipper) zOrder(x, y float64) int32 {
	return int32(spread((x-c.minX)*c.invSize) | spread((y-c.minY)*c.invSize)<<1)
}
func (c *clipper) indexCurve(start *node) {
	nodes := make([]*node, 0)
	for p := start; ; {
		p.z = c.zOrder(p.x, p.y)
		nodes = append(nodes, p)
		p = p.next
		if p == start {
			break
		}
	}
	if len(nodes) <= 32 {
		for i := 1; i < len(nodes); i++ {
			n, j := nodes[i], i
			for j > 0 && nodes[j-1].z > n.z {
				nodes[j] = nodes[j-1]
				j--
			}
			nodes[j] = n
		}
	} else {
		buf := make([]*node, len(nodes))
		for shift := uint(0); shift < 32; shift += 8 {
			var counts [256]int
			for _, n := range nodes {
				counts[(uint32(n.z)>>shift)&255]++
			}
			sum := 0
			for i, count := range counts {
				counts[i] = sum
				sum += count
			}
			for _, n := range nodes {
				k := (uint32(n.z) >> shift) & 255
				buf[counts[k]] = n
				counts[k]++
			}
			nodes, buf = buf, nodes
		}
	}
	var prev *node
	for _, p := range nodes {
		p.prevZ = prev
		if prev != nil {
			prev.nextZ = p
		}
		prev = p
	}
	prev.nextZ = nil
}
