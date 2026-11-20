package main

import "math"

type node struct {
	id                                                               string
	kids                                                             []int
	parent, depth, height, number, ancestor, defaultAncestor, thread int
	value, x, y, r, x0, y0, x1, y1, z, m, c, s                       float64
}

func circle(x, y, r float64) *node { return &node{x: x, y: y, r: r} }
func ints(n int) []int             { return make([]int, n) }
func floats(n int) []float64       { return make([]float64, n) }
func real(x int) float64           { return float64(x) }
func integer(x float64) int        { return int(x) }
func minf(x, y float64) float64    { return math.Min(x, y) }
func maxf(x, y float64) float64    { return math.Max(x, y) }
func sqrtf(x float64) float64      { return math.Sqrt(x) }
func absf(x float64) float64       { return math.Abs(x) }
func floorf(x float64) float64     { return math.Floor(x) }
func roundf(x float64) float64 {
	if x >= -0.5 && x < 0 {
		return math.Copysign(0, -1)
	}
	return math.Floor(x + 0.5)
}
func invalidBasis() int { panic("invalid enclosing basis") }

// d3-hierarchy 3.1.3 layout algorithms. Child and contour links are arena indices.
func separation(nodes []*node, a int, b int) float64 {
	return (func() float64 {
		if nodes[a].parent == nodes[b].parent {
			return 1.0
		}
		return 2.0
	})()
}

func contour(nodes []*node, v int, left bool) int {
	kids := nodes[v].kids
	if len(kids) == 0 {
		return nodes[v].thread
	}
	return (func() int {
		if left {
			return kids[0]
		}
		return kids[(len(kids) - 1)]
	})()
}

func apportion(nodes []*node, v int, previous int, fallback int) int {
	ancestor := fallback
	if previous < 0 {
		return ancestor
	}
	vip := v
	vop := v
	vim := previous
	vom := nodes[nodes[v].parent].kids[0]
	sip := nodes[vip].m
	sop := nodes[vop].m
	sim := nodes[vim].m
	som := nodes[vom].m
	vim = contour(nodes, vim, false)
	vip = contour(nodes, vip, true)
	for (vim >= 0) && (vip >= 0) {
		vom = contour(nodes, vom, true)
		vop = contour(nodes, vop, false)
		nodes[vop].ancestor = v
		shift := ((((nodes[vim].z + sim) - nodes[vip].z) - sip) + separation(nodes, vim, vip))
		if shift > 0.0 {
			anc := nodes[vim].ancestor
			wm := (func() int {
				if nodes[anc].parent == nodes[v].parent {
					return anc
				}
				return ancestor
			})()
			change := (shift / real((nodes[v].number - nodes[wm].number)))
			nodes[v].c = (nodes[v].c - change)
			nodes[v].s = (nodes[v].s + shift)
			nodes[wm].c = (nodes[wm].c + change)
			nodes[v].z = (nodes[v].z + shift)
			nodes[v].m = (nodes[v].m + shift)
			sip = (sip + shift)
			sop = (sop + shift)
		}
		sim = (sim + nodes[vim].m)
		sip = (sip + nodes[vip].m)
		som = (som + nodes[vom].m)
		sop = (sop + nodes[vop].m)
		vim = contour(nodes, vim, false)
		vip = contour(nodes, vip, true)
	}
	if (vim >= 0) && (contour(nodes, vop, false) < 0) {
		nodes[vop].thread = vim
		nodes[vop].m = (nodes[vop].m + (sim - sop))
	}
	if (vip >= 0) && (contour(nodes, vom, true) < 0) {
		nodes[vom].thread = vip
		nodes[vom].m = (nodes[vom].m + (sip - som))
		ancestor = v
	}
	return ancestor
}

