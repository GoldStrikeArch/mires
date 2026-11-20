package main

import "math"

func nextHE(e int) int { return e - e%3 + (e+1)%3 }
func prevHE(e int) int { return e - e%3 + (e+2)%3 }

func refine(t []int, coords []float64, dim int) {
	n := len(t)
	if n < 6 {
		return
	}
	he := make([]int, n)
	for i := range he {
		he[i] = -1
	}
	size := 1
	for size < n*4 {
		size *= 2
	}
	// A zero slot is empty, -1 is consumed, and other slots hold half-edge + 1.
	table := make([]int, size)
	stack := make([]int, 0, n)
	pending := make([]bool, n)
	for e, a := range t {
		b := t[nextHE(e)]
		lo, hi := uint32(min(a, b)), uint32(max(a, b))
		h := int((lo*0x9e3779b1 ^ hi*0x85ebca6b) & uint32(size-1))
		for table[h] != 0 {
			s := table[h] - 1
			if s >= 0 {
				sa, sb := t[s], t[nextHE(s)]
				if (sa == a && sb == b) || (sa == b && sb == a) {
					he[e], he[s], table[h] = s, e, -1
					pending[s] = true
					stack = append(stack, s)
					break
				}
			}
			h = (h + 1) & (size - 1)
		}
		if table[h] == 0 {
			table[h] = e + 1
		}
	}
	for len(stack) > 0 {
		i := len(stack) - 1
		a := stack[i]
		stack = stack[:i]
		pending[a] = false
		b := he[a]
		if b < 0 {
			continue
		}
		ar, al := prevHE(a), nextHE(a)
		bl, br := prevHE(b), nextHE(b)
		p0, pr, pl, p1 := t[ar], t[a], t[al], t[bl]
		x0, y0 := coords[p0*dim], coords[p0*dim+1]
		xr, yr := coords[pr*dim], coords[pr*dim+1]
		xl, yl := coords[pl*dim], coords[pl*dim+1]
		x1, y1 := coords[p1*dim], coords[p1*dim+1]
		if !inCircle(x0, y0, xr, yr, xl, yl, x1, y1) && orient(x0, y0, xr, yr, x1, y1) > 0 && orient(x0, y0, x1, y1, xl, yl) > 0 {
			t[a], t[b] = p1, p0
			hbl, har := he[bl], he[ar]
			he[a], he[b] = hbl, har
			if hbl >= 0 {
				he[hbl] = a
			}
			if har >= 0 {
				he[har] = b
			}
			he[ar], he[bl] = bl, ar
			for _, e := range [4]int{a, b, al, br} {
				if he[e] >= 0 && !pending[e] {
					pending[e] = true
					stack = append(stack, e)
				}
			}
		}
	}
}

func orient(ax, ay, bx, by, cx, cy float64) float64 {
	return (bx-ax)*(cy-ay) - (by-ay)*(cx-ax)
}
func inCircle(ax, ay, bx, by, cx, cy, px, py float64) bool {
	dx, dy, ex, ey, fx, fy := ax-px, ay-py, bx-px, by-py, cx-px, cy-py
	ap, bp, cp := dx*dx+dy*dy, ex*ex+ey*ey, fx*fx+fy*fy
	s := ap + bp + cp
	return dx*(ey*cp-bp*fy)-dy*(ex*cp-bp*fx)+ap*(ex*fy-ey*fx) <= 1e-13*s*s
}

func deviation(data []float64, holes []int, dim int, t []int) float64 {
	end := len(data)
	if len(holes) != 0 {
		end = dim * holes[0]
	}
	polygonArea := math.Abs(signedArea(data, 0, end, dim))
	for i, h := range holes {
		end = len(data)
		if i+1 < len(holes) {
			end = holes[i+1] * dim
		}
		polygonArea -= math.Abs(signedArea(data, h*dim, end, dim))
	}
	trianglesArea := 0.0
	for i := 0; i < len(t); i += 3 {
		a, b, c := t[i]*dim, t[i+1]*dim, t[i+2]*dim
		trianglesArea += math.Abs((data[a]-data[c])*(data[b+1]-data[a+1]) - (data[a]-data[b])*(data[c+1]-data[a+1]))
	}
	if trianglesArea == 0 {
		maxCoord := 0.0
		for i := 0; i < len(data); i += dim {
			maxCoord = max(maxCoord, math.Abs(data[i]), math.Abs(data[i+1]))
		}
		if math.Abs(polygonArea) <= float64(len(data))*maxCoord*maxCoord*0x1p-52 {
			return 0
		}
		return 1
	}
	return math.Abs((trianglesArea - polygonArea) / polygonArea)
}
