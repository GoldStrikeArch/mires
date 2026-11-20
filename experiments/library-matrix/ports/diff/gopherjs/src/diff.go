package main

import (
	"strings"
	"unicode/utf8"
)

const (
	keep = iota
	remove
	add
)

type span struct{ kind, count, previous int }
type path struct{ old, last int }

func smaller(a, b int) int {
	if a < b {
		return a
	}
	return b
}
func larger(a, b int) int {
	if a > b {
		return a
	}
	return b
}

func myers(old, new []string, separate bool) []span {
	limit := len(old) + len(new)
	base := limit + 1
	best := make([]path, base*2+1)
	for i := range best {
		best[i].old = -2
	}
	arena := []span{{}}
	advance := func(p *path, kind, count int) {
		if count == 0 {
			return
		}
		if separate {
			for i := 0; i < count; i++ {
				arena = append(arena, span{kind, 1, p.last})
				p.last = len(arena) - 1
			}
			return
		}
		prev := p.last
		if prev != 0 && arena[prev].kind == kind {
			count += arena[prev].count
			prev = arena[prev].previous
		}
		arena = append(arena, span{kind, count, prev})
		p.last = len(arena) - 1
	}
	common := func(p *path, k int) int {
		from, at := p.old, p.old-k
		for p.old+1 < len(old) && at+1 < len(new) && old[p.old+1] == new[at+1] {
			p.old++
			at++
		}
		advance(p, keep, p.old-from)
		return at
	}
	finish := func(i int) []span {
		var r []span
		for i > 0 {
			r = append(r, arena[i])
			i = arena[i].previous
		}
		for i, j := 0, len(r)-1; i < j; i, j = i+1, j-1 {
			r[i], r[j] = r[j], r[i]
		}
		return r
	}
	p := path{-1, 0}
	n := common(&p, 0)
	if p.old+1 == len(old) && n+1 == len(new) {
		return finish(p.last)
	}
	best[base] = p
	low, high := -limit, limit
	for d := 1; d <= limit; d++ {
		first, last := larger(low, -d), smaller(high, d)
		for k := first; k <= last; k += 2 {
			i := base + k
			r, a := best[i-1], best[i+1]
			best[i-1].old = -2
			canR := r.old >= -1 && r.old+1 < len(old)
			canA := a.old >= -1 && a.old-k >= 0 && a.old-k < len(new)
			if !canA && !canR {
				best[i].old = -2
				continue
			}
			if !canR || (canA && r.old < a.old) {
				p = a
				advance(&p, add, 1)
			} else {
				p = r
				p.old++
				advance(&p, remove, 1)
			}
			n = common(&p, k)
			if p.old+1 == len(old) && n+1 == len(new) {
				return finish(p.last)
			}
			best[i] = p
			if p.old+1 == len(old) {
				high = smaller(high, k-1)
			}
			if n+1 == len(new) {
				low = larger(low, k+1)
			}
		}
	}
	return nil
}
func space(c rune) bool {
	return c >= 9 && c <= 13 || c == 32 || c == 160 || c == 0x1680 || c >= 0x2000 && c <= 0x200a || c == 0x2028 || c == 0x2029 || c == 0x202f || c == 0x205f || c == 0x3000 || c == 0xfeff
}
func word(c rune) bool {
	return c >= 'a' && c <= 'z' || c >= 'A' && c <= 'Z' || c >= '0' && c <= '9' || c == '_' || c == 0xad || c >= 0xc0 && c <= 0xd6 || c >= 0xd8 && c <= 0xf6 || c >= 0xf8 && c <= 0x2c6 || c >= 0x2c8 && c <= 0x2d7 || c >= 0x2de && c <= 0x2ff || c >= 0x1e00 && c <= 0x1eff
}
func words(s string) []string {
	var out, parts []string
	start, prev := 0, -1
	for i := 0; i < len(s); {
		c, width := point(s[i:])
		class := 2
		if word(c) {
			class = 1
		} else if space(c) {
			class = 0
		}
		if class != prev || class == 2 {
			if i > start {
				parts = append(parts, s[start:i])
			}
			start = i
		}
		i += width
		prev = class
	}
	if start < len(s) {
		parts = append(parts, s[start:])
	}
	prevWS := ""
	for _, part := range parts {
		ch, _ := utf8.DecodeRuneInString(part)
		n := len(out)
		if space(ch) {
			if n == 0 {
				out = append(out, part)
			} else {
				out[n-1] += part
			}
			prevWS = part
			continue
		}
		if prevWS != "" && n == 1 && out[0] == prevWS {
			out[0] += part
		} else {
			out = append(out, prevWS+part)
		}
		prevWS = ""
	}
	return out
}

type options struct{ ignoreCase, individual, stripCR, ignoreWS, newlineToken, ignoreEOF bool }