func tidy(nodes []*node, pre []int, post []int, w float64, h float64, fixed bool) {
	root := pre[0]
	for _, v := range pre {
		nodes[v].ancestor = v
		nodes[v].thread = (-1)
		nodes[v].defaultAncestor = (-1)
		kids := nodes[v].kids
		for i := 0; i < len(kids); i++ {
			nodes[kids[i]].number = i
		}
	}
	for _, v := range post {
		n := nodes[v]
		p := n.parent
		prev := (-1)
		if n.number > 0 {
			prev = nodes[p].kids[(n.number - 1)]
		}
		kids := n.kids
		if len(kids) > 0 {
			shift := 0.0
			change := 0.0
			for i := len(kids) - 1; i >= 0; i-- {
				child := nodes[kids[i]]
				child.z = (child.z + shift)
				child.m = (child.m + shift)
				change = (change + child.c)
				shift = (shift + (child.s + change))
			}
			mid := ((nodes[kids[0]].z + nodes[kids[(len(kids)-1)]].z) / 2.0)
			if prev < 0 {
				n.z = mid
			} else {
				n.z = (nodes[prev].z + separation(nodes, v, prev))
				n.m = (n.z - mid)
			}
		} else {
			if prev >= 0 {
				n.z = (nodes[prev].z + separation(nodes, v, prev))
			}
		}
		if p >= 0 {
			ancestor := nodes[p].defaultAncestor
			if ancestor < 0 {
				ancestor = nodes[p].kids[0]
			}
			nodes[p].defaultAncestor = apportion(nodes, v, prev, ancestor)
		}
	}
	offset := (-nodes[root].z)
	left := root
	right := root
	depth := 0
	for _, v := range pre {
		n := nodes[v]
		modifier := (func() float64 {
			if v == root {
				return offset
			}
			return nodes[n.parent].m
		})()
		n.x = (n.z + modifier)
		n.m = (n.m + modifier)
		if n.x < nodes[left].x {
			left = v
		}
		if n.x > nodes[right].x {
			right = v
		}
		if n.depth > depth {
			depth = n.depth
		}
	}
	margin := (func() float64 {
		if left == right {
			return 1.0
		}
		return (separation(nodes, left, right) / 2.0)
	})()
	tx := (margin - nodes[left].x)
	kx := (w / ((nodes[right].x + margin) + tx))
	ky := (h / real((func() int {
		if depth == 0 {
			return 1
		}
		return depth
	})()))
	for _, v := range pre {
		n := nodes[v]
		if fixed {
			n.x = (n.x * w)
			n.y = (real(n.depth) * h)
		} else {
			n.x = ((n.x + tx) * kx)
			n.y = (real(n.depth) * ky)
		}
	}
}

func cluster(nodes []*node, pre []int, post []int, w float64, h float64, fixed bool) {
	previous := (-1)
	x := 0.0
	for _, v := range post {
		n := nodes[v]
		if len(n.kids) == 0 {
			if previous >= 0 {
				x = (x + separation(nodes, v, previous))
			}
			n.x = x
			previous = v
		} else {
			height := 0.0
			total := 0.0
			for _, c := range n.kids {
				total = (total + nodes[c].x)
				height = maxf(height, nodes[c].y)
			}
			n.x = (total / real(len(n.kids)))
			n.y = (1.0 + height)
		}
	}
	left := post[0]
	right := previous
	x0 := (nodes[left].x - (separation(nodes, left, right) / 2.0))
	x1 := (nodes[right].x + (separation(nodes, right, left) / 2.0))
	rx := nodes[pre[0]].x
	ry := nodes[pre[0]].y
	for _, v := range post {
		n := nodes[v]
		if fixed {
			n.x = ((n.x - rx) * w)
			n.y = ((ry - n.y) * h)
		} else {
			n.x = (((n.x - x0) / (x1 - x0)) * w)
			n.y = ((1.0 - (func() float64 {
				if ry != 0.0 {
					return (n.y / ry)
				}
				return 1.0
			})()) * h)
		}
	}
}

func tileSlice(nodes []*node, kids []int, start int, stop int, total float64, x0 float64, y0 float64, x1 float64, y1 float64, dice bool) {
	x := x0
	y := y0
	scale := (func() float64 {
		if total == 0.0 {
			return total
		}
		return (func() float64 {
			if dice {
				return ((x1 - x) / total)
			}
			return ((y1 - y) / total)
		})()
	})()
	for i := start; i < stop; i++ {
		n := nodes[kids[i]]
		n.x0 = x
		n.y0 = y
		n.x1 = x1
		n.y1 = y1
		if dice {
			x = (x + (n.value * scale))
			n.x1 = x
		} else {
			y = (y + (n.value * scale))
			n.y1 = y
		}
	}
}

