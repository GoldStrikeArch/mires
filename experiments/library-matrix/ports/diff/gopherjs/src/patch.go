package main

import (
	"fmt"
	"github.com/gopherjs/gopherjs/js"
	"regexp"
	"strconv"
	"strings"
)

type hunk struct {
	oldStart, oldLines, newStart, newLines int
	lines                                  []string
}
type patch struct {
	meta  *js.Object
	hunks []hunk
}

func (h *hunk) emit(kind int, lines []string) {
	for _, l := range lines {
		mark := " "
		if kind == add {
			mark = "+"
		} else if kind == remove {
			mark = "-"
		}
		if strings.HasSuffix(l, "\n") {
			h.lines = append(h.lines, mark+l[:len(l)-1])
		} else {
			h.lines = append(h.lines, mark+l, "\\ No newline at end of file")
		}
		if kind != remove {
			h.newLines++
		}
		if kind != add {
			h.oldLines++
		}
	}
}
func create(input *js.Object, ctx int) patch {
	meta := object()
	for _, k := range []string{"oldFileName", "newFileName", "oldHeader", "newHeader"} {
		meta.Set(k, input.Get(k))
	}
	p := patch{meta: meta}
	before := lines(readText(input.Get("before")), options{})
	after := lines(readText(input.Get("after")), options{})
	spans := myers(before, after, false)
	old, new, lastKeep := 0, 0, 0
	h := hunk{}
	active := false
	for i, r := range spans {
		if r.kind == keep {
			if active {
				close := i+1 == len(spans) || r.count > 2*ctx
				count := r.count
				if close {
					count = smaller(ctx, count)
				}
				h.emit(keep, after[new:new+count])
				if close {
					p.hunks = append(p.hunks, h)
					h = hunk{}
					active = false
				}
			}
			lastKeep = r.count
		} else {
			if !active {
				k := smaller(ctx, lastKeep)
				h.oldStart = old + 1 - k
				h.newStart = new + 1 - k
				h.emit(keep, after[new-k:new])
				active = true
			}
			if r.kind == add {
				h.emit(add, after[new:new+r.count])
			} else {
				h.emit(remove, before[old:old+r.count])
			}
		}
		if r.kind != add {
			old += r.count
		}
		if r.kind != remove {
			new += r.count
		}
	}
	if active {
		p.hunks = append(p.hunks, h)
	}
	return p
}
func quote(s string) string {
	s = readText(writeText(s).Call("toWellFormed"))
	need := false
	for i := 0; i < len(s); i++ {
		c := s[i]
		if c < 32 || c > 126 || c == '"' || c == '\\' {
			need = true
			break
		}
	}
	if !need {
		return s
	}
	var b strings.Builder
	b.WriteByte('"')
	escapes := "abtnvfr"
	for i := 0; i < len(s); i++ {
		c := s[i]
		switch {
		case c >= 7 && c <= 13:
			b.WriteByte('\\')
			b.WriteByte(escapes[c-7])
		case c == '"' || c == '\\':
			b.WriteByte('\\')
			b.WriteByte(c)
		case c < 32 || c > 126:
			fmt.Fprintf(&b, "\\%03o", c)
		default:
			b.WriteByte(c)
		}
	}
	b.WriteByte('"')
	return b.String()
}
func trimText(s string) string { return strings.TrimFunc(s, space) }

