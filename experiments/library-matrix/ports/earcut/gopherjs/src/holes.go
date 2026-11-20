package main

import "math"

type block struct {
	head, stop             *node
	minX, minY, maxX, maxY float64
}

func (b *block) include(p *node) {
	b.minX, b.minY = min(b.minX, p.x), min(b.minY, p.y)
	b.maxX, b.maxY = max(b.maxX, p.x), max(b.maxY, p.y)
}
func (c *clipper) indexSegment(head, stop *node) {
	p := head
	for {
		b := &block{head: p, minX: p.x, maxX: p.x, minY: p.y, maxY: p.y}
		for k := 0; k < 16; k++ {
			p.z = int32(len(c.blocks))
			p = p.next
			b.include(p)
			if p == stop {
				break
			}
		}
		b.stop = p
		c.blocks = append(c.blocks, b)
		if p == stop {
			return
		}
	}
}
func live(p *node) *node {
	for p.prev.next != p {
		p = p.next
	}
	return p
}
func leftmost(start *node) *node {
	left := start
	for p := start.next; p != start; p = p.next {
		if p.x < left.x || (p.x == left.x && p.y < left.y) {
			left = p
		}
	}
	return left
}
func holeLess(a, b *node) bool {
	if a.x != b.x {
		return a.x < b.x
	}
	if a.y != b.y {
		return a.y < b.y
	}
	return (a.next.y-a.y)/(a.next.x-a.x) < (b.next.y-b.y)/(b.next.x-b.x)
}

// Stable mergesort keeps tied leftmost corners in their original hole order.
func sortHoles(q, tmp []*node) {
	if len(q) < 2 {
		return
	}
	mid := len(q) / 2
	sortHoles(q[:mid], tmp[:mid])
	sortHoles(q[mid:], tmp[mid:])
	i, j := 0, mid
	for k := range q {
		if j == len(q) || (i < mid && !holeLess(q[j], q[i])) {
			tmp[k] = q[i]
			i++
		} else {
			tmp[k] = q[j]
			j++
		}
	}
	copy(q, tmp)
}
func (c *clipper) eliminateHoles(data []float64, holes []int, dim int, outer *node) *node {
	queue := make([]*node, len(holes))
	for i, start := range holes {
		end := len(data)
		if i+1 < len(holes) {
			end = holes[i+1] * dim
		}
		ring := c.linkedList(data, start*dim, end, dim, false)
		ring.steiner = ring == ring.next
		queue[i] = leftmost(ring)
	}
	sortHoles(queue, make([]*node, len(queue)))
	c.indexSegment(outer, outer)
	c.indexActive = true
	for _, hole := range queue {
		bridge := c.findBridge(hole, outer)
		if bridge == nil {
			continue
		}
		reverse := splitPolygon(bridge, hole)
		c.indexSegment(bridge, reverse.next.next)
		c.filter(reverse, reverse.next)
		outer = c.filter(bridge, bridge.next)
	}
	c.indexActive = false
	return c.filter(outer, outer)
}

func (c *clipper) findBridge(hole, outer *node) *node {
	if equal(hole, outer) {
		return outer
	}
	hx, hy := hole.x, hole.y
	qx := math.Inf(-1)
	var m *node
	for _, b := range c.blocks {
		if hy < b.minY || hy > b.maxY || b.minX > hx || b.maxX <= qx {
			continue
		}
		p, stop := live(b.head), live(b.stop)
		b.head, b.stop = p, stop
		for {
			n := p.next
			if p.prev.next == p {
				if equal(hole, n) {
					return n
				}
				if hy <= p.y && hy >= n.y && n.y != p.y {
					x := p.x + (hy-p.y)*(n.x-p.x)/(n.y-p.y)
					if x <= hx && x > qx {
						qx, m = x, n
						if p.x < n.x {
							m = p
						}
						if x == hx {
							return m
						}
					}
				}
			}
			p = n
			if p == stop {
				break
			}
		}
	}
	if m == nil {
		return nil
	}
	mx, my := m.x, m.y
	ax, cx := qx, hx
	if hy < my {
		ax, cx = hx, qx
	}
	tanMin := math.Inf(1)
	for _, b := range c.blocks {
		if b.maxX < mx || b.minX > hx || b.maxY < min(hy, my) || b.minY > max(hy, my) {
			continue
		}
		p, stop := live(b.head), live(b.stop)
		b.head, b.stop = p, stop
		for {
			if p.prev.next == p && hx > p.x && p.x >= mx && pointInTriangle(ax, hy, mx, my, cx, hy, p.x, p.y) {
				tan := math.Abs(hy-p.y) / (hx - p.x)
				if (locallyInside(p, hole) || (p.y == hy && p.next.y == hy && p.next.x > hx)) &&
					(tan < tanMin || tan == tanMin && (p.x > m.x || p.x == m.x && area(m.prev, m, p.prev) < 0 && area(p.next, m, m.next) < 0)) {
					m, tanMin = p, tan
				}
			}
			p = p.next
			if p == stop {
				break
			}
		}
	}
	return m
}