func lines(s string, o options) []string {
	var out []string
	start := 0
	for i := 0; i < len(s); i++ {
		if s[i] != '\n' {
			continue
		}
		end := i
		if end > start && s[end-1] == '\r' {
			end--
		}
		if o.newlineToken {
			if end > start {
				out = append(out, s[start:end])
			}
			if o.stripCR {
				out = append(out, "\n")
			} else {
				out = append(out, s[end:i+1])
			}
		} else if o.stripCR && end < i {
			out = append(out, s[start:end]+"\n")
		} else {
			out = append(out, s[start:i+1])
		}
		start = i + 1
	}
	if start < len(s) {
		out = append(out, s[start:])
	}
	return out
}
func tokens(op, s string, o options) []string {
	switch op {
	case "diffWords":
		return words(s)
	case "diffLines", "diffJson":
		return lines(s, o)
	}
	var out []string
	for len(s) > 0 {
		_, width := point(s)
		out = append(out, s[:width])
		s = s[width:]
	}
	return out
}
func compareKeys(op string, v []string, o options) []string {
	out := make([]string, len(v))
	for i, s := range v {
		if op == "diffWords" || op == "diffLines" && o.ignoreWS && (!o.newlineToken || !strings.Contains(s, "\n")) {
			s = strings.TrimFunc(s, space)
		} else if op == "diffLines" && o.ignoreEOF && !o.newlineToken {
			s = strings.TrimSuffix(s, "\n")
		}
		if op == "diffJson" {
			s = strings.ReplaceAll(strings.ReplaceAll(s, ",\r", "\r"), ",\n", "\n")
		}
		if o.ignoreCase {
			s = lowercase(s)
		}
		out[i] = s
	}
	return out
}

type change struct {
	kind, count int
	value       string
}

func changes(op string, old, new []string, o options) []change {
	spans := myers(compareKeys(op, old, o), compareKeys(op, new, o), o.individual)
	var out []change
	a, b := 0, 0
	for _, r := range spans {
		var v strings.Builder
		for j := 0; j < r.count; j++ {
			var s string
			if r.kind == remove || op == "diffJson" && r.kind == keep && len(old[a+j]) > len(new[b+j]) {
				s = old[a+j]
			} else {
				s = new[b+j]
			}
			if op == "diffWords" && j > 0 {
				s = strings.TrimLeftFunc(s, space)
			}
			v.WriteString(s)
		}
		if r.kind != remove {
			b += r.count
		}
		if r.kind != add {
			a += r.count
		}
		out = append(out, change{r.kind, r.count, v.String()})
	}
	if op == "diffWords" && !o.individual {
		cleanWords(out)
	}
	return out
}
func leftWS(s string) string  { return s[:len(s)-len(strings.TrimLeftFunc(s, space))] }
func rightWS(s string) string { return s[len(strings.TrimRightFunc(s, space)):] }
func prefix(a, b string) string {
	n := 0
	for i, c := range a {
		if i >= len(b) {
			break
		}
		d, k := utf8.DecodeRuneInString(b[i:])
		if c != d {
			break
		}
		n = i + k
	}
	return a[:n]
}
func suffix(a, b string) string {
	n := 0
	for n < len(a) && n < len(b) {
		x, k := utf8.DecodeLastRuneInString(a[:len(a)-n])
		y, _ := utf8.DecodeLastRuneInString(b[:len(b)-n])
		if x != y {
			break
		}
		n += k
	}
	return a[len(a)-n:]
}
func overlap(a, b string) string {
	for i := range a {
		if strings.HasPrefix(b, a[i:]) {
			return a[i:]
		}
	}
	return ""
}
func cleanWords(ch []change) {
	var start, del, ins *change
	adjust := func(end *change) {
		switch {
		case del != nil && ins != nil:
			op, np := leftWS(del.value), leftWS(ins.value)
			os, ns := rightWS(del.value), rightWS(ins.value)
			if start != nil {
				p := prefix(op, np)
				start.value = strings.TrimSuffix(start.value, np) + p
				del.value = strings.TrimPrefix(del.value, p)
				ins.value = strings.TrimPrefix(ins.value, p)
			}
			if end != nil {
				s := suffix(os, ns)
				end.value = s + strings.TrimPrefix(end.value, ns)
				del.value = strings.TrimSuffix(del.value, s)
				ins.value = strings.TrimSuffix(ins.value, s)
			}
		case ins != nil:
			if start != nil {
				ins.value = strings.TrimLeftFunc(ins.value, space)
			}
			if end != nil {
				end.value = strings.TrimLeftFunc(end.value, space)
			}
		case del != nil:
			if start != nil && end != nil {
				w := leftWS(end.value)
				p := prefix(w, leftWS(del.value))
				s := suffix(w[len(p):], rightWS(del.value))
				del.value = strings.TrimSuffix(strings.TrimPrefix(del.value, p), s)
				end.value = s + strings.TrimPrefix(end.value, w)
				start.value = strings.TrimSuffix(start.value, w) + w[:len(w)-len(s)]
			} else if start != nil {
				del.value = strings.TrimPrefix(del.value, overlap(rightWS(start.value), leftWS(del.value)))
			} else if end != nil {
				del.value = strings.TrimSuffix(del.value, overlap(rightWS(del.value), leftWS(end.value)))
			}
		}
	}
	for i := range ch {
		switch ch[i].kind {
		case add:
			ins = &ch[i]
		case remove:
			del = &ch[i]
		default:
			adjust(&ch[i])
			start = &ch[i]
			del = nil
			ins = nil
		}
	}
	adjust(nil)
}