func quoted(s string) (string, int, bool) {
	if !strings.HasPrefix(s, "\"") {
		return "", 0, false
	}
	var b strings.Builder
	for i := 1; i < len(s); i++ {
		c := s[i]
		if c == '"' {
			return b.String(), i + 1, true
		}
		if c != '\\' {
			b.WriteByte(c)
			continue
		}
		i++
		if i >= len(s) {
			break
		}
		c = s[i]
		if c >= '0' && c <= '7' {
			bytes := js.Global.Get("Array").New()
			for {
				if i+3 > len(s) {
					return "", 0, false
				}
				v, e := strconv.ParseUint(s[i:i+3], 8, 16)
				if e != nil {
					return "", 0, false
				}
				bytes.Call("push", int(v)&255)
				i += 2
				if i+2 >= len(s) || s[i+1] != '\\' || s[i+2] < '0' || s[i+2] > '7' {
					break
				}
				i += 2
			}
			decoded := js.Global.Get("TextDecoder").New().Call("decode", js.Global.Get("Uint8Array").New(bytes))
			b.WriteString(decoded.String())
			continue
		}
		if j := strings.IndexByte("abtnvfr", c); j >= 0 {
			b.WriteByte(byte(7 + j))
		} else if c == '"' || c == '\\' {
			b.WriteByte(c)
		} else {
			break
		}
	}
	return "", 0, false
}
func unquote(s string) string {
	if n, _, ok := quoted(s); ok {
		return n
	}
	return s
}
func format(p patch) string {
	old, new := p.meta.Get("oldFileName"), p.meta.Get("newFileName")
	var b strings.Builder
	if old != js.Undefined && new != js.Undefined && readText(old) == readText(new) {
		fmt.Fprintf(&b, "Index: %s\n", readText(old))
	}
	b.WriteString("===================================================================\n")
	if old != js.Undefined && new != js.Undefined {
		for _, x := range [][]string{{"--- ", "oldFileName", "oldHeader"}, {"+++ ", "newFileName", "newHeader"}} {
			b.WriteString(x[0] + quote(readText(p.meta.Get(x[1]))))
			if h := p.meta.Get(x[2]); h.Bool() {
				b.WriteByte('\t')
				b.WriteString(readText(h))
			}
			b.WriteByte('\n')
		}
	}
	for _, h := range p.hunks {
		o, n := h.oldStart, h.newStart
		if h.oldLines == 0 {
			o--
		}
		if h.newLines == 0 {
			n--
		}
		fmt.Fprintf(&b, "@@ -%d,%d +%d,%d @@\n", o, h.oldLines, n, h.newLines)
		for _, l := range h.lines {
			b.WriteString(l)
			b.WriteByte('\n')
		}
	}
	return b.String()
}
func (p patch) write(reverse bool) *js.Object {
	all := arr(len(p.hunks))
	for i, h := range p.hunks {
		v := object()
		if reverse {
			v.Set("oldLines", h.oldLines)
			v.Set("oldStart", h.oldStart)
			v.Set("newLines", h.newLines)
			v.Set("newStart", h.newStart)
		} else {
			v.Set("oldStart", h.oldStart)
			v.Set("oldLines", h.oldLines)
			v.Set("newStart", h.newStart)
			v.Set("newLines", h.newLines)
		}
		a := arr(len(h.lines))
		for j, l := range h.lines {
			a.SetIndex(j, writeText(l))
		}
		v.Set("lines", a)
		all.SetIndex(i, v)
	}
	p.meta.Set("hunks", all)
	return p.meta
}
func readPatch(v *js.Object) patch {
	p := patch{meta: object()}
	ks := js.Keys(v)
	for _, k := range ks {
		p.meta.Set(k, v.Get(k))
	}
	all := v.Get("hunks")
	for i := 0; i < all.Length(); i++ {
		x := all.Index(i)
		h := hunk{oldStart: x.Get("oldStart").Int(), oldLines: x.Get("oldLines").Int(), newStart: x.Get("newStart").Int(), newLines: x.Get("newLines").Int()}
		a := x.Get("lines")
		for j := 0; j < a.Length(); j++ {
			h.lines = append(h.lines, readText(a.Index(j)))
		}
		p.hunks = append(p.hunks, h)
	}
	return p
}
func gitNames(p *js.Object, s string) {
	if old, n, ok := quoted(s); ok {
		if rest := trimText(s[n:]); rest != "" {
			p.Set("oldFileName", writeText(old))
			p.Set("newFileName", writeText(unquote(rest)))
		}
		return
	}
	if q := strings.IndexByte(s, '"'); q > 0 {
		if name, _, ok := quoted(s[q:]); ok {
			p.Set("oldFileName", writeText(trimText(s[:q])))
			p.Set("newFileName", writeText(name))
		}
		return
	}
	if strings.HasPrefix(s, "a/") {
		var at []int
		for i := 0; i < len(s); {
			n := strings.Index(s[i:], " b/")
			if n < 0 {
				break
			}
			i += n
			at = append(at, i)
			i++
		}
		if len(at) > 0 {
			i := at[len(at)/2]
			p.Set("oldFileName", writeText(s[:i]))
			p.Set("newFileName", writeText(s[i+1:]))
		}
	}
}
func extended(p *js.Object, line string) {
	for _, t := range [][]string{{"rename from ", "oldFileName", "isRename", "a/"}, {"rename to ", "newFileName", "isRename", "b/"}, {"copy from ", "oldFileName", "isCopy", "a/"}, {"copy to ", "newFileName", "isCopy", "b/"}} {
		if strings.HasPrefix(line, t[0]) {
			p.Set(t[1], writeText(t[3]+unquote(line[len(t[0]):])))
			p.Set(t[2], true)
			return
		}
	}
	for _, t := range [][]string{{"new file mode ", "isCreate", "newMode"}, {"deleted file mode ", "isDelete", "oldMode"}, {"new mode ", "", "newMode"}, {"old mode ", "", "oldMode"}} {
		if strings.HasPrefix(line, t[0]) {
			if t[1] != "" {
				p.Set(t[1], true)
			}
			p.Set(t[2], writeText(line[len(t[0]):]))
			return
		}
	}
	if strings.HasPrefix(line, "Binary files ") {
		p.Set("isBinary", true)
	}
}
func fileHeader(l string) bool {
	return (strings.HasPrefix(l, "---") || strings.HasPrefix(l, "+++")) && len(l) > 3 && (l[3] == ' ' || l[3] == '\t')
}
func header(l string) bool {
	return strings.HasPrefix(l, "diff --git ") || strings.HasPrefix(l, "Index: ") || strings.HasPrefix(l, "diff -r ")
}