func binary(nodes []*node, kids []int, sums []float64, start int, stop int, total float64, x0 float64, y0 float64, x1 float64, y1 float64) {
	if start >= (stop - 1) {
		n := nodes[kids[start]]
		n.x0 = x0
		n.y0 = y0
		n.x1 = x1
		n.y1 = y1
	} else {
		off := sums[start]
		target := ((total / 2.0) + off)
		k := (start + 1)
		hi := (stop - 1)
		for k < hi {
			mid := ((k + hi) / 2)
			if sums[mid] < target {
				k = (mid + 1)
			} else {
				hi = mid
			}
		}
		if ((target - sums[(k-1)]) < (sums[k] - target)) && ((start + 1) < k) {
			k = (k - 1)
		}
		left := (sums[k] - off)
		right := (total - left)
		if (x1 - x0) > (y1 - y0) {
			x := (func() float64 {
				if total != 0.0 {
					return (((x0 * right) + (x1 * left)) / total)
				}
				return x1
			})()
			binary(nodes, kids, sums, start, k, left, x0, y0, x, y1)
			binary(nodes, kids, sums, k, stop, right, x, y0, x1, y1)
		} else {
			y := (func() float64 {
				if total != 0.0 {
					return (((y0 * right) + (y1 * left)) / total)
				}
				return y1
			})()
			binary(nodes, kids, sums, start, k, left, x0, y0, x1, y)
			binary(nodes, kids, sums, k, stop, right, x0, y, x1, y1)
		}
	}
}

func squarify(nodes []*node, kids []int, value float64, left float64, top float64, x1 float64, y1 float64) {
	total := value
	x := left
	y := top
	i := 0
	j := 0
	ratio := ((1.0 + sqrtf(5.0)) / 2.0)
	for i < len(kids) {
		dx := (x1 - x)
		dy := (y1 - y)
		row := nodes[kids[j]].value
		j = (j + 1)
		for (row == 0.0) && (j < len(kids)) {
			row = nodes[kids[j]].value
			j = (j + 1)
		}
		low := row
		high := row
		alpha := (maxf((dy/dx), (dx/dy)) / (total * ratio))
		beta := ((row * row) * alpha)
		best := maxf((high / beta), (beta / low))
		finished := false
		for (j < len(kids)) && (!finished) {
			v := nodes[kids[j]].value
			row = (row + v)
			low = minf(low, v)
			high = maxf(high, v)
			beta = ((row * row) * alpha)
			score := maxf((high / beta), (beta / low))
			if score > best {
				row = (row - v)
				finished = true
			} else {
				best = score
				j = (j + 1)
			}
		}
		if dx < dy {
			bottom := (func() float64 {
				if total != 0.0 {
					return (y + ((dy * row) / total))
				}
				return y1
			})()
			tileSlice(nodes, kids, i, j, row, x, y, x1, bottom, true)
			if total != 0.0 {
				y = bottom
			}
		} else {
			right := (func() float64 {
				if total != 0.0 {
					return (x + ((dx * row) / total))
				}
				return x1
			})()
			tileSlice(nodes, kids, i, j, row, x, y, right, y1, false)
			if total != 0.0 {
				x = right
			}
		}
		total = (total - row)
		i = j
	}
}

