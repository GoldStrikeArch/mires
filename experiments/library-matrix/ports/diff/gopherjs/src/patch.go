package main

import (
	"fmt"
	"math"
	"strconv"
	"strings"

	"github.com/gopherjs/gopherjs/js"
)

// Hunk ranges are JavaScript numbers: a parsed header can exceed int32 or be
// NaN, exactly as `+match[1]` gives it in parse.js.
type hunk struct {
	oldStart, oldLines, newStart, newLines float64
	lines                                  []string
}

// meta is the host object being built, so its keys keep the order in which
// jsdiff first assigns them.
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
func create(input *js.Object, context float64) patch {
	meta := object()
	for _, k := range []string{"oldFileName", "newFileName", "oldHeader", "newHeader"} {
		meta.Set(k, input.Get(k))
	}
	p := patch{meta: meta}
	before := lines(readText(input.Get("before")), options{})
	after := lines(readText(input.Get("after")), options{})
	// context is a JavaScript number, used only through min, slice and <=; any value from the
	// line count up acts as the line count does, so it is clamped there.
	ctx := len(before) + len(after)
	if context < float64(ctx) {
		ctx = int(context)
	}
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
					count = min(ctx, count)
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
				k := min(ctx, lastKeep)
				h.oldStart = float64(old + 1 - k)
				h.newStart = float64(new + 1 - k)
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
		// The start printed for an empty range is one lower.
		o, n := int(h.oldStart), int(h.newStart)
		if h.oldLines == 0 {
			o--
		}
		if h.newLines == 0 {
			n--
		}
		fmt.Fprintf(&b, "@@ -%d,%d +%d,%d @@\n", o, int(h.oldLines), n, int(h.newLines))
		for _, l := range h.lines {
			b.WriteString(l)
			b.WriteByte('\n')
		}
	}
	return b.String()
}
func (p patch) write() *js.Object {
	all := arr(len(p.hunks))
	for i, h := range p.hunks {
		v := object()
		v.Set("oldStart", h.oldStart)
		v.Set("oldLines", h.oldLines)
		v.Set("newStart", h.newStart)
		v.Set("newLines", h.newLines)
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

// Line classes of parse.js: anchored regexes, where \s is the ECMAScript
// whitespace set (space) and \w is ASCII.
const underline = "==================================================================="

func spaceAt(s string, i int) bool {
	if i >= len(s) {
		return false
	}
	c, _ := point(s[i:])
	return space(c)
}
func spaceAfter(line, prefix string) bool {
	return strings.HasPrefix(line, prefix) && spaceAt(line, len(prefix))
}

// /^diff --git /
func isGitHeader(line string) bool { return strings.HasPrefix(line, "diff --git ") }

// /^(---|\+\+\+)\s/
func isFileHeader(line string) bool { return spaceAfter(line, "---") || spaceAfter(line, "+++") }

// /^@@\s/
func isHunkHeader(line string) bool { return spaceAfter(line, "@@") }

func wordAt(s string, i int) bool {
	if i >= len(s) {
		return false
	}
	c := s[i]
	return c >= 'a' && c <= 'z' || c >= 'A' && c <= 'Z' || c >= '0' && c <= '9' || c == '_'
}

// hgPrefix is the end of `diff(?: -r \w+)+` in a match of /^diff(?: -r \w+)+\s/.
// The repetition is greedy; a last group not followed by whitespace is given
// back, and the space that starts it matches \s.
func hgPrefix(line string) (int, bool) {
	if !strings.HasPrefix(line, "diff") {
		return 0, false
	}
	var ends []int
	for at := 4; strings.HasPrefix(line[at:], " -r ") && wordAt(line, at+4); {
		for at += 4; wordAt(line, at); at++ {
		}
		ends = append(ends, at)
	}
	n := len(ends)
	switch {
	case n > 0 && spaceAt(line, ends[n-1]):
		return ends[n-1], true
	case n >= 2:
		return ends[n-2], true
	}
	return 0, false
}

// indexHeader is the length of the match of /^(?:Index:|diff(?: -r \w+)+)\s+/.
func indexHeader(line string) (int, bool) {
	start, ok := 6, strings.HasPrefix(line, "Index:")
	if !ok {
		start, ok = hgPrefix(line)
	}
	if !ok || !spaceAt(line, start) {
		return 0, false
	}
	return len(line) - len(strings.TrimLeftFunc(line[start:], space)), true
}

// isDiffHeader: a Git, Index: or Mercurial header.
func isDiffHeader(line string) bool {
	_, index := indexHeader(line)
	return isGitHeader(line) || index
}

// restOfLine is the capture of `(.*)`: `.` stops at \r, U+2028 and U+2029.
func restOfLine(s string) string {
	if end := strings.IndexAny(s, "\r  "); end >= 0 {
		return s[:end]
	}
	return s
}

// mode is the capture of `(\d+)`, ASCII digits only.
func mode(s string) (string, bool) {
	n := 0
	for n < len(s) && s[n] >= '0' && s[n] <= '9' {
		n++
	}
	return s[:n], n > 0
}

// quoted is parseQuotedFileName: the unescaped name and the length of the
// quoted text, or false for an unterminated quote or an unknown escape. A run
// of octal escapes holds raw bytes (Uint8Array keeps the low byte), decoded
// together by the host TextDecoder.
func quoted(s string) (string, int, bool) {
	if !strings.HasPrefix(s, "\"") {
		return "", 0, false
	}
	octal := func(i int) bool { return i < len(s) && s[i] >= '0' && s[i] <= '7' }
	value := func(i int) int { return int(s[i]-'0')<<6 | int(s[i+1]-'0')<<3 | int(s[i+2]-'0') }
	var b strings.Builder
	for j := 1; j < len(s); {
		if s[j] == '"' {
			return b.String(), j + 1, true
		}
		if s[j] != '\\' || j+1 == len(s) {
			b.WriteByte(s[j])
			j++
			continue
		}
		j++
		var c byte
		switch s[j] {
		case 'a':
			c = '\a'
		case 'b':
			c = '\b'
		case 'f':
			c = '\f'
		case 'n':
			c = '\n'
		case 'r':
			c = '\r'
		case 't':
			c = '\t'
		case 'v':
			c = '\v'
		case '\\', '"':
			c = s[j]
		case '0', '1', '2', '3', '4', '5', '6', '7':
			if !octal(j+1) || !octal(j+2) {
				return "", 0, false
			}
			bytes := js.Global.Get("Array").New()
			bytes.Call("push", value(j)&255)
			for j += 3; j < len(s) && s[j] == '\\' && octal(j+1); j += 4 {
				if !octal(j+2) || !octal(j+3) {
					return "", 0, false
				}
				bytes.Call("push", value(j+1)&255)
			}
			b.WriteString(readText(js.Global.Get("TextDecoder").New().Call("decode", js.Global.Get("Uint8Array").New(bytes))))
			continue
		default:
			return "", 0, false
		}
		b.WriteByte(c)
		j++
	}
	return "", 0, false
}

// unquote is unquoteIfQuoted.
func unquote(s string) string {
	if n, _, ok := quoted(s); ok {
		return n
	}
	return s
}

// gitNames is parseGitDiffHeader: the names after "diff --git ", when they
// can be split. Both quoted forms skip one UTF-16 unit, assumed to be a space.
func gitNames(rest string) (string, string, bool) {
	if strings.HasPrefix(rest, "\"") {
		old, n, ok := quoted(rest)
		if !ok {
			return "", "", false
		}
		after := dropFirstUnit(rest[n:])
		if !strings.HasPrefix(after, "\"") {
			return old, after, true
		}
		new, _, ok := quoted(after)
		return old, new, ok
	}
	if q := strings.IndexByte(rest, '"'); q > 0 {
		new, _, ok := quoted(rest[q:])
		return dropLastUnit(rest[:q]), new, ok
	}
	if strings.HasPrefix(rest, "a/") {
		// Split on the middle " b/", which is right when both names agree.
		var splits []int
		for from := 1; ; {
			i := strings.Index(rest[from:], " b/")
			if i < 0 {
				break
			}
			splits = append(splits, from+i)
			from += i + 1
		}
		if len(splits) > 0 {
			mid := splits[len(splits)/2]
			return rest[:mid], rest[mid+1:], true
		}
	}
	return "", "", false
}

// extendedHeader reads one Git extended header line; at most one of
// parse.js's regexes matches it.
func extendedHeader(p *js.Object, line string) {
	name := func(key, prefix, tag string) {
		p.Set(key, writeText(prefix+unquote(restOfLine(line[len(tag):]))))
	}
	setMode := func(key, tag, flag string) {
		if m, ok := mode(line[len(tag):]); ok {
			if flag != "" {
				p.Set(flag, true)
			}
			p.Set(key, writeText(m))
		}
	}
	switch {
	case strings.HasPrefix(line, "rename from "):
		name("oldFileName", "a/", "rename from ")
		p.Set("isRename", true)
	case strings.HasPrefix(line, "rename to "):
		name("newFileName", "b/", "rename to ")
		p.Set("isRename", true)
	case strings.HasPrefix(line, "copy from "):
		name("oldFileName", "a/", "copy from ")
		p.Set("isCopy", true)
	case strings.HasPrefix(line, "copy to "):
		name("newFileName", "b/", "copy to ")
		p.Set("isCopy", true)
	case strings.HasPrefix(line, "new file mode "):
		setMode("newMode", "new file mode ", "isCreate")
	case strings.HasPrefix(line, "deleted file mode "):
		setMode("oldMode", "deleted file mode ", "isDelete")
	case strings.HasPrefix(line, "old mode "):
		setMode("oldMode", "old mode ", "")
	case strings.HasPrefix(line, "new mode "):
		setMode("newMode", "new mode ", "")
	case strings.HasPrefix(line, "Binary files "):
		p.Set("isBinary", true)
	}
}

// parseFileHeader reads a --- or +++ line, if there is one.
func parseFileHeader(body []string, i int, p *js.Object) int {
	if i >= len(body) || !isFileHeader(body[i]) {
		return i
	}
	line := body[i]
	data := strings.SplitN(trimText(line[3:]), "\t", 3)
	name, header := data[0], ""
	if len(data) > 1 {
		header = trimText(data[1])
	}
	if strings.HasPrefix(name, "\"") {
		name = unquote(name)
	} else {
		name = strings.ReplaceAll(name, "\\\\", "\\")
	}
	side := "new"
	if line[0] == '-' {
		side = "old"
	}
	p.Set(side+"FileName", writeText(name))
	p.Set(side+"Header", writeText(header))
	return i + 1
}

// digits reads a run of ASCII digits as a JavaScript number; a huge run
// rounds to infinity, as `+s` does.
func digits(s string, i int) (float64, int, bool) {
	j := i
	for j < len(s) && s[j] >= '0' && s[j] <= '9' {
		j++
	}
	if j == i {
		return 0, i, false
	}
	n, _ := strconv.ParseFloat(s[i:j], 64)
	return n, j, true
}

type hunkRange struct {
	oldStart, newStart float64
	oldLines, newLines *float64
}

// rangeAt matches /@@ -(\d+)(?:,(\d+))? \+(\d+)(?:,(\d+))? @@/ from i, just
// after its leading "@@ -".
func rangeAt(line string, i int) (hunkRange, bool) {
	var r hunkRange
	count := func(i int) (*float64, int) {
		if i < len(line) && line[i] == ',' {
			if n, j, ok := digits(line, i+1); ok {
				return &n, j
			}
		}
		return nil, i
	}
	var ok bool
	if r.oldStart, i, ok = digits(line, i); !ok {
		return r, false
	}
	r.oldLines, i = count(i)
	if !strings.HasPrefix(line[i:], " +") {
		return r, false
	}
	if r.newStart, i, ok = digits(line, i+2); !ok {
		return r, false
	}
	r.newLines, i = count(i)
	return r, strings.HasPrefix(line[i:], " @@")
}

// rangeIn finds the leftmost match, which String.prototype.split finds
// anywhere in the line.
func rangeIn(line string) (hunkRange, bool) {
	for from := 0; ; {
		at := strings.Index(line[from:], "@@ -")
		if at < 0 {
			return hunkRange{}, false
		}
		if r, ok := rangeAt(line, from+at+4); ok {
			return r, true
		}
		from += at + 1
	}
}

// parseHunk: an unmatched header gives NaN starts and one line per side.
func parseHunk(body []string, i int) (hunk, int) {
	r, ok := rangeIn(body[i])
	i++
	h := hunk{oldStart: math.NaN(), oldLines: 1, newStart: math.NaN(), newLines: 1}
	if ok {
		h.oldStart, h.newStart = r.oldStart, r.newStart
		if r.oldLines != nil {
			h.oldLines = *r.oldLines
		}
		if r.newLines != nil {
			h.newLines = *r.newLines
		}
	}
	if h.oldLines == 0 {
		h.oldStart++
	}
	if h.newLines == 0 {
		h.newStart++
	}
	adds, removes := 0, 0
	for ; i < len(body) && (float64(removes) < h.oldLines || float64(adds) < h.newLines || strings.HasPrefix(body[i], "\\")); i++ {
		l := body[i]
		// An empty line is context, except the last one: ''[0] is undefined.
		var operation byte
		if l != "" {
			operation = l[0]
		} else if i != len(body)-1 {
			operation = ' '
		}
		switch operation {
		case '+':
			adds++
		case '-':
			removes++
		case ' ':
			adds++
			removes++
		case '\\':
		default:
			panic("hunk contained an invalid line")
		}
		h.lines = append(h.lines, l)
	}
	if adds == 0 && h.newLines == 1 {
		h.newLines = 0
	}
	if removes == 0 && h.oldLines == 1 {
		h.oldLines = 0
	}
	if float64(adds) != h.newLines {
		panic("added line count did not match")
	}
	if float64(removes) != h.oldLines {
		panic("removed line count did not match")
	}
	if i < len(body) && body[i] != "" && strings.IndexByte("+ -", body[i][0]) >= 0 && !isFileHeader(body[i]) {
		panic("hunk has more lines than expected")
	}
	return h, i
}

// parseIndex reads one file's metadata, file headers and hunks.
func parseIndex(body []string, i int) (patch, int) {
	p := patch{meta: object()}
	p.meta.Set("hunks", js.Undefined)
	seen := false
	for i < len(body) {
		line := body[i]
		if isFileHeader(line) || isHunkHeader(line) {
			break
		}
		if isGitHeader(line) {
			// A second diff header belongs to the next file; this one ends here.
			if seen {
				return p, i
			}
			seen = true
			p.meta.Set("isGit", true)
			if old, new, ok := gitNames(line[len("diff --git "):]); ok {
				p.meta.Set("oldFileName", writeText(old))
				p.meta.Set("newFileName", writeText(new))
			}
			for i++; i < len(body) && !isFileHeader(body[i]) && !isHunkHeader(body[i]) && !isDiffHeader(body[i]); i++ {
				extendedHeader(p.meta, body[i])
			}
			continue
		}
		if n, ok := indexHeader(line); ok {
			if seen {
				return p, i
			}
			seen = true
			p.meta.Set("index", writeText(trimText(line[n:])))
		}
		i++
	}
	i = parseFileHeader(body, i, p.meta)
	i = parseFileHeader(body, i, p.meta)
	if (p.meta.Get("oldFileName") == js.Undefined) != (p.meta.Get("newFileName") == js.Undefined) {
		panic("missing file header")
	}
	for i < len(body) {
		line := body[i]
		if isDiffHeader(line) || isFileHeader(line) || strings.HasPrefix(line, underline) {
			break
		}
		if isHunkHeader(line) {
			var h hunk
			h, i = parseHunk(body, i)
			p.hunks = append(p.hunks, h)
		} else {
			i++
		}
	}
	return p, i
}
func parse(s string) []patch {
	body := strings.Split(s, "\n")
	var out []patch
	for i := 0; i < len(body); {
		var p patch
		p, i = parseIndex(body, i)
		out = append(out, p)
	}
	return out
}

// reversePatch works on the host values, as Object.assign copies them.
func swapPrefix(v *js.Object) *js.Object {
	if v == js.Undefined {
		return v
	}
	name := readText(v)
	switch {
	case strings.HasPrefix(name, "a/"):
		return writeText("b/" + name[2:])
	case strings.HasPrefix(name, "b/"):
		return writeText("a/" + name[2:])
	}
	return v
}
func reverseHunk(h *js.Object) *js.Object {
	x := object()
	x.Set("oldLines", h.Get("newLines"))
	x.Set("oldStart", h.Get("newStart"))
	x.Set("newLines", h.Get("oldLines"))
	x.Set("newStart", h.Get("oldStart"))
	source := h.Get("lines")
	lines := arr(source.Length())
	for i := 0; i < source.Length(); i++ {
		l := readText(source.Index(i))
		switch {
		case strings.HasPrefix(l, "-"):
			lines.SetIndex(i, writeText("+"+l[1:]))
		case strings.HasPrefix(l, "+"):
			lines.SetIndex(i, writeText("-"+l[1:]))
		default:
			lines.SetIndex(i, source.Index(i))
		}
	}
	x.Set("lines", lines)
	return x
}
func reverse(v *js.Object) *js.Object {
	git := v.Get("isGit").Bool()
	name := func(k string) *js.Object {
		if git {
			return swapPrefix(v.Get(k))
		}
		return v.Get(k)
	}
	o := object()
	for _, k := range js.Keys(v) {
		o.Set(k, v.Get(k))
	}
	o.Set("oldFileName", name("newFileName"))
	o.Set("oldHeader", v.Get("newHeader"))
	o.Set("newFileName", name("oldFileName"))
	o.Set("newHeader", v.Get("oldHeader"))
	o.Set("oldMode", v.Get("newMode"))
	o.Set("newMode", v.Get("oldMode"))
	o.Set("isCreate", v.Get("isDelete"))
	o.Set("isDelete", v.Get("isCreate"))
	source := v.Get("hunks")
	hunks := arr(source.Length())
	for i := 0; i < source.Length(); i++ {
		hunks.SetIndex(i, reverseHunk(source.Index(i)))
	}
	o.Set("hunks", hunks)
	if v.Get("isCopy").Bool() {
		// Reversing a copy deletes the copy destination.
		o.Set("newFileName", "/dev/null")
		o.Set("newHeader", js.Undefined)
		o.Set("isDelete", true)
		for _, k := range []string{"isCreate", "isCopy", "isRename"} {
			o.Delete(k)
		}
		o.Set("hunks", arr(0))
	}
	return o
}

func readPatch(v *js.Object) patch {
	p := patch{meta: v}
	all := v.Get("hunks")
	for i := 0; i < all.Length(); i++ {
		x := all.Index(i)
		h := hunk{oldStart: x.Get("oldStart").Float(), oldLines: x.Get("oldLines").Float(), newStart: x.Get("newStart").Float(), newLines: x.Get("newLines").Float()}
		a := x.Get("lines")
		for j := 0; j < a.Length(); j++ {
			h.lines = append(h.lines, readText(a.Index(j)))
		}
		p.hunks = append(p.hunks, h)
	}
	return p
}

// Line-ending conversion of apply.js and line-endings.js.
func onlyWinLineEndings(s string) bool {
	if !strings.Contains(s, "\r\n") || strings.HasPrefix(s, "\n") {
		return false
	}
	for i := 1; i < len(s); i++ {
		if s[i] == '\n' && s[i-1] != '\r' {
			return false
		}
	}
	return true
}
func onlyUnixLineEndings(s string) bool {
	return !strings.Contains(s, "\r\n") && strings.Contains(s, "\n")
}
func beforeMarker(lines []string, i int) bool {
	return i+1 < len(lines) && strings.HasPrefix(lines[i+1], "\\")
}
func isUnix(p patch) bool {
	for _, h := range p.hunks {
		for _, l := range h.lines {
			if !strings.HasPrefix(l, "\\") && strings.HasSuffix(l, "\r") {
				return false
			}
		}
	}
	return true
}
func isWin(p patch) bool {
	some := false
	for _, h := range p.hunks {
		for i, l := range h.lines {
			some = some || strings.HasSuffix(l, "\r")
			if !strings.HasPrefix(l, "\\") && !strings.HasSuffix(l, "\r") && !beforeMarker(h.lines, i) {
				return false
			}
		}
	}
	return some
}
func convertLines(p patch, f func(lines []string, i int) string) patch {
	q := patch{meta: p.meta, hunks: make([]hunk, len(p.hunks))}
	for k, h := range p.hunks {
		h.lines = make([]string, len(p.hunks[k].lines))
		for i := range h.lines {
			h.lines[i] = f(p.hunks[k].lines, i)
		}
		q.hunks[k] = h
	}
	return q
}
func unixToWin(p patch) patch {
	return convertLines(p, func(lines []string, i int) string {
		l := lines[i]
		if strings.HasPrefix(l, "\\") || strings.HasSuffix(l, "\r") || beforeMarker(lines, i) {
			return l
		}
		return l + "\r"
	})
}
func winToUnix(p patch) patch {
	return convertLines(p, func(lines []string, i int) string { return strings.TrimSuffix(lines[i], "\r") })
}

// distance is distanceIterator: the positions after start, alternating
// forward and backward by increasing distance, within [min, max]. Positions
// are JavaScript numbers, so a NaN start yields nothing, as in npm.
type distance struct {
	start, min, max                                 float64
	wantForward, backwardExhausted, forwardExhausted bool
	offset                                          float64
}

func (d *distance) next() (float64, bool) {
	if d.wantForward && !d.forwardExhausted {
		if d.backwardExhausted {
			d.offset++
		} else {
			d.wantForward = false
		}
		if d.start+d.offset <= d.max {
			return d.start + d.offset, true
		}
		d.forwardExhausted = true
	}
	if !d.backwardExhausted {
		if !d.forwardExhausted {
			d.wantForward = true
		}
		if d.min <= d.start-d.offset {
			position := d.start - d.offset
			d.offset++
			return position, true
		}
		d.backwardExhausted = true
		return d.next()
	}
	return 0, false
}

// sourceLine is lines[i], where a position outside the array, or NaN, is undefined.
func sourceLine(lines []string, i float64) (string, bool) {
	if i >= 0 && i < float64(len(lines)) {
		return lines[int(i)], true
	}
	return "", false
}

// fit is applyHunk with no errors allowed: every removed and context line
// equals the source line at its position. Trailing context is given back.
func fit(hunk []string, lines []string, toPos float64) ([]string, float64, bool) {
	var patched []string
	context := 0
	for _, l := range hunk {
		operation, content := byte(' '), ""
		if l != "" {
			operation, content = l[0], l[1:]
		}
		switch operation {
		case '-':
			if line, ok := sourceLine(lines, toPos); !ok || line != content {
				return nil, 0, false
			}
			toPos++
			context = 0
		case '+':
			patched = append(patched, content)
			context = 0
		case ' ':
			context++
			if line, ok := sourceLine(lines, toPos); !ok || line != content {
				return nil, 0, false
			}
			patched = append(patched, content)
			toPos++
		}
	}
	return patched[:len(patched)-context], toPos - float64(context) - 1, true
}

// copyLines appends the source lines from `from` while below `until`; a
// position outside the source reads as undefined, which joins as an empty line.
func copyLines(result []string, lines []string, from, until float64) []string {
	for i := from; i < until; i++ {
		line, _ := sourceLine(lines, i)
		result = append(result, line)
	}
	return result
}
func apply(source string, p patch) interface{} {
	if onlyWinLineEndings(source) && isUnix(p) {
		p = unixToWin(p)
	} else if onlyUnixLineEndings(source) && isWin(p) {
		p = winToUnix(p)
	}
	lines := strings.Split(source, "\n")
	if len(p.hunks) == 0 {
		return writeText(source)
	}
	removeEOFNL, addEOFNL := false, false
	previous := ""
	for _, l := range p.hunks[len(p.hunks)-1].lines {
		if strings.HasPrefix(l, "\\") {
			if strings.HasPrefix(previous, "+") {
				removeEOFNL = true
			} else if strings.HasPrefix(previous, "-") {
				addEOFNL = true
			}
		}
		previous = l
	}
	eof := lines[len(lines)-1] == ""
	switch {
	case removeEOFNL && addEOFNL:
		if eof {
			return false
		}
	case removeEOFNL:
		if !eof {
			return false
		}
		lines = lines[:len(lines)-1]
	case addEOFNL:
		if eof {
			return false
		}
		lines = append(lines, "")
	}
	// Positions are JavaScript numbers: a NaN start fits only a hunk with no
	// removed or context line, and then every later position is NaN too.
	var result []string
	minLine, offset := 0.0, 0.0
	for _, h := range p.hunks {
		// The declared position first, then the distance iterator around it.
		toPos := h.oldStart + offset - 1
		candidates := distance{start: toPos, min: minLine, max: float64(len(lines)) - h.oldLines, wantForward: true, offset: 1}
		patched, last, ok := fit(h.lines, lines, toPos)
		for !ok {
			if toPos, ok = candidates.next(); !ok {
				return false
			}
			patched, last, ok = fit(h.lines, lines, toPos)
		}
		result = append(copyLines(result, lines, minLine, toPos), patched...)
		minLine = last + 1
		offset = toPos + 1 - h.oldStart
	}
	return writeText(strings.Join(copyLines(result, lines, minLine, float64(len(lines))), "\n"))
}