var rangeHeader = regexp.MustCompile("@@ -([0-9]+)(?:,([0-9]+))? \\+([0-9]+)(?:,([0-9]+))? @@")

func count(s string, d int) int {
	if s == "" {
		return d
	}
	n, e := strconv.Atoi(s)
	if e != nil {
		panic("invalid count")
	}
	return n
}
func parseHunk(body []string, i int) (hunk, int) {
	m := rangeHeader.FindStringSubmatch(body[i])
	if m == nil {
		panic("invalid hunk header")
	}
	i++
	h := hunk{oldStart: count(m[1], 0), oldLines: count(m[2], 1), newStart: count(m[3], 0), newLines: count(m[4], 1)}
	if h.oldLines == 0 {
		h.oldStart++
	}
	if h.newLines == 0 {
		h.newStart++
	}
	old, new := 0, 0
	for i < len(body) && (old < h.oldLines || new < h.newLines || strings.HasPrefix(body[i], "\\")) {
		l := body[i]
		if l == "" && i+1 == len(body) {
			panic("invalid line")
		}
		mark := byte(' ')
		if l != "" {
			mark = l[0]
		}
		switch mark {
		case ' ':
			old++
			new++
		case '+':
			new++
		case '-':
			old++
		case '\\':
		default:
			panic("invalid line")
		}
		h.lines = append(h.lines, l)
		i++
	}
	if old == 0 && h.oldLines == 1 {
		h.oldLines = 0
	}
	if new == 0 && h.newLines == 1 {
		h.newLines = 0
	}
	if old != h.oldLines || new != h.newLines {
		panic("invalid line count")
	}
	if i < len(body) && body[i] != "" && strings.ContainsRune("+ -", rune(body[i][0])) && !fileHeader(body[i]) {
		panic("extra line")
	}
	return h, i
}
func parse(s string) []patch {
	body := strings.Split(s, "\n")
	var out []patch
	for i := 0; i < len(body); {
		p := patch{meta: object()}
		p.meta.Set("hunks", js.Undefined)
		seen := false
		for i < len(body) && !fileHeader(body[i]) && !strings.HasPrefix(body[i], "@@ ") {
			l := body[i]
			if header(l) {
				if seen {
					break
				}
				seen = true
				if strings.HasPrefix(l, "diff --git ") {
					p.meta.Set("isGit", true)
					gitNames(p.meta, l[11:])
				} else if strings.HasPrefix(l, "Index: ") {
					p.meta.Set("index", writeText(trimText(l[7:])))
				} else {
					p.meta.Set("index", writeText(l[strings.LastIndex(l, " ")+1:]))
				}
			} else if p.meta.Get("isGit").Bool() {
				extended(p.meta, l)
			}
			i++
		}
		for n := 0; n < 2 && i < len(body) && fileHeader(body[i]); n++ {
			l := body[i]
			i++
			cols := strings.SplitN(trimText(l[3:]), "\t", 3)
			key := "new"
			if l[0] == '-' {
				key = "old"
			}
			name := cols[0]
			if strings.HasPrefix(name, "\"") {
				name = unquote(name)
			} else {
				name = strings.ReplaceAll(name, "\\\\", "\\")
			}
			p.meta.Set(key+"FileName", writeText(name))
			h := ""
			if len(cols) > 1 {
				h = trimText(cols[1])
			}
			p.meta.Set(key+"Header", writeText(h))
		}
		if (p.meta.Get("oldFileName") == js.Undefined) != (p.meta.Get("newFileName") == js.Undefined) {
			panic("missing file header")
		}
		for i < len(body) {
			l := body[i]
			if header(l) || fileHeader(l) || strings.HasPrefix(l, "===================================================================") {
				break
			}
			if strings.HasPrefix(l, "@@ ") {
				var h hunk
				h, i = parseHunk(body, i)
				p.hunks = append(p.hunks, h)
			} else {
				i++
			}
		}
		out = append(out, p)
	}
	return out
}
func reverse(v *js.Object) *js.Object {
	p := readPatch(v)
	for _, pair := range [][]string{{"oldFileName", "newFileName"}, {"oldHeader", "newHeader"}, {"newFileName", "oldFileName"}, {"newHeader", "oldHeader"}, {"oldMode", "newMode"}, {"newMode", "oldMode"}, {"isCreate", "isDelete"}, {"isDelete", "isCreate"}} {
		value := v.Get(pair[1])
		if strings.HasSuffix(pair[0], "FileName") && value != js.Undefined && v.Get("isGit").Bool() {
			s := readText(value)
			if strings.HasPrefix(s, "a/") {
				p.meta.Set(pair[0], writeText("b/"+s[2:]))
				continue
			}
			if strings.HasPrefix(s, "b/") {
				p.meta.Set(pair[0], writeText("a/"+s[2:]))
				continue
			}
		}
		p.meta.Set(pair[0], value)
	}
	if v.Get("isCopy").Bool() {
		p.meta.Set("newFileName", "/dev/null")
		p.meta.Set("newHeader", js.Undefined)
		p.meta.Set("isDelete", true)
		for _, name := range []string{"isCreate", "isCopy", "isRename"} {
			p.meta.Delete(name)
		}
		p.hunks = nil
	}
	for i := range p.hunks {
		h := &p.hunks[i]
		h.oldStart, h.newStart = h.newStart, h.oldStart
		h.oldLines, h.newLines = h.newLines, h.oldLines
		for j, l := range h.lines {
			if strings.HasPrefix(l, "+") {
				h.lines[j] = "-" + l[1:]
			} else if strings.HasPrefix(l, "-") {
				h.lines[j] = "+" + l[1:]
			}
		}
	}
	return p.write(true)
}
func convert(source string, p patch) {
	win := strings.Contains(source, "\r\n")
	unix := strings.Contains(strings.ReplaceAll(source, "\r\n", ""), "\n")
	if win == unix {
		return
	}
	pw, pu := false, false
	for _, h := range p.hunks {
		for i, l := range h.lines {
			if strings.HasPrefix(l, "\\") {
				continue
			}
			if strings.HasSuffix(l, "\r") {
				pw = true
			} else if i+1 == len(h.lines) || !strings.HasPrefix(h.lines[i+1], "\\") {
				pu = true
			}
		}
	}
	for _, h := range p.hunks {
		for i, l := range h.lines {
			if win && !pw && !strings.HasPrefix(l, "\\") && (i+1 == len(h.lines) || !strings.HasPrefix(h.lines[i+1], "\\")) {
				h.lines[i] += "\r"
			} else if unix && pw && !pu {
				h.lines[i] = strings.TrimSuffix(l, "\r")
			}
		}
	}
}
func fits(h hunk, lines []string, i int) bool {
	for _, l := range h.lines {
		if strings.HasPrefix(l, "+") || strings.HasPrefix(l, "\\") {
			continue
		}
		content := ""
		if l != "" {
			content = l[1:]
		}
		if i >= len(lines) || lines[i] != content {
			return false
		}
		i++
	}
	return true
}
func apply(source string, p patch) interface{} {
	if len(p.hunks) == 0 {
		return writeText(source)
	}
	convert(source, p)
	v := strings.Split(source, "\n")
	ae, re := false, false
	var prev byte
	for _, l := range p.hunks[len(p.hunks)-1].lines {
		if strings.HasPrefix(l, "\\") {
			if prev == '-' {
				ae = true
			}
			if prev == '+' {
				re = true
			}
		}
		prev = ' '
		if l != "" {
			prev = l[0]
		}
	}
	eof := v[len(v)-1] == ""
	switch {
	case ae && re:
		if eof {
			return false
		}
	case re:
		if !eof {
			return false
		}
		v = v[:len(v)-1]
	case ae:
		if eof {
			return false
		}
		v = append(v, "")
	}
	min, offset := 0, 0
	var result []string
	for _, h := range p.hunks {
		max := len(v) - h.oldLines
		if max < min {
			return false
		}
		wanted := h.oldStart + offset - 1
		start := larger(min, smaller(max, wanted))
		pos := -1
		for d := 0; d <= larger(start-min, max-start); d++ {
			hi, lo := start+d, start-d
			if hi <= max && fits(h, v, hi) {
				pos = hi
				break
			}
			if lo >= min && lo < hi && fits(h, v, lo) {
				pos = lo
				break
			}
		}
		if pos < 0 {
			return false
		}
		result = append(result, v[min:pos]...)
		cur, ctx := pos, 0
		for _, l := range h.lines {
			op := byte(' ')
			if l != "" {
				op = l[0]
			}
			switch op {
			case ' ':
				result = append(result, v[cur])
				cur++
				ctx++
			case '-':
				cur++
				ctx = 0
			case '+':
				result = append(result, l[1:])
				ctx = 0
			}
		}
		result = result[:len(result)-ctx]
		min = cur - ctx
		offset = pos + 1 - h.oldStart
	}
	return writeText(strings.Join(append(result, v[min:]...), "\n"))
}