func rectangles(nodes []*node, pre []int, w float64, h float64, p float64, partition bool, tile string, rounded bool) {
	root := nodes[pre[0]]
	levels := real((root.height + 1))
	root.x1 = w
	root.y1 = h
	if partition {
		root.x0 = p
		root.y0 = p
		root.y1 = (h / levels)
	}
	for _, v := range pre {
		n := nodes[v]
		kids := n.kids
		inset := (func() float64 {
			if n.parent >= 0 {
				return (p / 2.0)
			}
			return 0.0
		})()
		if partition {
			if len(kids) > 0 {
				tileSlice(nodes, kids, 0, len(kids), n.value, n.x0, ((h * real((n.depth + 1))) / levels), n.x1, ((h * real((n.depth + 2))) / levels), true)
			}
			n.x1 = (n.x1 - p)
			n.y1 = (n.y1 - p)
		} else {
			n.x0 = (n.x0 + inset)
			n.y0 = (n.y0 + inset)
			n.x1 = (n.x1 - inset)
			n.y1 = (n.y1 - inset)
		}
		if n.x1 < n.x0 {
			n.x0 = ((n.x0 + n.x1) / 2.0)
			n.x1 = n.x0
		}
		if n.y1 < n.y0 {
			n.y0 = ((n.y0 + n.y1) / 2.0)
			n.y1 = n.y0
		}
		if (!partition) && (len(kids) > 0) {
			gap := (p - (p / 2.0))
			x := (n.x0 + gap)
			y := (n.y0 + gap)
			xx := (n.x1 - gap)
			yy := (n.y1 - gap)
			if xx < x {
				x = ((x + xx) / 2.0)
				xx = x
			}
			if yy < y {
				y = ((y + yy) / 2.0)
				yy = y
			}
			if tile == "binary" {
				sums := floats((len(kids) + 1))
				for j := 0; j < len(kids); j++ {
					sums[(j + 1)] = (sums[j] + nodes[kids[j]].value)
				}
				binary(nodes, kids, sums, 0, len(kids), n.value, x, y, xx, yy)
			} else {
				if tile == "sliceDice" {
					tileSlice(nodes, kids, 0, len(kids), n.value, x, y, xx, yy, ((n.depth % 2) == 0))
				} else {
					squarify(nodes, kids, n.value, x, y, xx, yy)
				}
			}
		}
	}
	if rounded {
		for _, v := range pre {
			n := nodes[v]
			n.x0 = roundf(n.x0)
			n.x1 = roundf(n.x1)
			n.y0 = roundf(n.y0)
			n.y1 = roundf(n.y1)
		}
	}
}

func weak(a *node, b *node) bool {
	dr := ((a.r - b.r) + (maxf(maxf(a.r, b.r), 1.0) * 1e-09))
	dx := (b.x - a.x)
	dy := (b.y - a.y)
	return ((dr > 0.0) && ((dr * dr) > ((dx * dx) + (dy * dy))))
}

func outside(a *node, b *node) bool {
	dr := (a.r - b.r)
	dx := (b.x - a.x)
	dy := (b.y - a.y)
	return ((dr < 0.0) || ((dr * dr) < ((dx * dx) + (dy * dy))))
}

func pair(a *node, b *node) *node {
	dx := (b.x - a.x)
	dy := (b.y - a.y)
	dr := (b.r - a.r)
	distance := sqrtf(((dx * dx) + (dy * dy)))
	return circle((((a.x + b.x) + ((dx / distance) * dr)) / 2.0), (((a.y + b.y) + ((dy / distance) * dr)) / 2.0), (((distance + a.r) + b.r) / 2.0))
}

func triple(a *node, b *node, c *node) *node {
	a2 := (a.x - b.x)
	a3 := (a.x - c.x)
	b2 := (a.y - b.y)
	b3 := (a.y - c.y)
	c2 := (b.r - a.r)
	c3 := (c.r - a.r)
	d1 := (((a.x * a.x) + (a.y * a.y)) - (a.r * a.r))
	d2 := (((d1 - (b.x * b.x)) - (b.y * b.y)) + (b.r * b.r))
	d3 := (((d1 - (c.x * c.x)) - (c.y * c.y)) + (c.r * c.r))
	ab := ((a3 * b2) - (a2 * b3))
	xa := ((((b2 * d3) - (b3 * d2)) / (ab * 2.0)) - a.x)
	xb := (((b3 * c2) - (b2 * c3)) / ab)
	ya := ((((a3 * d2) - (a2 * d3)) / (ab * 2.0)) - a.y)
	yb := (((a2 * c3) - (a3 * c2)) / ab)
	qa := (((xb * xb) + (yb * yb)) - 1.0)
	qb := (2.0 * ((a.r + (xa * xb)) + (ya * yb)))
	qc := (((xa * xa) + (ya * ya)) - (a.r * a.r))
	r := (-(func() float64 {
		if absf(qa) > 1e-06 {
			return ((qb + sqrtf(((qb * qb) - ((4.0 * qa) * qc)))) / (2.0 * qa))
		}
		return (qc / qb)
	})())
	return circle(((a.x + xa) + (xb * r)), ((a.y + ya) + (yb * r)), r)
}

func covers(nodes []*node, basis []int, count int, a *node) bool {
	i := 0
	for (i < count) && weak(a, nodes[basis[i]]) {
		i = (i + 1)
	}
	return (i == count)
}

func extend(nodes []*node, basis []int, count int, p int) int {
	if covers(nodes, basis, count, nodes[p]) {
		basis[0] = p
		return 1
	}
	for i := 0; i < count; i++ {
		x := basis[i]
		if outside(nodes[p], nodes[x]) && covers(nodes, basis, count, pair(nodes[x], nodes[p])) {
			basis[0] = x
			basis[1] = p
			return 2
		}
	}
	for i := 0; i < count; i++ {
		x := basis[i]
		for j := (i + 1); j < count; j++ {
			y := basis[j]
			if outside(pair(nodes[x], nodes[y]), nodes[p]) && outside(pair(nodes[x], nodes[p]), nodes[y]) && outside(pair(nodes[y], nodes[p]), nodes[x]) && covers(nodes, basis, count, triple(nodes[x], nodes[y], nodes[p])) {
				basis[0] = x
				basis[1] = y
				basis[2] = p
				return 3
			}
		}
	}
	return invalidBasis()
}

func enclose(nodes []*node, front []int, n int, seed []float64) *node {
	for j := n - 1; j >= 0; j-- {
		step := ((1664525.0 * seed[0]) + 1013904223.0)
		seed[0] = (step - (floorf((step / 4294967296.0)) * 4294967296.0))
		k := integer(((seed[0] / 4294967296.0) * real((j + 1))))
		save := front[j]
		front[j] = front[k]
		front[k] = save
	}
	basis := ints(3)
	count := 0
	i := 0
	result := circle(0.0, 0.0, 0.0)
	for i < n {
		p := front[i]
		if (count > 0) && weak(result, nodes[p]) {
			i = (i + 1)
		} else {
			count = extend(nodes, basis, count, p)
			if count == 1 {
				result = circle(nodes[p].x, nodes[p].y, nodes[p].r)
			} else {
				if count == 2 {
					result = pair(nodes[basis[0]], nodes[basis[1]])
				} else {
					result = triple(nodes[basis[0]], nodes[basis[1]], nodes[basis[2]])
				}
			}
			i = 0
		}
	}
	return result
}

func place(b *node, a *node, c *node) {
	dx := (b.x - a.x)
	dy := (b.y - a.y)
	d2 := ((dx * dx) + (dy * dy))
	if d2 != 0.0 {
		a2 := ((a.r + c.r) * (a.r + c.r))
		b2 := ((b.r + c.r) * (b.r + c.r))
		if a2 > b2 {
			x := (((d2 + b2) - a2) / (2.0 * d2))
			y := sqrtf(maxf(0.0, ((b2 / d2) - (x * x))))
			c.x = ((b.x - (x * dx)) - (y * dy))
			c.y = ((b.y - (x * dy)) + (y * dx))
		} else {
			x := (((d2 + a2) - b2) / (2.0 * d2))
			y := sqrtf(maxf(0.0, ((a2 / d2) - (x * x))))
			c.x = ((a.x + (x * dx)) - (y * dy))
			c.y = ((a.y + (x * dy)) + (y * dx))
		}
	} else {
		c.x = (a.x + c.r)
		c.y = a.y
	}
}

func intersects(a *node, b *node) bool {
	dr := ((a.r + b.r) - 1e-06)
	dx := (b.x - a.x)
	dy := (b.y - a.y)
	return ((dr > 0.0) && ((dr * dr) > ((dx * dx) + (dy * dy))))
}

func score(a *node, b *node) float64 {
	ab := (a.r + b.r)
	dx := (((a.x * b.r) + (b.x * a.r)) / ab)
	dy := (((a.y * b.r) + (b.y * a.r)) / ab)
	return ((dx * dx) + (dy * dy))
}

func siblings(nodes []*node, kids []int, seed []float64) float64 {
	n := len(kids)
	a := 0
	b := 1
	nodes[kids[0]].x = 0.0
	nodes[kids[0]].y = 0.0
	if n == 1 {
		return nodes[kids[0]].r
	}
	nodes[kids[0]].x = (-nodes[kids[1]].r)
	nodes[kids[1]].x = nodes[kids[0]].r
	nodes[kids[1]].y = 0.0
	if n == 2 {
		return (nodes[kids[0]].r + nodes[kids[1]].r)
	}
	place(nodes[kids[1]], nodes[kids[0]], nodes[kids[2]])
	after := ints(n)
	before := ints(n)
	after[0] = 1
	before[2] = 1
	after[1] = 2
	before[0] = 2
	after[2] = 0
	before[1] = 0
	i := 3
	for i < n {
		place(nodes[kids[a]], nodes[kids[b]], nodes[kids[i]])
		j := after[b]
		k := before[a]
		sj := nodes[kids[b]].r
		sk := nodes[kids[a]].r
		retry := false
		more := true
		for more && (!retry) {
			if sj <= sk {
				if intersects(nodes[kids[j]], nodes[kids[i]]) {
					b = j
					after[a] = b
					before[b] = a
					retry = true
				} else {
					sj = (sj + nodes[kids[j]].r)
					j = after[j]
				}
			} else {
				if intersects(nodes[kids[k]], nodes[kids[i]]) {
					a = k
					after[a] = b
					before[b] = a
					retry = true
				} else {
					sk = (sk + nodes[kids[k]].r)
					k = before[k]
				}
			}
			more = (j != after[k])
		}
		if !retry {
			before[i] = a
			after[i] = b
			after[a] = i
			before[b] = i
			b = i
			best := score(nodes[kids[a]], nodes[kids[after[a]]])
			j = after[i]
			for j != b {
				candidateScore := score(nodes[kids[j]], nodes[kids[after[j]]])
				if candidateScore < best {
					best = candidateScore
					a = j
				}
				j = after[j]
			}
			b = after[a]
			i = (i + 1)
		}
	}
	front := ints(n)
	front[0] = kids[b]
	count := 1
	cursor := after[b]
	for cursor != b {
		front[count] = kids[cursor]
		count = (count + 1)
		cursor = after[cursor]
	}
	enclosure := enclose(nodes, front, count, seed)
	for _, id := range kids {
		nodes[id].x = (nodes[id].x - enclosure.x)
		nodes[id].y = (nodes[id].y - enclosure.y)
	}
	return enclosure.r
}

func packChildren(nodes []*node, post []int, padding float64, seed []float64) {
	pad := (func() float64 {
		if padding != padding {
			return 0.0
		}
		return padding
	})()
	for _, i := range post {
		kids := nodes[i].kids
		if len(kids) > 0 {
			if pad != 0.0 {
				for _, k := range kids {
					nodes[k].r = (nodes[k].r + pad)
				}
			}
			radius := siblings(nodes, kids, seed)
			if pad != 0.0 {
				for _, k := range kids {
					nodes[k].r = (nodes[k].r - pad)
				}
			}
			nodes[i].r = (radius + pad)
		}
	}
}

func pack(nodes []*node, pre []int, post []int, w float64, h float64, p float64) {
	root := nodes[pre[0]]
	root.x = (w / 2.0)
	root.y = (h / 2.0)
	for _, i := range pre {
		n := nodes[i]
		if len(n.kids) == 0 {
			raw := sqrtf(n.value)
			n.r = (func() float64 {
				if raw == raw {
					return maxf(0.0, raw)
				}
				return 0.0
			})()
		}
	}
	seed := floats(1)
	seed[0] = 1.0
	packChildren(nodes, post, 0.0, seed)
	packChildren(nodes, post, (p * (root.r / minf(w, h))), seed)
	k := (minf(w, h) / (2.0 * root.r))
	for _, i := range pre {
		n := nodes[i]
		n.r = (n.r * k)
		if n.parent >= 0 {
			n.x = (nodes[n.parent].x + (k * n.x))
			n.y = (nodes[n.parent].y + (k * n.y))
		}
	}
}
